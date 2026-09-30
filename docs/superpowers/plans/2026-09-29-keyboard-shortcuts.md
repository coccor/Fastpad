# Keyboard Shortcuts Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A VS Code–style Keyboard Shortcuts page in the Settings dialog that lists every command with its keys and lets the user change, add, remove and reset them, saved as overrides in `fastpad.ini`.

**Architecture:** A pure `window::keymap` module owns key strokes, command IDs, the default bindings and the resolved keymap. `config` stores the raw `key.<id>=` lines as text. The main window holds the resolved `Keymap` in `App` and rebuilds the Win32 accelerator table, menus and palette hints from it. The Settings dialog gains a left nav and a second page whose behaviour lives in a pure `shortcuts_model` and whose painting, hit-testing and search field live in `shortcuts_page`.

**Tech Stack:** Rust 2024, `windows-sys` 0.61.2 (Win32 accelerators, EDIT control, clipboard), the existing `soft_paint` Direct2D/GDI painter.

**Spec:** `docs/superpowers/specs/2026-09-29-keyboard-shortcuts-design.md`

## Global Constraints

- Branch `feat/keyboard-shortcuts`, stacked on `feat/settings-dialog`. `main` is protected: the work lands by PR.
- No chords. No `when` clauses. No "reset all".
- Storage: overrides only, `key.<command-id>=<keys>` in `fastpad.ini`, keys joined with `, `; empty value = unbound; no line = defaults. No migration, no compatibility readers.
- Key text: modifiers in the order `Ctrl+Shift+Alt+`, VS Code key names (`=`, `\`, `PageUp`, `Numpad0`, `NumpadAdd`, …).
- Conflicts are allowed. User beats default; among defaults the earlier `DEFAULT_BINDINGS` entry wins; among user bindings the command whose ID sorts first wins.
- Each confirmed change applies and saves at once. No OK/Cancel.
- Latency: nothing new runs before first input. The defaults are a `const` table; overrides apply in the deferred `WM_FASTPAD_LOAD_SETTINGS` unit. Check with `fastpad-bench` (Task 11).
- Tests follow the repo style: a `// Break caught:` comment naming the regression each test guards.
- Compile with `cargo clippy --all-targets -- -D warnings`. Run only the targeted tests per task. The full suite runs once, in Task 11. Window tests need `-- --test-threads=1`.
- Before any live run of the app, back up `%LocalAppData%\FastPad\fastpad.ini` and restore it afterwards.
- `windows-sys` features are not audited (only the `windows` crate is), so adding `Win32_System_DataExchange` needs no change to `tools/audit-dependencies.ps1`.

## Review Focus

1. **A hand-edited `key.file.save=A`** (a plain typing key) must be refused with a `fastpad.ini:` warning and must not bind `A`; otherwise typing "a" saves. Pinned in Task 2 (`from_ini_refuses_keys_that_would_stop_typing`).
2. **Two user overrides on the same key** must resolve the same way every run, and the page must show the clash. Pinned in Task 2 (`two_user_bindings_on_one_key_resolve_by_command_id`).
3. **Setting a command's keys back to exactly its defaults** must remove the `key.` line, not write a duplicate of the defaults, so a later change of the defaults still reaches the user. Pinned in Task 2 (`with_keys_equal_to_the_defaults_drops_the_override`) and Task 4 (window test).
4. **F10 and Alt combinations while recording** arrive as `WM_SYSKEYDOWN`. They must reach the recorder instead of opening the system menu or beeping. Pinned in Task 10 (`recording_sees_f10_and_refuses_it`).
5. **Record-keys search mode** must not also insert the key's character into the field (`S` then `s`). Pinned in Task 10 (`record_keys_search_shows_only_the_stroke`).

---

## File Structure

| File | Responsibility |
|---|---|
| Create `src/window/keymap.rs` | `KeyStroke` (parse/format), command IDs, `DEFAULT_BINDINGS`, `Keymap` (resolve, conflicts, overrides), `bindable`. Pure. |
| Create `src/window/shortcuts_model.rs` | The page's rows, filter, selection, recording box and the effects of each input. Pure. |
| Create `src/window/shortcuts_page.rs` | The page's layout, painting, hit-testing, and the search EDIT with its subclass. |
| Create `src/platform/clipboard.rs` | `set_text` for Copy command ID. |
| Modify `src/config/persisted.rs`, `src/config/defaults.rs` | `key_overrides` in `Settings`/`SettingsDelta`, parse, `remove_setting`. |
| Modify `src/window/menus.rs` | Accelerator table from a `Keymap`; menu labels get key text from the keymap; `track_choice`. |
| Modify `src/window/command_palette.rs` | Hints from the keymap; new palette entry. |
| Modify `src/window/commands.rs` | `CommandId::OpenKeyboardShortcuts = 237`. |
| Modify `src/app.rs` | `App.keymap`. |
| Modify `src/window/main_window.rs` | `keymap`, `install_keymap`, `set_command_keys`, `reset_command_keys`, loading, `show_keyboard_shortcuts`. |
| Modify `src/window/settings_model.rs` | `Page`, nav/search/table focus stops, `Key::NextPage`, `Effect::ShowPage`. |
| Modify `src/window/settings_dialog.rs` | Nav column, pages, routing input to the page, the search field's colours and messages. |
| Modify `src/window/notebook_view.rs`, `src/window/inline_name.rs` | Drop a typed-in hint; a test reads the keymap. |
| Modify `src/window/mod.rs`, `src/platform/mod.rs`, `Cargo.toml` | Module declarations, `Win32_System_DataExchange`. |

---

### Task 1: Key strokes

**Files:**
- Create: `src/window/keymap.rs`
- Modify: `src/window/mod.rs` (add `pub(crate) mod keymap;` in alphabetical position, after `pub(crate) mod inline_name;` or wherever `k` sorts)

**Interfaces:**
- Produces: `KeyStroke { ctrl: bool, shift: bool, alt: bool, vk: u16 }` (`Copy`, `Eq`, `Hash`, `Debug`), `KeyStroke::new(ctrl, shift, alt, vk) -> Self` (const), `KeyStroke::from_key(vk, ctrl, shift, alt) -> Option<Self>`, `KeyStroke::parse(&str) -> Option<Self>`, `KeyStroke::parts(self) -> Vec<String>`, `KeyStroke::text(self) -> String`, `KeyStroke::accel_flags(self) -> u8`, `impl Display`.

- [ ] **Step 1: Write the failing tests**

Create `src/window/keymap.rs` with only the test module first:

```rust
//! Keyboard shortcuts: key strokes, the commands' stable IDs, the default bindings, and the
//! user's overrides resolved into the bindings every surface reads (keyboard shortcuts spec §3).
//! Pure: no window handles.

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
```

Add `pub(crate) mod keymap;` to `src/window/mod.rs`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib window::keymap`
Expected: compile errors (`KeyStroke`, `NAMED_KEYS`, `VK_*` not found).

- [ ] **Step 3: Implement `KeyStroke`**

Insert above the test module:

```rust
use std::fmt;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_ADD, VK_BACK, VK_CONTROL, VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END, VK_ESCAPE,
    VK_F1, VK_F24, VK_HOME, VK_INSERT, VK_LEFT, VK_MENU, VK_MULTIPLY, VK_NEXT, VK_NUMPAD0,
    VK_NUMPAD9, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7,
    VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_PRIOR, VK_RETURN, VK_RIGHT,
    VK_SPACE, VK_SUBTRACT, VK_TAB, VK_UP,
};
#[cfg(test)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_F3, VK_LWIN, VK_SHIFT};
use windows_sys::Win32::UI::WindowsAndMessaging::{FALT, FCONTROL, FSHIFT};

/// One key with its modifiers: what a shortcut is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct KeyStroke {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub vk: u16,
}

/// Keys named by a word or by the character they type on a US layout (spec §3.1). Letters,
/// digits, F-keys and numpad digits are named by `key_name`'s ranges instead.
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
```

`VK_CONTROL` and `VK_MENU` are used only in tests at this point. If clippy flags them as unused imports, move them into the `#[cfg(test)]` import line; Task 2 uses neither outside tests.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib window::keymap`
Expected: 6 passed.

- [ ] **Step 5: Clippy and commit**

Run: `cargo clippy --all-targets -- -D warnings` (expect dead-code warnings to be absent because every item is used by tests; if `dead_code` fires for non-test builds, add `#[cfg_attr(not(test), allow(dead_code, reason = "used from Task 2 on"))]` on the items and remove it in Task 4).

```bash
git add src/window/keymap.rs src/window/mod.rs
git commit -m "feat: key strokes that spell and parse like VS Code's"
```

---

### Task 2: Command IDs, default bindings and the resolved keymap

**Files:**
- Modify: `src/window/keymap.rs`

**Interfaces:**
- Consumes: `KeyStroke` (Task 1).
- Produces:
  - `pub(crate) const COMMAND_IDS: &[(CommandId, &str)]`, `command_id(CommandId) -> Option<&'static str>`, `command_for_id(&str) -> Option<CommandId>` (case-sensitive).
  - `pub(crate) const DEFAULT_BINDINGS: [(KeyStroke, CommandId); 66]`, `default_keys(CommandId) -> Vec<KeyStroke>`.
  - `enum Source { Default, User }`, `struct Binding { stroke, command, source }`.
  - `struct Keymap` (`Clone`, `Debug`, `Eq`, `PartialEq`): `defaults()`, `from_ini(&BTreeMap<String, String>) -> (Keymap, Vec<String>)`, `bindings() -> &[Binding]`, `keys_of(CommandId) -> Vec<KeyStroke>`, `is_user(CommandId) -> bool`, `command_for(KeyStroke) -> Option<CommandId>`, `conflicts(KeyStroke, except: CommandId) -> Vec<CommandId>`, `first_text(CommandId) -> Option<String>`, `with_keys(CommandId, Vec<KeyStroke>) -> Keymap`, `without_override(CommandId) -> Keymap`.
  - `ini_key(CommandId) -> Option<String>` (`"key.file.save"`), `ini_value(&[KeyStroke]) -> String` (`"Ctrl+S, Ctrl+T"`).
  - `bindable(KeyStroke) -> Result<(), &'static str>`, `const TYPING_REFUSAL: &str`.

- [ ] **Step 1: Write the failing tests**

Append to the test module in `src/window/keymap.rs`:

```rust
    use crate::window::commands::CommandId;
    use std::collections::BTreeMap;

    fn stroke(text: &str) -> KeyStroke {
        KeyStroke::parse(text).unwrap()
    }

    fn ini(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn every_command_has_one_unique_id() {
        // Break caught: a new command with no ID (it can't be rebound or saved), or two
        // commands sharing an ID so one override moves both.
        let mut seen = std::collections::HashSet::new();
        for value in 0..=u16::MAX {
            if let Ok(command) = CommandId::try_from(value) {
                let id = command_id(command).unwrap_or_else(|| panic!("{command:?} has no ID"));
                assert!(seen.insert(id), "duplicate ID {id}");
                assert_eq!(command_for_id(id), Some(command));
            }
        }
        assert_eq!(seen.len(), COMMAND_IDS.len());
        assert_eq!(command_for_id("File.Save"), None, "IDs are case-sensitive");
    }

    #[test]
    fn defaults_match_the_accelerator_table() {
        // Break caught: a shortcut lost or changed while moving the table into the keymap.
        // Deleted in Task 4 together with `menus::accelerator_specs`.
        let old = crate::window::menus::accelerator_specs();
        assert_eq!(old.len(), DEFAULT_BINDINGS.len());
        for (spec, (stroke, command)) in old.iter().zip(DEFAULT_BINDINGS) {
            assert_eq!((spec.modifiers, spec.key, spec.command), (stroke.accel_flags(), stroke.vk, command));
        }
    }

    #[test]
    fn the_defaults_resolve_with_first_key_text() {
        let keymap = Keymap::defaults();
        assert_eq!(keymap.command_for(stroke("Ctrl+S")), Some(CommandId::Save));
        assert_eq!(keymap.first_text(CommandId::Save).as_deref(), Some("Ctrl+S"));
        assert_eq!(keymap.first_text(CommandId::ZoomIn).as_deref(), Some("Ctrl+="));
        assert_eq!(keymap.keys_of(CommandId::ZoomIn).len(), 3);
        assert_eq!(keymap.first_text(CommandId::About), None);
        assert!(!keymap.is_user(CommandId::Save));
        assert!(keymap.bindings().iter().all(|binding| binding.source == Source::Default));
    }

    #[test]
    fn a_user_override_replaces_the_commands_defaults_and_beats_them_on_a_shared_key() {
        // Break caught: an override added next to the defaults instead of replacing them, or a
        // default winning the key the user just took.
        let (keymap, warnings) = Keymap::from_ini(&ini(&[("search.find", "Ctrl+S, F9")]));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(keymap.keys_of(CommandId::Find), [stroke("Ctrl+S"), stroke("F9")]);
        assert!(keymap.is_user(CommandId::Find));
        assert_eq!(keymap.command_for(stroke("Ctrl+F")), None);
        assert_eq!(keymap.command_for(stroke("Ctrl+S")), Some(CommandId::Find));
        assert_eq!(keymap.conflicts(stroke("Ctrl+S"), CommandId::Find), [CommandId::Save]);
        assert_eq!(keymap.conflicts(stroke("Ctrl+S"), CommandId::Save), [CommandId::Find]);
        assert!(keymap.conflicts(stroke("F9"), CommandId::Find).is_empty());
    }

    #[test]
    fn two_user_bindings_on_one_key_resolve_by_command_id() {
        // Break caught: which of two clashing overrides wins changing from run to run.
        let (keymap, _) = Keymap::from_ini(&ini(&[("view.zoomIn", "F9"), ("edit.undo", "F9")]));
        assert_eq!(keymap.command_for(stroke("F9")), Some(CommandId::Undo));
        assert_eq!(keymap.conflicts(stroke("F9"), CommandId::Undo), [CommandId::ZoomIn]);
    }

    #[test]
    fn an_empty_value_unbinds_and_bad_parts_are_warned_and_skipped() {
        // Break caught: a typo dropping the whole fastpad.ini line, or an unknown command
        // silently ignored.
        let (keymap, warnings) = Keymap::from_ini(&ini(&[
            ("file.save", ""),
            ("file.open", "Ctrl+Foo, F9"),
            ("nope.command", "Ctrl+Q"),
            ("file.new", "Ctrl+Foo"),
        ]));
        assert!(keymap.keys_of(CommandId::Save).is_empty());
        assert!(keymap.is_user(CommandId::Save));
        assert_eq!(keymap.keys_of(CommandId::Open), [stroke("F9")]);
        // Nothing usable on the line: the defaults stay.
        assert_eq!(keymap.keys_of(CommandId::New), [stroke("Ctrl+N"), stroke("Ctrl+T")]);
        assert!(!keymap.is_user(CommandId::New));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("key.nope.command") && w.contains("unknown command")));
        assert!(warnings.iter().any(|w| w.contains("key.file.open") && w.contains("Ctrl+Foo")));
    }

    #[test]
    fn from_ini_refuses_keys_that_would_stop_typing() {
        // Break caught: `key.file.save=A` in fastpad.ini binding plain A, so typing "a" saves.
        let (keymap, warnings) = Keymap::from_ini(&ini(&[("file.save", "A, Ctrl+Alt+S")]));
        assert_eq!(keymap.keys_of(CommandId::Save), [stroke("Ctrl+Alt+S")]);
        assert_eq!(keymap.command_for(stroke("A")), None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("key.file.save") && warnings[0].contains("\"A\""));
    }

    #[test]
    fn with_keys_equal_to_the_defaults_drops_the_override() {
        // Break caught: resetting by hand leaving `key.file.new=Ctrl+N, Ctrl+T` in fastpad.ini,
        // so a later change to the defaults never reaches the user.
        let changed = Keymap::defaults().with_keys(CommandId::New, vec![stroke("F9")]);
        assert!(changed.is_user(CommandId::New));
        let back = changed.with_keys(CommandId::New, vec![stroke("Ctrl+N"), stroke("Ctrl+T")]);
        assert!(!back.is_user(CommandId::New));
        assert_eq!(back, Keymap::defaults());
        assert_eq!(changed.without_override(CommandId::New), Keymap::defaults());
    }

    #[test]
    fn the_comma_key_survives_a_list_of_keys() {
        // Break caught: `key.preferences.openSettings=Ctrl+,, F9` split at the comma key's own
        // comma, so Settings loses Ctrl+, after one save.
        let keys = vec![stroke("Ctrl+,"), stroke("F9"), stroke("Shift+Alt+,")];
        let value = ini_value(&keys);
        assert_eq!(value, "Ctrl+,, F9, Shift+Alt+,");
        let (keymap, warnings) = Keymap::from_ini(&ini(&[("preferences.openSettings", &value)]));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(keymap.keys_of(CommandId::OpenSettings), keys);
    }

    #[test]
    fn with_keys_drops_repeats_and_keeps_order() {
        let keymap = Keymap::defaults().with_keys(CommandId::Save, vec![stroke("F9"), stroke("F8"), stroke("F9")]);
        assert_eq!(keymap.keys_of(CommandId::Save), [stroke("F9"), stroke("F8")]);
        assert_eq!(ini_value(&keymap.keys_of(CommandId::Save)), "F9, F8");
        assert_eq!(ini_key(CommandId::Save).as_deref(), Some("key.file.save"));
    }

    #[test]
    fn typing_keys_need_ctrl_or_alt() {
        // Break caught: plain letters, Space or Backspace accepted as shortcuts, so they stop
        // typing; or F-keys and arrows refused although they type nothing.
        for text in ["A", "Shift+A", "5", "Space", "Backspace", "Delete", "Shift+Delete", "Tab", "=", "Numpad5", "NumpadAdd", "Enter", "Escape"] {
            assert_eq!(bindable(stroke(text)), Err(TYPING_REFUSAL), "{text}");
        }
        for text in ["Ctrl+A", "Alt+A", "F9", "Shift+F9", "Home", "Ctrl+Space", "Shift+PageDown", "Insert", "Ctrl+Numpad5"] {
            assert_eq!(bindable(stroke(text)), Ok(()), "{text}");
        }
        assert!(bindable(stroke("Alt+Numpad0")).is_err(), "Alt codes");
        assert!(bindable(stroke("F10")).is_err(), "menu");
        assert!(bindable(stroke("Shift+F10")).is_err(), "context menu");
        assert_eq!(bindable(stroke("Ctrl+F10")), Ok(()));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib window::keymap`
Expected: compile errors (`COMMAND_IDS`, `Keymap`, `DEFAULT_BINDINGS`, … not found).

- [ ] **Step 3: Implement**

Add below the `KeyStroke` code (above the tests):

```rust
use crate::window::commands::CommandId;
use std::collections::BTreeMap;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_F3, VK_F6, VK_F10, VK_SHIFT};

/// Every command's stable ID, used in `key.<id>=` lines (spec §3.2). Never rename one: a
/// renamed ID orphans the user's saved override.
pub(crate) const COMMAND_IDS: &[(CommandId, &str)] = &[
    (CommandId::New, "file.new"),
    (CommandId::Open, "file.open"),
    (CommandId::OpenFolder, "file.openNotebook"),
    (CommandId::OpenRecentFolder, "file.openRecentNotebook"),
    (CommandId::QuickOpen, "file.goToNote"),
    (CommandId::Save, "file.save"),
    (CommandId::SaveAs, "file.saveAs"),
    (CommandId::CloseTab, "file.closeTab"),
    (CommandId::CloseAllTabs, "file.closeAllTabs"),
    (CommandId::ToggleRestoreSession, "file.toggleRestoreSession"),
    (CommandId::Exit, "file.exit"),
    (CommandId::Undo, "edit.undo"),
    (CommandId::Redo, "edit.redo"),
    (CommandId::Cut, "edit.cut"),
    (CommandId::Copy, "edit.copy"),
    (CommandId::Paste, "edit.paste"),
    (CommandId::Find, "search.find"),
    (CommandId::Replace, "search.replace"),
    (CommandId::FindNext, "search.findNext"),
    (CommandId::FindPrevious, "search.findPrevious"),
    (CommandId::ReplaceInNotes, "search.replaceInNotes"),
    (CommandId::SearchToggleCase, "search.toggleMatchCase"),
    (CommandId::SearchToggleWholeWord, "search.toggleWholeWord"),
    (CommandId::SearchToggleRegex, "search.toggleRegex"),
    (CommandId::FormatJson, "json.format"),
    (CommandId::ValidateJson, "json.validate"),
    (CommandId::LanguagePlainText, "language.plainText"),
    (CommandId::LanguageBash, "language.bash"),
    (CommandId::LanguageBatch, "language.batch"),
    (CommandId::LanguageC, "language.c"),
    (CommandId::LanguageCSharp, "language.csharp"),
    (CommandId::LanguageCpp, "language.cpp"),
    (CommandId::LanguageCss, "language.css"),
    (CommandId::LanguageEnv, "language.env"),
    (CommandId::LanguageHtml, "language.html"),
    (CommandId::LanguageIni, "language.ini"),
    (CommandId::LanguageJavaScript, "language.javascript"),
    (CommandId::LanguageJson, "language.json"),
    (CommandId::LanguageMarkdown, "language.markdown"),
    (CommandId::LanguagePowerShell, "language.powershell"),
    (CommandId::LanguageProperties, "language.properties"),
    (CommandId::LanguagePython, "language.python"),
    (CommandId::LanguageRust, "language.rust"),
    (CommandId::LanguageSql, "language.sql"),
    (CommandId::LanguageSvg, "language.svg"),
    (CommandId::LanguageToml, "language.toml"),
    (CommandId::LanguageTypeScript, "language.typescript"),
    (CommandId::LanguageXml, "language.xml"),
    (CommandId::LanguageYaml, "language.yaml"),
    (CommandId::MarkdownPreviewCycle, "markdown.cyclePreview"),
    (CommandId::MarkdownPreviewSide, "markdown.previewSide"),
    (CommandId::MarkdownPreviewFull, "markdown.previewFull"),
    (CommandId::MarkdownPreviewClose, "markdown.closePreview"),
    (CommandId::NextTab, "view.nextTab"),
    (CommandId::PreviousTab, "view.previousTab"),
    (CommandId::SelectTab1, "view.selectTab1"),
    (CommandId::SelectTab2, "view.selectTab2"),
    (CommandId::SelectTab3, "view.selectTab3"),
    (CommandId::SelectTab4, "view.selectTab4"),
    (CommandId::SelectTab5, "view.selectTab5"),
    (CommandId::SelectTab6, "view.selectTab6"),
    (CommandId::SelectTab7, "view.selectTab7"),
    (CommandId::SelectTab8, "view.selectTab8"),
    (CommandId::SelectTab9, "view.selectTab9"),
    (CommandId::CommandPalette, "view.commandPalette"),
    (CommandId::ToggleSidebar, "view.toggleSidebar"),
    (CommandId::ShowNotebookView, "view.showNotebook"),
    (CommandId::ShowSearchView, "view.showSearch"),
    (CommandId::ShowFavoritesView, "view.showFavorites"),
    (CommandId::FocusNextPane, "view.focusNextPane"),
    (CommandId::FocusPreviousPane, "view.focusPreviousPane"),
    (CommandId::SplitRight, "view.splitRight"),
    (CommandId::SplitDown, "view.splitDown"),
    (CommandId::CloseGroup, "view.closeGroup"),
    (CommandId::FocusGroup1, "view.focusGroup1"),
    (CommandId::FocusGroup2, "view.focusGroup2"),
    (CommandId::FocusGroup3, "view.focusGroup3"),
    (CommandId::FocusGroup4, "view.focusGroup4"),
    (CommandId::FocusGroup5, "view.focusGroup5"),
    (CommandId::FocusGroup6, "view.focusGroup6"),
    (CommandId::FocusGroup7, "view.focusGroup7"),
    (CommandId::FocusGroup8, "view.focusGroup8"),
    (CommandId::FocusLastGroup, "view.focusLastGroup"),
    (CommandId::MoveTabToNextGroup, "view.moveToNextGroup"),
    (CommandId::MoveTabToPreviousGroup, "view.moveToPreviousGroup"),
    (CommandId::ZoomIn, "view.zoomIn"),
    (CommandId::ZoomOut, "view.zoomOut"),
    (CommandId::ZoomReset, "view.zoomReset"),
    (CommandId::ToggleWordWrap, "view.toggleWordWrap"),
    (CommandId::ToggleLineNumbers, "view.toggleLineNumbers"),
    (CommandId::FontSizeIncrease, "view.fontSizeIncrease"),
    (CommandId::FontSizeDecrease, "view.fontSizeDecrease"),
    (CommandId::FontSizeReset, "view.fontSizeReset"),
    (CommandId::ThemeSystem, "theme.system"),
    (CommandId::ThemeLight, "theme.light"),
    (CommandId::ThemeDark, "theme.dark"),
    (CommandId::ThemeCatppuccin, "theme.catppuccin"),
    (CommandId::ThemeCatppuccinLatte, "theme.catppuccinLatte"),
    (CommandId::ThemeCatppuccinFrappe, "theme.catppuccinFrappe"),
    (CommandId::ThemeCatppuccinMacchiato, "theme.catppuccinMacchiato"),
    (CommandId::ThemeCatppuccinMocha, "theme.catppuccinMocha"),
    (CommandId::FileIconsMaterial, "fileIcons.material"),
    (CommandId::FileIconsMinimal, "fileIcons.minimal"),
    (CommandId::FileIconsSolid, "fileIcons.solid"),
    (CommandId::TabWidth2, "editor.tabWidth2"),
    (CommandId::TabWidth4, "editor.tabWidth4"),
    (CommandId::TabWidth8, "editor.tabWidth8"),
    (CommandId::ToggleInsertSpaces, "editor.toggleInsertSpaces"),
    (CommandId::ToggleShowWhitespace, "editor.toggleShowWhitespace"),
    (CommandId::ToggleHighlightCurrentLine, "editor.toggleHighlightCurrentLine"),
    (CommandId::ToggleNotesMode, "notes.toggleNotesMode"),
    (CommandId::ToggleFolderAutosave, "notes.toggleAutosave"),
    (CommandId::CloseNotebook, "notebook.close"),
    (CommandId::ToggleNotebookFavorite, "notebook.toggleFavorite"),
    (CommandId::NoteNew, "notebook.newNote"),
    (CommandId::NoteNewFolder, "notebook.newFolder"),
    (CommandId::NoteReloadFromDisk, "note.reloadFromDisk"),
    (CommandId::NoteKeepMine, "note.keepMine"),
    (CommandId::NoteTogglePin, "note.togglePin"),
    (CommandId::NoteMoveToNotebook, "note.moveToNotebook"),
    (CommandId::NoteRevealInExplorer, "note.revealInExplorer"),
    (CommandId::NoteRename, "note.rename"),
    (CommandId::NoteDelete, "note.delete"),
    (CommandId::OpenSettings, "preferences.openSettings"),
    (CommandId::EditSettingsFile, "preferences.editSettingsFile"),
    (CommandId::About, "help.about"),
];

pub(crate) fn command_id(command: CommandId) -> Option<&'static str> {
    COMMAND_IDS
        .iter()
        .find(|(candidate, _)| *candidate == command)
        .map(|(_, id)| *id)
}

pub(crate) fn command_for_id(id: &str) -> Option<CommandId> {
    COMMAND_IDS
        .iter()
        .find(|(_, candidate)| *candidate == id)
        .map(|(command, _)| *command)
}

const C: u8 = 1;
const S: u8 = 2;
const A: u8 = 4;

const fn key(modifiers: u8, vk: u16) -> KeyStroke {
    KeyStroke::new(modifiers & C != 0, modifiers & S != 0, modifiers & A != 0, vk)
}

const fn ch(byte: u8) -> u16 {
    byte as u16
}

/// FastPad's shortcuts before any override, in precedence order (spec §3.3).
pub(crate) const DEFAULT_BINDINGS: [(KeyStroke, CommandId); 66] = [
    (key(C, ch(b'N')), CommandId::New),
    (key(C, ch(b'T')), CommandId::New),
    (key(C, ch(b'O')), CommandId::Open),
    (key(C | S, ch(b'O')), CommandId::OpenFolder),
    (key(C | S, ch(b'M')), CommandId::NoteMoveToNotebook),
    (key(C, ch(b'S')), CommandId::Save),
    (key(C | S, ch(b'S')), CommandId::SaveAs),
    (key(C, ch(b'W')), CommandId::CloseTab),
    (key(C, ch(b'F')), CommandId::Find),
    (key(C, ch(b'H')), CommandId::Replace),
    (key(C | S, ch(b'H')), CommandId::ReplaceInNotes),
    (key(0, VK_F3), CommandId::FindNext),
    (key(S, VK_F3), CommandId::FindPrevious),
    (key(C, ch(b'Z')), CommandId::Undo),
    (key(C, ch(b'Y')), CommandId::Redo),
    (key(C | S, ch(b'F')), CommandId::ShowSearchView),
    (key(S | A, ch(b'F')), CommandId::FormatJson),
    (key(C, VK_TAB), CommandId::NextTab),
    (key(C | S, VK_TAB), CommandId::PreviousTab),
    // Ctrl+digits focus editor groups; Alt+digits select tabs (split editors spec §6).
    (key(C, ch(b'1')), CommandId::FocusGroup1),
    (key(C, ch(b'2')), CommandId::FocusGroup2),
    (key(C, ch(b'3')), CommandId::FocusGroup3),
    (key(C, ch(b'4')), CommandId::FocusGroup4),
    (key(C, ch(b'5')), CommandId::FocusGroup5),
    (key(C, ch(b'6')), CommandId::FocusGroup6),
    (key(C, ch(b'7')), CommandId::FocusGroup7),
    (key(C, ch(b'8')), CommandId::FocusGroup8),
    (key(C, ch(b'9')), CommandId::FocusLastGroup),
    (key(C, VK_NUMPAD0 + 1), CommandId::FocusGroup1),
    (key(C, VK_NUMPAD0 + 2), CommandId::FocusGroup2),
    (key(C, VK_NUMPAD0 + 3), CommandId::FocusGroup3),
    (key(C, VK_NUMPAD0 + 4), CommandId::FocusGroup4),
    (key(C, VK_NUMPAD0 + 5), CommandId::FocusGroup5),
    (key(C, VK_NUMPAD0 + 6), CommandId::FocusGroup6),
    (key(C, VK_NUMPAD0 + 7), CommandId::FocusGroup7),
    (key(C, VK_NUMPAD0 + 8), CommandId::FocusGroup8),
    (key(C, VK_NUMPAD9), CommandId::FocusLastGroup),
    (key(A, ch(b'1')), CommandId::SelectTab1),
    (key(A, ch(b'2')), CommandId::SelectTab2),
    (key(A, ch(b'3')), CommandId::SelectTab3),
    (key(A, ch(b'4')), CommandId::SelectTab4),
    (key(A, ch(b'5')), CommandId::SelectTab5),
    (key(A, ch(b'6')), CommandId::SelectTab6),
    (key(A, ch(b'7')), CommandId::SelectTab7),
    (key(A, ch(b'8')), CommandId::SelectTab8),
    (key(A, ch(b'9')), CommandId::SelectTab9),
    (key(C | A, VK_RIGHT), CommandId::MoveTabToNextGroup),
    (key(C | A, VK_LEFT), CommandId::MoveTabToPreviousGroup),
    // "+" shares a key with "=" on most layouts, so Ctrl+Shift+= is Ctrl++ as typed.
    (key(C, VK_OEM_PLUS), CommandId::ZoomIn),
    (key(C | S, VK_OEM_PLUS), CommandId::ZoomIn),
    (key(C, VK_ADD), CommandId::ZoomIn),
    (key(C, VK_OEM_MINUS), CommandId::ZoomOut),
    (key(C, VK_SUBTRACT), CommandId::ZoomOut),
    (key(C, ch(b'0')), CommandId::ZoomReset),
    (key(C, VK_NUMPAD0), CommandId::ZoomReset),
    (key(C, ch(b'P')), CommandId::QuickOpen),
    (key(C | S, ch(b'P')), CommandId::CommandPalette),
    (key(C, VK_OEM_COMMA), CommandId::OpenSettings),
    (key(C | S, ch(b'V')), CommandId::MarkdownPreviewCycle),
    (key(C, ch(b'B')), CommandId::ToggleSidebar),
    (key(C | S, ch(b'E')), CommandId::ShowNotebookView),
    (key(A, ch(b'Z')), CommandId::ToggleWordWrap),
    (key(0, VK_F6), CommandId::FocusNextPane),
    (key(S, VK_F6), CommandId::FocusPreviousPane),
    // The backslash key on a US layout (split editors spec §6).
    (key(C, VK_OEM_5), CommandId::SplitRight),
    (key(C | S, VK_OEM_5), CommandId::SplitDown),
];

pub(crate) fn default_keys(command: CommandId) -> Vec<KeyStroke> {
    DEFAULT_BINDINGS
        .iter()
        .filter(|(_, candidate)| *candidate == command)
        .map(|(stroke, _)| *stroke)
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Source {
    Default,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Binding {
    pub stroke: KeyStroke,
    pub command: CommandId,
    pub source: Source,
}

/// The defaults with the user's overrides applied: each overridden command has exactly the
/// user's keys (none when unbound), every other command its defaults. `bindings` is in
/// precedence order: user bindings (by command ID), then defaults (in table order).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Keymap {
    overrides: Vec<(CommandId, Vec<KeyStroke>)>,
    bindings: Vec<Binding>,
}

pub(crate) const TYPING_REFUSAL: &str = "Needs Ctrl or Alt: it would stop typing.";

/// Whether `stroke` may be a shortcut: a key that types or edits text needs Ctrl or Alt, Alt
/// with a numpad digit types an Alt code, and F10 / Shift+F10 belong to the menus (spec §6.5).
pub(crate) fn bindable(stroke: KeyStroke) -> Result<(), &'static str> {
    let vk = stroke.vk;
    let types = matches!(
        vk,
        0x30..=0x39
            | 0x41..=0x5A
            | VK_NUMPAD0..=VK_NUMPAD9
            | VK_SPACE
            | VK_BACK
            | VK_DELETE
            | VK_TAB
            | VK_RETURN
            | VK_ESCAPE
            | VK_ADD
            | VK_SUBTRACT
            | VK_MULTIPLY
            | VK_DIVIDE
            | VK_DECIMAL
            | VK_OEM_PLUS
            | VK_OEM_MINUS
            | VK_OEM_COMMA
            | VK_OEM_PERIOD
            | VK_OEM_1
            | VK_OEM_2
            | VK_OEM_3
            | VK_OEM_4
            | VK_OEM_5
            | VK_OEM_6
            | VK_OEM_7
    );
    if types && !stroke.ctrl && !stroke.alt {
        return Err(TYPING_REFUSAL);
    }
    if stroke.alt && !stroke.ctrl && (VK_NUMPAD0..=VK_NUMPAD9).contains(&vk) {
        return Err("Alt with a numpad digit types a character code.");
    }
    if vk == VK_F10 && !stroke.ctrl && !stroke.alt {
        return Err("F10 and Shift+F10 open the menus.");
    }
    Ok(())
}

/// `key.<id>`, the `fastpad.ini` key of `command`'s override.
pub(crate) fn ini_key(command: CommandId) -> Option<String> {
    command_id(command).map(|id| format!("key.{id}"))
}

/// The `fastpad.ini` value for `keys`: `Ctrl+S, Ctrl+T`, or empty when there are none.
pub(crate) fn ini_value(keys: &[KeyStroke]) -> String {
    keys.iter().map(|stroke| stroke.text()).collect::<Vec<_>>().join(", ")
}

/// A `fastpad.ini` value's keys, trimmed, empties dropped. A comma right after `+` is the comma
/// key (`Ctrl+,`), not a separator.
fn split_keys(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for c in value.chars() {
        if c == ',' && !current.trim_end().ends_with('+') {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

impl Keymap {
    pub(crate) fn defaults() -> Self {
        Self::resolve(Vec::new())
    }

    fn resolve(mut overrides: Vec<(CommandId, Vec<KeyStroke>)>) -> Self {
        overrides.sort_by_key(|(command, _)| command_id(*command));
        let mut bindings = Vec::with_capacity(DEFAULT_BINDINGS.len());
        for (command, keys) in &overrides {
            bindings.extend(keys.iter().map(|stroke| Binding {
                stroke: *stroke,
                command: *command,
                source: Source::User,
            }));
        }
        for (stroke, command) in DEFAULT_BINDINGS {
            if !overrides.iter().any(|(overridden, _)| *overridden == command) {
                bindings.push(Binding {
                    stroke,
                    command,
                    source: Source::Default,
                });
            }
        }
        Self { overrides, bindings }
    }

    /// The keymap `fastpad.ini`'s `key.<id>=` lines make (`entries` maps id to value), and one
    /// warning per part it skipped (spec §4).
    pub(crate) fn from_ini(entries: &BTreeMap<String, String>) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut overrides = Vec::new();
        for (id, value) in entries {
            let Some(command) = command_for_id(id) else {
                warnings.push(format!("key.{id}: unknown command"));
                continue;
            };
            let mut keys = Vec::new();
            let mut any = false;
            for part in split_keys(value) {
                let part = part.as_str();
                any = true;
                match KeyStroke::parse(part).map(|stroke| (stroke, bindable(stroke))) {
                    Some((stroke, Ok(()))) => {
                        if !keys.contains(&stroke) {
                            keys.push(stroke);
                        }
                    }
                    Some((_, Err(reason))) => {
                        warnings.push(format!("key.{id}: \"{part}\" can't be a shortcut: {reason}"));
                    }
                    None => warnings.push(format!("key.{id}: unknown key \"{part}\"")),
                }
            }
            // A value with nothing usable keeps the defaults; an empty one unbinds.
            if keys.is_empty() && any {
                continue;
            }
            overrides.push((command, keys));
        }
        (Self::resolve(overrides), warnings)
    }

    pub(crate) fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub(crate) fn keys_of(&self, command: CommandId) -> Vec<KeyStroke> {
        self.bindings
            .iter()
            .filter(|binding| binding.command == command)
            .map(|binding| binding.stroke)
            .collect()
    }

    pub(crate) fn is_user(&self, command: CommandId) -> bool {
        self.overrides.iter().any(|(overridden, _)| *overridden == command)
    }

    /// The command `stroke` runs: the first binding in precedence order.
    pub(crate) fn command_for(&self, stroke: KeyStroke) -> Option<CommandId> {
        self.bindings
            .iter()
            .find(|binding| binding.stroke == stroke)
            .map(|binding| binding.command)
    }

    /// The commands other than `except` bound to `stroke`, each once, in precedence order.
    pub(crate) fn conflicts(&self, stroke: KeyStroke, except: CommandId) -> Vec<CommandId> {
        let mut commands = Vec::new();
        for binding in &self.bindings {
            if binding.stroke == stroke && binding.command != except && !commands.contains(&binding.command) {
                commands.push(binding.command);
            }
        }
        commands
    }

    /// The text menus and the palette show for `command`: its first key.
    pub(crate) fn first_text(&self, command: CommandId) -> Option<String> {
        self.bindings
            .iter()
            .find(|binding| binding.command == command)
            .map(|binding| binding.stroke.text())
    }

    /// This keymap with `command` bound to exactly `keys` (repeats dropped). Keys equal to the
    /// defaults drop the override instead.
    pub(crate) fn with_keys(&self, command: CommandId, keys: Vec<KeyStroke>) -> Self {
        let mut unique = Vec::with_capacity(keys.len());
        for stroke in keys {
            if !unique.contains(&stroke) {
                unique.push(stroke);
            }
        }
        if unique == default_keys(command) {
            return self.without_override(command);
        }
        let mut overrides = self.without_override(command).overrides;
        overrides.push((command, unique));
        Self::resolve(overrides)
    }

    pub(crate) fn without_override(&self, command: CommandId) -> Self {
        Self::resolve(
            self.overrides
                .iter()
                .filter(|(overridden, _)| *overridden != command)
                .cloned()
                .collect(),
        )
    }
}
```

Remove `VK_SHIFT` from the non-test import if clippy flags it unused (it is only needed in tests; keep it in the `#[cfg(test)]` line from Task 1). Keep `VK_F3` in the non-test import now (the defaults use it) and drop it from the test-only line.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib window::keymap`
Expected: 17 passed.

- [ ] **Step 5: Clippy and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add src/window/keymap.rs
git commit -m "feat: keymap with command IDs, the default bindings and user overrides"
```

---

### Task 3: `key.` lines in fastpad.ini and removing a line

**Files:**
- Modify: `src/config/persisted.rs` (`Settings`, `apply_delta`, `SettingsDelta`, `parse` doc, `apply_line`, new `remove_setting*`)
- Modify: `src/config/defaults.rs` (`default_settings`)
- Modify: `src/config/mod.rs` if it re-exports `save_setting` (re-export `remove_setting` and `remove_setting_to` the same way)

**Interfaces:**
- Produces: `Settings.key_overrides: BTreeMap<String, String>` (id → value text), `SettingsDelta.key_overrides`, `persisted::remove_setting_text(source, key) -> String`, `config::remove_setting(key) -> Result<()>`, `config::remove_setting_to(path, key) -> Result<()>`.

- [ ] **Step 1: Write the failing tests**

Add to `persisted.rs`'s test module:

```rust
    #[test]
    fn key_lines_are_collected_as_text_for_the_keymap() {
        // Break caught: `key.` lines reported as unknown settings, or their values trimmed of
        // the `=` key's name.
        let delta = parse("key.file.save = Ctrl+Alt+S\nkey.view.zoomIn=Ctrl+=, Ctrl+NumpadAdd\nkey.file.new=\nkey.=F9\n");
        assert_eq!(delta.key_overrides.get("file.save").map(String::as_str), Some("Ctrl+Alt+S"));
        assert_eq!(delta.key_overrides.get("view.zoomIn").map(String::as_str), Some("Ctrl+=, Ctrl+NumpadAdd"));
        assert_eq!(delta.key_overrides.get("file.new").map(String::as_str), Some(""));
        assert_eq!(delta.warnings.len(), 1, "{:?}", delta.warnings);
        assert_eq!(delta.warnings[0].line, 4);
        let mut settings = crate::config::default_settings();
        settings.apply_delta(&delta);
        assert_eq!(settings.key_overrides.len(), 3);
    }

    #[test]
    fn removing_a_key_drops_only_its_lines() {
        // Break caught: Reset leaving `key.file.save=` behind (which unbinds Save), or
        // rewriting the rest of the user's file.
        let source = "\u{feff}key.file.save=F9\r\n# mine\r\ntheme=dark\r\nkey.file.save = F8\r\n";
        assert_eq!(remove_setting_text(source, "key.file.save"), "\u{feff}# mine\r\ntheme=dark\r\n");
        assert_eq!(remove_setting_text("theme=dark", "key.file.save"), "theme=dark");
        assert_eq!(remove_setting_text("# key.file.save=F9\n", "key.file.save"), "# key.file.save=F9\n");
    }

    #[test]
    fn removing_from_a_missing_file_is_nothing_to_do() {
        let directory = std::env::temp_dir().join(format!(
            "fastpad-remove-setting-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("fastpad.ini");
        remove_setting_to(&path, "key.file.save").unwrap();
        assert!(!path.exists());
        save_setting_to(&path, "key.file.save", "F9").unwrap();
        save_setting_to(&path, "theme", "dark").unwrap();
        remove_setting_to(&path, "key.file.save").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theme=dark\n");
        let _ = std::fs::remove_dir_all(&directory);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib config::persisted`
Expected: compile errors (`key_overrides`, `remove_setting_text`, `remove_setting_to` not found).

- [ ] **Step 3: Implement**

In `Settings`, after `highlight_current_line`:

```rust
    /// `key.<command-id>=` lines, id to value, as written: the keymap validates them when
    /// settings load (keyboard shortcuts spec §4).
    pub key_overrides: std::collections::BTreeMap<String, String>,
```

In `SettingsDelta`, before `warnings`:

```rust
    pub key_overrides: std::collections::BTreeMap<String, String>,
```

At the end of `apply_delta`:

```rust
        for (id, value) in &delta.key_overrides {
            self.key_overrides.insert(id.clone(), value.clone());
        }
```

At the top of `apply_line`, before the `match`:

```rust
    if let Some(id) = key.strip_prefix("key.") {
        if id.is_empty() {
            warn(delta, line_number, key, value);
        } else {
            delta.key_overrides.insert(id.to_owned(), value.to_owned());
        }
        return;
    }
```

Extend `parse`'s doc comment's list of recognized keys with: "`key.<command-id>` lines are collected as text into `key_overrides` for the keymap to validate."

In `defaults.rs`'s `default_settings`: `key_overrides: std::collections::BTreeMap::new(),`.

After `save_setting_to`, add:

```rust
/// Returns `source` without any line naming `key`; everything else is kept byte for byte. A BOM
/// on a removed first line stays at the start of the file.
pub fn remove_setting_text(source: &str, key: &str) -> String {
    let mut output = String::with_capacity(source.len());
    for raw_line in source.split_inclusive('\n') {
        let content = raw_line.trim_end_matches(['\r', '\n']);
        let bom = content.starts_with('\u{feff}');
        let names_key = trim_ascii(content.trim_start_matches('\u{feff}'))
            .split_once('=')
            .is_some_and(|(line_key, _)| trim_ascii(line_key) == key);
        if names_key {
            if bom {
                output.push('\u{feff}');
            }
        } else {
            output.push_str(raw_line);
        }
    }
    output
}

/// Removes one setting from the settings file. A missing file has nothing to remove.
pub fn remove_setting(key: &str) -> Result<()> {
    remove_setting_to(&settings_file_path()?, key)
}

pub fn remove_setting_to(path: &Path, key: &str) -> Result<()> {
    let source = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|_| crate::FastPadError::Invariant("the settings file is not valid UTF-8"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let updated = remove_setting_text(&source, key);
    if updated == source {
        return Ok(());
    }
    crate::file::saver::save_atomic(path, updated.as_bytes())
}
```

Check `src/config/mod.rs` for how `save_setting`/`save_setting_to` are re-exported and re-export `remove_setting` and `remove_setting_to` alongside them.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib config::`
Expected: all config tests pass (including the existing ones).

- [ ] **Step 5: Clippy and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add src/config
git commit -m "feat: collect key. lines from fastpad.ini and remove a setting line"
```

---

### Task 4: The accelerator table comes from the keymap

**Files:**
- Modify: `src/window/menus.rs` (delete `AcceleratorSpec`, `accelerator_specs`, `accelerator`, `virtual_key`; `AcceleratorTable::create(&Keymap)`; test helpers)
- Modify: `src/window/keymap.rs` (delete `defaults_match_the_accelerator_table`)
- Modify: `src/window/command_palette.rs` (`shortcut_text` reads `Keymap::defaults()` for now; `Ctrl++` → `Ctrl+=`)
- Modify: `src/window/inline_name.rs` (test at ~line 1588)
- Modify: `src/app.rs` (`keymap` field)
- Modify: `src/window/main_window.rs` (`keymap`, `install_keymap`, `apply_loaded_settings`, `set_command_keys`, `reset_command_keys`, test `remove_setting`)

**Interfaces:**
- Consumes: `Keymap`, `KeyStroke`, `ini_key`, `ini_value`, `default_keys` (Task 2); `Settings.key_overrides`, `config::remove_setting(_to)` (Task 3).
- Produces: `AcceleratorTable::create(keymap: &Keymap) -> Result<Self>`; `#[cfg(test)] AcceleratorTable::entries(&self) -> Vec<ACCEL>`; `App.keymap: Keymap`; `main_window::keymap(hwnd) -> Keymap`; `main_window::set_command_keys(hwnd, CommandId, Vec<KeyStroke>)`; `main_window::reset_command_keys(hwnd, CommandId)`.

- [ ] **Step 1: Write the failing window tests**

In `main_window.rs`'s test module (next to the settings dialog tests around line 26420), add:

```rust
    #[test]
    fn rebinding_save_rebuilds_the_accelerator_table_and_saves_one_line() {
        // Break caught: a new key saved but the old table still dispatching, or Reset leaving a
        // `key.file.save=` line that unbinds Save on the next start.
        use crate::window::keymap::KeyStroke;
        use windows_sys::Win32::UI::WindowsAndMessaging::{FALT, FCONTROL, FVIRTKEY};
        let scratch = RecoveryScratch::new("keymap-rebind");
        let ini = scratch.path().join("fastpad.ini");
        std::fs::write(&ini, "# kept\r\n").unwrap();
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        let save_entries = |hwnd| {
            app_mut(hwnd)
                .accelerators
                .as_ref()
                .unwrap()
                .entries()
                .into_iter()
                .filter(|entry| entry.cmd == CommandId::Save as u16)
                .map(|entry| (entry.fVirt, entry.key))
                .collect::<Vec<_>>()
        };
        assert_eq!(save_entries(window.hwnd), [(FVIRTKEY | FCONTROL, u16::from(b'S'))]);

        super::set_command_keys(window.hwnd, CommandId::Save, vec![KeyStroke::parse("Ctrl+Alt+S").unwrap()]);
        assert_eq!(save_entries(window.hwnd), [(FVIRTKEY | FCONTROL | FALT, u16::from(b'S'))]);
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "# kept\r\nkey.file.save=Ctrl+Alt+S\r\n");
        assert_eq!(
            app_mut(window.hwnd).settings.key_overrides.get("file.save").map(String::as_str),
            Some("Ctrl+Alt+S")
        );

        super::reset_command_keys(window.hwnd, CommandId::Save);
        assert_eq!(save_entries(window.hwnd), [(FVIRTKEY | FCONTROL, u16::from(b'S'))]);
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "# kept\r\n");
        assert!(app_mut(window.hwnd).settings.key_overrides.is_empty());

        // Keys equal to the defaults are a reset too.
        super::set_command_keys(window.hwnd, CommandId::Save, vec![]);
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "# kept\r\nkey.file.save=\r\n");
        super::set_command_keys(window.hwnd, CommandId::Save, vec![KeyStroke::parse("Ctrl+S").unwrap()]);
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "# kept\r\n");
        super::save_settings_to(None);
    }

    #[test]
    fn loaded_key_overrides_rebuild_the_table_and_warn_about_bad_lines() {
        // Break caught: `key.` lines read but never applied, or an unknown command dropped
        // without telling the user.
        let window = ProductionWindow::new(make_app());
        let mut settings = crate::config::default_settings();
        settings.key_overrides.insert("search.find".into(), "F9".into());
        settings.key_overrides.insert("nope.command".into(), "F8".into());
        super::apply_loaded_settings(window.hwnd, settings, Vec::new());
        let app = app_mut(window.hwnd);
        assert!(app.keymap.is_user(CommandId::Find));
        assert!(
            app.accelerators.as_ref().unwrap().entries().iter().any(|entry| {
                entry.cmd == CommandId::Find as u16 && entry.key == windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F9
            })
        );
        assert!(
            app.notifications
                .pending()
                .iter()
                .any(|notification| notification.message.contains("key.nope.command"))
        );
    }
```

If `Notification::message` isn't visible from here, use whatever accessor `notification.rs` offers (read the struct; add `pub` to the field if needed).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib main_window::tests::rebinding_save -- --test-threads=1`
Expected: compile errors (`set_command_keys`, `entries`, `keymap` not found).

- [ ] **Step 3: Implement the accelerator table and the test helpers in `menus.rs`**

Delete `AcceleratorSpec`, `accelerator_specs`, `accelerator` and `virtual_key`, and the now-unused VK imports. Replace `AlignedAccelerators` and `AcceleratorTable::create`:

```rust
#[derive(Debug)]
pub(crate) struct AcceleratorTable(HACCEL);

impl AcceleratorTable {
    /// The table for `keymap`'s bindings, in its precedence order: `TranslateAcceleratorW`
    /// takes the first entry that matches, so a user binding shadows a default on its key.
    pub(crate) fn create(keymap: &crate::window::keymap::Keymap) -> Result<Self> {
        let accelerators = keymap
            .bindings()
            .iter()
            .map(|binding| ACCEL {
                fVirt: FVIRTKEY | binding.stroke.accel_flags(),
                key: binding.stroke.vk,
                cmd: binding.command as u16,
            })
            .collect::<Vec<_>>();
        // Windows refuses a table of no entries; with every command unbound there is none.
        if accelerators.is_empty() {
            return Err(crate::FastPadError::Invariant("no keyboard shortcuts are bound"));
        }
        // `ACCEL` alone aligns to 2 bytes, but `CreateAcceleratorTableW` rejects a buffer that
        // is not on a 4-byte boundary with `ERROR_NOACCESS`. A `u32` buffer pins the alignment.
        let bytes = std::mem::size_of_val(accelerators.as_slice());
        let mut aligned = vec![0u32; bytes.div_ceil(4).max(1)];
        unsafe {
            std::ptr::copy_nonoverlapping(
                accelerators.as_ptr().cast::<u8>(),
                aligned.as_mut_ptr().cast::<u8>(),
                bytes,
            );
        }
        let handle = unsafe {
            CreateAcceleratorTableW(aligned.as_ptr().cast::<ACCEL>(), accelerators.len() as i32)
        };
        if handle.is_null() {
            Err(last_error())
        } else {
            Ok(Self(handle))
        }
    }

    pub(crate) fn raw(&self) -> HACCEL {
        self.0
    }

    /// The table's entries, as Windows stores them.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> Vec<ACCEL> {
        use windows_sys::Win32::UI::WindowsAndMessaging::CopyAcceleratorTableW;
        let count = unsafe { CopyAcceleratorTableW(self.0, std::ptr::null_mut(), 0) };
        let mut entries = vec![ACCEL { fVirt: 0, key: 0, cmd: 0 }; count.max(0) as usize];
        unsafe { CopyAcceleratorTableW(self.0, entries.as_mut_ptr(), count) };
        entries
    }
}
```

In `menus.rs`'s test module, replace `use super::accelerator_specs;` with a helper that rebuilds the old shape from the keymap, so the existing assertions keep guarding the defaults unchanged:

```rust
    #[derive(Clone, Copy)]
    struct Spec {
        modifiers: u8,
        key: u16,
        command: CommandId,
    }

    fn accelerator_specs() -> Vec<Spec> {
        crate::window::keymap::Keymap::defaults()
            .bindings()
            .iter()
            .map(|binding| Spec {
                modifiers: binding.stroke.accel_flags(),
                key: binding.stroke.vk,
                command: binding.command,
            })
            .collect()
    }
```

In `the_native_accelerator_table_is_created_from_a_four_byte_aligned_buffer`, replace the body with:

```rust
        let table = super::AcceleratorTable::create(&crate::window::keymap::Keymap::defaults())
            .expect("accelerator table");
        assert_eq!(table.entries().len(), 66);
```

Delete `defaults_match_the_accelerator_table` from `keymap.rs` (its reference is gone).

In `inline_name.rs`'s `only_ctrl_z_and_ctrl_y_among_the_field_keys_are_accelerators`, replace the `specs`/`bound` lines with:

```rust
        let keymap = crate::window::keymap::Keymap::defaults();
        let bound = |modifiers: u8, key: u16| {
            keymap
                .bindings()
                .iter()
                .any(|binding| binding.stroke.accel_flags() == modifiers && binding.stroke.vk == key)
        };
```

- [ ] **Step 4: Point the palette's `shortcut_text` at the keymap**

In `command_palette.rs`, drop the `menus::{AcceleratorSpec, accelerator_specs}` import and replace `shortcut_text` with:

```rust
/// The first keyboard shortcut bound to `command`, spelled the way the menus spell shortcuts.
pub(crate) fn shortcut_text(command: CommandId) -> Option<String> {
    crate::window::keymap::Keymap::defaults().first_text(command)
}
```

(Task 5 replaces this with the window's keymap.) In the palette tests, change the Zoom In expectation from `"Ctrl++"` to `"Ctrl+="`.

- [ ] **Step 5: `App.keymap` and the main window functions**

In `app.rs`: add the field after `accelerators`:

```rust
    /// The shortcuts in force: the defaults until settings load, then with the user's overrides.
    pub(crate) keymap: crate::window::keymap::Keymap,
```

and in `App::new` build it before the table:

```rust
            keymap: crate::window::keymap::Keymap::defaults(),
            accelerators: AcceleratorTable::create(&crate::window::keymap::Keymap::defaults()).ok(),
```

In `main_window.rs`, next to `change_setting`:

```rust
/// The shortcuts in force for `hwnd`'s window (the defaults before it has an App).
pub(crate) fn keymap(hwnd: HWND) -> crate::window::keymap::Keymap {
    unsafe { app_ptr(hwnd) }.map_or_else(crate::window::keymap::Keymap::defaults, |app| {
        unsafe { app.as_ref() }.keymap.clone()
    })
}

/// Puts `keymap` in force: the accelerator table is rebuilt now; the menu bar, which spells the
/// keys, is rebuilt the next time it opens.
fn install_keymap(app: &mut App, keymap: crate::window::keymap::Keymap) {
    app.accelerators = crate::window::menus::AcceleratorTable::create(&keymap).ok();
    if app.menu_mode.is_none() {
        app.menu_bar = None;
    }
    app.keymap = keymap;
}

/// Gives `command` exactly `keys` (keyboard shortcuts spec §6.6): applies at once and saves its
/// `key.<id>=` line, or removes the line when `keys` are the defaults.
pub(crate) fn set_command_keys(
    hwnd: HWND,
    command: CommandId,
    keys: Vec<crate::window::keymap::KeyStroke>,
) {
    use crate::window::keymap::{ini_key, ini_value};
    let Some(key) = ini_key(command) else {
        return;
    };
    let id = key["key.".len()..].to_owned();
    // The App borrow ends before saving: a failed save pushes a notice, which borrows it again.
    let Some(saved) = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let keymap = app.keymap.with_keys(command, keys);
        if keymap == app.keymap {
            return None;
        }
        let value = keymap.is_user(command).then(|| ini_value(&keymap.keys_of(command)));
        match &value {
            Some(value) => app.settings.key_overrides.insert(id, value.clone()),
            None => app.settings.key_overrides.remove(&id),
        };
        install_keymap(app, keymap);
        Some(value)
    }) else {
        return;
    };
    let result = match saved {
        Some(value) => save_setting(&key, &value),
        None => remove_setting(&key),
    };
    if let Err(error) = result {
        push_notice(hwnd, format!("FastPad could not save fastpad.ini: {error}"));
    }
}

/// Gives `command` its default keys back and removes its `key.<id>=` line.
pub(crate) fn reset_command_keys(hwnd: HWND, command: CommandId) {
    set_command_keys(hwnd, command, crate::window::keymap::default_keys(command));
}
```

Next to the two `save_setting` functions add the matching pair:

```rust
#[cfg(not(test))]
fn remove_setting(key: &str) -> Result<()> {
    crate::config::remove_setting(key)
}

/// Like `save_setting` in tests: only a path a test chose with `save_settings_to` is touched.
#[cfg(test)]
fn remove_setting(key: &str) -> Result<()> {
    TEST_SETTINGS_PATH.with(|path| match path.borrow().as_deref() {
        Some(path) => crate::config::remove_setting_to(path, key),
        None => Ok(()),
    })
}
```

In `apply_loaded_settings`, inside the `if let Some(mut app)` block, right after the `for warning in &warnings` loop:

```rust
        let (keymap, problems) =
            crate::window::keymap::Keymap::from_ini(&app.settings.key_overrides);
        for problem in problems {
            app.notifications.push(format!("fastpad.ini: {problem}"));
        }
        if keymap != app.keymap {
            install_keymap(app, keymap);
        }
```

`remove_setting` in `main_window.rs` shadows nothing; if the name collides with an import, name it `remove_saved_setting`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo clippy --all-targets -- -D warnings`
Then: `cargo test --lib window::menus window::keymap window::command_palette window::inline_name`
Then: `cargo test --lib main_window::tests::rebinding_save main_window::tests::loaded_key_overrides -- --test-threads=1`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src/window src/app.rs
git commit -m "feat: build the accelerator table from the keymap and save rebinds"
```

---

### Task 5: Menus and the palette spell the keymap's keys

**Files:**
- Modify: `src/window/menus.rs` (labels, `create_popup`, `MenuBar::create`, `track_popup`, `show_tab_menu`, `show_tab_strip_menu`)
- Modify: `src/window/notebook_view.rs:2305` (drop `\tCtrl+Shift+M`)
- Modify: `src/window/main_window.rs:7805` (`MenuBar::create(&app.keymap)`), `:2553` (`set_entries`)
- Modify: `src/window/command_palette.rs` (`set_entries`, `shown_keys`, `draw_item`, drop `shortcut_text`)

**Interfaces:**
- Consumes: `Keymap::first_text`, `main_window::keymap(hwnd)` (Task 4).
- Produces: `MenuBar::create(keymap: &Keymap)`, `CommandPalette::set_entries(entries, keymap: &Keymap)`, `#[cfg(test)] CommandPalette::shown_shortcut(index) -> Option<&str>`.

- [ ] **Step 1: Write the failing tests**

In `menus.rs`'s test module:

```rust
    fn label(menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU, command: CommandId) -> String {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuStringW, MF_BYCOMMAND};
        let mut buffer = [0u16; 128];
        let length = unsafe {
            GetMenuStringW(menu, command as u32, buffer.as_mut_ptr(), buffer.len() as i32, MF_BYCOMMAND)
        };
        String::from_utf16_lossy(&buffer[..length.max(0) as usize])
    }

    #[test]
    fn menu_labels_spell_the_keymaps_first_key() {
        // Break caught: a rebound command whose menu still shows the old key, or an unbound one
        // still showing a key that does nothing.
        use super::MenuBar;
        use crate::window::keymap::{KeyStroke, Keymap};
        let defaults = MenuBar::create(&Keymap::defaults()).unwrap();
        assert_eq!(label(defaults.dropdown(0), CommandId::Save), "&Save\tCtrl+S");
        assert_eq!(label(defaults.dropdown(0), CommandId::CloseAllTabs), "Close a&ll tabs");
        let view = crate::window::menu_band::VIEW_MENU_INDEX;
        assert_eq!(label(defaults.dropdown(view), CommandId::ZoomIn), "Zoom &in\tCtrl+=");

        let keymap = Keymap::defaults()
            .with_keys(CommandId::Save, vec![KeyStroke::parse("Ctrl+Alt+S").unwrap()])
            .with_keys(CommandId::Undo, vec![]);
        let custom = MenuBar::create(&keymap).unwrap();
        assert_eq!(label(custom.dropdown(0), CommandId::Save), "&Save\tCtrl+Alt+S");
        assert_eq!(label(custom.dropdown(1), CommandId::Undo), "&Undo");
    }
```

In `command_palette.rs`'s tests, replace the `shortcut_text` import with a local helper so the existing expectations stay:

```rust
    fn shortcut_text(command: CommandId) -> Option<String> {
        crate::window::keymap::Keymap::defaults().first_text(command)
    }
```

and add:

```rust
    #[test]
    fn palette_rows_show_the_windows_keys() {
        // Break caught: the palette hinting the default key after the user rebound it.
        use crate::window::keymap::{KeyStroke, Keymap};
        let keymap = Keymap::defaults().with_keys(CommandId::Save, vec![KeyStroke::parse("F9").unwrap()]);
        let entries = filter_entries("file: save", |_| true);
        assert_eq!(entries[0].command, CommandId::Save);
        let shown = entries.iter().map(|entry| keymap.first_text(entry.command)).collect::<Vec<_>>();
        assert_eq!(shown[0].as_deref(), Some("F9"));
    }
```

(`set_entries` itself is exercised by the existing window tests that open the palette.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib window::menus`
Expected: compile error (`MenuBar::create` takes no argument).

- [ ] **Step 3: Implement the menus**

1. Remove every `\t<key>` suffix from the labels of commands that have a default binding, in `MenuBar::create`, `editor_layout`, `show_tab_menu` and `show_tab_strip_menu` (some use a literal tab character instead of `\t`; remove those too). Examples: `"&Save\tCtrl+S"` → `"&Save"`, `"&Close tab\tCtrl+W"` → `"&Close tab"`, `"Zoom &in\tCtrl++"` → `"Zoom &in"`, `"New tab\tCtrl+N"` → `"New tab"`. In `notebook_view.rs:2305` change `"Move to notebook...\tCtrl+Shift+M"` to `"Move to notebook..."`. Keep `"Rename...\tF2"` and `"Delete...\tDel"`: the tree handles those keys itself, they are not bindings.
2. `create_popup(entries: &[MenuEntry], keymap: &Keymap)`: when appending a `MenuEntry::Command(label, command)`:

```rust
            MenuEntry::Command(label, command) => {
                // A label that spells its own key (the tree's F2, Del) keeps it; every other
                // command shows its first key from the keymap (keyboard shortcuts spec §5).
                let text = match keymap.first_text(*command) {
                    Some(key) if !label.contains('\t') => format!("{label}\t{key}"),
                    _ => (*label).to_owned(),
                };
                let text = wide_null(&text);
                unsafe { AppendMenuW(menu, MF_STRING, *command as usize, text.as_ptr()) }
            }
```

   and pass `keymap` to the recursive `create_popup(children, keymap)`.
3. `MenuBar::create(keymap: &Keymap)` passes it to each `create_popup`. Update every test call to `MenuBar::create(&crate::window::keymap::Keymap::defaults())`.
4. `track_popup`: after the test-answer block, `let keymap = crate::window::main_window::keymap(hwnd);` and `create_popup(entries, &keymap)`.
5. `main_window.rs:7805`: `app.menu_bar = MenuBar::create(&app.keymap).ok();`.

- [ ] **Step 4: Implement the palette**

In `CommandPalette` add a field next to `shown`:

```rust
    /// Each shown row's key text, from the window's keymap when the rows were set.
    shown_keys: Vec<Option<String>>,
```

(initialize `shown_keys: Vec::new()` in `create`). Replace `set_entries`:

```rust
    /// Records the rows to list and their keys; `fill_list` then puts them in the list box.
    pub(crate) fn set_entries(&mut self, entries: Vec<PaletteEntry>, keymap: &crate::window::keymap::Keymap) {
        self.shown_keys = entries.iter().map(|entry| keymap.first_text(entry.command)).collect();
        self.shown = entries;
    }
```

In `draw_item`, replace `(entry.label.to_owned(), shortcut_text(entry.command))` with:

```rust
            let shortcut = index.and_then(|index| self.shown_keys.get(index)).cloned().flatten();
            (entry.label.to_owned(), shortcut)
```

Delete `pub(crate) fn shortcut_text`. At `main_window.rs:2553`, fetch the keymap before borrowing the palette:

```rust
        let keymap = keymap(hwnd);
        if let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
        {
            palette.set_entries(entries, &keymap);
        }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo clippy --all-targets -- -D warnings`
Then: `cargo test --lib window::menus window::command_palette window::notebook_view`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/window
git commit -m "feat: menus and palette spell each command's key from the keymap"
```

---

### Task 6: The Keyboard Shortcuts page model

**Files:**
- Create: `src/window/shortcuts_model.rs`
- Modify: `src/window/mod.rs` (`pub(crate) mod shortcuts_model;`)

**Interfaces:**
- Consumes: `Keymap`, `KeyStroke`, `COMMAND_IDS`, `bindable` (Task 2); `command_palette::ENTRIES`.
- Produces:
  - `title(CommandId) -> Option<&'static str>`
  - `struct ShortcutRow { command, title: &'static str, id: &'static str, stroke: Option<KeyStroke>, user: bool }`
  - `enum Filter { Text(String), Key(KeyStroke) }`
  - `struct Recording { command, replace: Option<KeyStroke>, stroke: Option<KeyStroke>, refusal: Option<&'static str> }`
  - `enum ShortcutsEffect { None, Repaint, SetKeys(CommandId, Vec<KeyStroke>), Reset(CommandId), CopyId(&'static str), FocusSearch, FocusTable, SetSearchText(String), Close }`
  - `struct ShortcutsModel { pub keymap, pub filter, pub rows, pub selected: usize, pub top: usize, pub visible: usize, pub recording: Option<Recording>, pub record_keys: bool }` with `new(keymap, visible)`, `refresh(keymap)`, `set_text(&str)`, `record_search_key(KeyStroke)`, `toggle_record_keys()`, `table_key(KeyStroke)`, `record_key(KeyStroke)`, `select(usize)`, `scroll(isize)`, `start_change()`, `start_add()`, `remove()`, `reset()`, `copy_id()`, `cancel_recording()`, `follow_conflicts()`, `conflict_count()`, `selected_row()`. All mutating methods return `ShortcutsEffect`, except `refresh`, `select` and `scroll`.

- [ ] **Step 1: Write the failing tests**

Create `src/window/shortcuts_model.rs` with the tests:

```rust
//! The Keyboard Shortcuts page's rows and what each input does to them (keyboard shortcuts spec
//! §6.2–§6.6). Pure: no window handles.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::keymap::{KeyStroke, Keymap, TYPING_REFUSAL};

    fn stroke(text: &str) -> KeyStroke {
        KeyStroke::parse(text).unwrap()
    }

    fn model(query: &str) -> ShortcutsModel {
        let mut model = ShortcutsModel::new(Keymap::defaults(), 10);
        model.set_text(query);
        model
    }

    #[test]
    fn every_command_has_a_title_and_a_row() {
        // Break caught: a command the page can't show (no title), so its keys can't be changed.
        let model = model("");
        for (command, _) in crate::window::keymap::COMMAND_IDS {
            assert!(title(*command).is_some(), "{command:?}");
            assert!(model.rows.iter().any(|row| row.command == *command), "{command:?}");
        }
        assert_eq!(title(CommandId::SelectTab3), Some("View: Select tab 3"));
    }

    #[test]
    fn a_command_has_one_row_per_key_and_an_unbound_one_has_one_empty_row() {
        let model = model("zoom in");
        let keys = model.rows.iter().map(|row| row.stroke).collect::<Vec<_>>();
        assert_eq!(keys, [Some(stroke("Ctrl+=")), Some(stroke("Ctrl+Shift+=")), Some(stroke("Ctrl+NumpadAdd"))]);
        let about = model_rows_for("about");
        assert_eq!(about, [None]);
    }

    fn model_rows_for(query: &str) -> Vec<Option<KeyStroke>> {
        model(query).rows.iter().map(|row| row.stroke).collect()
    }

    #[test]
    fn text_matches_title_id_or_key_text() {
        // Break caught: typing "ctrl+s" or a command ID finding nothing.
        let titles = |query: &str| model(query).rows.iter().map(|row| row.title).collect::<Vec<_>>();
        assert!(titles("save as").contains(&"File: Save as..."));
        assert!(titles("file.saveAs").contains(&"File: Save as..."));
        let by_key = model("ctrl + s");
        assert!(by_key.rows.iter().all(|row| row.stroke.is_some_and(|s| s.text().to_lowercase().replace(' ', "").contains("ctrl+s"))));
        assert!(by_key.rows.iter().any(|row| row.command == CommandId::Save));
    }

    #[test]
    fn record_keys_search_filters_by_the_exact_key() {
        let mut model = model("");
        assert_eq!(model.toggle_record_keys(), ShortcutsEffect::SetSearchText(String::new()));
        assert!(model.record_keys);
        assert_eq!(model.record_search_key(stroke("Ctrl+S")), ShortcutsEffect::SetSearchText("Ctrl+S".into()));
        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].command, CommandId::Save);
        model.toggle_record_keys();
        assert!(!model.record_keys);
        assert_eq!(model.filter, Filter::Text(String::new()));
    }

    #[test]
    fn arrows_move_the_selection_and_up_from_the_top_returns_to_the_search() {
        let mut model = model("");
        assert_eq!(model.table_key(stroke("Down")), ShortcutsEffect::Repaint);
        assert_eq!(model.selected, 1);
        assert_eq!(model.table_key(stroke("End")), ShortcutsEffect::Repaint);
        assert_eq!(model.selected, model.rows.len() - 1);
        assert_eq!(model.top, model.rows.len() - 10);
        model.table_key(stroke("Home"));
        assert_eq!((model.selected, model.top), (0, 0));
        assert_eq!(model.table_key(stroke("Up")), ShortcutsEffect::FocusSearch);
        assert_eq!(model.table_key(stroke("Escape")), ShortcutsEffect::Close);
    }

    #[test]
    fn changing_a_key_replaces_only_that_key() {
        // Break caught: changing Zoom In's second key dropping the other two.
        let mut model = model("zoom in");
        model.select(1);
        assert_eq!(model.table_key(stroke("Enter")), ShortcutsEffect::Repaint);
        assert!(model.recording.is_some());
        assert_eq!(model.record_key(stroke("F9")), ShortcutsEffect::Repaint);
        assert_eq!(
            model.record_key(stroke("Enter")),
            ShortcutsEffect::SetKeys(CommandId::ZoomIn, vec![stroke("Ctrl+="), stroke("F9"), stroke("Ctrl+NumpadAdd")])
        );
        assert!(model.recording.is_none());
    }

    #[test]
    fn adding_appends_and_an_empty_row_adds() {
        let mut model = model("save as");
        model.table_key(stroke("Ctrl+Enter"));
        model.record_key(stroke("F9"));
        assert_eq!(
            model.record_key(stroke("Enter")),
            ShortcutsEffect::SetKeys(CommandId::SaveAs, vec![stroke("Ctrl+Shift+S"), stroke("F9")])
        );
        let mut about = self::model("about");
        about.table_key(stroke("Enter"));
        about.record_key(stroke("F9"));
        assert_eq!(about.record_key(stroke("Enter")), ShortcutsEffect::SetKeys(CommandId::About, vec![stroke("F9")]));
    }

    #[test]
    fn recording_refuses_typing_keys_and_escape_cancels() {
        // Break caught: plain A accepted as a shortcut, or Enter with a refused key saving it.
        let mut model = model("save as");
        model.table_key(stroke("Enter"));
        model.record_key(stroke("A"));
        assert_eq!(model.recording.as_ref().unwrap().refusal, Some(TYPING_REFUSAL));
        assert_eq!(model.record_key(stroke("Enter")), ShortcutsEffect::None);
        assert!(model.recording.is_some());
        // Keys that Enter and Escape mean in the box are recorded with modifiers.
        model.record_key(stroke("Ctrl+Enter"));
        assert_eq!(model.recording.as_ref().unwrap().refusal, None);
        assert_eq!(model.record_key(stroke("Escape")), ShortcutsEffect::Repaint);
        assert!(model.recording.is_none());
    }

    #[test]
    fn confirming_the_same_key_changes_nothing() {
        let mut model = model("save as");
        model.table_key(stroke("Enter"));
        model.record_key(stroke("Ctrl+Shift+S"));
        assert_eq!(model.record_key(stroke("Enter")), ShortcutsEffect::Repaint);
    }

    #[test]
    fn conflicts_count_other_commands_and_the_link_filters_by_the_key() {
        let mut model = model("save as");
        model.table_key(stroke("Enter"));
        model.record_key(stroke("F3"));
        assert_eq!(model.conflict_count(), 1);
        assert_eq!(model.follow_conflicts(), ShortcutsEffect::SetSearchText("F3".into()));
        assert!(model.recording.is_none());
        assert!(model.record_keys);
        assert_eq!(model.rows.iter().map(|row| row.command).collect::<Vec<_>>(), [CommandId::FindNext]);
    }

    #[test]
    fn remove_unbinds_the_key_and_reset_is_only_for_user_rows() {
        let mut model = model("save as");
        assert_eq!(model.reset(), ShortcutsEffect::None);
        assert_eq!(model.table_key(stroke("Delete")), ShortcutsEffect::SetKeys(CommandId::SaveAs, vec![]));
        model.refresh(Keymap::defaults().with_keys(CommandId::SaveAs, vec![]));
        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].stroke, None);
        assert!(model.rows[0].user);
        assert_eq!(model.table_key(stroke("Delete")), ShortcutsEffect::None);
        assert_eq!(model.reset(), ShortcutsEffect::Reset(CommandId::SaveAs));
        assert_eq!(model.table_key(stroke("Ctrl+C")), ShortcutsEffect::CopyId("file.saveAs"));
    }

    #[test]
    fn refresh_keeps_the_selected_command() {
        // Break caught: the selection jumping to the top after every change.
        let mut model = model("zoom");
        let index = model.rows.iter().position(|row| row.command == CommandId::ZoomOut).unwrap();
        model.select(index);
        model.refresh(Keymap::defaults().with_keys(CommandId::ZoomOut, vec![stroke("F9")]));
        assert_eq!(model.selected_row().unwrap().command, CommandId::ZoomOut);
        assert_eq!(model.selected_row().unwrap().stroke, Some(stroke("F9")));
    }

    #[test]
    fn an_empty_filter_result_keeps_the_keys_harmless() {
        let mut model = model("no such command anywhere");
        assert!(model.rows.is_empty());
        assert_eq!(model.table_key(stroke("Enter")), ShortcutsEffect::None);
        assert_eq!(model.table_key(stroke("Delete")), ShortcutsEffect::None);
        assert_eq!(model.table_key(stroke("Down")), ShortcutsEffect::Repaint);
        assert_eq!(model.selected, 0);
    }
}
```

Add `pub(crate) mod shortcuts_model;` to `src/window/mod.rs`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib window::shortcuts_model`
Expected: compile errors.

- [ ] **Step 3: Implement**

Above the tests:

```rust
use crate::window::commands::CommandId;
use crate::window::keymap::{COMMAND_IDS, KeyStroke, Keymap, bindable};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_NEXT, VK_PRIOR, VK_RETURN, VK_UP,
};

/// Names for the bound commands the palette doesn't list (spec §2).
const EXTRA_TITLES: [(CommandId, &str); 22] = [
    (CommandId::SelectTab1, "View: Select tab 1"),
    (CommandId::SelectTab2, "View: Select tab 2"),
    (CommandId::SelectTab3, "View: Select tab 3"),
    (CommandId::SelectTab4, "View: Select tab 4"),
    (CommandId::SelectTab5, "View: Select tab 5"),
    (CommandId::SelectTab6, "View: Select tab 6"),
    (CommandId::SelectTab7, "View: Select tab 7"),
    (CommandId::SelectTab8, "View: Select tab 8"),
    (CommandId::SelectTab9, "View: Select tab 9"),
    (CommandId::CommandPalette, "View: Show command palette"),
    (CommandId::MarkdownPreviewCycle, "Markdown Preview: Cycle"),
    (CommandId::FocusNextPane, "View: Focus next pane"),
    (CommandId::FocusPreviousPane, "View: Focus previous pane"),
    (CommandId::FocusGroup1, "View: Focus editor group 1"),
    (CommandId::FocusGroup2, "View: Focus editor group 2"),
    (CommandId::FocusGroup3, "View: Focus editor group 3"),
    (CommandId::FocusGroup4, "View: Focus editor group 4"),
    (CommandId::FocusGroup5, "View: Focus editor group 5"),
    (CommandId::FocusGroup6, "View: Focus editor group 6"),
    (CommandId::FocusGroup7, "View: Focus editor group 7"),
    (CommandId::FocusGroup8, "View: Focus editor group 8"),
    (CommandId::FocusLastGroup, "View: Focus last editor group"),
];
```

(22 entries. `every_command_has_a_title_and_a_row` fails if a command is missing, so count against that test, not by hand.)

```rust
/// What the page calls `command`: its palette label, or its name from `EXTRA_TITLES`.
pub(crate) fn title(command: CommandId) -> Option<&'static str> {
    crate::window::command_palette::ENTRIES
        .iter()
        .find(|entry| entry.command == command)
        .map(|entry| entry.label)
        .or_else(|| {
            EXTRA_TITLES
                .iter()
                .find(|(candidate, _)| *candidate == command)
                .map(|(_, title)| *title)
        })
}

/// One row: one key of a command, or a command with none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShortcutRow {
    pub command: CommandId,
    pub title: &'static str,
    pub id: &'static str,
    pub stroke: Option<KeyStroke>,
    /// Whether the command's keys are the user's.
    pub user: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Filter {
    Text(String),
    Key(KeyStroke),
}

fn matches(row: &ShortcutRow, filter: &Filter) -> bool {
    match filter {
        Filter::Key(stroke) => row.stroke == Some(*stroke),
        Filter::Text(text) => {
            let compact = |text: &str| text.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect::<String>();
            let needle = text.trim().to_lowercase();
            needle.is_empty()
                || row.title.to_lowercase().contains(&needle)
                || row.id.to_lowercase().contains(&needle)
                || row.stroke.is_some_and(|stroke| compact(&stroke.text()).contains(&compact(&needle)))
        }
    }
}

/// Every row `filter` keeps: commands sorted by title, each command's keys in binding order.
pub(crate) fn rows(keymap: &Keymap, filter: &Filter) -> Vec<ShortcutRow> {
    let mut commands = COMMAND_IDS
        .iter()
        .filter_map(|&(command, id)| Some((command, title(command)?, id)))
        .collect::<Vec<_>>();
    commands.sort_by_key(|(_, title, _)| title.to_lowercase());
    let mut rows = Vec::new();
    for (command, title, id) in commands {
        let user = keymap.is_user(command);
        let keys = keymap.keys_of(command);
        let strokes = if keys.is_empty() { vec![None] } else { keys.into_iter().map(Some).collect() };
        rows.extend(
            strokes
                .into_iter()
                .map(|stroke| ShortcutRow { command, title, id, stroke, user })
                .filter(|row| matches(row, filter)),
        );
    }
    rows
}

/// The recording box: the command being given a key, the key it replaces (none when adding),
/// and what was pressed so far.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Recording {
    pub command: CommandId,
    pub replace: Option<KeyStroke>,
    pub stroke: Option<KeyStroke>,
    pub refusal: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ShortcutsEffect {
    None,
    Repaint,
    /// Give the command exactly these keys; the dialog applies and saves them.
    SetKeys(CommandId, Vec<KeyStroke>),
    Reset(CommandId),
    CopyId(&'static str),
    FocusSearch,
    FocusTable,
    /// Put this text in the search field.
    SetSearchText(String),
    Close,
}

const fn plain(vk: u16) -> KeyStroke {
    KeyStroke::new(false, false, false, vk)
}

#[derive(Clone, Debug)]
pub(crate) struct ShortcutsModel {
    pub keymap: Keymap,
    pub filter: Filter,
    pub rows: Vec<ShortcutRow>,
    pub selected: usize,
    /// The first row shown.
    pub top: usize,
    /// How many rows the table shows.
    pub visible: usize,
    pub recording: Option<Recording>,
    /// Whether the search field records keys instead of taking text.
    pub record_keys: bool,
}

impl ShortcutsModel {
    pub(crate) fn new(keymap: Keymap, visible: usize) -> Self {
        let filter = Filter::Text(String::new());
        let rows = rows(&keymap, &filter);
        Self { keymap, filter, rows, selected: 0, top: 0, visible: visible.max(1), recording: None, record_keys: false }
    }

    pub(crate) fn selected_row(&self) -> Option<&ShortcutRow> {
        self.rows.get(self.selected)
    }

    /// Re-reads the rows from `keymap`, keeping the selected command (on the same key when it
    /// still has it).
    pub(crate) fn refresh(&mut self, keymap: Keymap) {
        let kept = self.selected_row().map(|row| (row.command, row.stroke));
        self.keymap = keymap;
        self.rows = rows(&self.keymap, &self.filter);
        if let Some((command, stroke)) = kept {
            let index = self
                .rows
                .iter()
                .position(|row| row.command == command && row.stroke == stroke)
                .or_else(|| self.rows.iter().position(|row| row.command == command));
            if let Some(index) = index {
                self.selected = index;
            }
        }
        self.clamp();
    }

    fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.rows = rows(&self.keymap, &self.filter);
        self.selected = 0;
        self.top = 0;
    }

    pub(crate) fn set_text(&mut self, text: &str) -> ShortcutsEffect {
        self.set_filter(Filter::Text(text.to_owned()));
        ShortcutsEffect::Repaint
    }

    pub(crate) fn record_search_key(&mut self, stroke: KeyStroke) -> ShortcutsEffect {
        self.set_filter(Filter::Key(stroke));
        ShortcutsEffect::SetSearchText(stroke.text())
    }

    pub(crate) fn toggle_record_keys(&mut self) -> ShortcutsEffect {
        self.record_keys = !self.record_keys;
        self.set_filter(Filter::Text(String::new()));
        ShortcutsEffect::SetSearchText(String::new())
    }

    fn clamp(&mut self) {
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        let max_top = self.rows.len().saturating_sub(self.visible);
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + self.visible {
            self.top = self.selected + 1 - self.visible;
        }
        self.top = self.top.min(max_top);
    }

    pub(crate) fn select(&mut self, index: usize) {
        self.selected = index;
        self.clamp();
    }

    /// Scrolls by `rows` (negative: up) without moving the selection.
    pub(crate) fn scroll(&mut self, rows: isize) {
        let max_top = self.rows.len().saturating_sub(self.visible);
        self.top = self.top.saturating_add_signed(rows).min(max_top);
    }

    pub(crate) fn table_key(&mut self, stroke: KeyStroke) -> ShortcutsEffect {
        let page = self.visible.saturating_sub(1).max(1);
        let last = self.rows.len().saturating_sub(1);
        match stroke {
            s if s == plain(VK_UP) && self.selected == 0 => ShortcutsEffect::FocusSearch,
            s if s == plain(VK_UP) => self.move_to(self.selected - 1),
            s if s == plain(VK_DOWN) => self.move_to((self.selected + 1).min(last)),
            s if s == plain(VK_PRIOR) => self.move_to(self.selected.saturating_sub(page)),
            s if s == plain(VK_NEXT) => self.move_to((self.selected + page).min(last)),
            s if s == plain(VK_HOME) => self.move_to(0),
            s if s == plain(VK_END) => self.move_to(last),
            s if s == plain(VK_RETURN) => self.start_change(),
            s if s == KeyStroke::new(true, false, false, VK_RETURN) => self.start_add(),
            s if s == plain(VK_DELETE) => self.remove(),
            s if s == KeyStroke::new(true, false, false, u16::from(b'C')) => self.copy_id(),
            s if s == plain(VK_ESCAPE) => ShortcutsEffect::Close,
            _ => ShortcutsEffect::None,
        }
    }

    fn move_to(&mut self, index: usize) -> ShortcutsEffect {
        self.select(index);
        ShortcutsEffect::Repaint
    }

    fn start(&mut self, replace: bool) -> ShortcutsEffect {
        let Some(row) = self.selected_row() else {
            return ShortcutsEffect::None;
        };
        self.recording = Some(Recording {
            command: row.command,
            replace: if replace { row.stroke } else { None },
            stroke: None,
            refusal: None,
        });
        ShortcutsEffect::Repaint
    }

    pub(crate) fn start_change(&mut self) -> ShortcutsEffect {
        self.start(true)
    }

    pub(crate) fn start_add(&mut self) -> ShortcutsEffect {
        self.start(false)
    }

    pub(crate) fn cancel_recording(&mut self) -> ShortcutsEffect {
        if self.recording.take().is_some() { ShortcutsEffect::Repaint } else { ShortcutsEffect::None }
    }

    /// A key pressed while the recording box is open. Plain Enter confirms and plain Escape
    /// cancels; anything else is the key being recorded.
    pub(crate) fn record_key(&mut self, stroke: KeyStroke) -> ShortcutsEffect {
        if stroke == plain(VK_ESCAPE) {
            return self.cancel_recording();
        }
        let Some(recording) = self.recording.as_mut() else {
            return ShortcutsEffect::None;
        };
        if stroke != plain(VK_RETURN) {
            recording.stroke = Some(stroke);
            recording.refusal = bindable(stroke).err();
            return ShortcutsEffect::Repaint;
        }
        let (Some(new), None) = (recording.stroke, recording.refusal) else {
            return ShortcutsEffect::None;
        };
        let recording = self.recording.take().expect("checked above");
        let current = self.keymap.keys_of(recording.command);
        let mut keys = current.clone();
        match recording.replace.and_then(|old| keys.iter().position(|key| *key == old)) {
            Some(index) => keys[index] = new,
            None => keys.push(new),
        }
        let mut unique = Vec::with_capacity(keys.len());
        for key in keys {
            if !unique.contains(&key) {
                unique.push(key);
            }
        }
        if unique == current {
            ShortcutsEffect::Repaint
        } else {
            ShortcutsEffect::SetKeys(recording.command, unique)
        }
    }

    /// How many other commands already use the recorded key.
    pub(crate) fn conflict_count(&self) -> usize {
        self.recording
            .as_ref()
            .and_then(|recording| Some(self.keymap.conflicts(recording.stroke?, recording.command).len()))
            .unwrap_or(0)
    }

    /// The "N existing commands" link: closes the box and searches for the recorded key.
    pub(crate) fn follow_conflicts(&mut self) -> ShortcutsEffect {
        let Some(stroke) = self.recording.take().and_then(|recording| recording.stroke) else {
            return ShortcutsEffect::None;
        };
        self.record_keys = true;
        self.record_search_key(stroke)
    }

    pub(crate) fn remove(&mut self) -> ShortcutsEffect {
        let Some(row) = self.selected_row() else {
            return ShortcutsEffect::None;
        };
        let Some(stroke) = row.stroke else {
            return ShortcutsEffect::None;
        };
        let keys = self.keymap.keys_of(row.command).into_iter().filter(|key| *key != stroke).collect();
        ShortcutsEffect::SetKeys(row.command, keys)
    }

    pub(crate) fn reset(&mut self) -> ShortcutsEffect {
        match self.selected_row() {
            Some(row) if row.user => ShortcutsEffect::Reset(row.command),
            _ => ShortcutsEffect::None,
        }
    }

    pub(crate) fn copy_id(&mut self) -> ShortcutsEffect {
        self.selected_row().map_or(ShortcutsEffect::None, |row| ShortcutsEffect::CopyId(row.id))
    }
}
```

Note on `table_key`'s `Up` arm: `self.selected - 1` is safe because the previous arm handles `selected == 0`. On an empty table `last` is 0, so Down keeps `selected == 0` and returns `Repaint`, as the last test expects.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib window::shortcuts_model`
Expected: 13 passed.

- [ ] **Step 5: Clippy and commit**

Run: `cargo clippy --all-targets -- -D warnings` (add `#[cfg_attr(not(test), allow(dead_code, reason = "wired into the dialog in Task 8"))]` on the module's items only if clippy fails on dead code; remove it in Task 8).

```bash
git add src/window/shortcuts_model.rs src/window/mod.rs
git commit -m "feat: Keyboard Shortcuts page model: rows, search, recording and row actions"
```

---

### Task 7: Dialog pages, left nav and the Open Keyboard Shortcuts command

**Files:**
- Modify: `src/window/settings_model.rs` (`Page`, `Focus::{Nav, Search, Table}`, `Key::NextPage`, `Effect::ShowPage`, `DialogModel.page`, `next_focus`)
- Modify: `src/window/settings_dialog.rs` (`Layout` nav + offsets, `Hit::Nav`, `show(…, page)`, nav painting, `run(ShowPage)`, Ctrl+PageUp/Down)
- Modify: `src/window/commands.rs` (`OpenKeyboardShortcuts = 237`)
- Modify: `src/window/keymap.rs` (`COMMAND_IDS` entry)
- Modify: `src/window/command_palette.rs` (`ENTRIES` 104 → 105)
- Modify: `src/window/main_window.rs` (`show_settings`, `show_keyboard_shortcuts`, `execute_command`)

**Interfaces:**
- Produces: `settings_model::Page { General, Shortcuts }` with `ALL`, `title()`; `DialogModel::new(page)`, `DialogModel::show_page(page) -> Effect`; `next_focus(current, forward, view, page)`; `settings_dialog::show(owner, colors, link_color, page) -> Outcome`; `main_window::show_keyboard_shortcuts(hwnd)`; `Hit::Nav(Page)`; `Layout.nav: RECT`, `Layout.nav_items: [RECT; 2]`; `#[cfg(test)] settings_dialog::current_page(dialog) -> Option<Page>`.

- [ ] **Step 1: Write the failing tests**

In `settings_model.rs`'s tests:

```rust
    #[test]
    fn the_nav_is_the_first_tab_stop_and_each_page_has_its_own_stops() {
        // Break caught: Tab never reaching the page list, or walking into the other page's
        // controls.
        let view = view();
        assert_eq!(next_focus(Focus::Close, true, &view, Page::General), Focus::Nav);
        assert_eq!(next_focus(Focus::Nav, true, &view, Page::General), Focus::Row(Row::Theme));
        assert_eq!(next_focus(Focus::Nav, true, &view, Page::Shortcuts), Focus::Search);
        assert_eq!(next_focus(Focus::Search, true, &view, Page::Shortcuts), Focus::Table);
        assert_eq!(next_focus(Focus::Table, true, &view, Page::Shortcuts), Focus::EditIni);
        assert_eq!(next_focus(Focus::Nav, false, &view, Page::Shortcuts), Focus::Close);
    }

    #[test]
    fn ctrl_page_keys_and_the_nav_arrows_switch_pages() {
        let view = view();
        let mut model = DialogModel::new(Page::General);
        assert_eq!(model.focus, Focus::Row(Row::Theme));
        assert_eq!(model.key(Key::NextPage { back: false }, &view), Effect::ShowPage(Page::Shortcuts));
        assert_eq!(model.show_page(Page::Shortcuts), Effect::Repaint);
        assert_eq!(model.focus, Focus::Search);
        model.focus = Focus::Nav;
        assert_eq!(model.key(Key::Up, &view), Effect::ShowPage(Page::General));
        assert_eq!(model.key(Key::Down, &view), Effect::None, "already the last page");
        model.show_page(Page::General);
        assert_eq!(model.focus, Focus::Nav, "switching from the nav keeps the focus there");
        assert_eq!(DialogModel::new(Page::Shortcuts).focus, Focus::Search);
    }
```

Update the existing callers and tests in `settings_model.rs`: `DialogModel::new()` → `DialogModel::new(Page::General)`; `next_focus(a, b, &view)` → `next_focus(a, b, &view, Page::General)`; any test that expects Tab from `Close` to wrap to `Row(Theme)` (or Shift+Tab from `Row(Theme)` to wrap to `Close`) now expects `Focus::Nav`.

In `settings_dialog.rs`'s tests, update `Layout::calculate` expectations for the new width (`assert_eq!(layout.width, 860)`), and add:

```rust
    #[test]
    fn the_nav_sits_left_of_the_cards_and_hits_its_pages() {
        // Break caught: cards painted under the nav, or a nav item that doesn't switch pages.
        let layout = Layout::calculate(96, 4000, 2000, 100);
        assert!(layout.nav.right <= layout.rows[0].left);
        assert_eq!(layout.body.left, layout.nav.right);
        for page in Page::ALL {
            let (x, y) = center(layout.nav_items[page as usize]);
            assert_eq!(layout.hit(x, y, 0, &view(), Page::General), Some(Hit::Nav(page)));
        }
        let (x, y) = center(layout.row_rect(Row::Theme, 0));
        assert_eq!(layout.hit(x, y, 0, &view(), Page::Shortcuts), None, "no General rows on the other page");
    }
```

Update every existing `layout.hit(x, y, scroll, &view)` call in the tests to pass `Page::General`.

In `commands.rs`, extend `settings_commands_are_235_and_236_and_need_no_document` into `settings_commands_are_235_to_237_and_need_no_document` with `(237, CommandId::OpenKeyboardShortcuts)`.

In `main_window.rs` tests, next to the settings dialog tests:

```rust
    #[test]
    fn open_keyboard_shortcuts_opens_settings_on_the_shortcuts_page() {
        // Break caught: the palette command opening Settings on General, or not at all.
        let window = ProductionWindow::new(make_app());
        crate::window::settings_dialog::answer_next(|dialog| unsafe {
            assert_eq!(
                crate::window::settings_dialog::current_page(dialog),
                Some(crate::window::settings_model::Page::Shortcuts)
            );
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
        });
        super::execute_command(window.hwnd, CommandId::OpenKeyboardShortcuts);
        assert_eq!(crate::window::settings_dialog::open_dialog(window.hwnd), None);
    }
```

(Import `PostMessageW`, `WM_KEYDOWN`, `VK_ESCAPE` inside the test like the neighbouring tests do.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib window::settings_model`
Expected: compile errors (`Page`, `Focus::Nav`, …).

- [ ] **Step 3: Implement `settings_model.rs`**

```rust
/// The dialog's pages, in nav order (keyboard shortcuts spec §6.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Page {
    General,
    Shortcuts,
}

impl Page {
    pub(crate) const ALL: [Self; 2] = [Self::General, Self::Shortcuts];

    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Shortcuts => "Keyboard Shortcuts",
        }
    }

    /// The first Tab stop on the page.
    const fn first_stop(self) -> Focus {
        match self {
            Self::General => Focus::Row(Row::ALL[0]),
            Self::Shortcuts => Focus::Search,
        }
    }
}
```

Add `Nav`, `Search`, `Table` to `Focus`. Replace `next_focus`:

```rust
/// Tab order: the nav, then the page's stops (General: its rows top to bottom, skipping
/// greyed-out ones; Keyboard Shortcuts: the search field, then the table), then the Edit
/// fastpad.ini link, then Close. It wraps around.
pub(crate) fn next_focus(current: Focus, forward: bool, view: &SettingsView, page: Page) -> Focus {
    let stops = match page {
        Page::General => Row::ALL
            .into_iter()
            .filter(|row| view.enabled(*row))
            .map(Focus::Row)
            .collect::<Vec<_>>(),
        Page::Shortcuts => vec![Focus::Search, Focus::Table],
    };
    let order = std::iter::once(Focus::Nav)
        .chain(stops)
        .chain([Focus::EditIni, Focus::Close])
        .collect::<Vec<_>>();
    let count = order.len();
    let next = match (order.iter().position(|focus| *focus == current), forward) {
        (Some(index), true) => (index + 1) % count,
        (Some(index), false) => (index + count - 1) % count,
        (None, true) => 0,
        (None, false) => count - 1,
    };
    order[next]
}
```

Add `NextPage { back: bool }` to `Key` and `ShowPage(Page)` to `Effect`. In `DialogModel`, add `pub page: Page`, change `new` to `new(page: Page)` with `focus: page.first_stop()`, and add:

```rust
    /// Shows `page`. The focus stays on the nav when it is there, else moves to the page's
    /// first stop.
    pub(crate) fn show_page(&mut self, page: Page) -> Effect {
        self.typed = None;
        self.page = page;
        if self.focus != Focus::Nav {
            self.focus = page.first_stop();
        }
        Effect::Repaint
    }
```

In `key`, pass `self.page` to `next_focus`, and add before the `_ =>` arm:

```rust
            Key::NextPage { back } => {
                let index = self.page as usize;
                let count = Page::ALL.len();
                let next = if back { (index + count - 1) % count } else { (index + 1) % count };
                Effect::ShowPage(Page::ALL[next])
            }
```

and inside the focus match:

```rust
                Focus::Nav => {
                    let index = self.page as usize;
                    let target = match key {
                        Key::Up => index.checked_sub(1),
                        Key::Down => (index + 1 < Page::ALL.len()).then_some(index + 1),
                        _ => None,
                    };
                    target.map_or(Effect::None, |index| Effect::ShowPage(Page::ALL[index]))
                }
                Focus::Search | Focus::Table => Effect::None,
```

- [ ] **Step 4: Implement the command**

- `commands.rs`: add `OpenKeyboardShortcuts = 237,` after `EditSettingsFile`, append `CommandId::OpenKeyboardShortcuts` to `COMMANDS` (size 126 → 127), and add `| Self::OpenKeyboardShortcuts` wherever `Self::EditSettingsFile` appears in a `matches!` list (grep `EditSettingsFile` in `commands.rs` and mirror each occurrence).
- `keymap.rs` `COMMAND_IDS`: `(CommandId::OpenKeyboardShortcuts, "preferences.openKeyboardShortcuts"),` after `openSettings`.
- `command_palette.rs` `ENTRIES` (104 → 105): after `"Preferences: Open Settings"` add `entry("Preferences: Open Keyboard Shortcuts", CommandId::OpenKeyboardShortcuts),`.
- `main_window.rs`: grep `CommandId::EditSettingsFile` and mirror every occurrence for `OpenKeyboardShortcuts` (the `execute_command` arm becomes `CommandId::OpenKeyboardShortcuts => show_keyboard_shortcuts(hwnd),`). Change `show_settings`:

```rust
/// The Settings dialog: File → Settings…, Ctrl+, and the activity bar's gear (settings dialog
/// spec §4.3).
pub(crate) fn show_settings(hwnd: HWND) {
    show_settings_page(hwnd, crate::window::settings_model::Page::General);
}

/// Preferences: Open Keyboard Shortcuts (keyboard shortcuts spec §2).
pub(crate) fn show_keyboard_shortcuts(hwnd: HWND) {
    show_settings_page(hwnd, crate::window::settings_model::Page::Shortcuts);
}

fn show_settings_page(hwnd: HWND, page: crate::window::settings_model::Page) {
    let outcome = crate::window::settings_dialog::show(hwnd, current_palette(hwnd), link_color(hwnd), page);
    if outcome == crate::window::settings_dialog::Outcome::EditIni {
        edit_settings_file(hwnd);
    }
}
```

- [ ] **Step 5: Implement the dialog's nav**

In `settings_dialog.rs`:

1. Constants: `WIDTH_AT_96_DPI` 640 → 860; add `NAV_WIDTH_AT_96_DPI: i32 = 180`, `NAV_ITEM_HEIGHT_AT_96_DPI: i32 = 32`, `NAV_INSET_AT_96_DPI: i32 = 8`, `NAV_BAR_WIDTH_AT_96_DPI: i32 = 3`.
2. `Layout`: add `pub nav: RECT, pub nav_items: [RECT; 2]`. In `calculate`, compute `let nav_width = scale(NAV_WIDTH_AT_96_DPI, dpi).min(width / 3);`, change `line`'s `left` to `nav_width + padding`, set `body.left = nav_width`, and after `body`:

```rust
        let nav = RECT { left: 0, top: title_height, right: nav_width, bottom: body.bottom };
        let item_height = scale(NAV_ITEM_HEIGHT_AT_96_DPI, dpi);
        let nav_inset = scale(NAV_INSET_AT_96_DPI, dpi);
        let nav_items = std::array::from_fn(|index| {
            let top = nav.top + nav_inset + index as i32 * item_height;
            RECT { left: nav_inset, top, right: nav_width - nav_inset, bottom: top + item_height }
        });
```

3. `Hit`: add `Nav(Page)` and (for Task 8) `Page(super::shortcuts_page::PageHit)`. Add the `Page` variant in Task 8, not now.
4. `Layout::hit(&self, x, y, scroll, view, page: Page)`: after the `edit_ini` check, add

```rust
        if let Some(index) = self.nav_items.iter().position(&inside) {
            return Some(Hit::Nav(Page::ALL[index]));
        }
        if page != Page::General {
            return None;
        }
```

5. `Dialog`: `model: DialogModel::new(page)`. `show(owner, colors, link_color, page: Page)` and `create(…, page)` pass it through.
6. `focus_of`: `Hit::Nav(_) => Focus::Nav`. `click_effect`: `Hit::Nav(page) => Effect::ShowPage(page)`.
7. `run`: `Effect::ShowPage(page) => { if let Some(dialog) = state(hwnd) { dialog.model.show_page(page); } invalidate(hwnd); }`.
8. `key_down`: before `model_key`, decode Ctrl+PageUp/PageDown:

```rust
    let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
    let key = if ctrl && matches!(virtual_key, VK_PRIOR | VK_NEXT) {
        Some(Key::NextPage { back: virtual_key == VK_PRIOR })
    } else {
        model_key(virtual_key)
    };
    let Some(key) = key else {
        return;
    };
```

   (import `VK_CONTROL`).
9. `compose`: the General body (`for section …` headings and `for row … compose_row`) only when `dialog.model.page == Page::General`. After the title row, paint the nav:

```rust
    frame.shape(Shape::Fill { rect: layout.nav, color: colors.strip_background });
    for page in Page::ALL {
        let item = layout.nav_items[page as usize];
        let current = dialog.model.page == page;
        let hot = dialog.hot == Some(Hit::Nav(page));
        if current || hot {
            tones.soft(frame, item, radius, if current { tones.control } else { tones.card_hot });
        }
        if current {
            let bar = scale(NAV_BAR_WIDTH_AT_96_DPI, dpi);
            let middle = (item.top + item.bottom) / 2;
            frame.shape(Shape::Round {
                rect: RECT { left: item.left, top: middle - scale(8, dpi), right: item.left + bar, bottom: middle + scale(8, dpi) },
                radius: bar / 2,
                color: tones.accent,
            });
        }
        frame.text(
            dialog.body_font,
            colors.editor_foreground,
            page.title(),
            RECT { left: item.left + scale(12, dpi), ..item },
            DT_LEFT,
        );
    }
```

   In the focus-ring `match`, add `Focus::Nav => Some((inset(layout.nav_items[dialog.model.page as usize], -outside), radius + outside)),` and `Focus::Search | Focus::Table => None,` (Task 8/9 fill these).
10. Update every `layout.hit(…)` call in `settings_dialog.rs` (`WM_SETCURSOR`, `WM_MOUSEMOVE`, `WM_LBUTTONDOWN`, `WM_LBUTTONUP`) to pass `dialog.model.page`.
11. Test accessor:

```rust
/// The page the open dialog shows.
#[cfg(test)]
pub(crate) fn current_page(dialog: HWND) -> Option<Page> {
    state(dialog).map(|dialog| dialog.model.page)
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo clippy --all-targets -- -D warnings`
Then: `cargo test --lib window::settings_model window::settings_dialog window::commands window::keymap window::command_palette window::shortcuts_model`
Then: `cargo test --lib main_window::tests::open_keyboard_shortcuts main_window::tests::settings -- --test-threads=1` (the existing settings dialog window tests must still pass: their first Tab from Theme still reaches File icons).
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src/window
git commit -m "feat: Settings dialog pages with a left nav and Open Keyboard Shortcuts"
```

---

### Task 8: The shortcuts page paints its table and takes the mouse

**Files:**
- Create: `src/window/shortcuts_page.rs` (layout, hit-testing, painting)
- Modify: `src/window/mod.rs` (`pub(crate) mod shortcuts_page;`)
- Modify: `src/window/settings_dialog.rs` (`Dialog.shortcuts`, `Dialog.page_layout`, `Hit::Page`, painting, wheel, clicks)

**Interfaces:**
- Consumes: `ShortcutsModel`, `ShortcutsEffect` (Task 6); `Page`, `Hit::Nav` (Task 7); `main_window::keymap`, `set_command_keys`, `reset_command_keys` (Task 4).
- Produces: `shortcuts_page::PageLayout::calculate(body, dpi)`, `.visible_rows()`, `.row_rect(slot)`, `.hit(x, y, &ShortcutsModel) -> Option<PageHit>`; `PageHit { RecordToggle, Row(usize), Pencil(usize), ConflictLink, RecordBox, OutsideRecordBox }`; `shortcuts_page::compose(frame, measure, layout, model, style)`; `settings_dialog::run_shortcuts(hwnd, ShortcutsEffect)`; `#[cfg(test)] settings_dialog::page_row_point(dialog, slot) -> (i32, i32)`, `#[cfg(test)] settings_dialog::shortcuts_model(dialog) -> Option<ShortcutsModel>`.

- [ ] **Step 1: Write the failing tests**

In `shortcuts_page.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::keymap::{KeyStroke, Keymap};
    use crate::window::shortcuts_model::ShortcutsModel;

    fn body() -> RECT {
        RECT { left: 180, top: 44, right: 860, bottom: 700 }
    }

    fn center(rect: RECT) -> (i32, i32) {
        ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    }

    #[test]
    fn the_table_fills_the_body_under_the_search_row() {
        // Break caught: the header or table overlapping the search field, or rows running past
        // the body into the footer.
        let layout = PageLayout::calculate(body(), 96);
        assert!(layout.search.bottom < layout.header.top);
        assert_eq!(layout.header.bottom, layout.table.top);
        assert!(layout.table.bottom <= body().bottom);
        let last = layout.row_rect(layout.visible_rows() - 1);
        assert!(last.bottom <= layout.table.bottom);
        assert!(layout.record_toggle.left > layout.search.right);
    }

    #[test]
    fn rows_pencils_and_the_record_box_hit() {
        let layout = PageLayout::calculate(body(), 96);
        let mut model = ShortcutsModel::new(Keymap::defaults(), layout.visible_rows());
        model.scroll(2);
        let row = layout.row_rect(1);
        let (x, y) = center(row);
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::Row(3)));
        let (x, y) = center(layout.pencil_rect(row));
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::Pencil(3)));
        let (x, y) = center(layout.record_toggle);
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::RecordToggle));

        model.select(3);
        model.start_change();
        let (x, y) = center(layout.record_box);
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::RecordBox));
        assert_eq!(layout.hit(layout.table.left + 1, layout.table.bottom - 1, &model), Some(PageHit::OutsideRecordBox));
        // The link only exists while the recorded key clashes.
        let (x, y) = center(layout.record_lines()[2]);
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::RecordBox));
        model.record_key(KeyStroke::parse("Ctrl+S").unwrap());
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::ConflictLink));
    }

    #[test]
    fn a_row_below_the_last_one_is_not_a_hit() {
        let layout = PageLayout::calculate(body(), 96);
        let mut model = ShortcutsModel::new(Keymap::defaults(), layout.visible_rows());
        model.set_text("save as");
        let (x, y) = center(layout.row_rect(3));
        assert_eq!(layout.hit(x, y, &model), None);
    }
}
```

In `main_window.rs` tests:

```rust
    #[test]
    fn a_double_click_on_a_row_opens_the_recording_box_and_f9_rebinds_it() {
        // Break caught: rows that select but never open the box, or a confirmed key that the
        // window never applies.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_F9, VK_RETURN};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP};
        let scratch = RecoveryScratch::new("shortcuts-double-click");
        let ini = scratch.path().join("fastpad.ini");
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        crate::window::settings_dialog::answer_next(|dialog| unsafe {
            let (x, y) = crate::window::settings_dialog::page_row_point(dialog, 0);
            let at = ((y as isize) << 16 | (x as isize & 0xffff)) as LPARAM;
            for _ in 0..2 {
                PostMessageW(dialog, WM_LBUTTONDOWN, 1, at);
                PostMessageW(dialog, WM_LBUTTONUP, 0, at);
            }
            let key = |vk: u16| PostMessageW(dialog, WM_KEYDOWN, usize::from(vk), 0);
            key(VK_F9);
            key(VK_RETURN);
            key(VK_ESCAPE);
        });
        super::show_keyboard_shortcuts(window.hwnd);
        let first = crate::window::shortcuts_model::ShortcutsModel::new(crate::window::keymap::Keymap::defaults(), 1)
            .rows[0]
            .clone();
        assert_eq!(app_mut(window.hwnd).keymap.keys_of(first.command).last().map(|stroke| stroke.text()).as_deref(), Some("F9"));
        super::save_settings_to(None);
        assert!(std::fs::read_to_string(&ini).unwrap().contains(&format!("key.{}=", first.id)));
    }
```

(`LPARAM` is already imported in the test module; if not, use `windows_sys::Win32::Foundation::LPARAM`.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib window::shortcuts_page`
Expected: compile errors.

- [ ] **Step 3: Implement `shortcuts_page.rs` layout, hits and painting**

```rust
//! The Keyboard Shortcuts page of the Settings dialog: where its parts sit, what the pointer is
//! on, and how it paints (keyboard shortcuts spec §6.2–§6.5). Behaviour lives in
//! `shortcuts_model`; `settings_dialog` routes input here.

use super::keymap::KeyStroke;
use super::palette::Palette;
use super::panel::{inset, scale};
use super::shortcuts_model::ShortcutsModel;
use super::soft_paint::{Frame, Shape, Tones};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{DT_CENTER, DT_LEFT, DT_RIGHT, HFONT};

const PADDING_AT_96_DPI: i32 = 20;
const SEARCH_TOP_AT_96_DPI: i32 = 12;
const FIELD_HEIGHT_AT_96_DPI: i32 = 30;
const GAP_AT_96_DPI: i32 = 8;
const HEADER_HEIGHT_AT_96_DPI: i32 = 28;
const ROW_HEIGHT_AT_96_DPI: i32 = 28;
const PENCIL_WIDTH_AT_96_DPI: i32 = 24;
const CELL_INSET_AT_96_DPI: i32 = 8;
const KEYCAP_HEIGHT_AT_96_DPI: i32 = 20;
const KEYCAP_PADDING_AT_96_DPI: i32 = 6;
const KEYCAP_GAP_AT_96_DPI: i32 = 4;
const RECORD_WIDTH_AT_96_DPI: i32 = 440;
const RECORD_HEIGHT_AT_96_DPI: i32 = 150;
const RECORD_INSET_AT_96_DPI: i32 = 16;
pub(crate) const GLYPH_PENCIL: &str = "\u{E70F}";
pub(crate) const GLYPH_KEYBOARD: &str = "\u{E765}";
const RECORD_PROMPT: &str = "Press desired key combination and then press ENTER.";
const HEADERS: [&str; 3] = ["Command", "Keybinding", "Source"];

/// What the pointer is on. While the recording box is open, everything outside it is
/// `OutsideRecordBox`: a click there closes the box.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageHit {
    RecordToggle,
    Row(usize),
    Pencil(usize),
    ConflictLink,
    RecordBox,
    OutsideRecordBox,
}

#[derive(Clone, Copy)]
pub(crate) struct PageLayout {
    pub search: RECT,
    pub record_toggle: RECT,
    pub header: RECT,
    pub table: RECT,
    /// Command, Keybinding and Source, left and right edges.
    pub columns: [(i32, i32); 3],
    pub record_box: RECT,
    pub row_height: i32,
    dpi: u32,
}

impl PageLayout {
    pub(crate) fn calculate(body: RECT, dpi: u32) -> Self {
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let field = scale(FIELD_HEIGHT_AT_96_DPI, dpi);
        let gap = scale(GAP_AT_96_DPI, dpi);
        let (left, right) = (body.left + padding, body.right - padding);
        let top = body.top + scale(SEARCH_TOP_AT_96_DPI, dpi);
        let record_toggle = RECT { left: right - field, top, right, bottom: top + field };
        let search = RECT { left, top, right: record_toggle.left - gap, bottom: top + field };
        let header_top = search.bottom + gap;
        let header = RECT { left, top: header_top, right, bottom: header_top + scale(HEADER_HEIGHT_AT_96_DPI, dpi) };
        let table = RECT { left, top: header.bottom, right, bottom: (body.bottom - gap).max(header.bottom) };
        let width = right - left;
        let command_end = left + width * 55 / 100;
        let keys_end = left + width * 85 / 100;
        let record_width = scale(RECORD_WIDTH_AT_96_DPI, dpi).min(width);
        let record_height = scale(RECORD_HEIGHT_AT_96_DPI, dpi).min(table.bottom - header.top);
        let box_left = (left + right - record_width) / 2;
        let box_top = (header.top + table.bottom - record_height) / 2;
        Self {
            search,
            record_toggle,
            header,
            table,
            columns: [(left, command_end), (command_end, keys_end), (keys_end, right)],
            record_box: RECT { left: box_left, top: box_top, right: box_left + record_width, bottom: box_top + record_height },
            row_height: scale(ROW_HEIGHT_AT_96_DPI, dpi),
            dpi,
        }
    }

    pub(crate) fn visible_rows(&self) -> usize {
        ((self.table.bottom - self.table.top) / self.row_height).max(1) as usize
    }

    /// The `slot`th visible row.
    pub(crate) fn row_rect(&self, slot: usize) -> RECT {
        let top = self.table.top + slot as i32 * self.row_height;
        RECT { left: self.table.left, top, right: self.table.right, bottom: top + self.row_height }
    }

    pub(crate) fn pencil_rect(&self, row: RECT) -> RECT {
        RECT { right: row.left + scale(PENCIL_WIDTH_AT_96_DPI, self.dpi), ..row }
    }

    fn cell(&self, row: RECT, column: usize) -> RECT {
        let (left, right) = self.columns[column];
        let inset = scale(CELL_INSET_AT_96_DPI, self.dpi);
        RECT { left: left + inset, right: right - inset, ..row }
    }

    /// The recording box's prompt, keys, and refusal-or-link lines.
    pub(crate) fn record_lines(&self) -> [RECT; 3] {
        let inner = inset(self.record_box, scale(RECORD_INSET_AT_96_DPI, self.dpi));
        let height = (inner.bottom - inner.top) / 3;
        std::array::from_fn(|index| RECT {
            top: inner.top + index as i32 * height,
            bottom: inner.top + (index as i32 + 1) * height,
            ..inner
        })
    }

    pub(crate) fn hit(&self, x: i32, y: i32, model: &ShortcutsModel) -> Option<PageHit> {
        let inside = |rect: &RECT| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
        if model.recording.is_some() {
            if model.conflict_count() > 0 && inside(&self.record_lines()[2]) {
                return Some(PageHit::ConflictLink);
            }
            return Some(if inside(&self.record_box) { PageHit::RecordBox } else { PageHit::OutsideRecordBox });
        }
        if inside(&self.record_toggle) {
            return Some(PageHit::RecordToggle);
        }
        if !inside(&self.table) {
            return None;
        }
        let slot = ((y - self.table.top) / self.row_height) as usize;
        let index = model.top + slot;
        if slot >= self.visible_rows() || index >= model.rows.len() {
            return None;
        }
        let row = self.row_rect(slot);
        Some(if inside(&self.pencil_rect(row)) { PageHit::Pencil(index) } else { PageHit::Row(index) })
    }
}

/// What the page paints with.
pub(crate) struct PageStyle<'a> {
    pub colors: &'a Palette,
    pub tones: &'a Tones,
    pub link_color: u32,
    pub body_font: HFONT,
    pub heading_font: HFONT,
    pub link_font: HFONT,
    pub glyph_font: HFONT,
    pub radius: i32,
    pub hot: Option<PageHit>,
    pub table_focused: bool,
    pub search_focused: bool,
}

/// Keycaps for `stroke` in `line`, from `left` (or centred when `left` is `None`).
fn keycaps<'a>(frame: &mut Frame<'a>, style: &PageStyle<'_>, measure: &dyn Fn(&str) -> i32, stroke: KeyStroke, line: RECT, left: Option<i32>, dpi: u32) {
    let parts = stroke.parts();
    let padding = scale(KEYCAP_PADDING_AT_96_DPI, dpi);
    let gap = scale(KEYCAP_GAP_AT_96_DPI, dpi);
    let plus = measure("+");
    let widths = parts.iter().map(|part| measure(part) + 2 * padding).collect::<Vec<_>>();
    let total = widths.iter().sum::<i32>() + (widths.len() as i32 - 1) * (plus + 2 * gap);
    let mut x = left.unwrap_or((line.left + line.right - total) / 2);
    let height = scale(KEYCAP_HEIGHT_AT_96_DPI, dpi);
    let top = (line.top + line.bottom - height) / 2;
    for (index, (part, width)) in parts.into_iter().zip(widths).enumerate() {
        if index > 0 {
            let sign = RECT { left: x + gap, top, right: x + gap + plus, bottom: top + height };
            frame.text(style.body_font, style.colors.muted_foreground, "+", sign, DT_CENTER);
            x += plus + 2 * gap;
        }
        let cap = RECT { left: x, top, right: x + width, bottom: top + height };
        style.tones.soft(frame, cap, style.radius, style.tones.control);
        frame.shape(Shape::Ring { rect: cap, radius: style.radius, width: 1, color: style.tones.outline.unwrap_or(style.tones.control_down) });
        frame.text(style.body_font, style.colors.editor_foreground, part, cap, DT_CENTER);
        x += width;
    }
}

/// Paints the page. `measure` gives a text's width in the body font.
pub(crate) fn compose<'a>(frame: &mut Frame<'a>, measure: &dyn Fn(&str) -> i32, layout: &PageLayout, model: &'a ShortcutsModel, style: &PageStyle<'_>) {
    let dpi = layout.dpi;
    let colors = style.colors;
    let tones = style.tones;

    // The search field's box (the EDIT sits inside it) and the record-keys toggle.
    tones.soft(frame, layout.search, style.radius, tones.control);
    let toggle_color = match (model.record_keys, style.hot == Some(PageHit::RecordToggle)) {
        (true, _) => tones.accent,
        (false, true) => tones.control_hot,
        (false, false) => tones.control,
    };
    tones.soft(frame, layout.record_toggle, style.radius, toggle_color);
    let glyph = if model.record_keys { tones.on_accent } else { colors.editor_foreground };
    frame.text(style.glyph_font, glyph, GLYPH_KEYBOARD, layout.record_toggle, DT_CENTER);

    // Header.
    for (column, title) in HEADERS.into_iter().enumerate() {
        let cell = layout.cell(layout.header, column);
        let cell = if column == 0 { RECT { left: cell.left + scale(PENCIL_WIDTH_AT_96_DPI, dpi), ..cell } } else { cell };
        frame.text(style.heading_font, colors.muted_foreground, title, cell, DT_LEFT);
    }
    frame.shape(Shape::Fill { rect: RECT { top: layout.header.bottom - 1, ..layout.header }, color: tones.card });

    // Rows.
    frame.clip(Some(layout.table));
    for slot in 0..layout.visible_rows() {
        let index = model.top + slot;
        let Some(row) = model.rows.get(index) else { break };
        let rect = layout.row_rect(slot);
        let selected = index == model.selected;
        let hot = matches!(style.hot, Some(PageHit::Row(i) | PageHit::Pencil(i)) if i == index);
        if selected || hot {
            let fill = match (selected, style.table_focused) {
                (true, true) => tones.control_hot,
                (true, false) => tones.control,
                (false, _) => tones.card_hot,
            };
            tones.soft(frame, rect, style.radius, fill);
        }
        if selected && style.table_focused {
            let bar = scale(3, dpi);
            frame.shape(Shape::Fill { rect: RECT { right: rect.left + bar, ..rect }, color: tones.accent });
        }
        if selected || hot {
            frame.text(style.glyph_font, colors.muted_foreground, GLYPH_PENCIL, layout.pencil_rect(rect), DT_CENTER);
        }
        let command = layout.cell(rect, 0);
        let command = RECT { left: command.left + scale(PENCIL_WIDTH_AT_96_DPI, dpi) - scale(CELL_INSET_AT_96_DPI, dpi), ..command };
        frame.text(style.body_font, colors.editor_foreground, row.title, command, DT_LEFT);
        if selected {
            frame.text(style.body_font, colors.muted_foreground, row.id, command, DT_RIGHT);
        }
        let keys = layout.cell(rect, 1);
        match row.stroke {
            Some(stroke) => keycaps(frame, style, measure, stroke, keys, Some(keys.left), dpi),
            None => frame.text(style.body_font, colors.muted_foreground, "\u{2014}", keys, DT_LEFT),
        }
        let source = match (row.user, row.stroke.is_some()) {
            (true, _) => "User",
            (false, true) => "Default",
            (false, false) => "",
        };
        frame.text(style.body_font, colors.muted_foreground, source, layout.cell(rect, 2), DT_LEFT);
    }
    frame.clip(None);

    // The recording box, over the table.
    if let Some(recording) = &model.recording {
        let box_rect = layout.record_box;
        tones.soft(frame, box_rect, style.radius, colors.panel_background());
        frame.shape(Shape::Ring { rect: box_rect, radius: style.radius, width: scale(2, dpi), color: tones.accent });
        let [prompt, keys, note] = layout.record_lines();
        frame.text(style.body_font, colors.editor_foreground, RECORD_PROMPT, prompt, DT_CENTER);
        if let Some(stroke) = recording.stroke {
            keycaps(frame, style, measure, stroke, keys, None, dpi);
        }
        if let Some(refusal) = recording.refusal {
            frame.text(style.body_font, colors.error_foreground, refusal, note, DT_CENTER);
        } else {
            let count = model.conflict_count();
            if count > 0 {
                let text = if count == 1 {
                    "1 existing command has this keybinding".to_owned()
                } else {
                    format!("{count} existing commands have this keybinding")
                };
                frame.text(style.link_font, style.link_color, text, note, DT_CENTER);
            }
        }
    }
}
```

Adjust names to what `Palette` actually offers (`panel_background()` is a method used by `settings_dialog::compose`; `error_foreground` and `muted_foreground` are fields). If `Frame::text` requires `&'a str` for `row.title`/`row.id`, they are `&'static str` and satisfy it; `format!` results pass as `String` through `impl Into<Cow<'a, str>>`.

- [ ] **Step 4: Wire the page into the dialog**

In `settings_dialog.rs`:

1. `Hit`: add `Page(super::shortcuts_page::PageHit)`.
2. `Dialog`: add fields

```rust
    page_layout: super::shortcuts_page::PageLayout,
    shortcuts: super::shortcuts_model::ShortcutsModel,
    /// When and on which row the last click in the table was, for double-clicks.
    last_row_click: Option<(u32, usize)>,
```

   In `create`, after `layout`:

```rust
    let page_layout = super::shortcuts_page::PageLayout::calculate(layout.body, dpi);
    let shortcuts = super::shortcuts_model::ShortcutsModel::new(
        super::main_window::keymap(owner),
        page_layout.visible_rows(),
    );
```

3. A single hit function used by every mouse handler instead of `layout.hit`:

```rust
/// What client point `x`, `y` is on, on the page shown. The recording box takes every click.
fn hit_at(dialog: &Dialog, x: i32, y: i32) -> Option<Hit> {
    let shortcuts = dialog.model.page == Page::Shortcuts;
    if shortcuts && dialog.shortcuts.recording.is_some() {
        return dialog.page_layout.hit(x, y, &dialog.shortcuts).map(Hit::Page);
    }
    dialog
        .layout
        .hit(x, y, dialog.scroll, &dialog.view, dialog.model.page)
        .or_else(|| {
            shortcuts
                .then(|| dialog.page_layout.hit(x, y, &dialog.shortcuts))
                .flatten()
                .map(Hit::Page)
        })
}
```

   Replace the `dialog.layout.hit(...)` calls in `WM_SETCURSOR`, `WM_MOUSEMOVE`, `WM_LBUTTONDOWN` and `WM_LBUTTONUP` with `hit_at(dialog, x, y)`.
4. `focus_of`: `Hit::Page(PageHit::RecordToggle) => Focus::Search, Hit::Page(_) => Focus::Table`. In `WM_LBUTTONDOWN`, do not call `set_focus` for `Hit::Page(PageHit::RecordBox | PageHit::OutsideRecordBox | PageHit::ConflictLink)` (return `Effect::None` for those, like greyed rows).
5. `click_effect`: `Hit::Page(_) => Effect::None` (page clicks go through `page_click`). Replace the body of `WM_LBUTTONUP` after the `swallowing` check with:

```rust
            let (x, y) = lparam_point(lparam);
            unsafe { ReleaseCapture() };
            let released = state(hwnd).and_then(|dialog| {
                let pressed = dialog.pressed.take()?;
                (hit_at(dialog, x, y) == Some(pressed)).then_some(pressed)
            });
            invalidate(hwnd);
            match released {
                Some(Hit::Page(hit)) => page_click(hwnd, hit),
                Some(hit) => {
                    let effect = state(hwnd).map(|dialog| click_effect(dialog, hit));
                    if let Some(effect) = effect {
                        run(hwnd, effect);
                    }
                }
                None => {}
            }
            0
```

6. `page_click` and `run_shortcuts`:

```rust
/// A click released on the shortcuts page.
fn page_click(hwnd: HWND, hit: super::shortcuts_page::PageHit) {
    use super::shortcuts_page::PageHit;
    use super::shortcuts_model::ShortcutsEffect;
    let now = unsafe { GetMessageTime() } as u32;
    let double_click_time = unsafe { GetDoubleClickTime() };
    let effect = state(hwnd).map(|dialog| {
        let model = &mut dialog.shortcuts;
        match hit {
            PageHit::RecordToggle => model.toggle_record_keys(),
            PageHit::Row(index) => {
                let double = dialog
                    .last_row_click
                    .is_some_and(|(time, row)| row == index && now.wrapping_sub(time) <= double_click_time);
                dialog.last_row_click = (!double).then_some((now, index));
                model.select(index);
                if double { model.start_change() } else { ShortcutsEffect::Repaint }
            }
            PageHit::Pencil(index) => {
                model.select(index);
                model.start_change()
            }
            PageHit::ConflictLink => model.follow_conflicts(),
            PageHit::RecordBox => ShortcutsEffect::None,
            PageHit::OutsideRecordBox => model.cancel_recording(),
        }
    });
    if let Some(effect) = effect {
        run_shortcuts(hwnd, effect);
    }
    if hit == PageHit::RecordToggle {
        run_shortcuts(hwnd, ShortcutsEffect::FocusSearch);
    }
}

/// Carries out a shortcuts page effect. As in `run`, no `Dialog` borrow may be alive: applying
/// keys runs main window code.
pub(crate) fn run_shortcuts(hwnd: HWND, effect: super::shortcuts_model::ShortcutsEffect) {
    use super::shortcuts_model::ShortcutsEffect;
    match effect {
        ShortcutsEffect::None => {}
        ShortcutsEffect::Repaint => invalidate(hwnd),
        ShortcutsEffect::SetKeys(command, keys) => {
            super::main_window::set_command_keys(owner(hwnd), command, keys);
            after_keymap_change(hwnd);
        }
        ShortcutsEffect::Reset(command) => {
            super::main_window::reset_command_keys(owner(hwnd), command);
            after_keymap_change(hwnd);
        }
        ShortcutsEffect::CopyId(id) => {
            // Task 10 puts `id` on the clipboard.
            let _ = id;
        }
        ShortcutsEffect::FocusSearch | ShortcutsEffect::FocusTable => {
            let focus = if effect == ShortcutsEffect::FocusSearch { Focus::Search } else { Focus::Table };
            let repaint = state(hwnd).map(|dialog| dialog.model.set_focus(focus, &dialog.view));
            if let Some(repaint) = repaint {
                run(hwnd, repaint);
            }
        }
        ShortcutsEffect::SetSearchText(_) => invalidate(hwnd), // Task 9 writes it to the field.
        ShortcutsEffect::Close => close(hwnd),
    }
}

/// Re-reads the keymap after a change applied, keeping the selection.
fn after_keymap_change(hwnd: HWND) {
    let keymap = super::main_window::keymap(owner(hwnd));
    if let Some(dialog) = state(hwnd) {
        dialog.shortcuts.refresh(keymap);
    }
    if unsafe { GetFocus() } != hwnd {
        unsafe { SetFocus(hwnd) };
    }
    invalidate(hwnd);
}
```

7. Keys: in `key_down`, after the dropdown-list handling and before the Ctrl+PageUp decoding, add:

```rust
    let stroke = current_stroke(virtual_key);
    let routed = state(hwnd).and_then(|dialog| {
        if dialog.model.page != Page::Shortcuts {
            return None;
        }
        if dialog.shortcuts.recording.is_some() {
            // Every key goes to the box; a modifier alone records nothing.
            return Some(stroke.map_or(super::shortcuts_model::ShortcutsEffect::None, |stroke| dialog.shortcuts.record_key(stroke)));
        }
        let stroke = stroke?;
        if stroke == KeyStroke::new(false, false, true, u16::from(b'K')) {
            return Some(dialog.shortcuts.toggle_record_keys());
        }
        if dialog.model.focus != Focus::Table {
            return None;
        }
        let effect = dialog.shortcuts.table_key(stroke);
        (effect != super::shortcuts_model::ShortcutsEffect::None).then_some(effect)
    });
    if let Some(effect) = routed {
        run_shortcuts(hwnd, effect);
        return;
    }
```

   with the helper:

```rust
/// The stroke `virtual_key` makes with the modifiers held now.
fn current_stroke(virtual_key: u16) -> Option<KeyStroke> {
    let held = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    KeyStroke::from_key(virtual_key, held(VK_CONTROL), held(VK_SHIFT), held(VK_MENU))
}
```

   (import `super::keymap::KeyStroke`, `VK_MENU`.)
8. `WM_SYSKEYDOWN`: before the existing Alt+Down arm add

```rust
        WM_SYSKEYDOWN
            if state(hwnd).is_some_and(|dialog| {
                dialog.model.page == Page::Shortcuts
                    && (dialog.shortcuts.recording.is_some()
                        || wparam as u16 == u16::from(b'K')
                        || dialog.model.focus == Focus::Table)
            }) =>
        {
            key_down(hwnd, wparam as u16);
            0
        }
        // No menu to open and no beep for Alt+letters on the shortcuts page.
        windows_sys::Win32::UI::WindowsAndMessaging::WM_SYSCHAR
            if state(hwnd).is_some_and(|dialog| dialog.model.page == Page::Shortcuts) => 0,
```

9. `WM_MOUSEWHEEL`: when `dialog.model.page == Page::Shortcuts`, `dialog.shortcuts.scroll(-(i32::from(delta) * 3 / 120) as isize); invalidate(hwnd);` instead of scrolling the General body.
10. Painting: `paint_into(dc, client, dialog)` builds a measure closure on `dc` with `body_font` and passes it into `compose`:

```rust
fn text_width(dc: HDC, font: HFONT, text: &str) -> i32 {
    use windows_sys::Win32::Foundation::SIZE;
    use windows_sys::Win32::Graphics::Gdi::GetTextExtentPoint32W;
    let wide = text.encode_utf16().collect::<Vec<_>>();
    let mut size = SIZE::default();
    unsafe {
        let previous = SelectObject(dc, font as _);
        GetTextExtentPoint32W(dc, wide.as_ptr(), wide.len() as i32, &mut size);
        SelectObject(dc, previous);
    }
    size.cx
}
```

   `compose` gains a last parameter `measure: &dyn Fn(&str) -> i32`, and `paint_into` passes it:

```rust
fn paint_into(dc: HDC, client: RECT, dialog: &Dialog) {
    let measure = |text: &str| text_width(dc, dialog.body_font, text);
    let mut frame = Frame::default();
    compose(&mut frame, client, dialog, &measure);
    frame.paint(dc, client, &dialog.canvas);
}
```

   In `compose`, when `dialog.model.page == Page::Shortcuts`:

```rust
        let style = super::shortcuts_page::PageStyle {
            colors,
            tones: &tones,
            link_color: dialog.link_color,
            body_font: dialog.body_font,
            heading_font: dialog.heading_font,
            link_font: dialog.link_font,
            glyph_font: dialog.glyph_font,
            radius,
            hot: match dialog.hot { Some(Hit::Page(hit)) => Some(hit), _ => None },
            table_focused: dialog.model.focus == Focus::Table,
            search_focused: dialog.model.focus == Focus::Search,
        };
        super::shortcuts_page::compose(frame, measure, &dialog.page_layout, &dialog.shortcuts, &style);
```

   and in the focus ring `match`: `Focus::Search => Some((inset(dialog.page_layout.search, -outside), radius + outside))`.
11. Test accessors:

```rust
/// The client centre of the shortcuts table's `slot`th visible row.
#[cfg(test)]
pub(crate) fn page_row_point(dialog: HWND, slot: usize) -> (i32, i32) {
    let rect = state(dialog).map(|dialog| dialog.page_layout.row_rect(slot)).unwrap_or_default();
    ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
}

/// A copy of the open dialog's shortcuts page state.
#[cfg(test)]
pub(crate) fn shortcuts_model(dialog: HWND) -> Option<super::shortcuts_model::ShortcutsModel> {
    state(dialog).map(|dialog| dialog.shortcuts.clone())
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo clippy --all-targets -- -D warnings`
Then: `cargo test --lib window::shortcuts_page window::settings_dialog`
Then: `cargo test --lib main_window::tests::a_double_click_on_a_row main_window::tests::open_keyboard_shortcuts main_window::tests::settings -- --test-threads=1`
Expected: all pass.

- [ ] **Step 6: Look at it**

Back up `%LocalAppData%\FastPad\fastpad.ini`, run `cargo run --release`, press Ctrl+Shift+P → "Open Keyboard Shortcuts", check both themes (Theme dropdown on General), hover and select rows, double-click a row, press F9 and Enter, check the menu shows F9. Restore `fastpad.ini`.

- [ ] **Step 7: Commit**

```bash
git add src/window
git commit -m "feat: Keyboard Shortcuts table with keycaps, recording box and mouse"
```

---

### Task 9: The search field and record-keys search

**Files:**
- Modify: `src/window/shortcuts_page.rs` (`create_search`, `search_proc`)
- Modify: `src/window/settings_dialog.rs` (`Dialog.search`, `Dialog.search_brush`, `WM_CTLCOLOREDIT`, `WM_COMMAND`/`EN_CHANGE`, `search_key`, `search_swallows_char`, `search_focused`, `paint_search_cue`, focus sync, `SetSearchText`)

**Interfaces:**
- Consumes: `ShortcutsModel::{set_text, record_search_key, toggle_record_keys}` (Task 6), `run_shortcuts` (Task 8).
- Produces: `shortcuts_page::create_search(dialog, rect, font) -> HWND`; `settings_dialog::search_key(dialog, vk) -> bool`, `search_swallows_char(dialog, c: u32, sys: bool) -> bool`, `search_focused(dialog)`, `paint_search_cue(dialog, edit)`; `#[cfg(test)] settings_dialog::search_hwnd(dialog) -> HWND`.

- [ ] **Step 1: Write the failing window tests**

```rust
    #[test]
    fn typing_in_the_search_filters_and_down_enters_the_table() {
        // Break caught: EN_CHANGE not reaching the model, or Down leaving the focus in the field.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CHAR, WM_KEYDOWN};
        let window = ProductionWindow::new(make_app());
        let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
        let record = seen.clone();
        crate::window::settings_dialog::answer_next(move |dialog| unsafe {
            let search = crate::window::settings_dialog::search_hwnd(dialog);
            for c in "save as".chars() {
                PostMessageW(search, WM_CHAR, c as usize, 0);
            }
            PostMessageW(search, WM_KEYDOWN, usize::from(VK_DOWN), 0);
            // Read the state from inside the loop, before Escape closes the dialog.
            crate::window::settings_dialog::answer_in_loop(dialog, move |dialog| {
                *record.borrow_mut() = crate::window::settings_dialog::shortcuts_model(dialog)
                    .map(|model| (model.rows.len(), model.rows[0].command, crate::window::settings_dialog::current_focus(dialog)));
                PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
            });
        });
        super::show_keyboard_shortcuts(window.hwnd);
        let (count, command, focus) = seen.borrow().clone().unwrap();
        assert_eq!((count, command), (1, CommandId::SaveAs));
        assert_eq!(focus, Some(crate::window::settings_model::Focus::Table));
    }

    #[test]
    fn record_keys_search_shows_only_the_stroke() {
        // Break caught: record-keys mode letting the key's character into the field ("Ss").
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextW, PostMessageW, WM_CHAR, WM_KEYDOWN};
        let window = ProductionWindow::new(make_app());
        let seen = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let record = seen.clone();
        crate::window::settings_dialog::answer_next(move |dialog| unsafe {
            let search = crate::window::settings_dialog::search_hwnd(dialog);
            crate::window::settings_dialog::toggle_record_keys_for_test(dialog);
            PostMessageW(search, WM_KEYDOWN, usize::from(b'S'), 0);
            PostMessageW(search, WM_CHAR, usize::from(b's'), 0);
            crate::window::settings_dialog::answer_in_loop(dialog, move |dialog| {
                let mut buffer = [0u16; 64];
                let length = GetWindowTextW(search, buffer.as_mut_ptr(), 64);
                *record.borrow_mut() = String::from_utf16_lossy(&buffer[..length as usize]);
                PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
            });
        });
        super::show_keyboard_shortcuts(window.hwnd);
        assert_eq!(seen.borrow().as_str(), "S");
    }
```

These need three more test hooks in `settings_dialog.rs` (add them in Step 3):
- `answer_in_loop(dialog, f)`: posts a private message (`WM_APP + 3`) whose handler pops and runs `f(dialog)` from a thread-local queue, so a test reads state after the posted input before it posts Escape.
- `current_focus(dialog) -> Option<Focus>`.
- `toggle_record_keys_for_test(dialog)`: runs `toggle_record_keys` and `run_shortcuts` (the Alt+K path, without needing a held Alt).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib main_window::tests::typing_in_the_search -- --test-threads=1`
Expected: compile errors (`search_hwnd`, `answer_in_loop`, …).

- [ ] **Step 3: Implement the field**

In `shortcuts_page.rs`:

```rust
use crate::platform::wide_null;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, ES_AUTOHSCROLL, GetWindowTextLengthW, SendMessageW, WM_CHAR, WM_KEYDOWN,
    WM_NCDESTROY, WM_PAINT, WM_SETFOCUS, WM_SETFONT, WM_SYSCHAR, WM_SYSKEYDOWN, WS_CHILD,
};

pub(crate) const SEARCH_CONTROL_ID: usize = 0x5348;
const SEARCH_HOOK_ID: usize = 0x5348_4B53;
const FIELD_TEXT_INSET_AT_96_DPI: i32 = 10;

/// The native EDIT inside the painted search box, vertically centred for a `text_height` font.
pub(crate) fn field_rect(search: RECT, text_height: i32, dpi: u32) -> RECT {
    let top = (search.top + search.bottom - text_height) / 2;
    RECT {
        left: search.left + scale(FIELD_TEXT_INSET_AT_96_DPI, dpi),
        top,
        right: search.right - scale(FIELD_TEXT_INSET_AT_96_DPI, dpi),
        bottom: top + text_height,
    }
}

/// The search field, hidden until the page shows. Its keys go to the dialog first through
/// `search_proc`.
pub(crate) fn create_search(dialog: HWND, rect: RECT, font: HFONT) -> HWND {
    let class = wide_null("EDIT");
    let edit = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | ES_AUTOHSCROLL as u32,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            dialog,
            SEARCH_CONTROL_ID as _,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if edit.is_null() {
        return edit;
    }
    unsafe {
        SendMessageW(edit, WM_SETFONT, font as usize, 0);
        SetWindowSubclass(edit, Some(search_proc), SEARCH_HOOK_ID, dialog as usize);
    }
    edit
}

unsafe extern "system" fn search_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    dialog: usize,
) -> LRESULT {
    let dialog = dialog as HWND;
    match message {
        WM_KEYDOWN | WM_SYSKEYDOWN if super::settings_dialog::search_key(dialog, wparam as u16) => return 0,
        WM_CHAR | WM_SYSCHAR
            if super::settings_dialog::search_swallows_char(dialog, wparam as u32, message == WM_SYSCHAR) =>
        {
            return 0;
        }
        WM_SETFOCUS => super::settings_dialog::search_focused(dialog),
        // The empty field shows its cue (EM_SETCUEBANNER needs ComCtl32 v6).
        WM_PAINT if unsafe { GetWindowTextLengthW(hwnd) } == 0 => {
            super::settings_dialog::paint_search_cue(dialog, hwnd);
            return 0;
        }
        WM_NCDESTROY => unsafe {
            RemoveWindowSubclass(hwnd, Some(search_proc), SEARCH_HOOK_ID);
        },
        _ => {}
    }
    // The EDIT repaints only the text it changes; the cue must go (or come back) whole.
    let was_empty = unsafe { GetWindowTextLengthW(hwnd) } == 0;
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if was_empty != (unsafe { GetWindowTextLengthW(hwnd) } == 0) {
        unsafe { InvalidateRect(hwnd, std::ptr::null(), 1) };
    }
    result
}
```

In `settings_dialog.rs`:

1. `Dialog`: add `search: HWND` and `search_brush: HBRUSH` (from `CreateSolidBrush(Tones::new(&colors).control)`); delete the brush in `Drop`; recreate it in `refresh` when the colours change. Create the field in `create` after `SetWindowLongPtrW`:

```rust
    let text_height = scale(18, dpi);
    let field = super::shortcuts_page::field_rect(page_layout.search, text_height, dpi);
    let search = super::shortcuts_page::create_search(dialog, field, body_font);
```

   store it, and in `show` after `ShowWindow(dialog, …)`: when the page is `Shortcuts`, `ShowWindow(search, SW_SHOW)` and `SetFocus(search)` instead of `SetFocus(dialog)`.
2. `run(Effect::ShowPage(page))`: after `show_page`, `ShowWindow(search, if page == Page::Shortcuts { SW_SHOW } else { SW_HIDE })`, then `sync_focus(hwnd)`.
3. Focus sync, called at the end of `run(Effect::Repaint)` and after `show_page`:

```rust
/// Puts the keyboard focus where the model has it: in the search field, or on the dialog.
fn sync_focus(hwnd: HWND) {
    let Some((wants_search, search)) = state(hwnd).map(|dialog| (dialog.model.focus == Focus::Search, dialog.search)) else {
        return;
    };
    let focus = unsafe { GetFocus() };
    if wants_search && focus != search {
        unsafe { SetFocus(search) };
    } else if !wants_search && focus == search {
        unsafe { SetFocus(hwnd) };
    }
}
```

4. Colours:

```rust
        windows_sys::Win32::UI::WindowsAndMessaging::WM_CTLCOLOREDIT => {
            let Some((foreground, background, brush)) = state(hwnd).map(|dialog| {
                (dialog.colors.editor_foreground, Tones::new(&dialog.colors).control, dialog.search_brush)
            }) else {
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            };
            let dc = wparam as HDC;
            unsafe {
                SetTextColor(dc, foreground);
                SetBkColor(dc, background);
            }
            brush as LRESULT
        }
```

5. `EN_CHANGE`:

```rust
        windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND
            if (wparam & 0xffff) == super::shortcuts_page::SEARCH_CONTROL_ID
                && ((wparam >> 16) & 0xffff) as u32 == windows_sys::Win32::UI::WindowsAndMessaging::EN_CHANGE =>
        {
            let search = state(hwnd).map(|dialog| dialog.search);
            let text = search.map(window_text).unwrap_or_default();
            // In record-keys mode the dialog writes the field itself.
            let effect = state(hwnd).and_then(|dialog| (!dialog.shortcuts.record_keys).then(|| dialog.shortcuts.set_text(&text)));
            if let Some(effect) = effect {
                run_shortcuts(hwnd, effect);
            }
            0
        }
```

   with `fn window_text(hwnd: HWND) -> String` using `GetWindowTextLengthW` + `GetWindowTextW`.
6. `run_shortcuts(SetSearchText(text))`: `let search = state(hwnd).map(|d| d.search); if let Some(search) = search { SetWindowTextW(search, wide_null(&text).as_ptr()) }; invalidate(hwnd);`.
7. The subclass entry points:

```rust
/// A key pressed in the search field; true when the dialog took it (keyboard shortcuts spec
/// §6.2).
pub(crate) fn search_key(dialog: HWND, virtual_key: u16) -> bool {
    use super::shortcuts_model::ShortcutsEffect;
    let stroke = current_stroke(virtual_key);
    let held = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    let Some(record_keys) = state(dialog).map(|d| d.shortcuts.record_keys) else {
        return false;
    };
    let plain = |key: u16| stroke == Some(KeyStroke::new(false, false, false, key));
    let effect = if virtual_key == VK_TAB && !held(VK_CONTROL) && !held(VK_MENU) {
        let effect = state(dialog).map(|d| d.model.key(Key::Tab { back: held(VK_SHIFT) }, &d.view));
        if let Some(effect) = effect {
            run(dialog, effect);
        }
        return true;
    } else if held(VK_CONTROL) && matches!(virtual_key, VK_PRIOR | VK_NEXT) {
        let effect = state(dialog).map(|d| d.model.key(Key::NextPage { back: virtual_key == VK_PRIOR }, &d.view));
        if let Some(effect) = effect {
            run(dialog, effect);
        }
        return true;
    } else if stroke == Some(KeyStroke::new(false, false, true, u16::from(b'K'))) || (record_keys && plain(VK_ESCAPE)) {
        state(dialog).map(|d| d.shortcuts.toggle_record_keys())
    } else if record_keys {
        // Every stroke becomes the filter; a modifier alone waits for its key.
        match stroke {
            Some(stroke) => state(dialog).map(|d| d.shortcuts.record_search_key(stroke)),
            None => return true,
        }
    } else if plain(VK_DOWN) || plain(VK_RETURN) {
        Some(ShortcutsEffect::FocusTable)
    } else if plain(VK_ESCAPE) {
        Some(ShortcutsEffect::Close)
    } else {
        return false;
    };
    if let Some(effect) = effect {
        run_shortcuts(dialog, effect);
    }
    true
}

/// Whether the field must not see a character: every one in record-keys mode, the Tab, Enter
/// and Escape characters it would beep at, and Alt+letters.
pub(crate) fn search_swallows_char(dialog: HWND, c: u32, sys: bool) -> bool {
    sys || matches!(c, 0x09 | 0x0d | 0x1b) || state(dialog).is_some_and(|d| d.shortcuts.record_keys)
}

/// The field took the focus (a click on it): the model follows.
pub(crate) fn search_focused(dialog: HWND) {
    if let Some(d) = state(dialog) {
        d.model.focus = Focus::Search;
    }
    invalidate(dialog);
}

/// The empty field's cue, in the muted colour.
pub(crate) fn paint_search_cue(dialog: HWND, edit: HWND) {
    use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, FillRect, PAINTSTRUCT, SetBkMode, TRANSPARENT};
    let Some((brush, color, font, cue)) = state(dialog).map(|d| {
        let cue = if d.shortcuts.record_keys { "Press keys to search" } else { "Type to search in keybindings" };
        (d.search_brush, d.colors.muted_foreground, d.body_font, cue)
    }) else {
        return;
    };
    let mut paint = PAINTSTRUCT::default();
    unsafe {
        let dc = BeginPaint(edit, &mut paint);
        let mut client = RECT::default();
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(edit, &mut client);
        FillRect(dc, &client, brush);
        let previous = SelectObject(dc, font as _);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        let mut text = wide_null(cue);
        DrawTextW(dc, text.as_mut_ptr(), -1, &mut client, DT_LEFT | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, previous);
        EndPaint(edit, &paint);
    }
}
```

   Toggling record mode must also repaint the field (the cue changes): in `run_shortcuts(SetSearchText)` also `InvalidateRect(search, null, 1)`.
8. Test hooks (all `#[cfg(test)]`): `search_hwnd(dialog) -> HWND`, `current_focus(dialog) -> Option<Focus>`, `toggle_record_keys_for_test(dialog)` (runs `toggle_record_keys` then `run_shortcuts`), and `answer_in_loop`:

```rust
#[cfg(test)]
const WM_TEST_ANSWER: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 3;

#[cfg(test)]
thread_local! {
    static IN_LOOP: std::cell::RefCell<std::collections::VecDeque<Answer>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// Runs `answer` from the dialog's loop after the input posted so far.
#[cfg(test)]
pub(crate) fn answer_in_loop(dialog: HWND, answer: impl FnOnce(HWND) + 'static) {
    IN_LOOP.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
    unsafe { PostMessageW(dialog, WM_TEST_ANSWER, 0, 0) };
}
```

   and in `dialog_proc`: `#[cfg(test)] WM_TEST_ANSWER => { if let Some(answer) = IN_LOOP.with(|a| a.borrow_mut().pop_front()) { answer(hwnd); } 0 }`. (A `#[cfg(test)]` match arm is allowed on its own arm.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo clippy --all-targets -- -D warnings`
Then: `cargo test --lib main_window::tests::typing_in_the_search main_window::tests::record_keys_search main_window::tests::a_double_click_on_a_row main_window::tests::settings -- --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/window
git commit -m "feat: Keyboard Shortcuts search field with record-keys mode"
```

---

### Task 10: Row context menu, Copy command ID and recording edge cases

**Files:**
- Create: `src/platform/clipboard.rs`
- Modify: `src/platform/mod.rs` (`pub mod clipboard;`), `Cargo.toml` (`"Win32_System_DataExchange"` in the `windows-sys` features, alphabetical)
- Modify: `src/window/menus.rs` (`track_choice`, `answer_next_choice`)
- Modify: `src/window/settings_dialog.rs` (`WM_RBUTTONUP`, `context_menu`, `CopyId`)

**Interfaces:**
- Produces: `platform::clipboard::set_text(owner: HWND, text: &str) -> crate::Result<()>`; `menus::track_choice(owner: HWND, window: HWND, items: &[(String, usize)], client: POINT) -> Option<usize>` (an empty label is a separator); `#[cfg(test)] menus::answer_next_choice(f: impl FnOnce(&[(String, usize)]) -> Option<usize>)`.

- [ ] **Step 1: Write the failing tests**

In `main_window.rs` tests:

```rust
    #[test]
    fn delete_unbinds_and_the_context_menus_reset_restores_the_defaults() {
        // Break caught: Reset offered for default rows, or leaving `key.file.saveAs=` behind.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_DOWN, VK_ESCAPE};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CHAR, WM_KEYDOWN, WM_RBUTTONUP};
        let scratch = RecoveryScratch::new("shortcuts-reset");
        let ini = scratch.path().join("fastpad.ini");
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        let offered = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = offered.clone();
        crate::window::menus::answer_next_choice(move |items| {
            *record.borrow_mut() = items.iter().map(|(label, _)| label.clone()).collect();
            items.iter().find(|(label, _)| label.starts_with("Reset")).map(|(_, id)| *id)
        });
        crate::window::settings_dialog::answer_next(|dialog| unsafe {
            let search = crate::window::settings_dialog::search_hwnd(dialog);
            for c in "save as".chars() {
                PostMessageW(search, WM_CHAR, c as usize, 0);
            }
            PostMessageW(search, WM_KEYDOWN, usize::from(VK_DOWN), 0);
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_DELETE), 0);
            crate::window::settings_dialog::answer_in_loop(dialog, |dialog| {
                let (x, y) = crate::window::settings_dialog::page_row_point(dialog, 0);
                PostMessageW(dialog, WM_RBUTTONUP, 0, ((y as isize) << 16 | (x as isize & 0xffff)) as LPARAM);
                PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
            });
        });
        super::show_keyboard_shortcuts(window.hwnd);
        assert!(offered.borrow().iter().any(|label| label.starts_with("Reset")));
        assert!(!offered.borrow().iter().any(|label| label.starts_with("Remove")), "the row has no key to remove");
        assert!(!app_mut(window.hwnd).keymap.is_user(CommandId::SaveAs));
        super::save_settings_to(None);
        assert_eq!(std::fs::read_to_string(&ini).unwrap_or_default(), "");
    }

    #[test]
    fn recording_sees_f10_and_refuses_it() {
        // Break caught: F10 (a WM_SYSKEYDOWN) opening the dialog's system menu or beeping
        // instead of reaching the recording box.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_F10, VK_RETURN};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN, WM_SYSKEYDOWN};
        let window = ProductionWindow::new(make_app());
        let refusal = std::rc::Rc::new(std::cell::RefCell::new(None));
        let record = refusal.clone();
        crate::window::settings_dialog::answer_next(move |dialog| unsafe {
            let search = crate::window::settings_dialog::search_hwnd(dialog);
            PostMessageW(search, WM_KEYDOWN, usize::from(VK_DOWN), 0);
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RETURN), 0);
            PostMessageW(dialog, WM_SYSKEYDOWN, usize::from(VK_F10), 0);
            crate::window::settings_dialog::answer_in_loop(dialog, move |dialog| {
                *record.borrow_mut() = crate::window::settings_dialog::shortcuts_model(dialog)
                    .and_then(|model| model.recording)
                    .and_then(|recording| recording.refusal);
                PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
                PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
            });
        });
        super::show_keyboard_shortcuts(window.hwnd);
        assert_eq!(*refusal.borrow(), Some("F10 and Shift+F10 open the menus."));
    }
```

In `src/platform/clipboard.rs`:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn text_round_trips_through_the_clipboard() {
        // Break caught: Copy command ID putting nothing, or ANSI bytes, on the clipboard.
        use windows_sys::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
        use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
        use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
        super::set_text(std::ptr::null_mut(), "file.saveAs").unwrap();
        unsafe {
            assert_ne!(OpenClipboard(std::ptr::null_mut()), 0);
            let data = GetClipboardData(u32::from(CF_UNICODETEXT));
            let text = GlobalLock(data as _) as *const u16;
            let mut length = 0;
            while *text.add(length) != 0 {
                length += 1;
            }
            let read = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
            GlobalUnlock(data as _);
            CloseClipboard();
            assert_eq!(read, "file.saveAs");
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib platform::clipboard`
Expected: compile errors.

- [ ] **Step 3: Implement the clipboard**

Add `"Win32_System_DataExchange",` to the `windows-sys` features in `Cargo.toml` (after `"Win32_System_Com_StructuredStorage"`), `pub mod clipboard;` to `src/platform/mod.rs`, and above the tests in `clipboard.rs`:

```rust
//! Plain text onto the clipboard, for the Keyboard Shortcuts page's Copy command ID.

use crate::Result;
use crate::platform::{last_error, wide_null};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalFree, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

/// Replaces the clipboard's contents with `text`. `owner` may be null.
pub fn set_text(owner: HWND, text: &str) -> Result<()> {
    let wide = wide_null(text);
    if unsafe { OpenClipboard(owner) } == 0 {
        return Err(last_error());
    }
    let result = (|| unsafe {
        EmptyClipboard();
        let memory = GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of_val(wide.as_slice()));
        if memory.is_null() {
            return Err(last_error());
        }
        let target = GlobalLock(memory).cast::<u16>();
        if target.is_null() {
            GlobalFree(memory);
            return Err(last_error());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
        GlobalUnlock(memory);
        // The clipboard owns the memory once SetClipboardData succeeds.
        if SetClipboardData(u32::from(CF_UNICODETEXT), memory).is_null() {
            GlobalFree(memory);
            return Err(last_error());
        }
        Ok(())
    })();
    unsafe { CloseClipboard() };
    result
}
```

If `crate::platform::{last_error, wide_null}` isn't re-exported from `platform`, use `crate::platform::win32::{last_error, wide_null}`. Adjust `memory` casts if `SetClipboardData` expects `HANDLE` and `GlobalAlloc` returns `HGLOBAL` (both are `*mut c_void` in windows-sys 0.61; add `as _` where the compiler asks).

- [ ] **Step 4: Implement `track_choice` and the context menu**

In `menus.rs`, next to `track_popup`:

```rust
#[cfg(test)]
type ChoiceAnswer = Box<dyn FnOnce(&[(String, usize)]) -> Option<usize>>;

#[cfg(test)]
thread_local! {
    static CHOICE_ANSWERS: RefCell<std::collections::VecDeque<ChoiceAnswer>> =
        const { RefCell::new(std::collections::VecDeque::new()) };
}

/// Answers the next `track_choice` instead of showing a popup.
#[cfg(test)]
pub(crate) fn answer_next_choice(answer: impl FnOnce(&[(String, usize)]) -> Option<usize> + 'static) {
    CHOICE_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// A popup of `items` (label and nonzero id; an empty label is a separator) at `window`'s
/// client point `client`, for a window whose owner `owner` is disabled (a modal dialog's).
/// Returns the id picked.
pub(crate) fn track_choice(owner: HWND, window: HWND, items: &[(String, usize)], client: POINT) -> Option<usize> {
    let _modal = ModalScope::enter(owner);
    #[cfg(test)]
    if let Some(answer) = CHOICE_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return answer(items);
    }
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return None;
    }
    for (label, id) in items {
        if label.is_empty() {
            unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()) };
        } else {
            let label = wide_null(label);
            unsafe { AppendMenuW(menu, MF_STRING, *id, label.as_ptr()) };
        }
    }
    let mut point = client;
    unsafe { ClientToScreen(window, &mut point) };
    let selected = unsafe {
        TrackPopupMenuEx(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, point.x, point.y, window, std::ptr::null())
    };
    unsafe { DestroyMenu(menu) };
    usize::try_from(selected).ok().filter(|id| *id != 0)
}
```

In `settings_dialog.rs`, handle right-clicks on rows:

```rust
        windows_sys::Win32::UI::WindowsAndMessaging::WM_RBUTTONUP => {
            let (x, y) = lparam_point(lparam);
            let row = state(hwnd).and_then(|dialog| match hit_at(dialog, x, y) {
                Some(Hit::Page(super::shortcuts_page::PageHit::Row(index) | super::shortcuts_page::PageHit::Pencil(index))) => {
                    dialog.shortcuts.select(index);
                    dialog.model.focus = Focus::Table;
                    Some(index)
                }
                _ => None,
            });
            if row.is_some() {
                sync_focus(hwnd);
                invalidate(hwnd);
                context_menu(hwnd, POINT { x, y });
            }
            0
        }
```

```rust
const CHANGE: usize = 1;
const ADD: usize = 2;
const REMOVE: usize = 3;
const RESET: usize = 4;
const COPY_ID: usize = 5;

/// The selected row's menu (keyboard shortcuts spec §6.4).
fn context_menu(hwnd: HWND, at: POINT) {
    let Some((user, has_key)) = state(hwnd).and_then(|dialog| dialog.shortcuts.selected_row().map(|row| (row.user, row.stroke.is_some()))) else {
        return;
    };
    let mut items = vec![
        ("Change Keybinding\tEnter".to_owned(), CHANGE),
        ("Add Keybinding\tCtrl+Enter".to_owned(), ADD),
    ];
    if has_key {
        items.push(("Remove Keybinding\tDelete".to_owned(), REMOVE));
    }
    if user {
        items.push(("Reset Keybinding".to_owned(), RESET));
    }
    items.push((String::new(), 0));
    items.push(("Copy Command ID\tCtrl+C".to_owned(), COPY_ID));
    let choice = super::menus::track_choice(owner(hwnd), hwnd, &items, at);
    let effect = state(hwnd).map(|dialog| {
        let model = &mut dialog.shortcuts;
        match choice {
            Some(CHANGE) => model.start_change(),
            Some(ADD) => model.start_add(),
            Some(REMOVE) => model.remove(),
            Some(RESET) => model.reset(),
            Some(COPY_ID) => model.copy_id(),
            _ => super::shortcuts_model::ShortcutsEffect::None,
        }
    });
    if let Some(effect) = effect {
        run_shortcuts(hwnd, effect);
    }
}
```

In `run_shortcuts`, `CopyId(id)`:

```rust
        ShortcutsEffect::CopyId(id) => {
            if let Err(error) = crate::platform::clipboard::set_text(hwnd, id) {
                super::main_window::push_notice(owner(hwnd), format!("FastPad could not copy to the clipboard: {error}"));
            }
        }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo clippy --all-targets -- -D warnings`
Then: `cargo test --lib platform::clipboard window::menus -- --test-threads=1`
Then: `cargo test --lib main_window::tests::delete_unbinds main_window::tests::recording_sees_f10 -- --test-threads=1`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/platform src/window
git commit -m "feat: row context menu and Copy command ID on the Keyboard Shortcuts page"
```

---

### Task 11: Startup latency, full suite and the live check

**Files:** none (verification only), unless a check fails.

- [ ] **Step 1: Startup benchmark**

On the parent branch first, then on this one:

```powershell
git stash list   # must be empty; commit or stash nothing of this branch
git switch feat/settings-dialog
cargo run --release --bin fastpad-bench -- --runs 100 --warmup 10 --output benchmarks/settings-dialog.jsonl
git switch feat/keyboard-shortcuts
cargo run --release --bin fastpad-bench -- --runs 100 --warmup 10 --output benchmarks/keyboard-shortcuts.jsonl
cargo run --release --bin fastpad-bench -- compare benchmarks/settings-dialog.jsonl benchmarks/keyboard-shortcuts.jsonl
```

Expected: no material regression in any milestone's p50/p95. If there is one, find what runs before first input (`App::new` building `Keymap::defaults()` twice is the only new startup work; build it once and clone if it shows) and fix it before continuing.

- [ ] **Step 2: Full test suite**

Run: `cargo test -- --test-threads=1`
Expected: all pass. Fix any failure, re-run only the failing tests, then the full suite once more.

- [ ] **Step 3: Live check**

Back up `%LocalAppData%\FastPad\fastpad.ini`. Run `cargo run --release` and check:
- Ctrl+, → General → Ctrl+PageDown → Keyboard Shortcuts; the nav, search cue, table and keycaps read well in System light and dark and a Catppuccin theme.
- Rebind Save to F9: F9 saves, Ctrl+S doesn't, File menu shows `Save  F9`, the palette's File: Save row shows F9, `fastpad.ini` has `key.file.save=F9`.
- Record `A`: the refusal shows and Enter does nothing. Record `F3` for Save: the link says 1 existing command; clicking it lists Find next.
- Right-click → Reset: the line is gone and Ctrl+S saves again.
- Restart FastPad with a hand-written `key.file.save=A` line: a notification names it and `A` still types.

Restore `fastpad.ini`.

- [ ] **Step 4: Commit any fixes, then hand over**

If the checks needed changes, commit them (`fix: …`). Then use superpowers:finishing-a-development-branch (the PR targets `feat/settings-dialog` until that merges).
