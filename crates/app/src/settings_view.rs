//! The Settings window (SPEC §7).

use copywren_core::now_ms;
use copywren_core::retention::Retention;
use copywren_core::settings::Settings;
use copywren_core::shortcut::{ModifierKey, Shortcut};
use copywren_macos::{system, workspace};
use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    Keystroke, PromptLevel, Render, SharedString, Subscription, Window, div, prelude::*, px, rgb,
    rgba,
};

use crate::state::AppState;
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    General,
    History,
    Privacy,
    Shortcuts,
}

impl Section {
    const ALL: [Section; 4] = [
        Section::General,
        Section::History,
        Section::Privacy,
        Section::Shortcuts,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::History => "History",
            Section::Privacy => "Privacy",
            Section::Shortcuts => "Shortcuts",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShortcutField {
    Activation,
    PasteQueue,
}

pub struct SettingsView {
    focus: FocusHandle,
    section: Section,
    recording: Option<ShortcutField>,
    bundle_input: String,
    bundle_input_focused: bool,
    message: Option<SharedString>,
    _subscription: Subscription,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let focus = cx.focus_handle();
        window.focus(&focus);
        Self {
            focus,
            section: Section::General,
            recording: None,
            bundle_input: String::new(),
            bundle_input_focused: false,
            message: None,
            _subscription: cx.observe(&state, |_, _, cx| cx.notify()),
        }
    }

    /// Switches to the `index`-th tab (for the documentation recorder).
    #[cfg(feature = "demo")]
    pub fn demo_section(&mut self, index: usize, cx: &mut Context<Self>) {
        self.section = Section::ALL[index.min(Section::ALL.len() - 1)];
        cx.notify();
    }

    fn update(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Settings)) {
        AppState::global(cx).update(cx, |state, cx| state.update_settings(cx, change));
    }

    pub fn confirm_erase(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.section = Section::History;
        let answer = window.prompt(
            PromptLevel::Warning,
            "Erase your clipboard history?",
            Some("Pinboards and the items on them are kept. This can't be undone."),
            &["Erase History", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |_, cx| {
            if answer.await == Ok(0) {
                cx.update(|_, cx| {
                    AppState::global(cx).update(cx, |state, cx| state.erase_history(cx));
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn set_retention(&mut self, retention: Retention, window: &mut Window, cx: &mut Context<Self>) {
        let state = AppState::global(cx);
        let (current, doomed) = {
            let state = state.read(cx);
            let doomed = retention
                .cutoff(now_ms())
                .and_then(|cutoff| state.store.count_history_older_than(cutoff).ok())
                .unwrap_or(0);
            (state.settings.keep_history, doomed)
        };
        if !retention.is_shorter_than(current) || doomed == 0 {
            self.update(cx, |s| s.keep_history = retention);
            return;
        }
        let detail = format!(
            "{} older than the new limit will be deleted. Pinboards are kept.",
            copywren_core::text::count_label(doomed as i64, "item")
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            "Apply the shorter history limit?",
            Some(&detail),
            &["Delete Older Items", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update(cx, |this, cx| {
                    this.update(cx, |s| s.keep_history = retention)
                })
                .ok();
            }
        })
        .detach();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if let Some(field) = self.recording {
            cx.stop_propagation();
            if keystroke.key == "escape" && !keystroke.modifiers.modified() {
                self.recording = None;
            } else if let Some(shortcut) = shortcut_from(keystroke) {
                self.recording = None;
                self.message = None;
                self.update(cx, move |s| match field {
                    ShortcutField::Activation => s.activation_shortcut = shortcut,
                    ShortcutField::PasteQueue => s.paste_queue_shortcut = shortcut,
                });
            } else {
                self.message = Some("Use ⌘, ⌃ or ⌥ together with a key.".into());
            }
            cx.notify();
            return;
        }
        if self.bundle_input_focused {
            cx.stop_propagation();
            match keystroke.key.as_str() {
                "enter" => self.add_ignored_app(None, cx),
                "escape" => self.bundle_input_focused = false,
                "backspace" => {
                    self.bundle_input.pop();
                }
                "v" if keystroke.modifiers.platform => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        self.bundle_input.push_str(text.trim());
                    }
                }
                _ if !keystroke.modifiers.platform && !keystroke.modifiers.control => {
                    if let Some(c) = &keystroke.key_char
                        && c.chars().all(|c| !c.is_control())
                    {
                        self.bundle_input.push_str(c);
                    }
                }
                _ => {}
            }
            cx.notify();
        }
    }

    fn add_ignored_app(&mut self, bundle: Option<String>, cx: &mut Context<Self>) {
        let bundle = bundle.unwrap_or_else(|| self.bundle_input.trim().to_string());
        if bundle.is_empty() {
            return;
        }
        self.bundle_input.clear();
        self.update(cx, move |s| {
            if !s
                .ignored_apps
                .iter()
                .any(|a| a.eq_ignore_ascii_case(&bundle))
            {
                s.ignored_apps.push(bundle);
            }
        });
    }

    // ---- sections ----------------------------------------------------------

    fn render_general(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let settings = AppState::global(cx).read(cx).settings.clone();
        let trusted = AppState::global(cx).read(cx).accessibility_trusted();
        let login = system::is_open_at_login();
        vec![
            toggle_row(
                "direct-paste",
                "Paste directly into the active app",
                Some(if trusted {
                    "Accessibility access is allowed.".into()
                } else {
                    "Needs Accessibility access. Without it, items are only copied.".into()
                }),
                settings.direct_paste,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.update(cx, |s| s.direct_paste = !s.direct_paste)
                }),
            ),
            if trusted {
                div().into_any_element()
            } else {
                div()
                    .pl(px(4.))
                    .child(
                        link_button("open-ax", "Open Accessibility Settings…").on_click(
                            cx.listener(|_, _: &ClickEvent, _, _| {
                                copywren_macos::input::is_trusted(true);
                                system::open_accessibility_settings();
                            }),
                        ),
                    )
                    .into_any_element()
            },
            toggle_row(
                "plain-text",
                "Always paste as plain text",
                None,
                settings.always_plain_text,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.update(cx, |s| s.always_plain_text = !s.always_plain_text)
                }),
            ),
            toggle_row(
                "sounds",
                "Sound effects",
                None,
                settings.sound_effects,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.update(cx, |s| s.sound_effects = !s.sound_effects)
                }),
            ),
            toggle_row(
                "menu-bar",
                "Show in menu bar",
                Some("When hidden, open Copywren with its shortcut.".into()),
                settings.show_in_menu_bar,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.update(cx, |s| s.show_in_menu_bar = !s.show_in_menu_bar)
                }),
            ),
            toggle_row(
                "login",
                "Open at login",
                (!system::is_bundled()).then(|| "Available when running Copywren.app.".into()),
                login,
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.message = system::set_open_at_login(!login).err().map(Into::into);
                    cx.notify();
                }),
            ),
        ]
    }

    fn render_history(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (keep, count) = {
            let state = AppState::global(cx).read(cx);
            (
                state.settings.keep_history,
                state.store.history_count().unwrap_or(0),
            )
        };
        vec![
            label_row("Keep history", None),
            div()
                .flex()
                .gap_1()
                .children(Retention::ALL.iter().map(|&r| {
                    segment(("keep", r as u64), r.label(), r == keep).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| this.set_retention(r, window, cx),
                    ))
                }))
                .into_any_element(),
            div()
                .text_sm()
                .text_color(theme::text_muted())
                .child(format!(
                    "{} in history. Pinboards are never deleted automatically.",
                    copywren_core::text::count_label(count as i64, "item")
                ))
                .into_any_element(),
            div()
                .pt_2()
                .child(link_button("erase", "Erase History…").on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.confirm_erase(window, cx)),
                ))
                .into_any_element(),
        ]
    }

    fn render_privacy(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (settings, recent) = {
            let state = AppState::global(cx).read(cx);
            (
                state.settings.clone(),
                state.store.recent_source_apps(12).unwrap_or_default(),
            )
        };
        let mut rows = vec![
            toggle_row(
                "confidential",
                "Ignore confidential content",
                Some("Skip passwords that apps mark as concealed.".into()),
                settings.ignore_confidential,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.update(cx, |s| s.ignore_confidential = !s.ignore_confidential)
                }),
            ),
            toggle_row(
                "transient",
                "Ignore transient content",
                Some("Skip temporary data that apps generate.".into()),
                settings.ignore_transient,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.update(cx, |s| s.ignore_transient = !s.ignore_transient)
                }),
            ),
            toggle_row(
                "screen-sharing",
                "Show during screen sharing",
                Some("When off, the panel is hidden from screenshots and screen sharing.".into()),
                settings.show_during_screen_sharing,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.update(cx, |s| {
                        s.show_during_screen_sharing = !s.show_during_screen_sharing
                    })
                }),
            ),
            label_row(
                "Ignore applications",
                Some("Nothing copied in these apps is saved."),
            ),
        ];
        for bundle in settings.ignored_apps.clone() {
            let name = workspace::app_name(&bundle).unwrap_or_else(|| bundle.clone());
            let remove = bundle.clone();
            rows.push(
                div()
                    .h(px(30.))
                    .px_2()
                    .rounded(px(6.))
                    .bg(theme::surface())
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(name)
                            .child(div().text_color(theme::text_muted()).child(bundle.clone())),
                    )
                    .child(
                        link_button(SharedString::from(format!("remove-{bundle}")), "Remove")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let remove = remove.clone();
                                this.update(cx, move |s| s.ignored_apps.retain(|a| *a != remove));
                            })),
                    )
                    .into_any_element(),
            );
        }
        let focused = self.bundle_input_focused;
        rows.push(
            div()
                .flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .id("bundle-input")
                        .flex_1()
                        .h(px(28.))
                        .px_2()
                        .rounded(px(6.))
                        .bg(theme::surface())
                        .border_1()
                        .border_color(if focused {
                            theme::accent()
                        } else {
                            theme::border()
                        })
                        .flex()
                        .items_center()
                        .text_sm()
                        .cursor_text()
                        .child(if self.bundle_input.is_empty() && !focused {
                            div()
                                .text_color(theme::text_muted())
                                .child("Bundle ID, e.g. com.agilebits.onepassword7")
                        } else {
                            div().child(self.bundle_input.clone())
                        })
                        .when(focused, |d| {
                            d.child(div().w(px(1.5)).h(px(14.)).bg(theme::accent()))
                        })
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.bundle_input_focused = true;
                            this.recording = None;
                            window.focus(&this.focus);
                            cx.notify();
                        })),
                )
                .child(segment("add-bundle", "Add", false).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.add_ignored_app(None, cx)),
                ))
                .into_any_element(),
        );
        let suggestions: Vec<(String, String)> = recent
            .into_iter()
            .filter(|(b, _)| {
                !settings
                    .ignored_apps
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case(b))
            })
            .map(|(b, n)| (b.clone(), n.unwrap_or(b)))
            .collect();
        if !suggestions.is_empty() {
            rows.push(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::text_muted())
                            .child("Recent apps:"),
                    )
                    .children(suggestions.into_iter().map(|(bundle, name)| {
                        segment(
                            SharedString::from(format!("suggest-{bundle}")),
                            &format!("+ {name}"),
                            false,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.add_ignored_app(Some(bundle.clone()), cx)
                            },
                        ))
                    }))
                    .into_any_element(),
            );
        }
        rows
    }

    fn render_shortcuts(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (settings, error) = {
            let state = AppState::global(cx).read(cx);
            (state.settings.clone(), state.hotkey_error.clone())
        };
        let recorder =
            |this: &Self, field: ShortcutField, shortcut: &Shortcut, cx: &mut Context<Self>| {
                let recording = this.recording == Some(field);
                div()
                    .id(match field {
                        ShortcutField::Activation => "rec-activation",
                        ShortcutField::PasteQueue => "rec-queue",
                    })
                    .w(px(150.))
                    .h(px(28.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if recording {
                        theme::accent()
                    } else {
                        theme::border()
                    })
                    .bg(theme::surface())
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .cursor_pointer()
                    .child(if recording {
                        "Press shortcut…".to_string()
                    } else {
                        shortcut.symbol()
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.recording = Some(field);
                        this.bundle_input_focused = false;
                        window.focus(&this.focus);
                        cx.notify();
                    }))
            };
        let modifier_picker =
            |id: &'static str,
             current: ModifierKey,
             options: &[ModifierKey],
             cx: &mut Context<Self>,
             apply: fn(&mut Settings, ModifierKey)| {
                div().flex().gap_1().children(options.iter().map(|&m| {
                    segment((id, m as u64), m.symbol(), m == current).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| this.update(cx, move |s| apply(s, m)),
                    ))
                }))
            };
        let mut rows = vec![
            setting_row(
                "Open Copywren",
                recorder(
                    self,
                    ShortcutField::Activation,
                    &settings.activation_shortcut,
                    cx,
                )
                .into_any_element(),
            ),
            setting_row(
                "Paste Queue",
                recorder(
                    self,
                    ShortcutField::PasteQueue,
                    &settings.paste_queue_shortcut,
                    cx,
                )
                .into_any_element(),
            ),
            setting_row(
                "Quick paste (+ 1–9)",
                modifier_picker(
                    "quick",
                    settings.quick_paste_modifier,
                    &[ModifierKey::Cmd, ModifierKey::Alt, ModifierKey::Ctrl],
                    cx,
                    |s, m| s.quick_paste_modifier = m,
                )
                .into_any_element(),
            ),
            setting_row(
                "Plain text (+ Return)",
                modifier_picker(
                    "plain",
                    settings.plain_text_modifier,
                    &[ModifierKey::Shift, ModifierKey::Alt, ModifierKey::Ctrl],
                    cx,
                    |s, m| s.plain_text_modifier = m,
                )
                .into_any_element(),
            ),
        ];
        if let Some(error) = error {
            rows.push(
                div()
                    .text_sm()
                    .text_color(theme::danger())
                    .child(error)
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .pt_2()
                .child(
                    link_button("reset-shortcuts", "Reset shortcuts to default").on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            let defaults = Settings::default();
                            this.update(cx, move |s| {
                                s.activation_shortcut = defaults.activation_shortcut;
                                s.paste_queue_shortcut = defaults.paste_queue_shortcut;
                                s.quick_paste_modifier = defaults.quick_paste_modifier;
                                s.plain_text_modifier = defaults.plain_text_modifier;
                            });
                        }),
                    ),
                )
                .into_any_element(),
        );
        rows.push(
            div()
                .pt_3()
                .text_xs()
                .text_color(theme::text_muted())
                .child("In the popup: ↑/↓ select · ←/→ switch pinboard · Return or click to paste · Space preview · ⌘C copy · ⌘⌫ delete · ⌘O open · ⌘R rename · type to search · Esc close")
                .into_any_element(),
        );
        rows
    }
}

/// Converts a recorded key press into a global shortcut, if it qualifies.
fn shortcut_from(keystroke: &Keystroke) -> Option<Shortcut> {
    let m = keystroke.modifiers;
    if !(m.platform || m.control || m.alt) {
        return None;
    }
    let key = match keystroke.key.as_str() {
        "enter" => "return",
        "shift" | "control" | "alt" | "platform" | "function" | "capslock" => return None,
        other => other,
    };
    let text = format!(
        "{}{}{}{}{key}",
        if m.control { "ctrl+" } else { "" },
        if m.alt { "alt+" } else { "" },
        if m.shift { "shift+" } else { "" },
        if m.platform { "cmd+" } else { "" },
    );
    text.parse().ok()
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = match self.section {
            Section::General => self.render_general(cx),
            Section::History => self.render_history(cx),
            Section::Privacy => self.render_privacy(cx),
            Section::Shortcuts => self.render_shortcuts(cx),
        };
        let data_dir = AppState::global(cx).read(cx).data_dir.clone();

        div()
            .id("settings-root")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e1e))
            .text_color(theme::text())
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                div()
                    .pt(px(36.))
                    .pb_3()
                    .flex()
                    .justify_center()
                    .gap_1()
                    .border_b_1()
                    .border_color(theme::border())
                    .children(Section::ALL.iter().map(|&section| {
                        segment(
                            ("section", section as u64),
                            section.label(),
                            section == self.section,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.section = section;
                                this.recording = None;
                                this.bundle_input_focused = false;
                                this.message = None;
                                cx.notify();
                            },
                        ))
                    })),
            )
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_6()
                    .py_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children(rows)
                    .when_some(self.message.clone(), |d, message| {
                        d.child(div().text_sm().text_color(rgb(0xff9f0a)).child(message))
                    }),
            )
            .child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .px_6()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(theme::border())
                    .text_xs()
                    .text_color(theme::text_muted())
                    .child(format!("Copywren {}", env!("CARGO_PKG_VERSION")))
                    .child(
                        link_button("data-dir", "Show Data in Finder").on_click(cx.listener(
                            move |_, _: &ClickEvent, _, _| {
                                workspace::reveal_in_finder(&[data_dir.display().to_string()]);
                            },
                        )),
                    ),
            )
    }
}

// ---- controls --------------------------------------------------------------

fn toggle_row(
    id: &'static str,
    title: &str,
    subtitle: Option<String>,
    on: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .cursor_pointer()
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().text_sm().child(title.to_string()))
                .when_some(subtitle, |d, s| {
                    d.child(div().text_xs().text_color(theme::text_muted()).child(s))
                }),
        )
        .child(switch(on))
        .on_click(on_click)
        .into_any_element()
}

fn switch(on: bool) -> impl IntoElement {
    div()
        .w(px(34.))
        .h(px(20.))
        .flex_none()
        .rounded_full()
        .p(px(2.))
        .flex()
        .when(on, |d| d.justify_end().bg(theme::accent()))
        .when(!on, |d| d.justify_start().bg(rgba(0xffffff33)))
        .child(div().size(px(16.)).rounded_full().bg(rgb(0xffffff)))
}

fn label_row(title: &str, subtitle: Option<&str>) -> AnyElement {
    div()
        .pt_2()
        .flex()
        .flex_col()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_string()),
        )
        .when_some(subtitle, |d, s| {
            d.child(
                div()
                    .text_xs()
                    .text_color(theme::text_muted())
                    .child(s.to_string()),
            )
        })
        .into_any_element()
}

fn setting_row(title: &str, control: AnyElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(div().text_sm().child(title.to_string()))
        .child(control)
        .into_any_element()
}

fn segment(
    id: impl Into<gpui::ElementId>,
    label: &str,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_3()
        .h(px(26.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .text_sm()
        .cursor_pointer()
        .when(selected, |d| {
            d.bg(theme::accent()).text_color(rgb(0xffffff))
        })
        .when(!selected, |d| {
            d.bg(theme::surface())
                .text_color(theme::text())
                .hover(|s| s.bg(theme::surface_hover()))
        })
        .child(label.to_string())
}

fn link_button(id: impl Into<gpui::ElementId>, label: &str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .text_sm()
        .text_color(theme::accent())
        .cursor_pointer()
        .hover(|s| s.text_color(rgb(0x409cff)))
        .child(label.to_string())
}
