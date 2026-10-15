//! System-wide shortcuts via Carbon hotkeys (`RegisterEventHotKey`), which
//! work for agent apps without Accessibility access.

use anyhow::{Result, anyhow};
use copywren_core::shortcut::Shortcut;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    TogglePanel,
    TogglePasteQueue,
    /// ⌘V while the Paste Queue holds snippets.
    QueuePaste,
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    registered: Vec<(HotKey, HotkeyAction)>,
}

impl Hotkeys {
    /// Starts listening; pressed hotkeys are delivered to `sender`.
    pub fn new(sender: smol::channel::Sender<u32>) -> Result<Self> {
        let manager = GlobalHotKeyManager::new().map_err(|e| anyhow!("{e}"))?;
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() == HotKeyState::Pressed {
                let _ = sender.try_send(event.id());
            }
        }));
        Ok(Self {
            manager,
            registered: Vec::new(),
        })
    }

    pub fn action_for(&self, id: u32) -> Option<HotkeyAction> {
        self.registered
            .iter()
            .find(|(hotkey, _)| hotkey.id() == id)
            .map(|(_, action)| *action)
    }

    pub fn is_registered(&self, action: HotkeyAction) -> bool {
        self.registered.iter().any(|(_, a)| *a == action)
    }

    pub fn register(&mut self, action: HotkeyAction, shortcut: &Shortcut) -> Result<()> {
        self.unregister(action);
        let hotkey = to_hotkey(shortcut)?;
        self.manager
            .register(hotkey)
            .map_err(|e| anyhow!("{} is unavailable: {e}", shortcut.symbol()))?;
        self.registered.push((hotkey, action));
        Ok(())
    }

    pub fn unregister(&mut self, action: HotkeyAction) {
        self.registered.retain(|(hotkey, a)| {
            if *a == action {
                let _ = self.manager.unregister(*hotkey);
                false
            } else {
                true
            }
        });
    }
}

pub fn paste_shortcut() -> Shortcut {
    Shortcut::new("v", true, false, false, false)
}

fn to_hotkey(shortcut: &Shortcut) -> Result<HotKey> {
    let mut mods = Modifiers::empty();
    if shortcut.cmd {
        mods |= Modifiers::SUPER;
    }
    if shortcut.shift {
        mods |= Modifiers::SHIFT;
    }
    if shortcut.alt {
        mods |= Modifiers::ALT;
    }
    if shortcut.ctrl {
        mods |= Modifiers::CONTROL;
    }
    let code =
        key_code(&shortcut.key).ok_or_else(|| anyhow!("unsupported key {}", shortcut.key))?;
    Ok(HotKey::new(Some(mods), code))
}

fn key_code(key: &str) -> Option<Code> {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(match c {
            'a' => Code::KeyA,
            'b' => Code::KeyB,
            'c' => Code::KeyC,
            'd' => Code::KeyD,
            'e' => Code::KeyE,
            'f' => Code::KeyF,
            'g' => Code::KeyG,
            'h' => Code::KeyH,
            'i' => Code::KeyI,
            'j' => Code::KeyJ,
            'k' => Code::KeyK,
            'l' => Code::KeyL,
            'm' => Code::KeyM,
            'n' => Code::KeyN,
            'o' => Code::KeyO,
            'p' => Code::KeyP,
            'q' => Code::KeyQ,
            'r' => Code::KeyR,
            's' => Code::KeyS,
            't' => Code::KeyT,
            'u' => Code::KeyU,
            'v' => Code::KeyV,
            'w' => Code::KeyW,
            'x' => Code::KeyX,
            'y' => Code::KeyY,
            'z' => Code::KeyZ,
            '0' => Code::Digit0,
            '1' => Code::Digit1,
            '2' => Code::Digit2,
            '3' => Code::Digit3,
            '4' => Code::Digit4,
            '5' => Code::Digit5,
            '6' => Code::Digit6,
            '7' => Code::Digit7,
            '8' => Code::Digit8,
            '9' => Code::Digit9,
            '`' => Code::Backquote,
            '-' => Code::Minus,
            '=' => Code::Equal,
            '[' => Code::BracketLeft,
            ']' => Code::BracketRight,
            '\\' => Code::Backslash,
            ';' => Code::Semicolon,
            '\'' => Code::Quote,
            ',' => Code::Comma,
            '.' => Code::Period,
            '/' => Code::Slash,
            _ => return None,
        });
    }
    Some(match key {
        "space" => Code::Space,
        "return" => Code::Enter,
        "tab" => Code::Tab,
        "escape" => Code::Escape,
        "backspace" => Code::Backspace,
        "delete" => Code::Delete,
        "up" => Code::ArrowUp,
        "down" => Code::ArrowDown,
        "left" => Code::ArrowLeft,
        "right" => Code::ArrowRight,
        "home" => Code::Home,
        "end" => Code::End,
        "pageup" => Code::PageUp,
        "pagedown" => Code::PageDown,
        "f1" => Code::F1,
        "f2" => Code::F2,
        "f3" => Code::F3,
        "f4" => Code::F4,
        "f5" => Code::F5,
        "f6" => Code::F6,
        "f7" => Code::F7,
        "f8" => Code::F8,
        "f9" => Code::F9,
        "f10" => Code::F10,
        "f11" => Code::F11,
        "f12" => Code::F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_parseable_key() {
        for key in ["a", "z", "0", "9", ".", "space", "f12", "return", "left"] {
            let shortcut: Shortcut = format!("cmd+{key}").parse().unwrap();
            assert!(to_hotkey(&shortcut).is_ok(), "{key}");
        }
        let hk = to_hotkey(&"cmd+shift+v".parse().unwrap()).unwrap();
        assert_eq!(
            hk,
            HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyV)
        );
    }
}
