//! Copywren: a clipboard manager for macOS. Behavior is specified in SPEC.md.

#[cfg(feature = "demo")]
mod demo;
mod hotkeys;
mod panel;
mod queue_view;
mod settings_view;
mod state;
mod theme;
mod util;

use copywren_macos::system;
use copywren_macos::window::NativeWindow;
use gpui::{
    App, AppContext, Application, Bounds, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, point, px, size,
};

use crate::panel::Panel;
use crate::state::{AppState, GlobalState};
use crate::util::native_window;

fn main() {
    Application::new().run(|cx| {
        system::become_accessory_app();

        // `cargo run -p copywren --features demo` with COPYWREN_DEMO_DIR set
        // records the documentation screenshots and video instead.
        #[cfg(feature = "demo")]
        if let Some(dir) = std::env::var_os("COPYWREN_DEMO_DIR") {
            demo::run(dir.into(), cx);
            return;
        }

        let state = match AppState::default_data_dir().and_then(AppState::open) {
            Ok(state) => cx.new(|_| state),
            Err(e) => {
                eprintln!("Copywren could not start: {e:#}");
                cx.quit();
                return;
            }
        };
        cx.set_global(GlobalState(state.clone()));

        let (panel, native) = match open_panel(cx) {
            Ok(panel) => panel,
            Err(e) => {
                eprintln!("Copywren could not open its popup: {e:#}");
                cx.quit();
                return;
            }
        };

        state.update(cx, |state, cx| {
            state.panel = Some(panel);
            state.panel_native = native;
            state.start(cx);
        });

        // Launch flags, handy for scripts and debugging.
        for arg in std::env::args().skip(1) {
            match arg.as_str() {
                "--show" => state.update(cx, |state, cx| state.show_panel(cx)),
                "--queue" => state.update(cx, |state, cx| state.toggle_queue(cx)),
                "--settings" => state.update(cx, |state, cx| state.open_settings(false, cx)),
                _ => {}
            }
        }
    });
}

/// Creates the popup window, hidden. It is shown on demand next to the text
/// cursor; its background is transparent so the content can round its
/// corners.
fn open_panel(cx: &mut App) -> anyhow::Result<(WindowHandle<Panel>, Option<NativeWindow>)> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(theme::POPUP_WIDTH), px(theme::POPUP_HEIGHT)),
        ))),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..WindowOptions::default()
    };
    let panel = cx.open_window(options, |window, cx| cx.new(|cx| Panel::new(window, cx)))?;
    let native = panel
        .update(cx, |_, window, _| native_window(window))
        .ok()
        .flatten();
    if let Some(native) = native {
        // AppKit calls back into GPUI from window operations, so run them
        // outside this borrow of the app.
        cx.spawn(async move |_| native.configure_panel()).detach();
    }
    Ok((panel, native))
}
