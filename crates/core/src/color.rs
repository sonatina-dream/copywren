//! Parsing of colour literals copied from design tools and code (SPEC §3.6).

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// 0.0 ..= 1.0
    pub a: f32,
}

impl Rgba {
    pub fn to_u32(self) -> u32 {
        (u32::from(self.r) << 16) | (u32::from(self.g) << 8) | u32::from(self.b)
    }

    /// Relative luminance in 0..=1, used to pick readable text on a swatch.
    pub fn luminance(self) -> f32 {
        fn channel(c: u8) -> f32 {
            let c = f32::from(c) / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }
}

/// Parses `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`, `rgb()`, `rgba()`, `hsl()`
/// and `hsla()` (comma or space separated). Returns `None` for anything else.
pub fn parse(input: &str) -> Option<Rgba> {
    let s = input.trim();
    if s.contains('\n') {
        return None;
    }
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    let lower = s.to_ascii_lowercase();
    let (name, args) = lower.split_once('(')?;
    let args = args.strip_suffix(')')?;
    let parts: Vec<&str> = args
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    match (name.trim(), parts.len()) {
        ("rgb", 3) | ("rgba", 4) | ("rgb", 4) => {
            let r = parse_channel(parts[0])?;
            let g = parse_channel(parts[1])?;
            let b = parse_channel(parts[2])?;
            let a = parts.get(3).map(|p| parse_alpha(p)).unwrap_or(Some(1.0))?;
            Some(Rgba { r, g, b, a })
        }
        ("hsl", 3) | ("hsla", 4) | ("hsl", 4) => {
            let h = parts[0].trim_end_matches("deg").parse::<f32>().ok()?;
            let s = parse_percent(parts[1])?;
            let l = parse_percent(parts[2])?;
            let a = parts.get(3).map(|p| parse_alpha(p)).unwrap_or(Some(1.0))?;
            let (r, g, b) = hsl_to_rgb(h, s, l);
            Some(Rgba { r, g, b, a })
        }
        _ => None,
    }
}

fn parse_hex(hex: &str) -> Option<Rgba> {
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|v| v * 17);
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        3 => Some(Rgba {
            r: nibble(0)?,
            g: nibble(1)?,
            b: nibble(2)?,
            a: 1.0,
        }),
        4 => Some(Rgba {
            r: nibble(0)?,
            g: nibble(1)?,
            b: nibble(2)?,
            a: f32::from(nibble(3)?) / 255.0,
        }),
        6 => Some(Rgba {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: 1.0,
        }),
        8 => Some(Rgba {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: f32::from(byte(6)?) / 255.0,
        }),
        _ => None,
    }
}

fn parse_channel(part: &str) -> Option<u8> {
    if let Some(pct) = part.strip_suffix('%') {
        let v = pct.parse::<f32>().ok()?;
        return (0.0..=100.0).contains(&v).then(|| (v * 2.55).round() as u8);
    }
    let v = part.parse::<f32>().ok()?;
    (0.0..=255.0).contains(&v).then(|| v.round() as u8)
}

fn parse_percent(part: &str) -> Option<f32> {
    let v = part.strip_suffix('%')?.parse::<f32>().ok()?;
    (0.0..=100.0).contains(&v).then_some(v / 100.0)
}

fn parse_alpha(part: &str) -> Option<f32> {
    if part.ends_with('%') {
        return parse_percent(part);
    }
    let v = part.parse::<f32>().ok()?;
    (0.0..=1.0).contains(&v).then_some(v)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 360.0;
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round() as u8
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 1.0 }
    }

    #[test]
    fn hex_forms() {
        assert_eq!(parse("#fff"), Some(rgb(255, 255, 255)));
        assert_eq!(parse("#FF8000"), Some(rgb(255, 128, 0)));
        assert_eq!(parse("  #102030  "), Some(rgb(16, 32, 48)));
        let with_alpha = parse("#00000080").unwrap();
        assert!((with_alpha.a - 128.0 / 255.0).abs() < 1e-6);
        assert!(parse("#f00f").is_some());
        assert_eq!(parse("#ggg"), None);
        assert_eq!(parse("#12345"), None);
        assert_eq!(parse("fff"), None);
    }

    #[test]
    fn functional_forms() {
        assert_eq!(parse("rgb(255, 0, 0)"), Some(rgb(255, 0, 0)));
        assert_eq!(parse("rgb(255 0 0)"), Some(rgb(255, 0, 0)));
        assert_eq!(parse("rgba(0,0,255,0.5)").unwrap().a, 0.5);
        assert_eq!(parse("rgb(100%, 0%, 0%)"), Some(rgb(255, 0, 0)));
        assert_eq!(parse("hsl(120, 100%, 50%)"), Some(rgb(0, 255, 0)));
        assert_eq!(parse("hsl(0deg 0% 50%)"), Some(rgb(128, 128, 128)));
        assert_eq!(parse("hsla(240, 100%, 50%, 0.25)").unwrap().a, 0.25);
        assert_eq!(parse("rgb(300, 0, 0)"), None);
        assert_eq!(parse("rgb(1,2)"), None);
        assert_eq!(parse("color(1,2,3)"), None);
        assert_eq!(parse("#fff\n#000"), None);
    }

    #[test]
    fn luminance_orders_black_and_white() {
        assert!(rgb(255, 255, 255).luminance() > 0.9);
        assert!(rgb(0, 0, 0).luminance() < 0.01);
        assert_eq!(rgb(0x12, 0x34, 0x56).to_u32(), 0x123456);
    }
}
