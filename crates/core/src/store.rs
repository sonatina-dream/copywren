//! SQLite persistence for items, payloads, thumbnails, pinboards and app
//! metadata (SPEC §2, §5.7, §6, §7).

use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::classify::Classified;
use crate::contents;
use crate::model::{ClipContents, Item, ItemId, ItemKind, Pinboard, PinboardId, SourceApp, Tab};
use crate::retention::Retention;

/// Number of colours in the UI's pinboard palette.
pub const PINBOARD_COLORS: u8 = 8;

const SCHEMA_VERSION: i64 = 1;

const ITEM_COLUMNS: &str = "id, kind, title, preview_text, checksum, created_at, timestamp, \
     source_bundle_id, source_app_name, pinboard_id, pin_order, char_count, image_width, \
     image_height, file_count";

pub struct Store {
    conn: Connection,
}

/// A classified capture, ready to be recorded.
pub struct NewClip<'a> {
    pub classified: &'a Classified,
    pub contents: &'a ClipContents,
    pub source: &'a SourceApp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    Inserted(ItemId),
    /// An identical history item existed and was moved to the front.
    Bumped(ItemId),
}

impl CaptureOutcome {
    pub fn id(self) -> ItemId {
        match self {
            CaptureOutcome::Inserted(id) | CaptureOutcome::Bumped(id) => id,
        }
    }
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).context("creating data directory")?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            bail!("database was created by a newer Copywren (schema {version})");
        }
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS pinboards (
                     id INTEGER PRIMARY KEY,
                     name TEXT NOT NULL,
                     color INTEGER NOT NULL,
                     position INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS items (
                     id INTEGER PRIMARY KEY,
                     kind INTEGER NOT NULL,
                     title TEXT,
                     preview_text TEXT NOT NULL,
                     checksum TEXT NOT NULL,
                     created_at INTEGER NOT NULL,
                     timestamp INTEGER NOT NULL,
                     source_bundle_id TEXT,
                     source_app_name TEXT,
                     pinboard_id INTEGER REFERENCES pinboards(id) ON DELETE CASCADE,
                     pin_order INTEGER NOT NULL DEFAULT 0,
                     char_count INTEGER NOT NULL DEFAULT 0,
                     image_width INTEGER,
                     image_height INTEGER,
                     file_count INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE INDEX IF NOT EXISTS items_by_tab ON items(pinboard_id, timestamp DESC);
                 CREATE INDEX IF NOT EXISTS items_by_checksum ON items(checksum);
                 CREATE TABLE IF NOT EXISTS item_data (
                     item_id INTEGER PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
                     contents BLOB NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS thumbnails (
                     item_id INTEGER PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
                     png BLOB NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS apps (
                     bundle_id TEXT PRIMARY KEY,
                     name TEXT,
                     icon_png BLOB
                 );",
            )?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(Self { conn })
    }

    // ---- capture -----------------------------------------------------------

    pub fn record_capture(&mut self, clip: NewClip<'_>, now: i64) -> Result<CaptureOutcome> {
        let tx = self.conn.transaction()?;
        let existing: Option<ItemId> = tx
            .query_row(
                "SELECT id FROM items WHERE pinboard_id IS NULL AND checksum = ?1
                 ORDER BY timestamp DESC LIMIT 1",
                [&clip.classified.checksum],
                |r| r.get(0),
            )
            .optional()?;
        let payload = contents::encode(clip.contents);
        let outcome = if let Some(id) = existing {
            tx.execute(
                "UPDATE items SET timestamp = ?2,
                     source_bundle_id = COALESCE(?3, source_bundle_id),
                     source_app_name = COALESCE(?4, source_app_name)
                 WHERE id = ?1",
                params![id, now, clip.source.bundle_id, clip.source.name],
            )?;
            tx.execute(
                "UPDATE item_data SET contents = ?2 WHERE item_id = ?1",
                params![id, payload],
            )?;
            CaptureOutcome::Bumped(id)
        } else {
            let c = clip.classified;
            tx.execute(
                "INSERT INTO items (kind, title, preview_text, checksum, created_at, timestamp,
                     source_bundle_id, source_app_name, pinboard_id, pin_order, char_count,
                     image_width, image_height, file_count)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?4, ?5, ?6, NULL, 0, ?7, ?8, ?9, ?10)",
                params![
                    c.kind.to_i64(),
                    c.preview_text,
                    c.checksum,
                    now,
                    clip.source.bundle_id,
                    clip.source.name,
                    c.char_count,
                    c.image_size.map(|s| s.0),
                    c.image_size.map(|s| s.1),
                    c.file_count,
                ],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO item_data (item_id, contents) VALUES (?1, ?2)",
                params![id, payload],
            )?;
            if let Some(png) = &c.thumbnail_png {
                tx.execute(
                    "INSERT INTO thumbnails (item_id, png) VALUES (?1, ?2)",
                    params![id, png],
                )?;
            }
            CaptureOutcome::Inserted(id)
        };
        tx.commit()?;
        Ok(outcome)
    }

    // ---- reading -----------------------------------------------------------

    pub fn items(&self, tab: Tab) -> Result<Vec<Item>> {
        let (sql, arg) = match tab {
            Tab::History => (
                format!(
                    "SELECT {ITEM_COLUMNS} FROM items WHERE pinboard_id IS NULL
                     ORDER BY timestamp DESC, id DESC"
                ),
                None,
            ),
            Tab::Pinboard(pinboard) => (
                format!(
                    "SELECT {ITEM_COLUMNS} FROM items WHERE pinboard_id = ?1
                     ORDER BY pin_order ASC, id DESC"
                ),
                Some(pinboard),
            ),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = match arg {
            Some(pinboard) => stmt.query_map([pinboard], row_to_item)?,
            None => stmt.query_map([], row_to_item)?,
        };
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn item(&self, id: ItemId) -> Result<Option<Item>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {ITEM_COLUMNS} FROM items WHERE id = ?1"),
                [id],
                row_to_item,
            )
            .optional()?)
    }

    pub fn contents(&self, id: ItemId) -> Result<ClipContents> {
        let blob: Vec<u8> = self
            .conn
            .query_row(
                "SELECT contents FROM item_data WHERE item_id = ?1",
                [id],
                |r| r.get(0),
            )
            .with_context(|| format!("no payload for item {id}"))?;
        contents::decode(&blob)
    }

    pub fn thumbnail(&self, id: ItemId) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT png FROM thumbnails WHERE item_id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn history_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items WHERE pinboard_id IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn known_app_names(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT source_app_name FROM items
             WHERE source_app_name IS NOT NULL ORDER BY source_app_name",
        )?;
        let names = stmt.query_map([], |r| r.get(0))?;
        names.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Source apps seen in stored items, most recently used first, as
    /// `(bundle_id, name)`.
    pub fn recent_source_apps(&self, limit: usize) -> Result<Vec<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_bundle_id, MAX(source_app_name) FROM items
             WHERE source_bundle_id IS NOT NULL
             GROUP BY source_bundle_id ORDER BY MAX(timestamp) DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ---- item actions ------------------------------------------------------

    /// Records that `id` was pasted: moves its history entry to the front,
    /// creating one when a pinboard item has no history twin (SPEC §4.1).
    /// Returns the id of the history item.
    pub fn mark_used(&mut self, id: ItemId, now: i64) -> Result<ItemId> {
        let tx = self.conn.transaction()?;
        let (pinboard, checksum): (Option<PinboardId>, String) = tx
            .query_row(
                "SELECT pinboard_id, checksum FROM items WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .with_context(|| format!("no item {id}"))?;
        let history_id = if pinboard.is_none() {
            id
        } else {
            let twin: Option<ItemId> = tx
                .query_row(
                    "SELECT id FROM items WHERE pinboard_id IS NULL AND checksum = ?1 LIMIT 1",
                    [&checksum],
                    |r| r.get(0),
                )
                .optional()?;
            match twin {
                Some(twin) => twin,
                None => copy_item(&tx, id, None, 0, now)?,
            }
        };
        tx.execute(
            "UPDATE items SET timestamp = ?2 WHERE id = ?1",
            params![history_id, now],
        )?;
        tx.commit()?;
        Ok(history_id)
    }

    pub fn delete_item(&mut self, id: ItemId) -> Result<()> {
        self.conn.execute("DELETE FROM items WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn rename_item(&mut self, id: ItemId, title: Option<&str>) -> Result<()> {
        let title = title.map(str::trim).filter(|t| !t.is_empty());
        self.conn.execute(
            "UPDATE items SET title = ?2 WHERE id = ?1",
            params![id, title],
        )?;
        Ok(())
    }

    /// Pins `id` to `pinboard` at the front. History items are copied (the
    /// history keeps its entry); items already on a pinboard move. Returns the
    /// pinned item's id. Pinning content the pinboard already holds returns
    /// the existing item instead of duplicating it.
    pub fn pin(&mut self, id: ItemId, pinboard: PinboardId, now: i64) -> Result<ItemId> {
        let tx = self.conn.transaction()?;
        let (current, checksum): (Option<PinboardId>, String) = tx
            .query_row(
                "SELECT pinboard_id, checksum FROM items WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .with_context(|| format!("no item {id}"))?;
        if current == Some(pinboard) {
            return Ok(id);
        }
        let duplicate: Option<ItemId> = tx
            .query_row(
                "SELECT id FROM items WHERE pinboard_id = ?1 AND checksum = ?2 LIMIT 1",
                params![pinboard, checksum],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = duplicate {
            if current.is_some() {
                tx.execute("DELETE FROM items WHERE id = ?1", [id])?;
            }
            tx.commit()?;
            return Ok(existing);
        }
        let order: i64 = tx.query_row(
            "SELECT COALESCE(MIN(pin_order), 1) - 1 FROM items WHERE pinboard_id = ?1",
            [pinboard],
            |r| r.get(0),
        )?;
        let pinned = if current.is_some() {
            tx.execute(
                "UPDATE items SET pinboard_id = ?2, pin_order = ?3 WHERE id = ?1",
                params![id, pinboard, order],
            )?;
            id
        } else {
            copy_item(&tx, id, Some(pinboard), order, now)?
        };
        tx.commit()?;
        Ok(pinned)
    }

    /// Moves a pinboard item to `new_index` within its pinboard.
    pub fn move_in_pinboard(&mut self, id: ItemId, new_index: usize) -> Result<()> {
        let tx = self.conn.transaction()?;
        let pinboard: Option<PinboardId> = tx
            .query_row("SELECT pinboard_id FROM items WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .with_context(|| format!("no item {id}"))?;
        let Some(pinboard) = pinboard else {
            bail!("item {id} is not pinned");
        };
        let mut ids: Vec<ItemId> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM items WHERE pinboard_id = ?1 ORDER BY pin_order ASC, id DESC",
            )?;
            let rows = stmt.query_map([pinboard], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        ids.retain(|&other| other != id);
        ids.insert(new_index.min(ids.len()), id);
        for (order, item) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE items SET pin_order = ?2 WHERE id = ?1",
                params![item, order as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- pinboards ---------------------------------------------------------

    pub fn pinboards(&self) -> Result<Vec<Pinboard>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, color, position FROM pinboards ORDER BY position, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Pinboard {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get::<_, i64>(2)? as u8,
                position: r.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn create_pinboard(&mut self, name: &str) -> Result<Pinboard> {
        let name = clean_name(name);
        let (count, position): (i64, i64) = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(MAX(position), -1) + 1 FROM pinboards",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let color = (count % i64::from(PINBOARD_COLORS)) as u8;
        self.conn.execute(
            "INSERT INTO pinboards (name, color, position) VALUES (?1, ?2, ?3)",
            params![name, color, position],
        )?;
        Ok(Pinboard {
            id: self.conn.last_insert_rowid(),
            name,
            color,
            position,
        })
    }

    pub fn rename_pinboard(&mut self, id: PinboardId, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE pinboards SET name = ?2 WHERE id = ?1",
            params![id, clean_name(name)],
        )?;
        Ok(())
    }

    pub fn set_pinboard_color(&mut self, id: PinboardId, color: u8) -> Result<()> {
        self.conn.execute(
            "UPDATE pinboards SET color = ?2 WHERE id = ?1",
            params![id, color % PINBOARD_COLORS],
        )?;
        Ok(())
    }

    /// Deletes the pinboard and every item on it.
    pub fn delete_pinboard(&mut self, id: PinboardId) -> Result<()> {
        self.conn
            .execute("DELETE FROM pinboards WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- retention ---------------------------------------------------------

    /// Deletes every history item; pinboards are kept.
    pub fn erase_history(&mut self) -> Result<usize> {
        Ok(self
            .conn
            .execute("DELETE FROM items WHERE pinboard_id IS NULL", [])?)
    }

    pub fn prune_history(&mut self, retention: Retention, now: i64) -> Result<usize> {
        let Some(cutoff) = retention.cutoff(now) else {
            return Ok(0);
        };
        Ok(self.conn.execute(
            "DELETE FROM items WHERE pinboard_id IS NULL AND timestamp < ?1",
            [cutoff],
        )?)
    }

    pub fn count_history_older_than(&self, cutoff: i64) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM items WHERE pinboard_id IS NULL AND timestamp < ?1",
            [cutoff],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    // ---- apps --------------------------------------------------------------

    pub fn upsert_app(
        &mut self,
        bundle_id: &str,
        name: Option<&str>,
        icon_png: Option<&[u8]>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO apps (bundle_id, name, icon_png) VALUES (?1, ?2, ?3)
             ON CONFLICT(bundle_id) DO UPDATE SET
                 name = COALESCE(excluded.name, name),
                 icon_png = COALESCE(excluded.icon_png, icon_png)",
            params![bundle_id, name, icon_png],
        )?;
        Ok(())
    }

    pub fn app_icon(&self, bundle_id: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT icon_png FROM apps WHERE bundle_id = ?1",
                [bundle_id],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten())
    }
}

fn copy_item(
    tx: &Transaction<'_>,
    id: ItemId,
    pinboard: Option<PinboardId>,
    pin_order: i64,
    now: i64,
) -> Result<ItemId> {
    tx.execute(
        "INSERT INTO items (kind, title, preview_text, checksum, created_at, timestamp,
             source_bundle_id, source_app_name, pinboard_id, pin_order, char_count,
             image_width, image_height, file_count)
         SELECT kind, title, preview_text, checksum, created_at, ?2, source_bundle_id,
             source_app_name, ?3, ?4, char_count, image_width, image_height, file_count
         FROM items WHERE id = ?1",
        params![id, now, pinboard, pin_order],
    )?;
    let new_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO item_data (item_id, contents) SELECT ?2, contents FROM item_data WHERE item_id = ?1",
        params![id, new_id],
    )?;
    tx.execute(
        "INSERT INTO thumbnails (item_id, png) SELECT ?2, png FROM thumbnails WHERE item_id = ?1",
        params![id, new_id],
    )?;
    Ok(new_id)
}

fn clean_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        "Untitled".into()
    } else {
        name.chars().take(64).collect()
    }
}

fn row_to_item(row: &Row<'_>) -> rusqlite::Result<Item> {
    let kind: i64 = row.get(1)?;
    let width: Option<u32> = row.get(12)?;
    let height: Option<u32> = row.get(13)?;
    Ok(Item {
        id: row.get(0)?,
        kind: ItemKind::from_i64(kind).unwrap_or(ItemKind::Text),
        title: row.get(2)?,
        preview_text: row.get(3)?,
        checksum: row.get(4)?,
        created_at: row.get(5)?,
        timestamp: row.get(6)?,
        source_bundle_id: row.get(7)?,
        source_app_name: row.get(8)?,
        pinboard_id: row.get(9)?,
        pin_order: row.get(10)?,
        char_count: row.get(11)?,
        image_size: width.zip(height),
        file_count: row.get(14)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::classify;
    use crate::model::{PasteboardEntry, Representation};

    const T0: i64 = 1_700_000_000_000;
    const DAY: i64 = 24 * 60 * 60 * 1000;

    fn mail() -> SourceApp {
        SourceApp {
            bundle_id: Some("com.apple.mail".into()),
            name: Some("Mail".into()),
        }
    }

    fn capture(store: &mut Store, text: &str, now: i64) -> CaptureOutcome {
        let contents = ClipContents::plain_text(text);
        let classified = classify(&contents).unwrap();
        store
            .record_capture(
                NewClip {
                    classified: &classified,
                    contents: &contents,
                    source: &mail(),
                },
                now,
            )
            .unwrap()
    }

    fn texts(store: &Store, tab: Tab) -> Vec<String> {
        store
            .items(tab)
            .unwrap()
            .into_iter()
            .map(|i| i.preview_text)
            .collect()
    }

    #[test]
    fn captures_newest_first_and_dedupes() {
        let mut store = Store::open_in_memory().unwrap();
        let a = capture(&mut store, "alpha", T0);
        capture(&mut store, "beta", T0 + 1);
        assert!(matches!(a, CaptureOutcome::Inserted(_)));
        assert_eq!(texts(&store, Tab::History), ["beta", "alpha"]);

        let again = capture(&mut store, "alpha", T0 + 2);
        assert_eq!(again, CaptureOutcome::Bumped(a.id()));
        assert_eq!(texts(&store, Tab::History), ["alpha", "beta"]);
        assert_eq!(store.history_count().unwrap(), 2);

        let item = store.item(a.id()).unwrap().unwrap();
        assert_eq!(item.source_app_name.as_deref(), Some("Mail"));
        assert_eq!(item.created_at, T0);
        assert_eq!(item.timestamp, T0 + 2);
    }

    #[test]
    fn stores_full_payload_and_thumbnail() {
        let mut store = Store::open_in_memory().unwrap();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(8, 4, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let contents = ClipContents::new(vec![PasteboardEntry::new(vec![
            Representation::new(crate::uti::PNG, png),
            Representation::new("com.example.extra", "meta"),
        ])]);
        let classified = classify(&contents).unwrap();
        let id = store
            .record_capture(
                NewClip {
                    classified: &classified,
                    contents: &contents,
                    source: &SourceApp::default(),
                },
                T0,
            )
            .unwrap()
            .id();
        assert_eq!(store.contents(id).unwrap(), contents);
        assert!(store.thumbnail(id).unwrap().is_some());
        let item = store.item(id).unwrap().unwrap();
        assert_eq!(item.kind, ItemKind::Image);
        assert_eq!(item.image_size, Some((8, 4)));
        assert!(item.source_app_name.is_none());
    }

    #[test]
    fn pinning_copies_from_history_and_moves_between_pinboards() {
        let mut store = Store::open_in_memory().unwrap();
        let work = store.create_pinboard("Work").unwrap();
        let home = store.create_pinboard("  ").unwrap();
        assert_eq!(home.name, "Untitled");
        assert_ne!(work.color, home.color);

        let a = capture(&mut store, "alpha", T0).id();
        let b = capture(&mut store, "beta", T0 + 1).id();
        let pa = store.pin(a, work.id, T0 + 2).unwrap();
        let pb = store.pin(b, work.id, T0 + 3).unwrap();
        assert_ne!(pa, a);
        assert_eq!(texts(&store, Tab::Pinboard(work.id)), ["beta", "alpha"]);
        assert_eq!(texts(&store, Tab::History), ["beta", "alpha"]);
        assert_eq!(store.contents(pa).unwrap(), store.contents(a).unwrap());

        // Pinning the same content again returns the existing pinned item.
        assert_eq!(store.pin(a, work.id, T0 + 4).unwrap(), pa);
        assert_eq!(store.items(Tab::Pinboard(work.id)).unwrap().len(), 2);

        // Pinboard → pinboard moves.
        assert_eq!(store.pin(pb, home.id, T0 + 5).unwrap(), pb);
        assert_eq!(texts(&store, Tab::Pinboard(work.id)), ["alpha"]);
        assert_eq!(texts(&store, Tab::Pinboard(home.id)), ["beta"]);
        // Pinning into the pinboard it's already on is a no-op.
        assert_eq!(store.pin(pb, home.id, T0 + 6).unwrap(), pb);
    }

    #[test]
    fn reorder_within_pinboard() {
        let mut store = Store::open_in_memory().unwrap();
        let board = store.create_pinboard("Snippets").unwrap();
        for (i, t) in ["a", "b", "c"].iter().enumerate() {
            let id = capture(&mut store, t, T0 + i as i64).id();
            store.pin(id, board.id, T0).unwrap();
        }
        assert_eq!(texts(&store, Tab::Pinboard(board.id)), ["c", "b", "a"]);
        let c = store.items(Tab::Pinboard(board.id)).unwrap()[0].id;
        store.move_in_pinboard(c, 99).unwrap();
        assert_eq!(texts(&store, Tab::Pinboard(board.id)), ["b", "a", "c"]);
        store.move_in_pinboard(c, 1).unwrap();
        assert_eq!(texts(&store, Tab::Pinboard(board.id)), ["b", "c", "a"]);
        let history_id = store.items(Tab::History).unwrap()[0].id;
        assert!(store.move_in_pinboard(history_id, 0).is_err());
    }

    #[test]
    fn mark_used_bumps_history() {
        let mut store = Store::open_in_memory().unwrap();
        let board = store.create_pinboard("Work").unwrap();
        let a = capture(&mut store, "alpha", T0).id();
        capture(&mut store, "beta", T0 + 1);
        assert_eq!(store.mark_used(a, T0 + 2).unwrap(), a);
        assert_eq!(texts(&store, Tab::History), ["alpha", "beta"]);

        let pinned = store.pin(a, board.id, T0 + 3).unwrap();
        capture(&mut store, "gamma", T0 + 4);
        // Twin exists: bumped, not duplicated.
        assert_eq!(store.mark_used(pinned, T0 + 5).unwrap(), a);
        assert_eq!(texts(&store, Tab::History), ["alpha", "gamma", "beta"]);

        // No twin: a history copy is created.
        store.erase_history().unwrap();
        let fresh = store.mark_used(pinned, T0 + 6).unwrap();
        assert_ne!(fresh, pinned);
        assert_eq!(texts(&store, Tab::History), ["alpha"]);
        assert_eq!(texts(&store, Tab::Pinboard(board.id)), ["alpha"]);
    }

    #[test]
    fn erase_and_prune_keep_pinboards() {
        let mut store = Store::open_in_memory().unwrap();
        let board = store.create_pinboard("Keep").unwrap();
        let old = capture(&mut store, "old", T0 - 40 * DAY).id();
        capture(&mut store, "recent", T0 - DAY);
        store.pin(old, board.id, T0).unwrap();

        assert_eq!(store.count_history_older_than(T0 - 30 * DAY).unwrap(), 1);
        assert_eq!(store.prune_history(Retention::Forever, T0).unwrap(), 0);
        assert_eq!(store.prune_history(Retention::Month, T0).unwrap(), 1);
        assert_eq!(texts(&store, Tab::History), ["recent"]);
        assert_eq!(texts(&store, Tab::Pinboard(board.id)), ["old"]);

        assert_eq!(store.erase_history().unwrap(), 1);
        assert!(texts(&store, Tab::History).is_empty());
        assert_eq!(texts(&store, Tab::Pinboard(board.id)), ["old"]);
    }

    #[test]
    fn deleting_cascades() {
        let mut store = Store::open_in_memory().unwrap();
        let board = store.create_pinboard("Temp").unwrap();
        let a = capture(&mut store, "alpha", T0).id();
        let pinned = store.pin(a, board.id, T0).unwrap();
        store.delete_pinboard(board.id).unwrap();
        assert!(store.item(pinned).unwrap().is_none());
        assert!(store.contents(pinned).is_err());
        assert!(store.pinboards().unwrap().is_empty());
        store.delete_item(a).unwrap();
        assert!(store.item(a).unwrap().is_none());
        assert!(store.contents(a).is_err());
    }

    #[test]
    fn renames_and_apps() {
        let mut store = Store::open_in_memory().unwrap();
        let a = capture(&mut store, "alpha", T0).id();
        store.rename_item(a, Some("  Greeting ")).unwrap();
        assert_eq!(
            store.item(a).unwrap().unwrap().title.as_deref(),
            Some("Greeting")
        );
        store.rename_item(a, Some("   ")).unwrap();
        assert_eq!(store.item(a).unwrap().unwrap().title, None);

        let board = store.create_pinboard("A").unwrap();
        store.rename_pinboard(board.id, "Renamed").unwrap();
        store.set_pinboard_color(board.id, 11).unwrap();
        let boards = store.pinboards().unwrap();
        assert_eq!(boards[0].name, "Renamed");
        assert_eq!(boards[0].color, 11 % PINBOARD_COLORS);

        assert_eq!(store.known_app_names().unwrap(), ["Mail"]);
        assert_eq!(
            store.recent_source_apps(5).unwrap(),
            [("com.apple.mail".to_string(), Some("Mail".to_string()))]
        );
        store
            .upsert_app("com.apple.mail", Some("Mail"), Some(b"png"))
            .unwrap();
        store.upsert_app("com.apple.mail", None, None).unwrap();
        assert_eq!(
            store.app_icon("com.apple.mail").unwrap().as_deref(),
            Some(&b"png"[..])
        );
        assert_eq!(store.app_icon("missing").unwrap(), None);
    }

    #[test]
    fn reopens_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/copywren.sqlite");
        {
            let mut store = Store::open(&path).unwrap();
            capture(&mut store, "persisted", T0);
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(texts(&store, Tab::History), ["persisted"]);
    }
}
