//! The popup: a compact floating window next to the text cursor with a
//! search field, pinboard tabs and a vertical list of clips (SPEC §4).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use copywren_core::model::{Item, ItemId, ItemKind, Pinboard, PinboardId, Tab};
use copywren_core::shortcut::ModifierKey;
use copywren_core::store::PINBOARD_COLORS;
use copywren_core::{classify, color, now_ms, search, text, timefmt};
use copywren_macos::workspace;
use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, Focusable, FontWeight, Image, ImageFormat,
    KeyDownEvent, ListAlignment, ListState, Modifiers, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point, Render, Rgba,
    SharedString, Subscription, Window, anchored, deferred, div, img, list, prelude::*, px, rgb,
    rgba,
};

use crate::state::AppState;
use crate::theme::{
    self, FOOTER_HEIGHT, HEADER_HEIGHT, PAD, POPUP_WIDTH, SEARCH_HEIGHT, TABS_HEIGHT,
};

/// The search is cleared once the popup has been closed this long.
const SEARCH_RESET: Duration = Duration::from_secs(60);
/// Rows of text shown per text card, and roughly how many characters fit
/// on one row (for `text::fit_rows`).
const TEXT_ROWS: usize = 4;
const TEXT_CHARS_PER_ROW: usize = 44;
/// Largest image thumbnail inside a card.
const THUMB_MAX: (f32, f32) = (POPUP_WIDTH - 4.0 * PAD, 140.0);
/// Keys like Page Down move the selection by this many cards.
const PAGE: usize = 5;

/// Popup icon (rendered from assets/icon.svg).
const HEADER_ICON: &[u8] = include_bytes!("../../../assets/icon-64.png");

pub struct Panel {
    focus: FocusHandle,
    tab: Tab,
    pinboards: Vec<Pinboard>,
    items: Vec<Item>,
    /// Indices into `items` that match the search, in display order.
    visible: Vec<usize>,
    known_apps: Vec<String>,
    query: String,
    /// Index into `visible`.
    selected: usize,
    seen_revision: Option<u64>,
    list: ListState,
    /// First card in view, captured each frame before the list lays out
    /// (the list can't be queried while it is rendering its items).
    first_visible: usize,
    modifiers: Modifiers,
    card_menu: Option<CardMenu>,
    tab_menu: Option<TabMenu>,
    confirm: Option<Confirm>,
    editing: Option<Editing>,
    preview: Option<Preview>,
    drag: Option<MoveDrag>,
    hidden_at: Option<Instant>,
    shown: bool,
    header_icon: Arc<Image>,
    thumbs: HashMap<ItemId, Option<Arc<Image>>>,
    _subscriptions: Vec<Subscription>,
}

struct CardMenu {
    item_id: ItemId,
    position: Point<Pixels>,
}

struct TabMenu {
    pinboard: PinboardId,
    position: Point<Pixels>,
}

enum Confirm {
    DeletePinboard { id: PinboardId, name: String },
}

enum Editing {
    NewPinboard {
        text: String,
        pin_item: Option<ItemId>,
    },
    RenamePinboard {
        id: PinboardId,
        text: String,
    },
    RenameItem {
        id: ItemId,
        text: String,
    },
}

impl Editing {
    fn text_mut(&mut self) -> &mut String {
        match self {
            Editing::NewPinboard { text, .. }
            | Editing::RenamePinboard { text, .. }
            | Editing::RenameItem { text, .. } => text,
        }
    }
}

struct Preview {
    item_id: ItemId,
    text: Option<SharedString>,
}

/// Dragging the popup by its header.
struct MoveDrag {
    start_mouse: (f64, f64),
    start_origin: (f64, f64),
}

/// Payload of a card being dragged onto a pinboard tab or another card.
#[derive(Clone)]
pub struct DraggedCard {
    id: ItemId,
    title: SharedString,
}

impl Render for DraggedCard {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1p5()
            .rounded(px(8.))
            .bg(theme::accent())
            .text_color(rgb(0xffffff))
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .shadow_lg()
            .child(self.title.clone())
    }
}

fn modifier_held(modifiers: &Modifiers, key: ModifierKey) -> bool {
    match key {
        ModifierKey::Cmd => modifiers.platform,
        ModifierKey::Shift => modifiers.shift,
        ModifierKey::Alt => modifiers.alt,
        ModifierKey::Ctrl => modifiers.control,
    }
}

fn is_typed_text(key_char: &str) -> bool {
    !key_char.is_empty() && key_char.chars().all(|c| !c.is_control())
}

/// Thumbnail display size: fits `THUMB_MAX`, never upscaled.
fn thumb_size(image: Option<(u32, u32)>) -> (f32, f32) {
    let Some((w, h)) = image.filter(|(w, h)| *w > 0 && *h > 0) else {
        return (THUMB_MAX.0, 90.0);
    };
    let (w, h) = (w as f32, h as f32);
    let scale = (THUMB_MAX.0 / w).min(THUMB_MAX.1 / h).min(1.0);
    ((w * scale).max(24.0), (h * scale).max(24.0))
}

impl Panel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let subscriptions = vec![
            cx.observe(&state, |this, _, cx| {
                this.reload_if_changed(cx);
                cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                let demo = AppState::global(cx).read(cx).demo;
                if !window.is_window_active() && this.shown && this.drag.is_none() && !demo {
                    AppState::global(cx).update(cx, |state, cx| state.hide_panel(cx));
                }
            }),
        ];
        Self {
            focus: cx.focus_handle(),
            tab: Tab::History,
            pinboards: Vec::new(),
            items: Vec::new(),
            visible: Vec::new(),
            known_apps: Vec::new(),
            query: String::new(),
            selected: 0,
            seen_revision: None,
            list: ListState::new(0, ListAlignment::Top, px(300.)),
            first_visible: 0,
            modifiers: Modifiers::default(),
            card_menu: None,
            tab_menu: None,
            confirm: None,
            editing: None,
            preview: None,
            drag: None,
            hidden_at: None,
            shown: false,
            header_icon: Arc::new(Image::from_bytes(ImageFormat::Png, HEADER_ICON.to_vec())),
            thumbs: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    // ---- lifecycle ---------------------------------------------------------

    pub fn did_show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hidden_at.is_some_and(|t| t.elapsed() > SEARCH_RESET) {
            self.query.clear();
        }
        self.close_overlays();
        self.reload(cx);
        self.selected = 0;
        self.list.scroll_to_reveal_item(0);
        self.modifiers = window.modifiers();
        self.shown = true;
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn did_hide(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.shown = false;
        self.hidden_at = Some(Instant::now());
        self.drag = None;
        self.close_overlays();
        cx.notify();
    }

    fn close_overlays(&mut self) {
        self.card_menu = None;
        self.tab_menu = None;
        self.confirm = None;
        self.editing = None;
        self.preview = None;
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        AppState::global(cx).update(cx, |state, cx| state.hide_panel(cx));
    }

    // ---- data --------------------------------------------------------------

    fn reload_if_changed(&mut self, cx: &mut Context<Self>) {
        let revision = AppState::global(cx).read(cx).revision;
        if self.seen_revision != Some(revision) {
            self.reload(cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let keep = self.selected_item().map(|item| item.id);
        let state = AppState::global(cx);
        let state = state.read(cx);
        self.pinboards = state.store.pinboards().unwrap_or_default();
        if let Tab::Pinboard(id) = self.tab
            && !self.pinboards.iter().any(|p| p.id == id)
        {
            self.tab = Tab::History;
        }
        self.items = state.store.items(self.tab).unwrap_or_default();
        self.known_apps = state.store.known_app_names().unwrap_or_default();
        self.seen_revision = Some(state.revision);
        let live: HashSet<ItemId> = self.items.iter().map(|i| i.id).collect();
        self.thumbs.retain(|id, _| live.contains(id));
        let fallback = self.selected;
        self.refilter(keep, fallback);
    }

    /// Recomputes the visible cards. The selection follows `keep` when it is
    /// still visible; otherwise it stays at `fallback` (so deleting a card
    /// selects the neighbour that slides into its place).
    fn refilter(&mut self, keep: Option<ItemId>, fallback: usize) {
        self.visible = search::filter(&self.items, &self.query, &self.known_apps);
        self.selected = keep
            .and_then(|id| self.visible.iter().position(|&i| self.items[i].id == id))
            .unwrap_or(fallback)
            .min(self.visible.len().saturating_sub(1));
        self.list.reset(self.visible.len());
        self.list.scroll_to_reveal_item(self.selected);
    }

    fn selected_item(&self) -> Option<&Item> {
        self.visible
            .get(self.selected)
            .and_then(|&i| self.items.get(i))
    }

    fn item_at(&self, pos: usize) -> Option<&Item> {
        self.visible.get(pos).and_then(|&i| self.items.get(i))
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.query = query;
        self.refilter(None, 0);
        cx.notify();
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            self.selected = 0;
            self.reload(cx);
            self.selected = 0;
            self.list.scroll_to_reveal_item(0);
        }
        self.close_overlays();
        cx.notify();
    }

    fn tabs(&self) -> Vec<Tab> {
        std::iter::once(Tab::History)
            .chain(self.pinboards.iter().map(|p| Tab::Pinboard(p.id)))
            .collect()
    }

    fn cycle_tab(&mut self, forward: bool, cx: &mut Context<Self>) {
        let tabs = self.tabs();
        let current = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
        let next = if forward {
            (current + 1) % tabs.len()
        } else {
            (current + tabs.len() - 1) % tabs.len()
        };
        self.set_tab(tabs[next], cx);
    }

    fn thumbnail(&mut self, id: ItemId, cx: &App) -> Option<Arc<Image>> {
        self.thumbs
            .entry(id)
            .or_insert_with(|| {
                let png = AppState::global(cx)
                    .read(cx)
                    .store
                    .thumbnail(id)
                    .ok()
                    .flatten()?;
                Some(Arc::new(Image::from_bytes(ImageFormat::Png, png)))
            })
            .clone()
    }

    fn first_visible_card(&self) -> usize {
        self.list.logical_scroll_top().item_ix
    }

    fn select(&mut self, pos: usize, cx: &mut Context<Self>) {
        if self.visible.is_empty() {
            return;
        }
        self.selected = pos.min(self.visible.len() - 1);
        self.list.scroll_to_reveal_item(self.selected);
        cx.notify();
    }

    // ---- actions -----------------------------------------------------------

    fn paste_at(&mut self, pos: usize, plain: bool, cx: &mut Context<Self>) {
        let Some(id) = self.item_at(pos).map(|i| i.id) else {
            return;
        };
        AppState::global(cx).update(cx, |state, cx| state.paste_item(id, plain, true, cx));
    }

    fn copy_selected(&mut self, plain: bool, cx: &mut Context<Self>) {
        let Some(id) = self.selected_item().map(|i| i.id) else {
            return;
        };
        AppState::global(cx).update(cx, |state, cx| state.paste_item(id, plain, false, cx));
    }

    fn delete_item(&mut self, id: ItemId, cx: &mut Context<Self>) {
        AppState::global(cx).update(cx, |state, cx| state.delete_item(id, cx));
        cx.notify();
    }

    fn pin(&mut self, id: ItemId, pinboard: PinboardId, cx: &mut Context<Self>) {
        AppState::global(cx).update(cx, |state, cx| state.pin_item(id, pinboard, cx));
    }

    fn start_rename_item(&mut self, id: ItemId, cx: &mut Context<Self>) {
        let current = self
            .items
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.title.clone())
            .unwrap_or_default();
        self.close_overlays();
        self.editing = Some(Editing::RenameItem { id, text: current });
        cx.notify();
    }

    fn start_new_pinboard(&mut self, pin_item: Option<ItemId>, cx: &mut Context<Self>) {
        self.close_overlays();
        self.editing = Some(Editing::NewPinboard {
            text: String::new(),
            pin_item,
        });
        cx.notify();
    }

    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        let state = AppState::global(cx);
        match editing {
            Editing::NewPinboard { text, pin_item } => {
                let created = state.update(cx, |state, cx| state.create_pinboard(&text, cx));
                if let (Some(pinboard), Some(item)) = (&created, pin_item) {
                    state.update(cx, |state, cx| state.pin_item(item, pinboard.id, cx));
                }
                if let Some(pinboard) = created {
                    self.set_tab(Tab::Pinboard(pinboard.id), cx);
                }
            }
            Editing::RenamePinboard { id, text } => {
                if !text.trim().is_empty() {
                    state.update(cx, |state, cx| state.rename_pinboard(id, &text, cx));
                }
            }
            Editing::RenameItem { id, text } => {
                state.update(cx, |state, cx| state.rename_item(id, &text, cx));
            }
        }
        cx.notify();
    }

    fn open_preview(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.selected_item().cloned() else {
            return;
        };
        let text = match item.kind {
            ItemKind::Image | ItemKind::Color => None,
            _ => AppState::global(cx)
                .read(cx)
                .store
                .contents(item.id)
                .ok()
                .and_then(|c| classify::plain_text_of(&c))
                .map(|t| {
                    let limit = t.char_indices().nth(100_000).map_or(t.len(), |(i, _)| i);
                    SharedString::from(t[..limit].replace('\t', "    "))
                }),
        };
        self.preview = Some(Preview {
            item_id: item.id,
            text,
        });
        cx.notify();
    }

    fn accept_confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(Confirm::DeletePinboard { id, .. }) = self.confirm.take() {
            AppState::global(cx).update(cx, |state, cx| state.delete_pinboard(id, cx));
            self.set_tab(Tab::History, cx);
        }
        cx.notify();
    }

    /// Pretends modifier keys are held (shows the quick-paste badges).
    #[cfg(feature = "demo")]
    pub fn demo_set_modifiers(&mut self, modifiers: Modifiers, cx: &mut Context<Self>) {
        self.modifiers = modifiers;
        cx.notify();
    }

    /// Opens the item menu for the selected card, as a right-click would.
    #[cfg(feature = "demo")]
    pub fn demo_open_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_item().map(|i| i.id) {
            self.close_overlays();
            self.card_menu = Some(CardMenu {
                item_id: id,
                position,
            });
            cx.notify();
        }
    }

    // ---- moving the popup --------------------------------------------------

    fn start_move(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = AppState::global(cx).read(cx).panel_frame() else {
            return;
        };
        self.drag = Some(MoveDrag {
            start_mouse: workspace::mouse_location(),
            start_origin: (frame.x, frame.y),
        });
    }

    fn drag_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = &self.drag else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let (mx, my) = workspace::mouse_location();
        let x = drag.start_origin.0 + (mx - drag.start_mouse.0);
        let y = drag.start_origin.1 + (my - drag.start_mouse.1);
        AppState::global(cx).update(cx, |state, cx| state.move_panel(x, y, cx));
    }

    // ---- keyboard ----------------------------------------------------------

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let keystroke = &event.keystroke;
        let m = keystroke.modifiers;
        let key = keystroke.key.as_str();
        let typed = keystroke.key_char.as_deref().filter(|c| is_typed_text(c));

        if self.confirm.is_some() {
            match key {
                "enter" => self.accept_confirm(cx),
                "escape" => self.confirm = None,
                _ => {}
            }
            cx.notify();
            return;
        }
        if let Some(editing) = self.editing.as_mut() {
            let text = editing.text_mut();
            match key {
                "enter" => return self.commit_edit(cx),
                "escape" => self.editing = None,
                "backspace" if m.platform => text.clear(),
                "backspace" if m.alt => {
                    let trimmed = text.trim_end().len();
                    let cut = text[..trimmed].rfind(' ').map_or(0, |i| i + 1);
                    text.truncate(cut);
                }
                "backspace" => {
                    text.pop();
                }
                "v" if m.platform => {
                    if let Some(pasted) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        text.push_str(pasted.lines().next().unwrap_or_default());
                    }
                }
                _ if !m.platform && !m.control => {
                    if let Some(typed) = typed {
                        text.push_str(typed);
                    }
                }
                _ => {}
            }
            cx.notify();
            return;
        }
        if self.card_menu.is_some() || self.tab_menu.is_some() {
            if key == "escape" {
                self.card_menu = None;
                self.tab_menu = None;
                cx.notify();
            }
            return;
        }
        if self.preview.is_some() && matches!(key, "escape" | "space") {
            self.preview = None;
            cx.notify();
            return;
        }

        let (quick_modifier, plain_modifier) = {
            let settings = &AppState::global(cx).read(cx).settings;
            (settings.quick_paste_modifier, settings.plain_text_modifier)
        };
        let plain = modifier_held(&m, plain_modifier);
        let last = self.visible.len().saturating_sub(1);

        match key {
            "escape" => {
                if !self.query.is_empty() {
                    self.set_query(String::new(), cx);
                } else {
                    self.hide(cx);
                }
            }
            "up" if m.platform => self.select(0, cx),
            "down" if m.platform => self.select(last, cx),
            "up" => self.select(self.selected.saturating_sub(1), cx),
            "down" => self.select(self.selected + 1, cx),
            "pageup" => self.select(self.selected.saturating_sub(PAGE), cx),
            "pagedown" => self.select(self.selected + PAGE, cx),
            "home" => self.select(0, cx),
            "end" => self.select(last, cx),
            "left" if !m.platform => self.cycle_tab(false, cx),
            "right" if !m.platform => self.cycle_tab(true, cx),
            "tab" => self.cycle_tab(!m.shift, cx),
            "]" if m.platform => self.cycle_tab(true, cx),
            "[" if m.platform => self.cycle_tab(false, cx),
            "enter" => self.paste_at(self.selected, plain, cx),
            "space" if self.query.is_empty() && !m.platform => self.open_preview(cx),
            "backspace" if m.platform => {
                if let Some(id) = self.selected_item().map(|i| i.id) {
                    self.delete_item(id, cx);
                }
            }
            "delete" => {
                if let Some(id) = self.selected_item().map(|i| i.id) {
                    self.delete_item(id, cx);
                }
            }
            "backspace" => {
                let mut query = self.query.clone();
                if m.alt {
                    let trimmed = query.trim_end().len();
                    query.truncate(query[..trimmed].rfind(' ').map_or(0, |i| i + 1));
                } else {
                    query.pop();
                }
                self.set_query(query, cx);
            }
            "c" if m.platform => self.copy_selected(plain, cx),
            "o" if m.platform => {
                if let Some(id) = self.selected_item().map(|i| i.id) {
                    AppState::global(cx).update(cx, |state, cx| state.open_item(id, cx));
                }
            }
            "r" if m.platform => {
                if let Some(id) = self.selected_item().map(|i| i.id) {
                    self.start_rename_item(id, cx);
                }
            }
            "n" if m.platform => self.start_new_pinboard(None, cx),
            "," if m.platform => {
                AppState::global(cx).update(cx, |state, cx| state.open_settings(false, cx));
            }
            digit
                if modifier_held(&m, quick_modifier)
                    && digit.len() == 1
                    && matches!(digit.as_bytes()[0], b'1'..=b'9') =>
            {
                let n = (digit.as_bytes()[0] - b'1') as usize;
                let pos = self.first_visible_card() + n;
                if pos < self.visible.len() {
                    self.paste_at(pos, plain, cx);
                }
            }
            _ if !m.platform && !m.control => {
                if let Some(typed) = typed {
                    let mut query = self.query.clone();
                    query.push_str(typed);
                    self.set_query(query, cx);
                }
            }
            _ => {}
        }
    }

    // ---- rendering ---------------------------------------------------------

    fn render_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (paused_label, queue_open) = {
            let state = AppState::global(cx).read(cx);
            let paused = state
                .pause
                .is_paused(now_ms())
                .then(|| match state.pause.resume_at() {
                    Some(at) => format!("Paused until {}", timefmt::clock(at)),
                    None => "Paused".to_string(),
                });
            (paused, state.queue_open)
        };
        div()
            .id("header")
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .px(px(PAD))
            .flex()
            .items_center()
            .gap_2()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.start_move(cx)),
            )
            .child(img(self.header_icon.clone()).size(px(18.)).flex_none())
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Copywren"),
            )
            .child(div().flex_1())
            .when(queue_open, |d| {
                d.child(status_chip("Paste Queue", theme::accent()))
            })
            .when_some(paused_label, |d, label| {
                d.child(status_chip(&label, rgb(0xff9f0a)))
            })
            .child(icon_button("settings", "⚙").on_click(cx.listener(
                |_, _: &ClickEvent, _, cx| {
                    AppState::global(cx).update(cx, |state, cx| state.open_settings(false, cx));
                },
            )))
            .child(
                icon_button("close", "×")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.hide(cx))),
            )
    }

    fn render_search(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let searching = self.editing.is_none() && self.shown;
        div()
            .h(px(SEARCH_HEIGHT))
            .flex_none()
            .px(px(PAD))
            .pb_2()
            .child(
                div()
                    .id("search")
                    .size_full()
                    .rounded(px(8.))
                    .bg(theme::surface())
                    .border_1()
                    .border_color(theme::border())
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_sm()
                    .overflow_hidden()
                    .child(div().text_color(theme::text_muted()).child("⌕"))
                    .child(if self.query.is_empty() {
                        div()
                            .flex()
                            .items_center()
                            .when(searching, |d| d.child(caret()))
                            .child(
                                div()
                                    .text_color(theme::text_muted())
                                    .child("Type to search your clipboard"),
                            )
                            .into_any_element()
                    } else {
                        div()
                            .flex()
                            .items_center()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_color(theme::text())
                                    .whitespace_nowrap()
                                    .child(self.query.clone()),
                            )
                            .when(searching, |d| d.child(caret()))
                            .into_any_element()
                    })
                    .child(div().flex_1())
                    .when(!self.query.is_empty(), |d| {
                        d.child(icon_button("clear-search", "×").on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| this.set_query(String::new(), cx),
                        )))
                    }),
            )
    }

    fn render_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs: Vec<AnyElement> =
            vec![self.render_tab(Tab::History, "Clipboard".into(), None, cx)];
        for pinboard in self.pinboards.clone() {
            let label = match &self.editing {
                Some(Editing::RenamePinboard { id, text }) if *id == pinboard.id => text.clone(),
                _ => pinboard.name.clone(),
            };
            tabs.push(self.render_tab(
                Tab::Pinboard(pinboard.id),
                label,
                Some(theme::pinboard_color(pinboard.color)),
                cx,
            ));
        }
        if let Some(Editing::NewPinboard { text, .. }) = &self.editing {
            let color =
                theme::pinboard_color((self.pinboards.len() % PINBOARD_COLORS as usize) as u8);
            tabs.push(
                chip(true)
                    .id("new-pinboard-editor")
                    .child(div().size(px(7.)).rounded_full().bg(color))
                    .child(if text.is_empty() {
                        "Name".to_string()
                    } else {
                        text.clone()
                    })
                    .child(caret())
                    .into_any_element(),
            );
        }
        div()
            .h(px(TABS_HEIGHT))
            .flex_none()
            .px(px(PAD))
            .pb_1()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .id("tabs")
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(icon_button("new-pinboard", "+").on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.start_new_pinboard(None, cx)),
            ))
    }

    fn render_tab(
        &mut self,
        tab: Tab,
        label: String,
        color: Option<Rgba>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.tab == tab;
        let editing = matches!(
            (&self.editing, tab),
            (Some(Editing::RenamePinboard { id, .. }), Tab::Pinboard(p)) if *id == p
        );
        let element_id = match tab {
            Tab::History => ("tab", 0u64),
            Tab::Pinboard(id) => ("tab", id as u64 + 1),
        };
        let mut chip = chip(selected)
            .id(element_id)
            .when_some(color, |d, c| {
                d.child(div().size(px(7.)).rounded_full().bg(c))
            })
            .child(label)
            .when(editing, |d| d.child(caret()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_tab(tab, cx)));
        if let Tab::Pinboard(pinboard) = tab {
            chip = chip
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.close_overlays();
                        this.tab_menu = Some(TabMenu {
                            pinboard,
                            position: event.position,
                        });
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .drag_over::<DraggedCard>(|style, _, _, _| style.bg(rgba(0x3b9eea66)))
                .on_drop(cx.listener(move |this, dragged: &DraggedCard, _, cx| {
                    this.pin(dragged.id, pinboard, cx);
                }));
        }
        chip.into_any_element()
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.visible.is_empty() {
            let (title, detail) = if !self.query.is_empty() {
                (
                    "Nothing found",
                    "Try another word, an app name, or a type like \"link\".",
                )
            } else if self.tab == Tab::History {
                (
                    "Nothing copied yet",
                    "Copy something and it will appear here.",
                )
            } else {
                (
                    "This pinboard is empty",
                    "Drag a clip onto its tab to pin it.",
                )
            };
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .px_6()
                .text_center()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_muted())
                        .child(detail),
                )
                .into_any_element();
        }
        div()
            .flex_1()
            .min_h_0()
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|this, ix: usize, _window, cx| this.render_card(ix, cx)),
                )
                .size_full(),
            )
            .into_any_element()
    }

    fn render_card(&mut self, pos: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.item_at(pos).cloned() else {
            return div().into_any_element();
        };
        let selected = pos == self.selected;
        let (app_color, icon, quick_modifier) = {
            let state = AppState::global(cx).read(cx);
            (
                state.app_color(item.source_bundle_id.as_deref()),
                state.app_icon(item.source_bundle_id.as_deref()),
                state.settings.quick_paste_modifier,
            )
        };
        let first = self.first_visible;
        let badge = (modifier_held(&self.modifiers, quick_modifier)
            && self.editing.is_none()
            && pos >= first
            && pos < first + 9)
            .then(|| format!("{}{}", quick_modifier.symbol(), pos - first + 1));
        let thumb = (item.kind == ItemKind::Image)
            .then(|| self.thumbnail(item.id, cx))
            .flatten();
        let renaming = match &self.editing {
            Some(Editing::RenameItem { id, text }) if *id == item.id => Some(text.clone()),
            _ => None,
        };
        let id = item.id;
        let in_pinboard = matches!(self.tab, Tab::Pinboard(_)) && self.query.is_empty();
        let full_index = self.visible[pos];

        let source = item
            .source_app_name
            .clone()
            .unwrap_or_else(|| item.kind.label().to_string());
        let app_mark = match icon {
            Some(icon) => img(icon).size(px(14.)).flex_none().into_any_element(),
            None => div()
                .size(px(8.))
                .flex_none()
                .rounded_full()
                .bg(theme::hsla(app_color))
                .into_any_element(),
        };
        let meta = div()
            .flex()
            .items_center()
            .gap_1p5()
            .text_xs()
            .text_color(theme::text_muted())
            .child(app_mark)
            .child(div().whitespace_nowrap().overflow_hidden().child(source))
            .child("·")
            .child(
                div()
                    .whitespace_nowrap()
                    .child(timefmt::relative(item.timestamp, now_ms())),
            )
            .child(div().flex_1())
            .when_some(badge, |d, badge| {
                d.child(
                    div()
                        .px_1()
                        .rounded(px(4.))
                        .bg(rgba(0xffffff1a))
                        .text_color(theme::text())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(badge),
                )
            })
            .child(
                div()
                    .id(("more", id as u64))
                    .px_1()
                    .rounded(px(4.))
                    .text_color(theme::text())
                    .cursor_pointer()
                    .opacity(if selected { 1.0 } else { 0.0 })
                    .group_hover("card", |s| s.opacity(1.0))
                    .hover(|s| s.bg(rgba(0xffffff1f)))
                    .child("⋯")
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        this.close_overlays();
                        this.selected = pos;
                        this.card_menu = Some(CardMenu {
                            item_id: id,
                            position: event.position(),
                        });
                        cx.stop_propagation();
                        cx.notify();
                    })),
            );

        let title = match (&renaming, &item.title) {
            (Some(text), _) => Some(
                div()
                    .flex()
                    .items_center()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if text.is_empty() {
                        "Name".to_string()
                    } else {
                        text.clone()
                    })
                    .child(caret())
                    .into_any_element(),
            ),
            (None, Some(title)) if !title.trim().is_empty() => Some(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(title.clone())
                    .into_any_element(),
            ),
            _ => None,
        };

        let drag = DraggedCard {
            id,
            title: item.display_title().into(),
        };

        let card = div()
            .id(("card", id as u64))
            .group("card")
            .p_2p5()
            .rounded(px(10.))
            .flex()
            .flex_col()
            .gap_1p5()
            .bg(if selected {
                theme::card_selected()
            } else {
                theme::card()
            })
            .border_1()
            .border_color(if selected {
                theme::accent()
            } else {
                theme::border()
            })
            .hover(|s| s.bg(theme::surface_hover()))
            .cursor_pointer()
            .child(meta)
            .children(title)
            .child(self.render_card_body(&item, thumb, TEXT_ROWS))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    this.card_menu = None;
                    this.tab_menu = None;
                    this.selected = pos;
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                // A single click pastes, as in the system clipboard panels
                // this window is modelled on. "⋯" stops propagation.
                if event.standard_click() {
                    this.paste_at(pos, event.modifiers().shift, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.close_overlays();
                    this.selected = pos;
                    this.card_menu = Some(CardMenu {
                        item_id: id,
                        position: event.position,
                    });
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_drag(drag, |dragged: &DraggedCard, _, _, cx| {
                cx.new(|_| dragged.clone())
            })
            .when(in_pinboard, |d| {
                d.drag_over::<DraggedCard>(|style, _, _, _| style.border_color(theme::accent()))
                    .on_drop(cx.listener(move |_, dragged: &DraggedCard, _, cx| {
                        AppState::global(cx).update(cx, |state, cx| {
                            state.move_in_pinboard(dragged.id, full_index, cx)
                        });
                    }))
            });

        div().px(px(PAD)).pb_2().child(card).into_any_element()
    }

    fn render_card_body(&self, item: &Item, thumb: Option<Arc<Image>>, rows: usize) -> AnyElement {
        match item.kind {
            ItemKind::Text => div()
                .text_sm()
                .line_height(px(19.))
                .text_color(theme::text())
                .child(text::fit_rows(&item.preview_text, rows, TEXT_CHARS_PER_ROW))
                .into_any_element(),
            ItemKind::Link => {
                let host = text::link_host(&item.preview_text)
                    .unwrap_or(&item.preview_text)
                    .to_string();
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(host),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::link())
                            .child(text::fit_rows(
                                &item.preview_text,
                                2,
                                TEXT_CHARS_PER_ROW + 6,
                            )),
                    )
                    .into_any_element()
            }
            ItemKind::Image => {
                let (w, h) = thumb_size(item.image_size);
                let dims = item
                    .image_size
                    .map(|(w, h)| format!("{w}×{h}"))
                    .unwrap_or_default();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(match thumb {
                        Some(thumb) => img(thumb)
                            .w(px(w))
                            .h(px(h))
                            .rounded(px(6.))
                            .object_fit(ObjectFit::Contain)
                            .into_any_element(),
                        None => div()
                            .w(px(w))
                            .h(px(h))
                            .rounded(px(6.))
                            .bg(theme::surface())
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .text_color(theme::text_muted())
                            .child("Preview unavailable")
                            .into_any_element(),
                    })
                    .child(div().text_xs().text_color(theme::text_muted()).child(dims))
                    .into_any_element()
            }
            ItemKind::Color => {
                let swatch = color::parse(&item.preview_text).map(|c| gpui::Rgba {
                    r: f32::from(c.r) / 255.0,
                    g: f32::from(c.g) / 255.0,
                    b: f32::from(c.b) / 255.0,
                    a: c.a,
                });
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(28.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme::border())
                            .when_some(swatch, |d, c| d.bg(c)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(item.preview_text.clone()),
                    )
                    .into_any_element()
            }
            ItemKind::File => {
                let paths: Vec<&str> = item.preview_text.lines().collect();
                let more = paths.len().saturating_sub(3);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(paths.iter().take(3).map(|path| {
                        let name = text::file_name(path).to_string();
                        let ext = name
                            .rsplit_once('.')
                            .map(|(_, e)| e.to_uppercase())
                            .filter(|e| e.len() <= 4)
                            .unwrap_or_else(|| "•".into());
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(30.))
                                    .h(px(18.))
                                    .flex_none()
                                    .rounded(px(3.))
                                    .bg(theme::surface_hover())
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(9.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::text_muted())
                                    .child(ext),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(name),
                            )
                    }))
                    .when(more > 0, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme::text_muted())
                                .child(format!("and {more} more")),
                        )
                    })
                    .into_any_element()
            }
        }
    }

    fn render_footer(&self) -> impl IntoElement {
        let hint = if self.editing.is_some() {
            "↩ Save   ⎋ Cancel"
        } else {
            "↩ Paste   ⇧↩ Plain text   ⌘⌫ Delete"
        };
        div()
            .h(px(FOOTER_HEIGHT))
            .flex_none()
            .px(px(PAD))
            .flex()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(theme::border())
            .text_xs()
            .text_color(theme::text_muted())
            .child(text::count_label(self.visible.len() as i64, "item"))
            .child(hint)
    }

    fn render_card_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.card_menu.as_ref()?;
        let item = self.items.iter().find(|i| i.id == menu.item_id)?.clone();
        let id = item.id;
        let position = menu.position;
        let current_board = item.pinboard_id;
        let pinboards: Vec<Pinboard> = self
            .pinboards
            .iter()
            .filter(|p| Some(p.id) != current_board)
            .cloned()
            .collect();

        let mut rows: Vec<AnyElement> = vec![
            menu_row(
                "paste",
                "Paste",
                "↩",
                cx.listener(move |this, _, _, cx| {
                    this.card_menu = None;
                    AppState::global(cx).update(cx, |s, cx| s.paste_item(id, false, true, cx));
                }),
            ),
            menu_row(
                "paste-plain",
                "Paste as Plain Text",
                "⇧↩",
                cx.listener(move |this, _, _, cx| {
                    this.card_menu = None;
                    AppState::global(cx).update(cx, |s, cx| s.paste_item(id, true, true, cx));
                }),
            ),
            menu_row(
                "copy",
                "Copy",
                "⌘C",
                cx.listener(move |this, _, _, cx| {
                    this.card_menu = None;
                    AppState::global(cx).update(cx, |s, cx| s.paste_item(id, false, false, cx));
                }),
            ),
            separator(),
        ];
        if matches!(item.kind, ItemKind::Link | ItemKind::File) {
            rows.push(menu_row(
                "open",
                "Open",
                "⌘O",
                cx.listener(move |this, _, _, cx| {
                    this.card_menu = None;
                    AppState::global(cx).update(cx, |s, cx| s.open_item(id, cx));
                }),
            ));
        }
        if item.kind == ItemKind::File {
            rows.push(menu_row(
                "reveal",
                "Show in Finder",
                "",
                cx.listener(move |this, _, _, cx| {
                    this.card_menu = None;
                    AppState::global(cx).update(cx, |s, cx| s.reveal_item(id, cx));
                }),
            ));
        }
        rows.push(menu_row(
            "rename",
            "Rename…",
            "⌘R",
            cx.listener(move |this, _, _, cx| {
                this.start_rename_item(id, cx);
            }),
        ));
        rows.push(menu_row(
            "preview",
            "Preview",
            "Space",
            cx.listener(move |this, _, _, cx| {
                this.card_menu = None;
                this.open_preview(cx);
            }),
        ));
        rows.push(separator());
        rows.push(
            div()
                .px_3()
                .pt_1()
                .text_xs()
                .text_color(theme::text_muted())
                .child(if current_board.is_some() {
                    "Move to"
                } else {
                    "Pin to"
                })
                .into_any_element(),
        );
        rows.push(
            div()
                .px_2()
                .py_1()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(pinboards.into_iter().map(|board| {
                    let board_id = board.id;
                    chip(false)
                        .id(("pin-to", board.id as u64))
                        .h(px(22.))
                        .text_xs()
                        .child(
                            div()
                                .size(px(7.))
                                .rounded_full()
                                .bg(theme::pinboard_color(board.color)),
                        )
                        .child(board.name)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.card_menu = None;
                            this.pin(id, board_id, cx);
                        }))
                }))
                .child(
                    chip(false)
                        .id("pin-new")
                        .h(px(22.))
                        .text_xs()
                        .child("+ New Pinboard")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.start_new_pinboard(Some(id), cx);
                        })),
                )
                .into_any_element(),
        );
        rows.push(separator());
        rows.push(menu_row(
            "delete",
            if current_board.is_some() {
                "Remove from Pinboard"
            } else {
                "Delete"
            },
            "⌘⌫",
            cx.listener(move |this, _, _, cx| {
                this.card_menu = None;
                this.delete_item(id, cx);
            }),
        ));

        Some(
            deferred(
                anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(6.))
                    .child(
                        menu_container("card-menu")
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.card_menu = None;
                                cx.notify();
                            }))
                            .children(rows),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn render_tab_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.tab_menu.as_ref()?;
        let board = self
            .pinboards
            .iter()
            .find(|p| p.id == menu.pinboard)?
            .clone();
        let id = board.id;
        let position = menu.position;
        let rows: Vec<AnyElement> = vec![
            menu_row(
                "rename-board",
                "Rename…",
                "",
                cx.listener(move |this, _, _, cx| {
                    let name = this
                        .pinboards
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    this.close_overlays();
                    this.editing = Some(Editing::RenamePinboard { id, text: name });
                    cx.notify();
                }),
            ),
            div()
                .px_3()
                .py_1()
                .flex()
                .gap_1p5()
                .children((0..PINBOARD_COLORS).map(|color| {
                    div()
                        .id(("board-color", color as u64))
                        .size(px(16.))
                        .rounded_full()
                        .bg(theme::pinboard_color(color))
                        .when(color == board.color, |d| {
                            d.border_2().border_color(rgb(0xffffff))
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.tab_menu = None;
                            AppState::global(cx)
                                .update(cx, |s, cx| s.recolor_pinboard(id, color, cx));
                        }))
                }))
                .into_any_element(),
            separator(),
            menu_row(
                "delete-board",
                "Delete Pinboard…",
                "",
                cx.listener(move |this, _, _, cx| {
                    let name = this
                        .pinboards
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    this.close_overlays();
                    this.confirm = Some(Confirm::DeletePinboard { id, name });
                    cx.notify();
                }),
            ),
        ];
        Some(
            deferred(
                anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(6.))
                    .child(
                        menu_container("tab-menu")
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.tab_menu = None;
                                cx.notify();
                            }))
                            .children(rows),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn render_confirm(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Confirm::DeletePinboard { name, .. } = self.confirm.as_ref()?;
        let title = format!("Delete “{name}”?");
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(rgba(0x00000099))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(300.))
                        .p_4()
                        .rounded(px(12.))
                        .bg(theme::surface())
                        .border_1()
                        .border_color(theme::border())
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(div().text_sm().text_color(theme::text_muted()).child(
                            "The pinboard and everything on it will be deleted. This can't be undone.",
                        ))
                        .child(
                            div()
                                .pt_2()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(button("confirm-cancel", "Cancel", false).on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.confirm = None;
                                        cx.notify();
                                    }),
                                ))
                                .child(button("confirm-delete", "Delete", true).on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.accept_confirm(cx)
                                    }),
                                )),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_preview(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let preview = self.preview.as_ref()?;
        let item = self.items.iter().find(|i| i.id == preview.item_id)?.clone();
        let text = preview.text.clone();
        let thumb = (item.kind == ItemKind::Image)
            .then(|| self.thumbnail(item.id, cx))
            .flatten();
        let content = match (text, thumb) {
            (Some(text), _) => div()
                .id("preview-text")
                .size_full()
                .overflow_y_scroll()
                .p_3()
                .text_sm()
                .child(text)
                .into_any_element(),
            (None, Some(thumb)) => img(thumb)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            (None, None) => div()
                .p_3()
                .child(self.render_card_body(&item, None, 12))
                .into_any_element(),
        };
        Some(
            div()
                .absolute()
                .top(px(HEADER_HEIGHT + SEARCH_HEIGHT + TABS_HEIGHT))
                .bottom(px(FOOTER_HEIGHT + 8.))
                .left(px(PAD))
                .right(px(PAD))
                .rounded(px(10.))
                .overflow_hidden()
                .bg(theme::card_selected())
                .border_1()
                .border_color(theme::accent())
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(28.))
                        .flex_none()
                        .px_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(theme::border())
                        .text_xs()
                        .text_color(theme::text_muted())
                        .child(format!(
                            "{} · {}",
                            item.display_title(),
                            item.source_app_name.clone().unwrap_or_default()
                        ))
                        .child("Space to close"),
                )
                .child(div().flex_1().overflow_hidden().child(content))
                .into_any_element(),
        )
    }
}

impl Focusable for Panel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Panel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.seen_revision.is_none() {
            self.reload(cx);
        }
        self.first_visible = self.first_visible_card();
        let header = self.render_header(cx).into_any_element();
        let search = self.render_search(cx).into_any_element();
        let tabs = self.render_tabs(cx).into_any_element();
        let items = self.render_list(cx);
        let footer = self.render_footer().into_any_element();
        let card_menu = self.render_card_menu(cx);
        let tab_menu = self.render_tab_menu(cx);
        let preview = self.render_preview(cx);
        let confirm = self.render_confirm(cx);

        div()
            .id("popup")
            .key_context("Popup")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::popup_bg())
            .rounded(px(12.))
            .border_1()
            .border_color(theme::border())
            .overflow_hidden()
            .text_color(theme::text())
            .on_key_down(cx.listener(Self::on_key_down))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                this.modifiers = event.modifiers;
                cx.notify();
            }))
            .on_mouse_move(
                cx.listener(|this, event: &MouseMoveEvent, _, cx| this.drag_move(event, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None),
            )
            .child(header)
            .child(search)
            .child(tabs)
            .child(items)
            .child(footer)
            .children(preview)
            .children(card_menu)
            .children(tab_menu)
            .children(confirm)
    }
}

// ---- small building blocks -------------------------------------------------

fn caret() -> impl IntoElement {
    div().w(px(1.5)).h(px(15.)).mx(px(1.)).bg(theme::accent())
}

fn chip(selected: bool) -> gpui::Div {
    div()
        .h(px(26.))
        .px_2p5()
        .flex_none()
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap_1p5()
        .text_sm()
        .whitespace_nowrap()
        .cursor_pointer()
        .when(selected, |d| {
            d.bg(theme::surface_hover())
                .text_color(theme::text())
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!selected, |d| {
            d.text_color(theme::text_muted())
                .hover(|s| s.bg(theme::surface()).text_color(theme::text()))
        })
}

fn icon_button(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(24.))
        .flex_none()
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(theme::text_muted())
        .hover(|s| s.bg(theme::surface_hover()).text_color(theme::text()))
        .child(label)
}

fn status_chip(label: &str, color: Rgba) -> impl IntoElement {
    div()
        .px_1p5()
        .rounded(px(5.))
        .border_1()
        .border_color(color)
        .text_xs()
        .text_color(color)
        .whitespace_nowrap()
        .child(label.to_string())
}

fn menu_container(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w(px(230.))
        .py_1()
        .rounded(px(8.))
        .bg(rgb(0x2c2c2f))
        .border_1()
        .border_color(theme::border())
        .shadow_lg()
        .text_sm()
        .text_color(theme::text())
        .flex()
        .flex_col()
}

fn menu_row(
    id: &'static str,
    label: &str,
    hint: &str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .mx_1()
        .px_2()
        .h(px(24.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .justify_between()
        .cursor_pointer()
        .hover(|s| s.bg(theme::accent()))
        .child(label.to_string())
        .child(
            div()
                .text_xs()
                .text_color(theme::text_muted())
                .child(hint.to_string()),
        )
        .on_click(on_click)
        .into_any_element()
}

fn separator() -> AnyElement {
    div()
        .my_1()
        .mx_2()
        .h(px(1.))
        .bg(theme::border())
        .into_any_element()
}

fn button(id: &'static str, label: &str, primary: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_3()
        .h(px(26.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .text_sm()
        .cursor_pointer()
        .when(primary, |d| d.bg(theme::danger()).text_color(rgb(0xffffff)))
        .when(!primary, |d| {
            d.bg(theme::surface_hover()).text_color(theme::text())
        })
        .child(label.to_string())
}

#[cfg(test)]
mod tests {
    //! Drive the real popup through GPUI's key dispatch, headlessly.

    use copywren_core::classify::classify;
    use copywren_core::model::{ClipContents, SourceApp, Tab};
    use copywren_core::store::NewClip;
    use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};

    use super::*;
    use crate::state::{AppState, GlobalState};

    struct Fixture {
        _dir: tempfile::TempDir,
        state: Entity<AppState>,
    }

    fn setup(cx: &mut TestAppContext) -> (Fixture, Entity<Panel>, &mut VisualTestContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let state = cx.update(|cx| {
            let state = cx.new(|_| AppState::open(path).unwrap());
            cx.set_global(GlobalState(state.clone()));
            state
        });
        state.update(cx, |state, _| {
            for (i, (text, app)) in [
                ("https://example.com/meeting-slides", "Mail"),
                ("beta two", "Notes"),
                ("alpha one", "Mail"),
            ]
            .iter()
            .enumerate()
            {
                let contents = ClipContents::plain_text(text);
                let classified = classify(&contents).unwrap();
                let source = SourceApp {
                    bundle_id: None,
                    name: Some(app.to_string()),
                };
                state
                    .store
                    .record_capture(
                        NewClip {
                            classified: &classified,
                            contents: &contents,
                            source: &source,
                        },
                        1_000 + i as i64,
                    )
                    .unwrap();
            }
            state.revision += 1;
            state.panel_visible = true;
        });
        let (panel, cx) = cx.add_window_view(Panel::new);
        panel.update_in(cx, |panel, window, cx| panel.did_show(window, cx));
        cx.run_until_parked();
        (Fixture { _dir: dir, state }, panel, cx)
    }

    fn visible_texts(panel: &Entity<Panel>, cx: &mut VisualTestContext) -> Vec<String> {
        panel.read_with(cx, |panel, _| {
            panel
                .visible
                .iter()
                .map(|&i| panel.items[i].preview_text.clone())
                .collect()
        })
    }

    #[gpui::test]
    fn newest_first_and_arrow_keys(cx: &mut TestAppContext) {
        let (_f, panel, cx) = setup(cx);
        assert_eq!(
            visible_texts(&panel, cx),
            [
                "alpha one",
                "beta two",
                "https://example.com/meeting-slides"
            ]
        );
        cx.simulate_keystrokes("down down");
        assert_eq!(panel.read_with(cx, |p, _| p.selected), 2);
        cx.simulate_keystrokes("down");
        assert_eq!(
            panel.read_with(cx, |p, _| p.selected),
            2,
            "selection stops at the end"
        );
        cx.simulate_keystrokes("cmd-up");
        assert_eq!(panel.read_with(cx, |p, _| p.selected), 0);
        cx.simulate_keystrokes("end");
        assert_eq!(panel.read_with(cx, |p, _| p.selected), 2);
        cx.simulate_keystrokes("pageup");
        assert_eq!(panel.read_with(cx, |p, _| p.selected), 0);
    }

    #[gpui::test]
    fn typing_searches_and_escape_clears(cx: &mut TestAppContext) {
        let (f, panel, cx) = setup(cx);
        cx.simulate_input("beta");
        assert_eq!(panel.read_with(cx, |p, _| p.query.clone()), "beta");
        assert_eq!(visible_texts(&panel, cx), ["beta two"]);

        cx.simulate_keystrokes("backspace backspace backspace backspace");
        assert_eq!(visible_texts(&panel, cx).len(), 3);

        // Smart tokens: an app name plus a type word.
        cx.simulate_input("mail link");
        assert_eq!(
            visible_texts(&panel, cx),
            ["https://example.com/meeting-slides"]
        );

        cx.simulate_keystrokes("escape");
        assert_eq!(panel.read_with(cx, |p, _| p.query.clone()), "");
        assert!(
            f.state.read_with(cx, |s, _| s.panel_visible),
            "first Esc only clears the search"
        );
        cx.simulate_keystrokes("escape");
        assert!(
            !f.state.read_with(cx, |s, _| s.panel_visible),
            "second Esc closes the popup"
        );
    }

    #[gpui::test]
    fn space_previews_only_when_not_searching(cx: &mut TestAppContext) {
        let (_f, panel, cx) = setup(cx);
        cx.simulate_keystrokes("space");
        assert!(panel.read_with(cx, |p, _| p.preview.is_some()));
        cx.simulate_keystrokes("space");
        assert!(panel.read_with(cx, |p, _| p.preview.is_none()));
        cx.simulate_input("alpha");
        cx.simulate_keystrokes("space");
        assert_eq!(panel.read_with(cx, |p, _| p.query.clone()), "alpha ");
        assert!(panel.read_with(cx, |p, _| p.preview.is_none()));
    }

    #[gpui::test]
    fn cmd_backspace_deletes_selected(cx: &mut TestAppContext) {
        let (f, panel, cx) = setup(cx);
        cx.simulate_keystrokes("down cmd-backspace");
        assert_eq!(
            visible_texts(&panel, cx),
            ["alpha one", "https://example.com/meeting-slides"]
        );
        assert_eq!(
            f.state
                .read_with(cx, |s, _| s.store.history_count().unwrap()),
            2
        );
        assert_eq!(panel.read_with(cx, |p, _| p.selected), 1);
    }

    #[gpui::test]
    fn rename_item_inline(cx: &mut TestAppContext) {
        let (f, panel, cx) = setup(cx);
        cx.simulate_keystrokes("cmd-r");
        cx.simulate_input("Greeting");
        cx.simulate_keystrokes("enter");
        let id = panel.read_with(cx, |p, _| p.selected_item().unwrap().id);
        let title = f
            .state
            .read_with(cx, |s, _| s.store.item(id).unwrap().unwrap().title);
        assert_eq!(title.as_deref(), Some("Greeting"));
        // Renamed items are found by their title.
        cx.simulate_input("greet");
        assert_eq!(visible_texts(&panel, cx), ["alpha one"]);
    }

    #[gpui::test]
    fn pinboards_via_keyboard(cx: &mut TestAppContext) {
        let (f, panel, cx) = setup(cx);
        cx.simulate_keystrokes("cmd-n");
        cx.simulate_input("Work");
        cx.simulate_keystrokes("enter");
        let boards = f.state.read_with(cx, |s, _| s.store.pinboards().unwrap());
        assert_eq!(boards.len(), 1);
        assert_eq!(boards[0].name, "Work");
        assert_eq!(
            panel.read_with(cx, |p, _| p.tab),
            Tab::Pinboard(boards[0].id)
        );
        assert!(visible_texts(&panel, cx).is_empty());

        // Left/right switch tabs.
        cx.simulate_keystrokes("left");
        assert_eq!(panel.read_with(cx, |p, _| p.tab), Tab::History);
        let id = panel.read_with(cx, |p, _| p.selected_item().unwrap().id);
        panel.update(cx, |p, cx| p.pin(id, boards[0].id, cx));
        cx.simulate_keystrokes("right");
        assert_eq!(visible_texts(&panel, cx), ["alpha one"]);
        cx.simulate_keystrokes("tab");
        assert_eq!(visible_texts(&panel, cx).len(), 3);
    }

    #[gpui::test]
    fn quick_paste_badges_follow_the_modifier(cx: &mut TestAppContext) {
        let (_f, panel, cx) = setup(cx);
        cx.simulate_modifiers_change(Modifiers {
            platform: true,
            ..Modifiers::default()
        });
        assert!(panel.read_with(cx, |p, _| p.modifiers.platform));
        cx.simulate_modifiers_change(Modifiers::default());
        assert!(!panel.read_with(cx, |p, _| p.modifiers.platform));
    }

    #[test]
    fn thumbnails_fit_without_upscaling() {
        assert_eq!(thumb_size(Some((100, 50))), (100.0, 50.0));
        let (w, h) = thumb_size(Some((4000, 1000)));
        assert!((w - THUMB_MAX.0).abs() < 0.01 && h < THUMB_MAX.1);
        let (w, h) = thumb_size(Some((500, 2000)));
        assert!((h - THUMB_MAX.1).abs() < 0.01 && w < THUMB_MAX.0);
        assert_eq!(thumb_size(None), (THUMB_MAX.0, 90.0));
    }
}
