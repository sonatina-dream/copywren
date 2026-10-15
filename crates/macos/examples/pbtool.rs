//! Clipboard helper for manual and scripted testing of Copywren.
//!
//!   pbtool save <file>        snapshot every item and type of the clipboard
//!   pbtool restore <file>     put a snapshot back
//!   pbtool text <text>        set plain text
//!   pbtool typed <uti> <text> set text plus an extra marker type (e.g. a
//!                             concealed-password marker)
//!   pbtool png <file>         set a PNG image
//!   pbtool show               print the current types and sizes

use copywren_core::contents;
use copywren_core::model::{ClipContents, PasteboardEntry, Representation};
use copywren_core::uti;
use copywren_macos::pasteboard;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_else(|| usage());
    match args.first().map(String::as_str) {
        Some("save") => {
            let snapshot = pasteboard::read();
            std::fs::write(arg(1), contents::encode(&snapshot)).expect("write snapshot");
            println!(
                "saved {} item(s), {} bytes",
                snapshot.entries.len(),
                snapshot.total_bytes()
            );
        }
        Some("restore") => {
            let bytes = std::fs::read(arg(1)).expect("read snapshot");
            let snapshot = contents::decode(&bytes).expect("decode snapshot");
            pasteboard::write(&snapshot);
            println!("restored {} item(s)", snapshot.entries.len());
        }
        Some("text") => {
            pasteboard::write_plain_text(&arg(1));
        }
        Some("typed") => {
            pasteboard::write(&ClipContents::new(vec![PasteboardEntry::new(vec![
                Representation::new(uti::PLAIN_TEXT, arg(2).into_bytes()),
                Representation::new(arg(1), Vec::new()),
            ])]));
        }
        Some("png") => {
            let png = std::fs::read(arg(1)).expect("read png");
            pasteboard::write(&ClipContents::new(vec![PasteboardEntry::new(vec![
                Representation::new(uti::PNG, png),
            ])]));
        }
        Some("show") => {
            for (i, entry) in pasteboard::read().entries.iter().enumerate() {
                for rep in &entry.representations {
                    println!("item {i}: {} ({} bytes)", rep.uti, rep.data.len());
                }
            }
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: pbtool save|restore <file> | text <text> | typed <uti> <text> | png <file> | show"
    );
    std::process::exit(2)
}
