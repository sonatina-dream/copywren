//! Where the popup appears: next to the text cursor (or mouse pointer),
//! fully on screen. All rectangles use Cocoa screen coordinates: origin at
//! the bottom-left of the primary display, y growing upwards.

/// Space between the anchor and the popup.
pub const GAP: f64 = 6.0;
/// Minimum distance between the popup and the screen edges.
pub const MARGIN: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn max_x(&self) -> f64 {
        self.x + self.width
    }

    pub fn max_y(&self) -> f64 {
        self.y + self.height
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.max_x() && y >= self.y && y < self.max_y()
    }

    /// Converts a rectangle from top-left-origin coordinates (the
    /// Accessibility API's) given the primary display's height.
    pub fn from_top_left(x: f64, y: f64, width: f64, height: f64, primary_height: f64) -> Self {
        Self::new(x, primary_height - y - height, width, height)
    }
}

/// Frame for a popup of `width`×`height` next to `anchor` inside `visible`
/// (the screen minus menu bar and Dock). Prefers sitting just below the
/// anchor with left edges aligned, flips above when there is no room below,
/// and as a last resort overlaps the anchor rather than leaving the screen.
pub fn popup_frame(anchor: Rect, width: f64, height: f64, visible: Rect) -> Rect {
    let min_x = visible.x + MARGIN;
    let max_x = (visible.max_x() - MARGIN - width).max(min_x);
    let min_y = visible.y + MARGIN;
    let max_y = (visible.max_y() - MARGIN - height).max(min_y);

    let x = anchor.x.clamp(min_x, max_x);
    let below = anchor.y - GAP - height;
    let above = anchor.max_y() + GAP;
    let y = if below >= min_y {
        below.min(max_y)
    } else if above <= max_y {
        above
    } else {
        below.clamp(min_y, max_y)
    };
    Rect::new(x, y, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1440.0,
        height: 875.0,
    };

    #[test]
    fn sits_below_the_caret() {
        let caret = Rect::new(300.0, 600.0, 2.0, 18.0);
        let frame = popup_frame(caret, 380.0, 520.0, SCREEN);
        assert_eq!(frame, Rect::new(300.0, 600.0 - GAP - 520.0, 380.0, 520.0));
    }

    #[test]
    fn flips_above_near_the_bottom() {
        let caret = Rect::new(300.0, 100.0, 2.0, 18.0);
        let frame = popup_frame(caret, 380.0, 520.0, SCREEN);
        assert_eq!(frame.y, 118.0 + GAP);
    }

    #[test]
    fn stays_on_screen_horizontally() {
        let caret = Rect::new(1400.0, 600.0, 2.0, 18.0);
        let frame = popup_frame(caret, 380.0, 520.0, SCREEN);
        assert_eq!(frame.max_x(), SCREEN.max_x() - MARGIN);
        let caret = Rect::new(-50.0, 600.0, 2.0, 18.0);
        assert_eq!(popup_frame(caret, 380.0, 520.0, SCREEN).x, MARGIN);
    }

    #[test]
    fn overlaps_when_neither_side_fits() {
        let short = Rect::new(0.0, 0.0, 1440.0, 600.0);
        let caret = Rect::new(300.0, 300.0, 2.0, 18.0);
        let frame = popup_frame(caret, 380.0, 520.0, short);
        assert!(frame.y >= MARGIN && frame.max_y() <= 600.0 - MARGIN + 0.001);
    }

    #[test]
    fn respects_offset_screens() {
        // A second display to the right and above the primary one.
        let second = Rect::new(1440.0, 200.0, 1920.0, 1055.0);
        let pointer = Rect::new(2000.0, 900.0, 0.0, 0.0);
        let frame = popup_frame(pointer, 380.0, 520.0, second);
        assert_eq!((frame.x, frame.y), (2000.0, 900.0 - GAP - 520.0));
    }

    #[test]
    fn converts_from_top_left() {
        let r = Rect::from_top_left(10.0, 100.0, 5.0, 20.0, 900.0);
        assert_eq!(r, Rect::new(10.0, 780.0, 5.0, 20.0));
        assert!(r.contains(12.0, 790.0));
        assert!(!r.contains(12.0, 801.0));
    }
}
