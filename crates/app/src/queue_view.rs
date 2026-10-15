//! The Paste Queue window (SPEC §6): the queue of snippets ⌘V will paste.

use copywren_core::queue::QueueDirection;
use gpui::{
    ClickEvent, Context, FontWeight, Render, Subscription, Window, div, prelude::*, px, rgb, rgba,
};

use crate::state::AppState;
use crate::theme;

pub struct QueueView {
    _subscription: Subscription,
}

impl QueueView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        Self {
            _subscription: cx.observe(&state, |_, _, cx| cx.notify()),
        }
    }
}

impl Render for QueueView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = AppState::global(cx);
        let state = state.read(cx);
        let direction = state.queue.direction;
        let snippets: Vec<(String, &'static str)> = state
            .queue
            .in_paste_order()
            .iter()
            .map(|s| {
                let line = s
                    .preview_text
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or(s.kind.label())
                    .trim()
                    .chars()
                    .take(80)
                    .collect();
                (line, s.kind.label())
            })
            .collect();

        let header = div()
            .h(px(40.))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_sm()
                    .child(format!("Paste Queue · {}", snippets.len())),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        icon_button(
                            "direction",
                            match direction {
                                QueueDirection::Fifo => "↓",
                                QueueDirection::Lifo => "↑",
                            },
                        )
                        .on_click(cx.listener(
                            |_, _: &ClickEvent, _, cx| {
                                AppState::global(cx)
                                    .update(cx, |s, cx| s.toggle_queue_direction(cx));
                            },
                        )),
                    )
                    .child(icon_button("close", "×").on_click(cx.listener(
                        |_, _: &ClickEvent, _, cx| {
                            AppState::global(cx).update(cx, |s, cx| s.close_queue(cx));
                        },
                    ))),
            );

        let list = if snippets.is_empty() {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .px_4()
                .text_center()
                .text_sm()
                .text_color(theme::text_muted())
                .child("Copy a few things.")
                .child("Then press ⌘V to paste them one by one.")
                .into_any_element()
        } else {
            div()
                .id("snippets")
                .flex_1()
                .overflow_y_scroll()
                .py_1()
                .children(
                    snippets
                        .into_iter()
                        .enumerate()
                        .map(|(index, (line, kind))| {
                            div()
                                .id(("snippet", index))
                                .mx_2()
                                .px_2()
                                .h(px(44.))
                                .rounded(px(8.))
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(index == 0, |d| d.bg(rgba(0x0a84ff33)))
                                .hover(|s| s.bg(theme::surface()))
                                .child(
                                    div()
                                        .w(px(18.))
                                        .text_xs()
                                        .text_color(theme::text_muted())
                                        .child(format!("{}", index + 1)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .flex()
                                        .flex_col()
                                        .overflow_hidden()
                                        .child(
                                            div()
                                                .text_sm()
                                                .whitespace_nowrap()
                                                .overflow_hidden()
                                                .child(line),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme::text_muted())
                                                .child(kind),
                                        ),
                                )
                                .child(icon_button(("remove", index), "×").on_click(cx.listener(
                                    move |_, _: &ClickEvent, _, cx| {
                                        AppState::global(cx)
                                            .update(cx, |s, cx| s.remove_queue_snippet(index, cx));
                                    },
                                )))
                        }),
                )
                .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::popup_bg())
            .rounded(px(12.))
            .border_1()
            .border_color(theme::border())
            .text_color(theme::text())
            .child(header)
            .child(list)
            .child(
                div()
                    .h(px(30.))
                    .flex_none()
                    .px_3()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(theme::border())
                    .text_xs()
                    .text_color(theme::text_muted())
                    .child(direction.label()),
            )
    }
}

fn icon_button(id: impl Into<gpui::ElementId>, label: &'static str) -> gpui::Stateful<gpui::Div> {
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
        .hover(|s| s.bg(theme::surface_hover()).text_color(rgb(0xffffff)))
        .child(label)
}
