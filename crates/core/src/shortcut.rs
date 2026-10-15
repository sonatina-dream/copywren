//! Keyboard shortcut and modifier settings, stored as text like
//! `cmd+shift+v` and displayed as `⇧⌘V`.

use std::fmt;
use std::str::FromStr;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
    /// Lowercase key name: `a`–`z`, `0`–`9`, `space`, `f1`… or punctuation.
    pub key: String,
}

const NAMED_KEYS: &[&str] = &[
    "space",
    "return",
    "tab",
    "escape",
    "backspace",
    "delete",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
];
const PUNCTUATION: &str = "`-=[]\\;',./";

impl Shortcut {
    pub fn new(key: &str, cmd: bool, shift: bool, alt: bool, ctrl: bool) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            cmd,
            key: key.to_ascii_lowercase(),
        }
    }

    /// Mac-style label, modifiers in system order: `⌃⌥⇧⌘V`.
    pub fn symbol(&self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push('⌃');
        }
        if self.alt {
            out.push('⌥');
        }
        if self.shift {
            out.push('⇧');
        }
        if self.cmd {
            out.push('⌘');
        }
        out.push_str(&key_symbol(&self.key));
        out
    }
}

fn key_symbol(key: &str) -> String {
    match key {
        "space" => "Space".into(),
        "return" => "↩".into(),
        "tab" => "⇥".into(),
        "escape" => "⎋".into(),
        "backspace" => "⌫".into(),
        "delete" => "⌦".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        other => other.to_uppercase(),
    }
}

fn is_valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => c.is_ascii_alphanumeric() || PUNCTUATION.contains(c),
        _ => {
            NAMED_KEYS.contains(&key)
                || key
                    .strip_prefix('f')
                    .and_then(|n| n.parse::<u8>().ok())
                    .is_some_and(|n| (1..=20).contains(&n))
        }
    }
}

impl FromStr for Shortcut {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut shortcut = Shortcut::default();
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key, modifiers) = parts
            .split_last()
            .ok_or_else(|| anyhow!("empty shortcut"))?;
        for m in modifiers {
            match ModifierKey::from_str(m)? {
                ModifierKey::Cmd => shortcut.cmd = true,
                ModifierKey::Shift => shortcut.shift = true,
                ModifierKey::Alt => shortcut.alt = true,
                ModifierKey::Ctrl => shortcut.ctrl = true,
            }
        }
        let key = key.to_ascii_lowercase();
        if !is_valid_key(&key) {
            bail!("unsupported key {key:?}");
        }
        if !(shortcut.cmd || shortcut.ctrl || shortcut.alt) {
            bail!("a global shortcut needs ⌘, ⌃ or ⌥");
        }
        shortcut.key = key;
        Ok(shortcut)
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("ctrl");
        }
        if self.alt {
            parts.push("alt");
        }
        if self.shift {
            parts.push("shift");
        }
        if self.cmd {
            parts.push("cmd");
        }
        parts.push(&self.key);
        write!(f, "{}", parts.join("+"))
    }
}

impl Serialize for Shortcut {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Shortcut {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A single modifier, used for the quick-paste and plain-text modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModifierKey {
    Cmd,
    Shift,
    Alt,
    Ctrl,
}

impl ModifierKey {
    pub const ALL: [ModifierKey; 4] = [
        ModifierKey::Cmd,
        ModifierKey::Shift,
        ModifierKey::Alt,
        ModifierKey::Ctrl,
    ];

    pub fn symbol(self) -> &'static str {
        match self {
            ModifierKey::Cmd => "⌘",
            ModifierKey::Shift => "⇧",
            ModifierKey::Alt => "⌥",
            ModifierKey::Ctrl => "⌃",
        }
    }
}

impl FromStr for ModifierKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" | "meta" | "⌘" => ModifierKey::Cmd,
            "shift" | "⇧" => ModifierKey::Shift,
            "alt" | "option" | "opt" | "⌥" => ModifierKey::Alt,
            "ctrl" | "control" | "⌃" => ModifierKey::Ctrl,
            other => bail!("unknown modifier {other:?}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format() {
        let s: Shortcut = "cmd+shift+v".parse().unwrap();
        assert_eq!(s, Shortcut::new("v", true, true, false, false));
        assert_eq!(s.to_string(), "shift+cmd+v");
        assert_eq!(s.symbol(), "⇧⌘V");
        let s: Shortcut = "Control + Option + Space".parse().unwrap();
        assert_eq!(s.symbol(), "⌃⌥Space");
        assert_eq!(s.to_string().parse::<Shortcut>().unwrap(), s);
        assert_eq!("cmd+f12".parse::<Shortcut>().unwrap().symbol(), "⌘F12");
        assert_eq!("cmd+.".parse::<Shortcut>().unwrap().key, ".");
    }

    #[test]
    fn rejects_bad_shortcuts() {
        assert!("v".parse::<Shortcut>().is_err());
        assert!("shift+v".parse::<Shortcut>().is_err());
        assert!("cmd+hyper+v".parse::<Shortcut>().is_err());
        assert!("cmd+vv".parse::<Shortcut>().is_err());
        assert!("cmd+f30".parse::<Shortcut>().is_err());
        assert!("".parse::<Shortcut>().is_err());
    }

    #[test]
    fn serde_uses_text() {
        let s: Shortcut = "cmd+shift+c".parse().unwrap();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"shift+cmd+c\"");
        assert_eq!(serde_json::from_str::<Shortcut>(&json).unwrap(), s);
        assert!(serde_json::from_str::<Shortcut>("\"q\"").is_err());
        assert_eq!(serde_json::to_string(&ModifierKey::Alt).unwrap(), "\"alt\"");
        assert_eq!("Option".parse::<ModifierKey>().unwrap(), ModifierKey::Alt);
    }
}
