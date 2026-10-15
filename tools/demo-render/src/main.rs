//! Composes the frames recorded by `copywren --features demo` into the README
//! video frames and screenshots.
//!
//!   demo-render <demo-dir> <docs-dir>
//!
//! Every recorded popup frame is placed in a drawn scene: a text editor with
//! a blinking caret, the popup just below it, and a caption naming the keys
//! being used. Nothing is captured from the screen; the scene is pure SVG.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use resvg::{tiny_skia, usvg};

const WIDTH: f32 = 1440.0;
const HEIGHT: f32 = 900.0;
const POPUP: (f32, f32) = (380.0, 520.0);
const PASTED: &str = "https://slides.example.com/q3-roadmap";

struct Frame {
    index: usize,
    popup: Option<PathBuf>,
    step: String,
    keys: String,
    caption: String,
    pasted: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [demo, docs] = args.as_slice() else {
        eprintln!("usage: demo-render <demo-dir> <docs-dir>");
        std::process::exit(2);
    };
    let demo = Path::new(demo);
    let docs = Path::new(docs);
    let icon = std::fs::canonicalize("assets/icon.svg").expect("run from the repository root");
    let frames = read_manifest(demo);

    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();

    // Video frames at 1×.
    let out = demo.join("composite");
    std::fs::create_dir_all(&out).expect("create composite dir");
    for (n, frame) in frames.iter().enumerate() {
        let svg = scene(frame, &icon);
        render(&svg, &options, 1.0, &out.join(format!("{n:05}.png")));
    }

    // Stills at 2×, picked from inside each step.
    let shots = docs.join("screenshots");
    std::fs::create_dir_all(&shots).expect("create screenshots dir");
    for (step, fraction, name) in [
        ("open", 1.0, "popup"),
        ("search", 1.0, "search"),
        ("preview", 0.7, "preview"),
        ("pinboards", 0.4, "pinboards"),
        ("quick", 0.7, "quick-paste"),
        ("menu", 0.7, "menu"),
        ("paste", 0.9, "pasted"),
    ] {
        let in_step: Vec<&Frame> = frames.iter().filter(|f| f.step == step).collect();
        if in_step.is_empty() {
            continue;
        }
        let pick = ((in_step.len() - 1) as f32 * fraction).round() as usize;
        let mut frame = in_step[pick];
        // Prefer a frame where the popup is visible, unless the step is the
        // one that hides it.
        if frame.popup.is_none()
            && step != "paste"
            && let Some(f) = in_step.iter().rev().find(|f| f.popup.is_some())
        {
            frame = f;
        }
        let still = Frame {
            index: 0, // caret on
            popup: frame.popup.clone(),
            step: frame.step.clone(),
            keys: frame.keys.clone(),
            caption: frame.caption.clone(),
            pasted: frame.pasted,
        };
        render(
            &scene(&still, &icon),
            &options,
            2.0,
            &shots.join(format!("{name}.png")),
        );
    }
    for (file, name) in [
        ("queue.png", "paste-queue"),
        ("settings-general.png", "settings"),
        ("settings-shortcuts.png", "settings-shortcuts"),
    ] {
        let path = demo.join("windows").join(file);
        if let Some((w, h)) = png_size(&path) {
            let svg = window_still(&path, w as f32 / 2.0, h as f32 / 2.0);
            render(&svg, &options, 2.0, &shots.join(format!("{name}.png")));
        }
    }
    println!(
        "{} video frames, stills in {}",
        frames.len(),
        shots.display()
    );
}

fn read_manifest(demo: &Path) -> Vec<Frame> {
    let text = std::fs::read_to_string(demo.join("manifest.tsv")).expect("read manifest.tsv");
    text.lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            let [index, file, step, keys, caption, pasted] = cols.as_slice() else {
                return None;
            };
            Some(Frame {
                index: index.parse().ok()?,
                popup: (*file != "-").then(|| demo.join("frames").join(file)),
                step: step.to_string(),
                keys: keys.to_string(),
                caption: caption.to_string(),
                pasted: *pasted == "true",
            })
        })
        .collect()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const DEFS: &str = r##"<defs>
  <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="#2F9EA3"/>
    <stop offset="1" stop-color="#2C3E8C"/>
  </linearGradient>
  <filter id="shadow" x="-20%" y="-20%" width="140%" height="140%">
    <feDropShadow dx="0" dy="18" stdDeviation="22" flood-color="#06102E" flood-opacity="0.45"/>
  </filter>
</defs>"##;

const FONT: &str = "Helvetica Neue, Helvetica, Arial, sans-serif";

fn scene(frame: &Frame, icon: &Path) -> String {
    let mut svg = String::new();
    let _ = write!(
        svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}">{DEFS}
<rect width="{WIDTH}" height="{HEIGHT}" fill="url(#bg)"/>"##
    );

    // Brand block on the right.
    let _ = write!(
        svg,
        r##"<image x="1010" y="170" width="170" height="170" href="{}"/>
<text x="1095" y="400" text-anchor="middle" font-family="{FONT}" font-size="50" font-weight="bold" fill="#FFFFFF">Copywren</text>
<text x="1095" y="440" text-anchor="middle" font-family="{FONT}" font-size="22" fill="#DCE8FF">Clipboard history for macOS</text>"##,
        icon.display()
    );

    // The editor the user is typing in.
    let _ = write!(
        svg,
        r##"<g filter="url(#shadow)"><rect x="80" y="70" width="820" height="560" rx="14" fill="#FFFFFF"/></g>
<path d="M80 84 a14 14 0 0 1 14 -14 h792 a14 14 0 0 1 14 14 v30 h-820 z" fill="#F1F1F4"/>
<line x1="80" y1="114" x2="900" y2="114" stroke="#E2E2E8"/>
<circle cx="106" cy="92" r="7" fill="#FF5F57"/><circle cx="128" cy="92" r="7" fill="#FEBC2E"/><circle cx="150" cy="92" r="7" fill="#28C840"/>
<text x="490" y="98" text-anchor="middle" font-family="{FONT}" font-size="16" fill="#6E6E73">Project update — Draft</text>
<g font-family="{FONT}" font-size="22" fill="#1D1D1F">
  <text x="130" y="176">Hi team,</text>
  <text x="130" y="226">Here's the deck for Thursday's review:</text>
</g>"##
    );
    let caret_on = (frame.index / 5).is_multiple_of(2);
    let caret = if caret_on {
        r##"<tspan fill="#0A84FF" font-weight="300">|</tspan>"##
    } else {
        ""
    };
    if frame.pasted {
        let _ = write!(
            svg,
            r##"<text x="130" y="270" font-family="{FONT}" font-size="22" fill="#0B66D6">{PASTED}{caret}</text>"##
        );
    } else {
        let _ = write!(
            svg,
            r##"<text x="128" y="270" font-family="{FONT}" font-size="22">{caret}</text>"##
        );
    }

    // The popup, just below the caret.
    if let Some(popup) = &frame.popup {
        let _ = write!(
            svg,
            r##"<image x="122" y="290" width="{}" height="{}" href="{}" filter="url(#shadow)"/>"##,
            POPUP.0,
            POPUP.1,
            popup.display()
        );
    }

    // Caption.
    if !frame.caption.is_empty() {
        let keys = if frame.keys.is_empty() {
            String::new()
        } else {
            format!(
                r##"<tspan fill="#8FD0FF" font-weight="bold">{}</tspan><tspan dx="16">"##,
                escape(&frame.keys)
            )
        };
        let close = if frame.keys.is_empty() {
            ""
        } else {
            "</tspan>"
        };
        let _ = write!(
            svg,
            r##"<rect x="200" y="826" width="1040" height="54" rx="27" fill="#0B1020" fill-opacity="0.82"/>
<text x="720" y="861" text-anchor="middle" font-family="{FONT}" font-size="24" fill="#FFFFFF">{keys}{}{close}</text>"##,
            escape(&frame.caption)
        );
    }
    svg.push_str("</svg>");
    svg
}

/// A window screenshot (at 2×) centred on the backdrop.
fn window_still(window: &Path, width: f32, height: f32) -> String {
    let margin = 70.0;
    let (w, h) = (width + 2.0 * margin, height + 2.0 * margin);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{DEFS}
<rect width="{w}" height="{h}" fill="url(#bg)"/>
<image x="{margin}" y="{margin}" width="{width}" height="{height}" href="{}" filter="url(#shadow)"/>
</svg>"##,
        window.display()
    )
}

fn render(svg: &str, options: &usvg::Options, scale: f32, out: &Path) {
    let tree = usvg::Tree::from_str(svg, options).expect("parse scene");
    let size = tree.size();
    let mut pixmap = tiny_skia::Pixmap::new(
        (size.width() * scale).round() as u32,
        (size.height() * scale).round() as u32,
    )
    .expect("pixmap");
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.save_png(out).expect("write png");
}

/// Width and height from a PNG's IHDR chunk.
fn png_size(path: &Path) -> Option<(u32, u32)> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 24 || &bytes[1..4] != b"PNG" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}
