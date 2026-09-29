//! Keyboard shortcuts: key strokes, the commands' stable IDs, the default bindings, and the
//! user's overrides resolved into the bindings every surface reads (keyboard shortcuts spec §3).
//! Pure: no window handles.

use std::fmt;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_ADD, VK_BACK, VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END, VK_ESCAPE,
    VK_F1, VK_F24, VK_HOME, VK_INSERT, VK_LEFT, VK_MULTIPLY, VK_NEXT, VK_NUMPAD0,
    VK_NUMPAD9, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7,
    VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_PRIOR, VK_RETURN, VK_RIGHT,
    VK_SPACE, VK_SUBTRACT, VK_TAB, VK_UP,
};
#[cfg(test)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_F3, VK_LWIN, VK_MENU, VK_SHIFT};
use windows_sys::Win32::UI::WindowsAndMessaging::{FALT, FCONTROL, FSHIFT};

/// One key with its modifiers: what a shortcut is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(not(test), allow(dead_code, reason = "used from Task 2 on"))]
pub(crate) struct KeyStroke {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub vk: u16,
}

/// Keys named by a word or by the character they type on a US layout (spec §3.1). Letters,
/// digits, F-keys and numpad digits are named by `key_name`'s ranges instead.
#[cfg_attr(not(test), allow(dead_code, reason = "used from Task 2 on"))]
const NAMED_KEYS: [(u16, &str); 31] = [
    (VK_RETURN, "Enter"),
    (VK_ESCAPE, "Escape"),
    (VK_SPACE, "Space"),
    (VK_TAB, "Tab"),
    (VK_BACK, "Backspace"),
    (VK_DELETE, "Delete"),
    (VK_INSERT, "Insert"),
    (VK_HOME, "Home"),
    (VK_END, "End"),
    (VK_PRIOR, "PageUp"),
    (VK_NEXT, "PageDown"),
    (VK_UP, "Up"),
    (VK_DOWN, "Down"),
    (VK_LEFT, "Left"),
    (VK_RIGHT, "Right"),
    (VK_OEM_PLUS, "="),
    (VK_OEM_MINUS, "-"),
    (VK_OEM_COMMA, ","),
    (VK_OEM_PERIOD, "."),
    (VK_OEM_2, "/"),
    (VK_OEM_5, "\\"),
    (VK_OEM_1, ";"),
    (VK_OEM_7, "'"),
    (VK_OEM_4, "["),
    (VK_OEM_6, "]"),
    (VK_OEM_3, "`"),
    (VK_ADD, "NumpadAdd"),
    (VK_SUBTRACT, "NumpadSubtract"),
    (VK_MULTIPLY, "NumpadMultiply"),
    (VK_DIVIDE, "NumpadDivide"),
    (VK_DECIMAL, "NumpadDecimal"),
];

#[cfg_attr(not(test), allow(dead_code, reason = "used from Task 2 on"))]
fn key_name(vk: u16) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some(char::from(vk as u8).to_string()),
        VK_F1..=VK_F24 => Some(format!("F{}", vk - VK_F1 + 1)),
        VK_NUMPAD0..=VK_NUMPAD9 => Some(format!("Numpad{}", vk - VK_NUMPAD0)),
        _ => NAMED_KEYS
            .iter()
            .find(|(key, _)| *key == vk)
            .map(|(_, name)| (*name).to_owned()),
    }
}

#[cfg_attr(not(test), allow(dead_code, reason = "used from Task 2 on"))]
fn key_from_name(name: &str) -> Option<u16> {
    let upper = name.to_ascii_uppercase();
    if let [byte] = upper.as_bytes()
        && byte.is_ascii_alphanumeric()
    {
        return Some(u16::from(*byte));
    }
    if let Some(number) = upper.strip_prefix('F').and_then(|digits| digits.parse::<u16>().ok())
        && (1..=24).contains(&number)
        && !upper[1..].starts_with('0')
    {
        return Some(VK_F1 + number - 1);
    }
    if let Some(digit) = upper.strip_prefix("NUMPAD")
        && let [byte @ b'0'..=b'9'] = digit.as_bytes()
    {
        return Some(VK_NUMPAD0 + u16::from(byte - b'0'));
    }
    NAMED_KEYS
        .iter()
        .find(|(_, key)| key.eq_ignore_ascii_case(name))
        .map(|(vk, _)| *vk)
}

#[cfg_attr(not(test), allow(dead_code, reason = "used from Task 2 on"))]
impl KeyStroke {
    pub(crate) const fn new(ctrl: bool, shift: bool, alt: bool, vk: u16) -> Self {
        Self { ctrl, shift, alt, vk }
    }

    /// The stroke a key press makes, or `None` for a modifier alone or a key with no name.
    pub(crate) fn from_key(vk: u16, ctrl: bool, shift: bool, alt: bool) -> Option<Self> {
        key_name(vk)?;
        Some(Self::new(ctrl, shift, alt, vk))
    }

    /// `Ctrl+Shift+S` and the like: case-insensitive, spaces around `+` ignored, modifiers in
    /// any order, the key last.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let parts = text.split('+').map(str::trim).collect::<Vec<_>>();
        let (key, modifiers) = parts.split_last()?;
        let mut stroke = Self::new(false, false, false, key_from_name(key)?);
        for modifier in modifiers {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => stroke.ctrl = true,
                "shift" => stroke.shift = true,
                "alt" => stroke.alt = true,
                _ => return None,
            }
        }
        Some(stroke)
    }

    /// The modifiers then the key, each as a keycap shows it.
    pub(crate) fn parts(self) -> Vec<String> {
        let mut parts = Vec::with_capacity(4);
        for (on, name) in [(self.ctrl, "Ctrl"), (self.shift, "Shift"), (self.alt, "Alt")] {
            if on {
                parts.push(name.to_owned());
            }
        }
        parts.push(key_name(self.vk).unwrap_or_else(|| format!("{:#04x}", self.vk)));
        parts
    }

    pub(crate) fn text(self) -> String {
        self.parts().join("+")
    }

    /// `ACCEL::fVirt`'s modifier bits (without `FVIRTKEY`).
    pub(crate) fn accel_flags(self) -> u8 {
        let mut flags = 0;
        if self.ctrl {
            flags |= FCONTROL;
        }
        if self.shift {
            flags |= FSHIFT;
        }
        if self.alt {
            flags |= FALT;
        }
        flags
    }
}

impl fmt::Display for KeyStroke {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_spell_like_vs_code_and_parse_back() {
        // Break caught: a stroke that formats one way and parses another, so a saved override
        // stops loading after one round trip through fastpad.ini.
        let cases = [
            (KeyStroke::new(true, false, false, u16::from(b'S')), "Ctrl+S"),
            (KeyStroke::new(true, true, false, u16::from(b'S')), "Ctrl+Shift+S"),
            (KeyStroke::new(false, true, true, u16::from(b'F')), "Shift+Alt+F"),
            (KeyStroke::new(true, false, false, VK_OEM_PLUS), "Ctrl+="),
            (KeyStroke::new(true, false, false, VK_OEM_5), "Ctrl+\\"),
            (KeyStroke::new(true, false, false, VK_OEM_COMMA), "Ctrl+,"),
            (KeyStroke::new(false, false, false, VK_F3), "F3"),
            (KeyStroke::new(false, true, false, VK_F24), "Shift+F24"),
            (KeyStroke::new(true, false, false, VK_NUMPAD0), "Ctrl+Numpad0"),
            (KeyStroke::new(true, false, false, VK_ADD), "Ctrl+NumpadAdd"),
            (KeyStroke::new(true, false, true, VK_RIGHT), "Ctrl+Alt+Right"),
            (KeyStroke::new(true, false, false, VK_PRIOR), "Ctrl+PageUp"),
            (KeyStroke::new(true, false, false, VK_TAB), "Ctrl+Tab"),
        ];
        for (stroke, text) in cases {
            assert_eq!(stroke.text(), text);
            assert_eq!(stroke.to_string(), text);
            assert_eq!(KeyStroke::parse(text), Some(stroke), "{text}");
        }
    }

    #[test]
    fn every_named_key_round_trips() {
        // Break caught: a key in the name table that parses to a different virtual key.
        let mut keys: Vec<u16> = (u16::from(b'0')..=u16::from(b'9'))
            .chain(u16::from(b'A')..=u16::from(b'Z'))
            .chain(VK_F1..=VK_F24)
            .chain(VK_NUMPAD0..=VK_NUMPAD9)
            .collect();
        keys.extend(NAMED_KEYS.iter().map(|(vk, _)| *vk));
        for vk in keys {
            let stroke = KeyStroke::new(true, false, false, vk);
            assert_eq!(KeyStroke::parse(&stroke.text()), Some(stroke), "{vk:#x}");
        }
    }

    #[test]
    fn parsing_ignores_case_and_spaces_and_puts_modifiers_in_order() {
        // Break caught: a hand-written "alt + shift + ctrl + z" rejected, or saved back in the
        // user's order so the same stroke has two spellings.
        let stroke = KeyStroke::parse(" alt + shift + control + z ").unwrap();
        assert_eq!(stroke, KeyStroke::new(true, true, true, u16::from(b'Z')));
        assert_eq!(stroke.text(), "Ctrl+Shift+Alt+Z");
        assert_eq!(KeyStroke::parse("pageup"), Some(KeyStroke::new(false, false, false, VK_PRIOR)));
    }

    #[test]
    fn nonsense_does_not_parse() {
        // Break caught: a typo in fastpad.ini silently binding some other key.
        for text in [
            "", "Ctrl+", "+S", "Hyper+S", "Ctrl+Foo", "Numpad10", "Numpad09", "F0", "F25",
            "Ctrl+S+X", "Ctrl++",
        ] {
            assert_eq!(KeyStroke::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn modifier_keys_and_unnamed_keys_are_not_strokes() {
        // Break caught: pressing Ctrl alone in the recording box recorded "Ctrl+Ctrl".
        for vk in [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, 0xE5] {
            assert_eq!(KeyStroke::from_key(vk, true, false, false), None, "{vk:#x}");
        }
        assert_eq!(
            KeyStroke::from_key(u16::from(b'K'), false, false, true),
            Some(KeyStroke::new(false, false, true, u16::from(b'K')))
        );
    }

    #[test]
    fn accelerator_flags_carry_the_modifiers() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{FALT, FCONTROL, FSHIFT};
        assert_eq!(KeyStroke::new(true, true, true, u16::from(b'A')).accel_flags(), FCONTROL | FSHIFT | FALT);
        assert_eq!(KeyStroke::new(false, false, false, VK_F3).accel_flags(), 0);
    }
}
