//! The application controller: owns the store and settings, captures the
//! clipboard, performs pastes and manages windows (SPEC §3–§9).
//!
//! Rule for this module: never touch an `NSWindow` or open a GPUI window
//! inside an `AppState` update. AppKit calls back into GPUI synchronously
//! from those operations, and GPUI's draw reads `AppState`, so both happen in
//! spawned tasks that run after the update has finished.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use copywren_core::classify::{self, Classified};
use copywren_core::model::{ClipContents, ItemId, ItemKind, Pinboard, PinboardId, SourceApp};
use copywren_core::pause::{PauseDuration, PauseState};
use copywren_core::privacy;
use copywren_core::queue::{PasteQueue, Snippet};
use copywren_core::settings::Settings;
use copywren_core::store::{NewClip, Store};
use copywren_core::{appcolor, now_ms, placement, timefmt};
use copywren_macos::ScreenRect;
use copywren_macos::status_item::{KeyMask, MenuEntry, StatusItem};
use copywren_macos::window::NativeWindow;
use copywren_macos::workspace::RunningApp;
use copywren_macos::{input, pasteboard, system, workspace};
use gpui::{
    App, AppContext, Bounds, Context, Entity, Global, Image, ImageFormat,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, point, px,
    size,
};

use crate::hotkeys::{HotkeyAction, Hotkeys, paste_shortcut};
use crate::panel::Panel;
use crate::queue_view::QueueView;
use crate::settings_view::SettingsView;
use crate::theme;
use crate::util::native_window;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Ticks between retention sweeps (10 minutes).
const PRUNE_EVERY_TICKS: u64 = 1_200;
/// Wait between hiding the panel and posting ⌘V.
const PASTE_DELAY: Duration = Duration::from_millis(80);
/// How long the ⌘V intercept stays off while a queue paste is delivered.
const QUEUE_REARM_DELAY: Duration = Duration::from_millis(250);

const QUEUE_SIZE: (f64, f64) = (300.0, 380.0);

/// Menu bar template image (rendered from assets/menubar.svg).
const MENU_BAR_ICON: &[u8] = include_bytes!("../../../assets/menubar.png");

mod menu_tag {
    pub const OPEN: i64 = 1;
    pub const QUEUE: i64 = 2;
    pub const PAUSE_BASE: i64 = 10;
    pub const RESUME: i64 = 20;
    pub const SETTINGS: i64 = 30;
    pub const ERASE: i64 = 31;
    pub const QUIT: i64 = 99;
}

pub struct GlobalState(pub Entity<AppState>);
impl Global for GlobalState {}

pub struct AppState {
    pub settings: Settings,
    settings_path: PathBuf,
    pub data_dir: PathBuf,
    pub store: Store,
    pub pause: PauseState,
    pub queue: PasteQueue,
    pub queue_open: bool,
    /// Bumped on every data change; views reload when it moves.
    pub revision: u64,
    pub panel_visible: bool,
    pub hotkey_error: Option<String>,
    pub panel: Option<WindowHandle<Panel>>,
    pub panel_native: Option<NativeWindow>,
    /// Set while `--features demo` records the documentation video: the
    /// popup opens in a fixed spot without taking focus and stays open.
    pub demo: bool,
    last_change_count: i64,
    self_change_count: i64,
    paste_target: Option<RunningApp>,
    accessibility_prompted: bool,
    ticks: u64,
    hotkeys: Option<Hotkeys>,
    status_item: Option<StatusItem>,
    queue_window: Option<(WindowHandle<QueueView>, NativeWindow)>,
    queue_window_opening: bool,
    settings_window: Option<WindowHandle<SettingsView>>,
    icons: HashMap<String, Option<Arc<Image>>>,
    colors: HashMap<String, u32>,
}

impl AppState {
    /// `~/Library/Application Support/Copywren`, or `$COPYWREN_DATA_DIR` so test
    /// runs and side-by-side builds can keep their own data.
    pub fn default_data_dir() -> Result<PathBuf> {
        match std::env::var_os("COPYWREN_DATA_DIR") {
            Some(dir) => Ok(PathBuf::from(dir)),
            None => Ok(dirs::data_dir()
                .context("no Application Support directory")?
                .join("Copywren")),
        }
    }

    pub fn open(data_dir: PathBuf) -> Result<Self> {
        let settings_path = data_dir.join("settings.json");
        let settings = Settings::load(&settings_path);
        let mut store = Store::open(&data_dir.join("copywren.sqlite"))?;
        store.prune_history(settings.keep_history, now_ms())?;
        let mut queue = PasteQueue::default();
        queue.direction = settings.paste_queue_direction;
        Ok(Self {
            settings,
            settings_path,
            data_dir,
            store,
            pause: PauseState::Running,
            queue,
            queue_open: false,
            revision: 0,
            panel_visible: false,
            hotkey_error: None,
            panel: None,
            panel_native: None,
            demo: false,
            last_change_count: pasteboard::change_count(),
            self_change_count: -1,
            paste_target: None,
            accessibility_prompted: false,
            ticks: 0,
            hotkeys: None,
            status_item: None,
            queue_window: None,
            queue_window_opening: false,
            settings_window: None,
            icons: HashMap::new(),
            colors: HashMap::new(),
        })
    }

    pub fn global(cx: &App) -> Entity<AppState> {
        cx.global::<GlobalState>().0.clone()
    }

    /// Hotkeys, menu bar item and the capture loop.
    /// Loads icons and colours for every app seen in the history.
    pub fn warm_app_meta(&mut self) {
        for (bundle, _) in self.store.recent_source_apps(500).unwrap_or_default() {
            self.ensure_app_meta(&bundle);
        }
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        self.warm_app_meta();

        let (hotkey_tx, hotkey_rx) = smol::channel::unbounded::<u32>();
        match Hotkeys::new(hotkey_tx) {
            Ok(hotkeys) => self.hotkeys = Some(hotkeys),
            Err(e) => self.hotkey_error = Some(e.to_string()),
        }
        self.register_hotkeys();
        cx.spawn(async move |this, cx| {
            while let Ok(id) = hotkey_rx.recv().await {
                if this
                    .update(cx, |state, cx| state.on_hotkey(id, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (menu_tx, menu_rx) = smol::channel::unbounded::<i64>();
        let status_item = StatusItem::new("doc.on.clipboard", move |tag| {
            let _ = menu_tx.try_send(tag);
        });
        status_item.set_template_png(MENU_BAR_ICON, 18.0, "Copywren");
        self.status_item = Some(status_item);
        self.refresh_status_item();
        cx.spawn(async move |this, cx| {
            while let Ok(tag) = menu_rx.recv().await {
                if this.update(cx, |state, cx| state.on_menu(tag, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                if this.update(cx, |state, cx| state.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    // ---- hotkeys and menu --------------------------------------------------

    fn register_hotkeys(&mut self) {
        let Some(hotkeys) = self.hotkeys.as_mut() else {
            return;
        };
        let mut errors = Vec::new();
        for (action, shortcut) in [
            (
                HotkeyAction::TogglePanel,
                &self.settings.activation_shortcut,
            ),
            (
                HotkeyAction::TogglePasteQueue,
                &self.settings.paste_queue_shortcut,
            ),
        ] {
            if let Err(e) = hotkeys.register(action, shortcut) {
                errors.push(e.to_string());
            }
        }
        self.hotkey_error = (!errors.is_empty()).then(|| errors.join("\n"));
        self.sync_queue_hotkey();
    }

    /// ⌘V is intercepted only while the open queue has something to paste.
    fn sync_queue_hotkey(&mut self) {
        let wanted = self.queue_open && !self.queue.is_empty();
        let Some(hotkeys) = self.hotkeys.as_mut() else {
            return;
        };
        let active = hotkeys.is_registered(HotkeyAction::QueuePaste);
        if wanted && !active {
            if let Err(e) = hotkeys.register(HotkeyAction::QueuePaste, &paste_shortcut()) {
                log::warn!("Paste Queue can't intercept ⌘V: {e}");
            }
        } else if !wanted && active {
            hotkeys.unregister(HotkeyAction::QueuePaste);
        }
    }

    fn on_hotkey(&mut self, id: u32, cx: &mut Context<Self>) {
        let Some(action) = self.hotkeys.as_ref().and_then(|h| h.action_for(id)) else {
            return;
        };
        match action {
            HotkeyAction::TogglePanel => self.toggle_panel(cx),
            HotkeyAction::TogglePasteQueue => self.toggle_queue(cx),
            HotkeyAction::QueuePaste => self.queue_paste_next(cx),
        }
    }

    fn on_menu(&mut self, tag: i64, cx: &mut Context<Self>) {
        match tag {
            menu_tag::OPEN => self.show_panel(cx),
            menu_tag::QUEUE => self.toggle_queue(cx),
            menu_tag::RESUME => self.resume(cx),
            menu_tag::SETTINGS => self.open_settings(false, cx),
            menu_tag::ERASE => self.open_settings(true, cx),
            menu_tag::QUIT => cx.quit(),
            tag if (menu_tag::PAUSE_BASE..menu_tag::PAUSE_BASE + 4).contains(&tag) => {
                let duration = PauseDuration::ALL[(tag - menu_tag::PAUSE_BASE) as usize];
                self.pause(duration, cx);
            }
            _ => {}
        }
    }

    fn refresh_status_item(&mut self) {
        let Some(item) = self.status_item.as_ref() else {
            return;
        };
        item.set_visible(self.settings.show_in_menu_bar);
        let paused = self.pause.is_paused(now_ms());
        item.set_dimmed(paused);
        let shortcut = &self.settings.activation_shortcut;
        let queue_shortcut = &self.settings.paste_queue_shortcut;
        let mask = |s: &copywren_core::shortcut::Shortcut| KeyMask {
            cmd: s.cmd,
            shift: s.shift,
            alt: s.alt,
            ctrl: s.ctrl,
        };
        let mut entries = vec![
            MenuEntry::Item {
                title: "Open Copywren".into(),
                tag: menu_tag::OPEN,
                enabled: true,
                key_equivalent: (shortcut.key.chars().count() == 1)
                    .then(|| (shortcut.key.clone(), mask(shortcut))),
            },
            MenuEntry::Item {
                title: if self.queue_open {
                    "Close Paste Queue"
                } else {
                    "Paste Queue"
                }
                .into(),
                tag: menu_tag::QUEUE,
                enabled: true,
                key_equivalent: (queue_shortcut.key.chars().count() == 1)
                    .then(|| (queue_shortcut.key.clone(), mask(queue_shortcut))),
            },
            MenuEntry::Separator,
        ];
        if paused {
            let label = match self.pause.resume_at() {
                Some(at) => format!("Paused until {}", timefmt::clock(at)),
                None => "Paused".into(),
            };
            entries.push(MenuEntry::Item {
                title: label,
                tag: 0,
                enabled: false,
                key_equivalent: None,
            });
            entries.push(MenuEntry::item("Resume", menu_tag::RESUME));
        } else {
            entries.push(MenuEntry::Submenu {
                title: "Pause".into(),
                entries: PauseDuration::ALL
                    .iter()
                    .enumerate()
                    .map(|(i, d)| MenuEntry::item(d.label(), menu_tag::PAUSE_BASE + i as i64))
                    .collect(),
            });
        }
        entries.extend([
            MenuEntry::Separator,
            MenuEntry::Item {
                title: "Settings…".into(),
                tag: menu_tag::SETTINGS,
                enabled: true,
                key_equivalent: Some((
                    ",".into(),
                    KeyMask {
                        cmd: true,
                        ..KeyMask::default()
                    },
                )),
            },
            MenuEntry::item("Erase History…", menu_tag::ERASE),
            MenuEntry::Separator,
            MenuEntry::Item {
                title: "Quit Copywren".into(),
                tag: menu_tag::QUIT,
                enabled: true,
                key_equivalent: Some((
                    "q".into(),
                    KeyMask {
                        cmd: true,
                        ..KeyMask::default()
                    },
                )),
            },
        ]);
        item.set_menu(&entries);
    }

    // ---- capture -----------------------------------------------------------

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        if self.pause.expire(now) {
            self.refresh_status_item();
            cx.notify();
        }
        self.ticks += 1;
        if self.ticks.is_multiple_of(PRUNE_EVERY_TICKS)
            && let Ok(removed) = self.store.prune_history(self.settings.keep_history, now)
            && removed > 0
        {
            self.bump(cx);
        }
        self.poll_clipboard(cx);
    }

    fn poll_clipboard(&mut self, cx: &mut Context<Self>) {
        let count = pasteboard::change_count();
        if count == self.last_change_count {
            return;
        }
        self.last_change_count = count;
        if count == self.self_change_count || self.pause.is_paused(now_ms()) {
            return;
        }
        let filter = self.settings.privacy_filter();
        // Check the type markers first, so secrets are never even read.
        if let Err(reason) = filter.check(&pasteboard::read_types(), None) {
            log::debug!("skipped clipboard change: {reason:?}");
            return;
        }
        let contents = privacy::sanitize(pasteboard::read());
        let front = workspace::frontmost_app();
        let declared = privacy::source_override(&contents);
        let bundle_id = declared
            .clone()
            .or_else(|| front.as_ref().and_then(|f| f.bundle_id.clone()));
        if let Err(reason) = filter.check(&contents, bundle_id.as_deref()) {
            log::debug!("skipped clipboard change: {reason:?}");
            return;
        }
        let name = match (&declared, &front) {
            (Some(declared), Some(front)) if front.bundle_id.as_deref() != Some(declared) => {
                workspace::app_name(declared)
            }
            (Some(declared), None) => workspace::app_name(declared),
            _ => front.and_then(|f| f.name),
        };
        let source = SourceApp { bundle_id, name };

        let classify_task = cx
            .background_executor()
            .spawn(async move { classify::classify(&contents).map(|c| (c, contents)) });
        cx.spawn(async move |this, cx| {
            if let Some((classified, contents)) = classify_task.await {
                this.update(cx, |state, cx| {
                    state.record(classified, contents, source, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    fn record(
        &mut self,
        classified: Classified,
        contents: ClipContents,
        source: SourceApp,
        cx: &mut Context<Self>,
    ) {
        let now = now_ms();
        let clip = NewClip {
            classified: &classified,
            contents: &contents,
            source: &source,
        };
        if let Err(e) = self.store.record_capture(clip, now) {
            log::error!("could not save clip: {e:#}");
            return;
        }
        if let Some(bundle) = &source.bundle_id {
            self.ensure_app_meta(bundle);
        }
        let _ = self.store.prune_history(self.settings.keep_history, now);
        if self.queue_open {
            self.queue.push(Snippet {
                kind: classified.kind,
                preview_text: classified.preview_text.clone(),
                contents,
            });
            self.sync_queue_hotkey();
        }
        if self.settings.sound_effects {
            system::play_sound("Pop");
        }
        self.bump(cx);
    }

    fn bump(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        cx.notify();
    }

    /// Caches the icon and header colour for `bundle`.
    pub fn ensure_app_meta(&mut self, bundle: &str) {
        if self.icons.contains_key(bundle) {
            return;
        }
        let png = self.store.app_icon(bundle).ok().flatten().or_else(|| {
            let png = workspace::app_icon_png(bundle, 64)?;
            let _ = self.store.upsert_app(bundle, None, Some(&png));
            Some(png)
        });
        let color = png
            .as_deref()
            .and_then(appcolor::dominant_color)
            .unwrap_or_else(|| appcolor::fallback_color(bundle));
        self.colors.insert(bundle.to_string(), color);
        self.icons.insert(
            bundle.to_string(),
            png.map(|p| Arc::new(Image::from_bytes(ImageFormat::Png, p))),
        );
    }

    pub fn app_icon(&self, bundle: Option<&str>) -> Option<Arc<Image>> {
        self.icons.get(bundle?).cloned().flatten()
    }

    pub fn app_color(&self, bundle: Option<&str>) -> u32 {
        bundle
            .and_then(|b| self.colors.get(b).copied())
            .unwrap_or(0x636366)
    }

    // ---- panel -------------------------------------------------------------

    pub fn toggle_panel(&mut self, cx: &mut Context<Self>) {
        if self.panel_visible {
            self.hide_panel(cx);
        } else {
            self.show_panel(cx);
        }
    }

    pub fn show_panel(&mut self, cx: &mut Context<Self>) {
        let (Some(native), Some(panel)) = (self.panel_native, self.panel) else {
            return;
        };
        let own_pid = std::process::id() as i32;
        self.paste_target = workspace::frontmost_app().filter(|app| app.pid != own_pid);
        // Ask for the caret before the popup takes keyboard focus.
        let frame = if self.demo {
            demo_frame()
        } else {
            popup_frame_near_caret()
        };
        let Some(frame) = frame else {
            return;
        };
        let shareable = self.settings.show_during_screen_sharing;
        let make_key = !self.demo;
        self.panel_visible = true;
        cx.notify();
        cx.spawn(async move |_, cx| {
            native.set_visible_in_screen_capture(shareable);
            native.show(frame, make_key);
            panel
                .update(cx, |panel, window, cx| panel.did_show(window, cx))
                .ok();
            debug_snapshot(native, "panel", cx).await;
        })
        .detach();
    }

    pub fn hide_panel(&mut self, cx: &mut Context<Self>) {
        if !self.panel_visible {
            return;
        }
        self.panel_visible = false;
        cx.notify();
        let (native, panel) = (self.panel_native, self.panel);
        cx.spawn(async move |_, cx| {
            if let Some(native) = native {
                native.hide();
            }
            if let Some(panel) = panel {
                panel
                    .update(cx, |panel, window, cx| panel.did_hide(window, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Current popup frame, for dragging it by its header.
    pub fn panel_frame(&self) -> Option<ScreenRect> {
        self.panel_native.map(|native| native.frame())
    }

    /// Moves the visible popup so its bottom-left corner is at `(x, y)`.
    pub fn move_panel(&mut self, x: f64, y: f64, cx: &mut Context<Self>) {
        let Some(native) = self.panel_native.filter(|_| self.panel_visible) else {
            return;
        };
        cx.spawn(async move |_, _| {
            let mut frame = native.frame();
            frame.x = x;
            frame.y = y;
            native.set_frame(frame);
        })
        .detach();
    }

    // ---- item actions ------------------------------------------------------

    /// Puts the item on the clipboard, hides the panel and, when direct paste
    /// is on and allowed, sends ⌘V to the app that was frontmost (SPEC §4.1).
    pub fn paste_item(&mut self, id: ItemId, plain: bool, direct: bool, cx: &mut Context<Self>) {
        let contents = match self.store.contents(id) {
            Ok(contents) => contents,
            Err(e) => {
                log::error!("could not load item {id}: {e:#}");
                return;
            }
        };
        self.write_to_pasteboard(&contents, plain || self.settings.always_plain_text);
        let _ = self.store.mark_used(id, now_ms());
        if self.settings.sound_effects {
            system::play_sound("Tink");
        }
        self.bump(cx);
        self.hide_panel(cx);

        if !(direct && self.settings.direct_paste) {
            return;
        }
        if !input::is_trusted(false) {
            if !self.accessibility_prompted {
                self.accessibility_prompted = true;
                input::is_trusted(true);
            }
            return;
        }
        let target = self.paste_target.clone();
        cx.spawn(async move |_, cx| {
            if let Some(target) = target {
                let front = workspace::frontmost_app().map(|app| app.pid);
                if front != Some(target.pid) {
                    workspace::activate(target.pid);
                }
            }
            cx.background_executor().timer(PASTE_DELAY).await;
            input::post_paste();
        })
        .detach();
    }

    fn write_to_pasteboard(&mut self, contents: &ClipContents, plain: bool) {
        let count = match plain.then(|| classify::plain_text_of(contents)).flatten() {
            Some(text) => pasteboard::write_plain_text(&text),
            None => pasteboard::write(contents),
        };
        self.self_change_count = count;
        self.last_change_count = count;
    }

    pub fn open_item(&mut self, id: ItemId, cx: &mut Context<Self>) {
        let Ok(Some(item)) = self.store.item(id) else {
            return;
        };
        let opened = match item.kind {
            ItemKind::Link => workspace::open_url(item.preview_text.trim()),
            ItemKind::File => self
                .store
                .contents(id)
                .ok()
                .and_then(|c| classify::file_paths(&c).into_iter().next())
                .is_some_and(|path| workspace::open_path(&path)),
            _ => false,
        };
        if opened {
            self.hide_panel(cx);
        }
    }

    pub fn reveal_item(&mut self, id: ItemId, cx: &mut Context<Self>) {
        let Ok(contents) = self.store.contents(id) else {
            return;
        };
        let paths = classify::file_paths(&contents);
        if !paths.is_empty() {
            workspace::reveal_in_finder(&paths);
            self.hide_panel(cx);
        }
    }

    pub fn delete_item(&mut self, id: ItemId, cx: &mut Context<Self>) {
        if self.store.delete_item(id).is_ok() {
            self.bump(cx);
        }
    }

    pub fn rename_item(&mut self, id: ItemId, title: &str, cx: &mut Context<Self>) {
        if self.store.rename_item(id, Some(title)).is_ok() {
            self.bump(cx);
        }
    }

    pub fn pin_item(&mut self, id: ItemId, pinboard: PinboardId, cx: &mut Context<Self>) {
        match self.store.pin(id, pinboard, now_ms()) {
            Ok(_) => self.bump(cx),
            Err(e) => log::error!("pin failed: {e:#}"),
        }
    }

    pub fn move_in_pinboard(&mut self, id: ItemId, index: usize, cx: &mut Context<Self>) {
        if self.store.move_in_pinboard(id, index).is_ok() {
            self.bump(cx);
        }
    }

    pub fn create_pinboard(&mut self, name: &str, cx: &mut Context<Self>) -> Option<Pinboard> {
        let pinboard = self.store.create_pinboard(name).ok()?;
        self.bump(cx);
        Some(pinboard)
    }

    pub fn rename_pinboard(&mut self, id: PinboardId, name: &str, cx: &mut Context<Self>) {
        if self.store.rename_pinboard(id, name).is_ok() {
            self.bump(cx);
        }
    }

    pub fn recolor_pinboard(&mut self, id: PinboardId, color: u8, cx: &mut Context<Self>) {
        if self.store.set_pinboard_color(id, color).is_ok() {
            self.bump(cx);
        }
    }

    pub fn delete_pinboard(&mut self, id: PinboardId, cx: &mut Context<Self>) {
        if self.store.delete_pinboard(id).is_ok() {
            self.bump(cx);
        }
    }

    pub fn erase_history(&mut self, cx: &mut Context<Self>) {
        if self.store.erase_history().is_ok() {
            self.bump(cx);
        }
    }

    // ---- pause -------------------------------------------------------------

    pub fn pause(&mut self, duration: PauseDuration, cx: &mut Context<Self>) {
        self.pause = duration.start(now_ms());
        self.refresh_status_item();
        cx.notify();
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) {
        self.pause = PauseState::Running;
        // Don't capture whatever was copied while paused.
        self.last_change_count = pasteboard::change_count();
        self.refresh_status_item();
        cx.notify();
    }

    // ---- Paste Queue -------------------------------------------------------

    pub fn toggle_queue(&mut self, cx: &mut Context<Self>) {
        if self.queue_open {
            self.close_queue(cx);
        } else {
            self.open_queue(cx);
        }
    }

    fn open_queue(&mut self, cx: &mut Context<Self>) {
        self.queue_open = true;
        self.queue.clear();
        self.sync_queue_hotkey();
        self.refresh_status_item();
        cx.notify();

        let Some((_, visible)) = workspace::screen_under_mouse() else {
            return;
        };
        let frame = ScreenRect {
            x: visible.x + visible.width - QUEUE_SIZE.0 - 16.0,
            y: visible.y + visible.height - QUEUE_SIZE.1 - 16.0,
            width: QUEUE_SIZE.0,
            height: QUEUE_SIZE.1,
        };
        if let Some((_, native)) = self.queue_window {
            cx.spawn(async move |_, _| native.show(frame, false))
                .detach();
            return;
        }
        if self.queue_window_opening {
            return;
        }
        self.queue_window_opening = true;
        cx.spawn(async move |this, cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(QUEUE_SIZE.0 as f32), px(QUEUE_SIZE.1 as f32)),
                ))),
                titlebar: None,
                focus: false,
                show: false,
                kind: WindowKind::PopUp,
                is_movable: true,
                is_resizable: false,
                is_minimizable: false,
                window_background: WindowBackgroundAppearance::Blurred,
                ..WindowOptions::default()
            };
            let handle = cx.open_window(options, |window, cx| {
                cx.new(|cx| QueueView::new(window, cx))
            })?;
            let native = handle.update(cx, |_, window, _| native_window(window))?;
            if let Some(native) = native {
                native.set_visible_in_screen_capture(true);
                native.show(frame, false);
                this.update(cx, |state, _| {
                    state.queue_window_opening = false;
                    state.queue_window = Some((handle, native));
                })?;
                debug_snapshot(native, "queue", cx).await;
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    pub fn close_queue(&mut self, cx: &mut Context<Self>) {
        self.queue_open = false;
        self.queue.clear();
        self.sync_queue_hotkey();
        self.refresh_status_item();
        cx.notify();
        if let Some((_, native)) = self.queue_window {
            cx.spawn(async move |_, _| native.hide()).detach();
        }
    }

    pub fn toggle_queue_direction(&mut self, cx: &mut Context<Self>) {
        self.queue.toggle_direction();
        self.settings.paste_queue_direction = self.queue.direction;
        self.save_settings();
        cx.notify();
    }

    pub fn remove_queue_snippet(&mut self, index: usize, cx: &mut Context<Self>) {
        self.queue.remove_in_paste_order(index);
        self.sync_queue_hotkey();
        cx.notify();
    }

    /// The intercepted ⌘V: put the next snippet on the clipboard, release the
    /// intercept, post a real ⌘V, then re-arm (SPEC §6).
    fn queue_paste_next(&mut self, cx: &mut Context<Self>) {
        let Some(snippet) = self.queue.take_next() else {
            self.sync_queue_hotkey();
            return;
        };
        self.write_to_pasteboard(&snippet.contents, false);
        if let Some(hotkeys) = self.hotkeys.as_mut() {
            hotkeys.unregister(HotkeyAction::QueuePaste);
        }
        cx.notify();
        if !input::is_trusted(true) {
            self.sync_queue_hotkey();
            return;
        }
        cx.spawn(async move |this, cx| {
            input::post_paste();
            cx.background_executor().timer(QUEUE_REARM_DELAY).await;
            this.update(cx, |state, cx| {
                state.sync_queue_hotkey();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- settings ----------------------------------------------------------

    pub fn update_settings(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Settings)) {
        let before = self.settings.clone();
        change(&mut self.settings);
        self.settings = self.settings.clone().sanitized();
        self.save_settings();
        if before.activation_shortcut != self.settings.activation_shortcut
            || before.paste_queue_shortcut != self.settings.paste_queue_shortcut
        {
            self.register_hotkeys();
        }
        if before.keep_history != self.settings.keep_history {
            let _ = self
                .store
                .prune_history(self.settings.keep_history, now_ms());
            self.revision += 1;
        }
        if before.show_during_screen_sharing != self.settings.show_during_screen_sharing
            && let Some(native) = self.panel_native
        {
            let visible = self.settings.show_during_screen_sharing;
            cx.spawn(async move |_, _| native.set_visible_in_screen_capture(visible))
                .detach();
        }
        self.refresh_status_item();
        cx.notify();
    }

    fn save_settings(&self) {
        if let Err(e) = self.settings.save(&self.settings_path) {
            log::error!("could not save settings: {e:#}");
        }
    }

    pub fn open_settings(&mut self, confirm_erase: bool, cx: &mut Context<Self>) {
        self.hide_panel(cx);
        let existing = self.settings_window;
        cx.spawn(async move |this, cx| {
            cx.update(|cx| cx.activate(true))?;
            if let Some(handle) = existing {
                let reused = handle.update(cx, |view, window, cx| {
                    window.activate_window();
                    if confirm_erase {
                        view.confirm_erase(window, cx);
                    }
                });
                if reused.is_ok() {
                    return anyhow::Ok(());
                }
            }
            let bounds = cx.update(|cx| WindowBounds::centered(size(px(620.), px(640.)), cx))?;
            let options = WindowOptions {
                window_bounds: Some(bounds),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Copywren Settings".into()),
                    ..Default::default()
                }),
                focus: true,
                show: true,
                kind: WindowKind::Normal,
                is_resizable: false,
                ..WindowOptions::default()
            };
            let handle = cx.open_window(options, |window, cx| {
                cx.new(|cx| SettingsView::new(window, cx))
            })?;
            if confirm_erase {
                handle.update(cx, |view, window, cx| view.confirm_erase(window, cx))?;
            }
            this.update(cx, |state, _| state.settings_window = Some(handle))?;
            if let Some(native) = handle.update(cx, |_, window, _| native_window(window))? {
                debug_snapshot(native, "settings", cx).await;
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    #[cfg(feature = "demo")]
    pub fn queue_native(&self) -> Option<NativeWindow> {
        self.queue_window.map(|(_, native)| native)
    }

    #[cfg(feature = "demo")]
    pub fn settings_window(&self) -> Option<WindowHandle<SettingsView>> {
        self.settings_window
    }

    pub fn accessibility_trusted(&self) -> bool {
        input::is_trusted(false)
    }
}

/// Debugging hook: with `COPYWREN_SNAPSHOT_DIR` set, saves `<name>.png` of a
/// window shortly after it appears. Own-window capture needs no permission.
async fn debug_snapshot(native: NativeWindow, name: &str, cx: &mut gpui::AsyncApp) {
    let Some(dir) = std::env::var_os("COPYWREN_SNAPSHOT_DIR") else {
        return;
    };
    cx.background_executor()
        .timer(Duration::from_millis(1200))
        .await;
    match native.snapshot_png() {
        Some(png) => {
            let _ = std::fs::write(PathBuf::from(dir).join(format!("{name}.png")), png);
        }
        None => log::warn!("{name} snapshot failed"),
    }
}

/// Popup frame next to the focused text cursor, or the mouse pointer when the
/// frontmost app doesn't expose its caret (SPEC §4).
fn popup_frame_near_caret() -> Option<ScreenRect> {
    let anchor = input::focused_caret_rect()
        .map(|(x, y, w, h)| {
            ScreenRect::from_top_left(x, y, w, h, workspace::primary_screen_height())
        })
        .unwrap_or_else(|| {
            let (x, y) = workspace::mouse_location();
            ScreenRect::new(x, y, 0.0, 0.0)
        });
    let (_, visible) = workspace::screen_containing(anchor.x, anchor.y)?;
    Some(placement::popup_frame(
        anchor,
        f64::from(theme::POPUP_WIDTH),
        f64::from(theme::POPUP_HEIGHT),
        visible,
    ))
}

/// While recording the demo, the popup sits in the bottom-right corner of the
/// main screen, out of the way.
fn demo_frame() -> Option<ScreenRect> {
    let (_, visible) = workspace::screen_containing(0.0, 0.0)?;
    let (width, height) = (
        f64::from(theme::POPUP_WIDTH),
        f64::from(theme::POPUP_HEIGHT),
    );
    Some(ScreenRect::new(
        visible.max_x() - width - 24.0,
        visible.y + 24.0,
        width,
        height,
    ))
}
