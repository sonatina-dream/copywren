//! Card header colours derived from the source app (SPEC §4: "header coloured
//! by the source app").

/// Fallback colours for apps without an icon, picked by hashing the bundle ID.
const FALLBACK: [u32; 8] = [
    0x5b6ee1, 0xe0703a, 0x3aa65c, 0xc2457a, 0x2f8fbf, 0x8a5cd6, 0xc7a02b, 0x5f7d8c,
];

/// A saturated, mid-brightness colour representative of an app icon, or
/// `None` when the icon is mostly grey or transparent.
pub fn dominant_color(icon_png: &[u8]) -> Option<u32> {
    let image = image::load_from_memory(icon_png).ok()?.to_rgba8();
    // 12 hue buckets weighted by saturation; mostly-grey pixels are ignored.
    let mut buckets = [(0f64, 0f64, 0f64, 0f64); 12];
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        if a < 128 {
            continue;
        }
        let (h, s, v) = hsv(r, g, b);
        if s < 0.25 || v < 0.2 {
            continue;
        }
        let weight = f64::from(s) * f64::from(v);
        let bucket = &mut buckets[((h / 30.0) as usize).min(11)];
        bucket.0 += weight * f64::from(r);
        bucket.1 += weight * f64::from(g);
        bucket.2 += weight * f64::from(b);
        bucket.3 += weight;
    }
    let total: f64 = buckets.iter().map(|b| b.3).sum();
    let pixels = f64::from(image.width() * image.height());
    let best = buckets
        .iter()
        .max_by(|a, b| a.3.total_cmp(&b.3))
        .filter(|b| b.3 > 0.0 && total > pixels * 0.02)?;
    let (r, g, b) = (best.0 / best.3, best.1 / best.3, best.2 / best.3);
    Some(darken_for_white_text(r, g, b))
}

pub fn fallback_color(key: &str) -> u32 {
    // FNV-1a: stable across runs and platforms.
    let mut hash: u32 = 0x811c_9dc5;
    for byte in key.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    FALLBACK[(hash % FALLBACK.len() as u32) as usize]
}

/// Scales the colour down until white text on it has reasonable contrast.
fn darken_for_white_text(r: f64, g: f64, b: f64) -> u32 {
    let luminance = |r: f64, g: f64, b: f64| (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
    let mut factor = 1.0;
    while luminance(r * factor, g * factor, b * factor) > 0.45 && factor > 0.3 {
        factor -= 0.05;
    }
    let channel = |c: f64| (c * factor).round().clamp(0.0, 255.0) as u32;
    (channel(r) << 16) | (channel(g) << 8) | channel(b)
}

fn hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { delta / max };
    (h, s, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(16, 16, |x, y| image::Rgba(f(x, y)));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn picks_the_main_hue() {
        // Mostly blue with a little red.
        let icon = png(|x, _| {
            if x < 13 {
                [20, 90, 220, 255]
            } else {
                [220, 30, 30, 255]
            }
        });
        let color = dominant_color(&icon).unwrap();
        let (r, g, b) = ((color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff);
        assert!(b > r && b > g, "expected blue, got {color:06x}");
    }

    #[test]
    fn grey_or_transparent_icons_have_no_color() {
        assert_eq!(dominant_color(&png(|_, _| [128, 128, 128, 255])), None);
        assert_eq!(dominant_color(&png(|_, _| [255, 0, 0, 0])), None);
        assert_eq!(dominant_color(b"not a png"), None);
    }

    #[test]
    fn bright_colors_are_darkened() {
        let color = dominant_color(&png(|_, _| [255, 240, 40, 255])).unwrap();
        let (r, g, b) = ((color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff);
        let lum = (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) / 255.0;
        assert!(lum <= 0.46, "luminance {lum}");
    }

    #[test]
    fn fallback_is_stable() {
        assert_eq!(
            fallback_color("com.apple.Notes"),
            fallback_color("com.apple.Notes")
        );
        assert!(FALLBACK.contains(&fallback_color("")));
    }
}
