//! Records the documentation screenshots and video (`--features demo`).
//!
//! Seeds a throw-away data directory with invented clips, then drives the
//! real popup through scripted keystrokes and saves a PNG of Copywren's own
//! windows every 100 ms. The clipboard is never read or written, no global
//! shortcuts are registered and nothing is pasted, so nothing belonging to
//! the person running it can end up in the frames. `tools/demo-render`
//! turns the frames into the README video and screenshots.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use copywren_core::classify::classify;
use copywren_core::model::{ClipContents, ItemId, PasteboardEntry, Representation, SourceApp};
use copywren_core::queue::Snippet;
use copywren_core::settings::Settings;
use copywren_core::store::{NewClip, Store};
use copywren_core::{now_ms, uti};
use copywren_macos::window::NativeWindow;
use gpui::{App, AppContext, AsyncApp, Entity, Keystroke, Modifiers, WindowHandle, point, px};

use crate::panel::Panel;
use crate::state::{AppState, GlobalState};
use crate::util::native_window;

const FRAME_INTERVAL: Duration = Duration::from_millis(100);
const MINUTE: i64 = 60_000;

/// What the video shows at the moment: written next to every frame.
struct Scene {
    step: &'static str,
    keys: &'static str,
    caption: &'static str,
    pasted: bool,
    done: bool,
}

pub fn run(dir: PathBuf, cx: &mut App) {
    if let Err(e) = start(dir, cx) {
        eprintln!("demo recording failed: {e:#}");
        cx.quit();
    }
}

fn start(dir: PathBuf, cx: &mut App) -> Result<()> {
    let data = dir.join("data");
    let frames = dir.join("frames");
    if data.exists() || frames.exists() {
        bail!("{} must be a new, empty directory", dir.display());
    }
    std::fs::create_dir_all(&frames)?;
    std::fs::create_dir_all(dir.join("windows"))?;
    seed(&data)?;

    let state = AppState::open(data)?;
    let state = cx.new(|_| state);
    cx.set_global(GlobalState(state.clone()));
    let (panel, native) = crate::open_panel(cx)?;
    let native = native.context("the popup has no native window")?;
    state.update(cx, |state, _| {
        state.demo = true;
        state.panel = Some(panel);
        state.panel_native = Some(native);
        state.warm_app_meta();
    });

    let scene = Rc::new(RefCell::new(Scene {
        step: "intro",
        keys: "",
        caption: "Copy as usual. Copywren keeps everything.",
        pasted: false,
        done: false,
    }));

    let recorder_scene = scene.clone();
    let recorder_state = state.clone();
    let manifest = dir.join("manifest.tsv");
    cx.spawn(async move |cx| {
        record(
            cx,
            recorder_state,
            native,
            recorder_scene,
            &frames,
            &manifest,
        )
        .await
    })
    .detach();

    cx.spawn(async move |cx| {
        if let Err(e) = script(cx, state, panel, scene, &dir).await {
            eprintln!("demo script failed: {e:#}");
        }
        cx.update(|cx| cx.quit()).ok();
    })
    .detach();
    Ok(())
}

/// Saves a frame every 100 ms until the script is done, plus a manifest line
/// per frame: index, frame file (or "-" while the popup is hidden), step,
/// keys, caption, pasted.
async fn record(
    cx: &mut AsyncApp,
    state: Entity<AppState>,
    native: NativeWindow,
    scene: Rc<RefCell<Scene>>,
    frames: &Path,
    manifest: &Path,
) {
    let mut lines = String::new();
    for index in 0.. {
        cx.background_executor().timer(FRAME_INTERVAL).await;
        if scene.borrow().done {
            break;
        }
        let visible = state.read_with(cx, |s, _| s.panel_visible).unwrap_or(false);
        let file = match visible.then(|| native.snapshot_png()).flatten() {
            Some(png) => {
                let name = format!("{index:05}.png");
                let _ = std::fs::write(frames.join(&name), png);
                name
            }
            None => "-".to_string(),
        };
        let scene = scene.borrow();
        lines.push_str(&format!(
            "{index}\t{file}\t{}\t{}\t{}\t{}\n",
            scene.step, scene.keys, scene.caption, scene.pasted
        ));
    }
    let _ = std::fs::write(manifest, lines);
}

async fn script(
    cx: &mut AsyncApp,
    state: Entity<AppState>,
    panel: WindowHandle<Panel>,
    scene: Rc<RefCell<Scene>>,
    dir: &Path,
) -> Result<()> {
    let set = |step, keys, caption| {
        let mut scene = scene.borrow_mut();
        scene.step = step;
        scene.keys = keys;
        scene.caption = caption;
    };

    pause(cx, 1600).await;

    set(
        "open",
        "⇧⌘V",
        "Your clipboard history opens right at the text cursor",
    );
    state.update(cx, |s, cx| s.show_panel(cx))?;
    pause(cx, 2600).await;

    set("browse", "↑ ↓", "Browse with the arrow keys");
    for _ in 0..3 {
        keys(cx, panel, "down");
        pause(cx, 550).await;
    }
    for _ in 0..3 {
        keys(cx, panel, "up");
        pause(cx, 350).await;
    }
    pause(cx, 400).await;

    set(
        "search",
        "Type",
        "Just type to search: words, app names or a type like “image”",
    );
    type_text(cx, panel, "slides").await;
    pause(cx, 1700).await;
    for _ in 0.."slides".len() {
        keys(cx, panel, "backspace");
        pause(cx, 60).await;
    }
    type_text(cx, panel, "image").await;
    pause(cx, 1800).await;

    set("search-clear", "Esc", "Esc clears the search");
    keys(cx, panel, "escape");
    pause(cx, 1200).await;

    set("preview", "Space", "Space shows the whole clip");
    keys(cx, panel, "down down");
    pause(cx, 500).await;
    keys(cx, panel, "space");
    pause(cx, 2600).await;
    keys(cx, panel, "space");
    pause(cx, 400).await;

    set("pinboards", "← →", "Pinboards keep the clips you reuse");
    keys(cx, panel, "right");
    pause(cx, 1900).await;
    keys(cx, panel, "right");
    pause(cx, 1900).await;
    keys(cx, panel, "left left");
    pause(cx, 600).await;

    set(
        "quick",
        "⌘1 – ⌘9",
        "Hold ⌘ to quick-paste one of the first nine",
    );
    keys(cx, panel, "cmd-up");
    modifiers(
        cx,
        panel,
        Modifiers {
            platform: true,
            ..Modifiers::default()
        },
    );
    pause(cx, 2300).await;
    modifiers(cx, panel, Modifiers::default());
    pause(cx, 300).await;

    set(
        "menu",
        "⋯",
        "Right-click or ⋯ to pin, rename, open or delete",
    );
    keys(cx, panel, "down");
    pause(cx, 400).await;
    panel.update(cx, |panel, _, cx| {
        panel.demo_open_menu(point(px(250.), px(215.)), cx)
    })?;
    pause(cx, 2800).await;
    keys(cx, panel, "escape");
    pause(cx, 500).await;

    set("paste", "↩", "Return pastes it into the app you were using");
    pause(cx, 900).await;
    // The real Return would write the clipboard and send ⌘V; the demo only
    // pretends, so the person recording keeps their clipboard.
    state.update(cx, |s, cx| s.hide_panel(cx))?;
    scene.borrow_mut().pasted = true;
    pause(cx, 2800).await;

    set("outro", "", "Copywren — free and open source");
    pause(cx, 1800).await;
    scene.borrow_mut().done = true;
    pause(cx, 300).await;

    capture_extra_windows(cx, state, dir).await
}

/// Stills of the Paste Queue and Settings windows.
async fn capture_extra_windows(
    cx: &mut AsyncApp,
    state: Entity<AppState>,
    dir: &Path,
) -> Result<()> {
    state.update(cx, |s, cx| s.toggle_queue(cx))?;
    pause(cx, 600).await;
    state.update(cx, |s, cx| {
        for text in ["Ada Lovelace", "ada@example.com", "+1 555 0100"] {
            s.queue.push(Snippet {
                kind: copywren_core::model::ItemKind::Text,
                preview_text: text.into(),
                contents: ClipContents::plain_text(text),
            });
        }
        cx.notify();
    })?;
    pause(cx, 900).await;
    if let Some(native) = state.read_with(cx, |s, _| s.queue_native())? {
        save(native, &dir.join("windows/queue.png"));
    }
    state.update(cx, |s, cx| s.close_queue(cx))?;

    state.update(cx, |s, cx| s.open_settings(false, cx))?;
    pause(cx, 1500).await;
    let settings = state.read_with(cx, |s, _| s.settings_window())?;
    if let Some(handle) = settings {
        for (section, name) in [(0, "settings-general"), (3, "settings-shortcuts")] {
            handle.update(cx, |view, _, cx| view.demo_section(section, cx))?;
            pause(cx, 700).await;
            if let Some(native) = handle.update(cx, |_, window, _| native_window(window))? {
                save(native, &dir.join(format!("windows/{name}.png")));
            }
        }
    }
    Ok(())
}

fn save(native: NativeWindow, path: &Path) {
    if let Some(png) = native.snapshot_png() {
        let _ = std::fs::write(path, png);
    }
}

async fn pause(cx: &mut AsyncApp, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

/// Sends space-separated keystrokes through GPUI's real key dispatch.
fn keys(cx: &mut AsyncApp, panel: WindowHandle<Panel>, keystrokes: &str) {
    // Like GPUI's own test helpers: update the window without leasing the
    // root view, which the dispatched key handler needs to update itself.
    cx.update_window(panel.into(), |_, window, cx| {
        for keystroke in keystrokes.split(' ') {
            if let Ok(keystroke) = Keystroke::parse(keystroke) {
                window.dispatch_keystroke(keystroke, cx);
            }
        }
    })
    .ok();
}

async fn type_text(cx: &mut AsyncApp, panel: WindowHandle<Panel>, text: &str) {
    for ch in text.chars() {
        let key = if ch == ' ' {
            "space".to_string()
        } else {
            ch.to_string()
        };
        keys(cx, panel, &key);
        pause(cx, 140).await;
    }
}

fn modifiers(cx: &mut AsyncApp, panel: WindowHandle<Panel>, modifiers: Modifiers) {
    panel
        .update(cx, |panel, _, cx| panel.demo_set_modifiers(modifiers, cx))
        .ok();
}

// ---- invented demo data -----------------------------------------------------

fn seed(data: &Path) -> Result<()> {
    std::fs::create_dir_all(data)?;
    Settings {
        // The recorder captures Copywren's own windows, which privacy mode
        // would hide.
        show_during_screen_sharing: true,
        ..Settings::default()
    }
    .save(&data.join("settings.json"))?;

    let mut store = Store::open(&data.join("copywren.sqlite"))?;
    let now = now_ms();
    let mut add = |minutes_ago: i64, app: (&str, &str), contents: ClipContents| -> Result<ItemId> {
        let classified = classify(&contents).context("demo clip did not classify")?;
        let source = SourceApp {
            bundle_id: Some(app.0.into()),
            name: Some(app.1.into()),
        };
        let clip = NewClip {
            classified: &classified,
            contents: &contents,
            source: &source,
        };
        Ok(store.record_capture(clip, now - minutes_ago * MINUTE)?.id())
    };
    let text = ClipContents::plain_text;
    let safari = ("com.apple.Safari", "Safari");

    let signature = add(
        4000,
        ("com.apple.mail", "Mail"),
        text("Best regards,\nAlex"),
    )?;
    add(
        1500,
        ("com.apple.TextEdit", "TextEdit"),
        text("Copywren keeps everything you copy. Press ⇧⌘V to bring it back."),
    )?;
    add(
        300,
        safari,
        text("https://docs.example.com/getting-started"),
    )?;
    add(
        130,
        ("com.apple.MobileSMS", "Messages"),
        text("On my way, see you in 10 minutes!"),
    )?;
    add(
        75,
        ("com.apple.iCal", "Calendar"),
        text("Design review: Thursday 10:30, room 4B"),
    )?;
    let files = add(48, ("com.apple.finder", "Finder"), demo_files())?;
    let color = add(31, ("com.apple.iWork.Pages", "Pages"), text("#3B9EEA"))?;
    add(22, ("com.apple.Preview", "Preview"), demo_image())?;
    let git = add(
        15,
        ("com.apple.Terminal", "Terminal"),
        text("git switch -c feature/clipboard-search\ncargo test --workspace"),
    )?;
    add(
        9,
        ("com.apple.Notes", "Notes"),
        text(
            "Launch checklist\n• Final copy review\n• Screenshots for the store page\n• Release notes draft\n• Tell the beta testers",
        ),
    )?;
    let link = add(4, safari, text("https://slides.example.com/q3-roadmap"))?;
    add(
        1,
        ("com.apple.mail", "Mail"),
        text("Thanks! I'll send the updated numbers before Friday's review."),
    )?;

    let work = store.create_pinboard("Work")?;
    store.set_pinboard_color(work.id, 5)?;
    let snippets = store.create_pinboard("Snippets")?;
    store.set_pinboard_color(snippets.id, 3)?;
    for id in [files, link] {
        store.pin(id, work.id, now)?;
    }
    for id in [color, git, signature] {
        store.pin(id, snippets.id, now)?;
    }
    // The signature lives only on its pinboard.
    store.delete_item(signature)?;
    Ok(())
}

fn demo_files() -> ClipContents {
    ClipContents::new(
        ["Q3%20Report.pdf", "Budget%202026.numbers"]
            .iter()
            .map(|name| {
                PasteboardEntry::new(vec![Representation::new(
                    uti::FILE_URL,
                    format!("file:///Users/demo/Documents/{name}"),
                )])
            })
            .collect(),
    )
}

/// A painted landscape: sky, sun, two mountain ranges and a lake.
fn demo_image() -> ClipContents {
    let (w, h) = (1200u32, 800u32);
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
        let t = t.clamp(0.0, 1.0);
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        let mut c = mix([112.0, 186.0, 226.0], [250.0, 214.0, 164.0], fy / 0.62);
        let sun = ((fx - 0.7) * 1.5).hypot(fy - 0.34);
        if sun < 0.075 {
            c = [255.0, 236.0, 170.0];
        } else if sun < 0.14 {
            c = mix([255.0, 228.0, 160.0], c, (sun - 0.075) / 0.065);
        }
        let far = 0.55 + 0.05 * (fx * 9.0).sin() + 0.025 * (fx * 23.0 + 1.0).sin();
        let near = 0.66 + 0.06 * (fx * 5.0 + 2.0).sin() + 0.02 * (fx * 17.0).sin();
        if fy > far {
            c = mix(
                [120.0, 140.0, 190.0],
                [90.0, 110.0, 160.0],
                (fy - far) * 6.0,
            );
        }
        if fy > near {
            c = mix([58.0, 100.0, 92.0], [36.0, 70.0, 66.0], (fy - near) * 5.0);
        }
        if fy > 0.8 {
            let ripple = 0.5 + 0.5 * (fx * 140.0 + fy * 60.0).sin();
            c = mix(
                [84.0, 150.0, 190.0],
                [60.0, 120.0, 165.0],
                (fy - 0.8) * 4.0 + ripple * 0.08,
            );
        }
        image::Rgba([c[0] as u8, c[1] as u8, c[2] as u8, 255])
    });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encode demo image");
    ClipContents::new(vec![PasteboardEntry::new(vec![Representation::new(
        uti::PNG,
        png,
    )])])
}
