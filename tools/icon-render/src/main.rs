//! Rasterizes an SVG to square PNGs.
//!
//!   icon-render <input.svg> <output.png> <pixels>

use resvg::{tiny_skia, usvg};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, pixels] = args.as_slice() else {
        eprintln!("usage: icon-render <input.svg> <output.png> <pixels>");
        std::process::exit(2);
    };
    let pixels: u32 = pixels.parse().expect("pixels must be a number");
    let data = std::fs::read(input).expect("read svg");
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).expect("parse svg");
    let mut pixmap = tiny_skia::Pixmap::new(pixels, pixels).expect("pixmap");
    let scale = pixels as f32 / tree.size().width().max(tree.size().height());
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.save_png(output).expect("write png");
}
