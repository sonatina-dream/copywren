//! Colours and sizes. The popup is dark, like the system's dark menus.

use gpui::{Hsla, Rgba, rgb, rgba};

pub const POPUP_WIDTH: f32 = 380.0;
pub const POPUP_HEIGHT: f32 = 520.0;
pub const HEADER_HEIGHT: f32 = 38.0;
pub const SEARCH_HEIGHT: f32 = 40.0;
pub const TABS_HEIGHT: f32 = 36.0;
pub const FOOTER_HEIGHT: f32 = 28.0;
/// Horizontal padding of the popup's content.
pub const PAD: f32 = 12.0;

pub fn popup_bg() -> Rgba {
    rgb(0x1e1e20)
}
pub fn surface() -> Rgba {
    rgb(0x2a2a2d)
}
pub fn surface_hover() -> Rgba {
    rgb(0x343438)
}
pub fn card() -> Rgba {
    rgb(0x29292c)
}
pub fn card_selected() -> Rgba {
    rgb(0x30343c)
}
pub fn text() -> Rgba {
    rgb(0xf2f2f4)
}
pub fn text_muted() -> Rgba {
    rgb(0x9a9aa1)
}
pub fn accent() -> Rgba {
    rgb(0x3b9eea)
}
pub fn link() -> Rgba {
    rgb(0x6cb8ff)
}
pub fn danger() -> Rgba {
    rgb(0xff453a)
}
pub fn border() -> Rgba {
    rgba(0xffffff1c)
}

/// Pinboard palette, indexed by `Pinboard::color`.
pub const PINBOARD_PALETTE: [u32; 8] = [
    0xff453a, 0xff9f0a, 0xffd60a, 0x30d158, 0x64d2ff, 0x0a84ff, 0xbf5af2, 0xff375f,
];

pub fn pinboard_color(index: u8) -> Rgba {
    rgb(PINBOARD_PALETTE[index as usize % PINBOARD_PALETTE.len()])
}

pub fn hsla(color: u32) -> Hsla {
    rgb(color).into()
}
