// Derived from Chromium chrome/test/chromedriver @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE.
use super::us_layout::{KeyDefinition, US_KEYBOARD_LAYOUT};
use crate::error::BrowserError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct Modifiers(pub u8);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const ALT: Modifiers = Modifiers(1);
    pub const CTRL: Modifiers = Modifiers(2);
    pub const META: Modifiers = Modifiers(4);
    pub const SHIFT: Modifiers = Modifiers(8);

    pub fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: Modifiers) {
        self.0 |= other.0;
    }

    pub fn bits(self) -> u32 {
        u32::from(self.0)
    }

    fn remove(&mut self, other: Modifiers) {
        self.0 &= !other.0;
    }

    fn toggle(&mut self, other: Modifiers) {
        self.0 ^= other.0;
    }

    fn intersects(self, other: Modifiers) -> bool {
        self.0 & other.0 != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Named(NamedKey),
    Char(char),
}

impl NamedKey {
    pub fn webdriver_char(self) -> char {
        match self {
            NamedKey::Enter => '\u{E007}',
            NamedKey::Tab => '\u{E004}',
            NamedKey::Escape => '\u{E00C}',
            NamedKey::Backspace => '\u{E003}',
            NamedKey::Delete => '\u{E017}',
            NamedKey::ArrowUp => '\u{E013}',
            NamedKey::ArrowDown => '\u{E015}',
            NamedKey::ArrowLeft => '\u{E012}',
            NamedKey::ArrowRight => '\u{E014}',
            NamedKey::Home => '\u{E011}',
            NamedKey::End => '\u{E010}',
            NamedKey::PageUp => '\u{E00E}',
            NamedKey::PageDown => '\u{E00F}',
            NamedKey::Space => '\u{E00D}',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventKind {
    RawKeyDown,
    KeyDown,
    Char,
    KeyUp,
}

impl KeyEventKind {
    fn wire_name(self) -> &'static str {
        match self {
            KeyEventKind::RawKeyDown => "rawKeyDown",
            KeyEventKind::KeyDown => "keyDown",
            KeyEventKind::Char => "char",
            KeyEventKind::KeyUp => "keyUp",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    pub kind: KeyEventKind,
    pub modifiers: u32,
    pub text: String,
    pub unmodified_text: String,
    pub key: String,
    pub code: String,
    pub windows_virtual_key_code: i64,
    pub location: u8,
    pub is_keypad: bool,
    pub commands: Vec<&'static str>,
}

impl KeyEvent {
    pub fn to_params(&self) -> serde_json::Value {
        let mut params = serde_json::Map::new();
        params.insert("type".into(), self.kind.wire_name().into());
        params.insert("modifiers".into(), self.modifiers.into());
        params.insert("text".into(), self.text.clone().into());
        params.insert("unmodifiedText".into(), self.unmodified_text.clone().into());
        params.insert(
            "windowsVirtualKeyCode".into(),
            self.windows_virtual_key_code.into(),
        );
        if !self.code.is_empty() {
            params.insert("code".into(), self.code.clone().into());
        }
        if !self.key.is_empty() {
            params.insert("key".into(), self.key.clone().into());
        }
        if !self.commands.is_empty() {
            params.insert("commands".into(), self.commands.clone().into());
        }
        // CDP accepts location 1 (left) and 2 (right) only; the keypad is flagged with isKeypad.
        if self.is_keypad || self.location == KEYPAD_LOCATION {
            params.insert("isKeypad".into(), true.into());
        } else if self.location != 0 {
            params.insert("location".into(), self.location.into());
        }
        serde_json::Value::Object(params)
    }
}

pub fn convert_keys(
    text: &str,
    sticky: &mut Modifiers,
    release_modifiers: bool,
) -> crate::Result<Vec<KeyEvent>> {
    convert_utf16(text.encode_utf16(), sticky, release_modifiers)
}

pub fn chord_events(key: Key, modifiers: Modifiers) -> crate::Result<Vec<KeyEvent>> {
    let mut keys: String = MODIFIER_KEYS
        .iter()
        .filter(|modifier| modifiers.contains(modifier.mask))
        .map(|modifier| modifier.left)
        .collect();
    keys.push(match key {
        Key::Named(named) => named.webdriver_char(),
        Key::Char(character) => character,
    });
    keys.push(NULL_KEY);
    let mut sticky = Modifiers::NONE;
    convert_keys(&keys, &mut sticky, false)
}

pub fn modifier_events(modifiers: Modifiers, down: bool) -> Vec<KeyEvent> {
    let pressed = MODIFIER_KEYS
        .iter()
        .filter(|modifier| modifiers.contains(modifier.mask));
    if down {
        let mut held = Modifiers::NONE;
        pressed
            .map(|modifier| {
                held.insert(modifier.mask);
                key_event(KeyEventKind::RawKeyDown, modifier.key_code, held, "", "")
            })
            .collect()
    } else {
        let mut held = Modifiers(modifiers.0 & ALL_MODIFIERS.0);
        pressed
            .rev()
            .map(|modifier| {
                held.remove(modifier.mask);
                key_event(KeyEventKind::KeyUp, modifier.key_code, held, "", "")
            })
            .collect()
    }
}

pub(crate) fn editing_commands(code: &str, modifiers: u32) -> Vec<&'static str> {
    let command_key = if cfg!(target_os = "macos") {
        Modifiers::META
    } else {
        Modifiers::CTRL
    };
    if modifiers & command_key.bits() == 0 {
        return Vec::new();
    }
    let shift = modifiers & Modifiers::SHIFT.bits() != 0;
    let command = match code {
        "KeyA" => "SelectAll",
        "KeyC" => "Copy",
        "KeyX" => "Cut",
        "KeyY" => "Redo",
        "KeyV" if shift => "PasteAndMatchStyle",
        "KeyV" => "Paste",
        "KeyZ" if shift => "Redo",
        "KeyZ" => "Undo",
        _ => "",
    };
    vec![command]
}

mod vk {
    pub const UNKNOWN: u16 = 0x00;
    pub const CANCEL: u16 = 0x03;
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const CLEAR: u16 = 0x0C;
    pub const RETURN: u16 = 0x0D;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const PAUSE: u16 = 0x13;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;
    pub const PRIOR: u16 = 0x21;
    pub const NEXT: u16 = 0x22;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const INSERT: u16 = 0x2D;
    pub const DELETE: u16 = 0x2E;
    pub const HELP: u16 = 0x2F;
    pub const COMMAND: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
    pub const NUMPAD0: u16 = 0x60;
    pub const NUMPAD1: u16 = 0x61;
    pub const NUMPAD2: u16 = 0x62;
    pub const NUMPAD3: u16 = 0x63;
    pub const NUMPAD4: u16 = 0x64;
    pub const NUMPAD5: u16 = 0x65;
    pub const NUMPAD6: u16 = 0x66;
    pub const NUMPAD7: u16 = 0x67;
    pub const NUMPAD8: u16 = 0x68;
    pub const NUMPAD9: u16 = 0x69;
    pub const MULTIPLY: u16 = 0x6A;
    pub const ADD: u16 = 0x6B;
    pub const SUBTRACT: u16 = 0x6D;
    pub const DECIMAL: u16 = 0x6E;
    pub const DIVIDE: u16 = 0x6F;
    pub const F1: u16 = 0x70;
    pub const F2: u16 = 0x71;
    pub const F3: u16 = 0x72;
    pub const F4: u16 = 0x73;
    pub const F5: u16 = 0x74;
    pub const F6: u16 = 0x75;
    pub const F7: u16 = 0x76;
    pub const F8: u16 = 0x77;
    pub const F9: u16 = 0x78;
    pub const F10: u16 = 0x79;
    pub const F11: u16 = 0x7A;
    pub const F12: u16 = 0x7B;
    pub const RSHIFT: u16 = 0xA1;
    pub const RCONTROL: u16 = 0xA3;
    pub const RMENU: u16 = 0xA5;
    pub const OEM_1: u16 = 0xBA;
    pub const OEM_PLUS: u16 = 0xBB;
    pub const OEM_COMMA: u16 = 0xBC;
    pub const DBE_DBCSCHAR: u16 = 0xF4;
}

const NULL_KEY: char = '\u{E000}';
const WEBDRIVER_KEY_BASE: u32 = 0xE000;
const KEYPAD_LOCATION: u8 = 3;
const ALL_MODIFIERS: Modifiers =
    Modifiers(Modifiers::ALT.0 | Modifiers::CTRL.0 | Modifiers::META.0 | Modifiers::SHIFT.0);
const TEXT_SUPPRESSING_MODIFIERS: Modifiers =
    Modifiers(Modifiers::ALT.0 | Modifiers::CTRL.0 | Modifiers::META.0);

#[rustfmt::skip]
const SPECIAL_WEBDRIVER_KEYS: [u16; 94] = [
    vk::UNKNOWN, vk::CANCEL, vk::HELP, vk::BACK, vk::TAB, vk::CLEAR, vk::RETURN, vk::RETURN,
    vk::SHIFT, vk::CONTROL, vk::MENU, vk::PAUSE, vk::ESCAPE, vk::SPACE, vk::PRIOR, vk::NEXT,
    vk::END, vk::HOME, vk::LEFT, vk::UP, vk::RIGHT, vk::DOWN, vk::INSERT, vk::DELETE,
    vk::OEM_1, vk::OEM_PLUS, vk::NUMPAD0, vk::NUMPAD1, vk::NUMPAD2, vk::NUMPAD3, vk::NUMPAD4, vk::NUMPAD5,
    vk::NUMPAD6, vk::NUMPAD7, vk::NUMPAD8, vk::NUMPAD9, vk::MULTIPLY, vk::ADD, vk::OEM_COMMA, vk::SUBTRACT,
    vk::DECIMAL, vk::DIVIDE, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN,
    vk::UNKNOWN, vk::F1, vk::F2, vk::F3, vk::F4, vk::F5, vk::F6, vk::F7,
    vk::F8, vk::F9, vk::F10, vk::F11, vk::F12, vk::COMMAND, vk::UNKNOWN, vk::UNKNOWN,
    vk::DBE_DBCSCHAR, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN,
    vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN, vk::UNKNOWN,
    vk::RSHIFT, vk::RCONTROL, vk::RMENU, vk::RWIN, vk::PRIOR, vk::NEXT, vk::END, vk::HOME,
    vk::LEFT, vk::UP, vk::RIGHT, vk::DOWN, vk::INSERT, vk::DELETE,
];

struct ModifierKey {
    mask: Modifiers,
    left: char,
    right: char,
    key_code: u16,
}

const MODIFIER_KEYS: [ModifierKey; 4] = [
    ModifierKey {
        mask: Modifiers::SHIFT,
        left: '\u{E008}',
        right: '\u{E050}',
        key_code: vk::SHIFT,
    },
    ModifierKey {
        mask: Modifiers::CTRL,
        left: '\u{E009}',
        right: '\u{E051}',
        key_code: vk::CONTROL,
    },
    ModifierKey {
        mask: Modifiers::ALT,
        left: '\u{E00A}',
        right: '\u{E052}',
        key_code: vk::MENU,
    },
    ModifierKey {
        mask: Modifiers::META,
        left: '\u{E03D}',
        right: '\u{E053}',
        key_code: vk::COMMAND,
    },
];

struct Keystroke {
    key_code: u16,
    modifiers: Modifiers,
    text: String,
    unmodified_text: String,
}

struct Converter {
    sticky: Modifiers,
    events: Vec<KeyEvent>,
}

impl Converter {
    fn push(&mut self, key: char, index: usize) -> crate::Result<()> {
        if key == NULL_KEY {
            self.release_all();
        } else if let Some(modifier) = MODIFIER_KEYS
            .iter()
            .find(|modifier| modifier.left == key || modifier.right == key)
        {
            self.toggle(modifier);
        } else if let Some(stroke) = keystroke(key, self.sticky, index)? {
            self.press(&stroke);
        }
        Ok(())
    }

    fn release_all(&mut self) {
        for modifier in &MODIFIER_KEYS {
            if self.sticky.contains(modifier.mask) {
                self.events.push(key_event(
                    KeyEventKind::KeyUp,
                    modifier.key_code,
                    Modifiers::NONE,
                    "",
                    "",
                ));
            }
        }
        self.sticky = Modifiers::NONE;
    }

    fn toggle(&mut self, modifier: &ModifierKey) {
        self.sticky.toggle(modifier.mask);
        let kind = if self.sticky.contains(modifier.mask) {
            KeyEventKind::RawKeyDown
        } else {
            KeyEventKind::KeyUp
        };
        self.events
            .push(key_event(kind, modifier.key_code, self.sticky, "", ""));
    }

    fn press(&mut self, stroke: &Keystroke) {
        let wrap_shift =
            stroke.modifiers.contains(Modifiers::SHIFT) && !self.sticky.contains(Modifiers::SHIFT);
        if wrap_shift {
            self.events.push(key_event(
                KeyEventKind::RawKeyDown,
                vk::SHIFT,
                self.sticky,
                "",
                "",
            ));
        }
        let mut kinds = vec![KeyEventKind::RawKeyDown];
        if !stroke.text.is_empty() {
            kinds.push(KeyEventKind::Char);
        }
        kinds.push(KeyEventKind::KeyUp);
        for kind in kinds {
            self.events.push(key_event(
                kind,
                stroke.key_code,
                stroke.modifiers,
                &stroke.text,
                &stroke.unmodified_text,
            ));
        }
        if wrap_shift {
            self.events.push(key_event(
                KeyEventKind::KeyUp,
                vk::SHIFT,
                self.sticky,
                "",
                "",
            ));
        }
    }
}

fn convert_utf16(
    units: impl IntoIterator<Item = u16>,
    sticky: &mut Modifiers,
    release_modifiers: bool,
) -> crate::Result<Vec<KeyEvent>> {
    let mut converter = Converter {
        sticky: *sticky,
        events: Vec::new(),
    };
    let mut index = 0;
    for decoded in char::decode_utf16(units) {
        let key = decoded.map_err(|_| BrowserError::InvalidArgument {
            message: "invalid surrogate in keys".to_owned(),
        })?;
        converter.push(key, index)?;
        index += key.len_utf16();
    }
    if release_modifiers {
        converter.release_all();
    }
    *sticky = converter.sticky;
    Ok(converter.events)
}

fn keystroke(key: char, sticky: Modifiers, index: usize) -> crate::Result<Option<Keystroke>> {
    if key == '\r' {
        return Ok(None);
    }
    let special = special_key_code(key);
    let Some(key_code) = special.or_else(|| shorthand_key_code(key)) else {
        return Ok(Some(character_keystroke(key, sticky)));
    };
    if key_code == vk::UNKNOWN {
        return Err(BrowserError::InvalidArgument {
            message: format!(
                "unknown WebDriver key U+{:04X} at string index {index}",
                u32::from(key)
            ),
        });
    }
    let (text, unmodified_text) = if key_code == vk::RETURN {
        ("\r".to_owned(), "\r".to_owned())
    } else if special.is_some() && !is_special_key_printable(key_code) {
        (String::new(), String::new())
    } else {
        key_texts(key_code, sticky)
    };
    Ok(Some(Keystroke {
        key_code,
        modifiers: sticky,
        text,
        unmodified_text,
    }))
}

fn character_keystroke(key: char, sticky: Modifiers) -> Keystroke {
    let Some((key_code, needs_shift)) = key_code_for_char(key) else {
        return Keystroke {
            key_code: vk::UNKNOWN,
            modifiers: sticky,
            text: key.to_string(),
            unmodified_text: key.to_string(),
        };
    };
    let mut modifiers = sticky;
    if needs_shift {
        modifiers.insert(Modifiers::SHIFT);
    }
    let (text, unmodified_text) = key_texts(key_code, modifiers);
    Keystroke {
        key_code,
        modifiers,
        text,
        unmodified_text,
    }
}

fn special_key_code(key: char) -> Option<u16> {
    let index = u32::from(key).checked_sub(WEBDRIVER_KEY_BASE)?;
    SPECIAL_WEBDRIVER_KEYS.get(index as usize).copied()
}

fn shorthand_key_code(key: char) -> Option<u16> {
    match key {
        '\n' => Some(vk::RETURN),
        '\t' => Some(vk::TAB),
        '\u{8}' => Some(vk::BACK),
        ' ' => Some(vk::SPACE),
        _ => None,
    }
}

fn is_special_key_printable(key_code: u16) -> bool {
    matches!(
        key_code,
        vk::TAB | vk::SPACE | vk::OEM_1 | vk::OEM_PLUS | vk::OEM_COMMA | vk::NUMPAD0..=vk::DIVIDE
    )
}

fn key_texts(key_code: u16, modifiers: Modifiers) -> (String, String) {
    let text = key_code_text(key_code, modifiers);
    let unmodified_text = key_code_text(key_code, Modifiers::NONE);
    if text.is_empty() || unmodified_text.is_empty() {
        return (String::new(), String::new());
    }
    (text.to_owned(), unmodified_text.to_owned())
}

fn key_code_text(key_code: u16, modifiers: Modifiers) -> &'static str {
    if modifiers.intersects(TEXT_SUPPRESSING_MODIFIERS) {
        return "";
    }
    LayoutKey::find(key_code).map_or("", |layout| layout.text(modifiers))
}

fn key_code_for_char(key: char) -> Option<(u16, bool)> {
    let mut buffer = [0; 4];
    let wanted: &str = key.encode_utf8(&mut buffer);
    US_KEYBOARD_LAYOUT
        .iter()
        .filter(|definition| definition.location != Some(KEYPAD_LOCATION))
        .find_map(|definition| {
            let layout = LayoutKey {
                definition,
                keypad_digit: false,
            };
            if layout.text(Modifiers::NONE) == wanted {
                Some((definition.key_code, false))
            } else if layout.text(Modifiers::SHIFT) == wanted {
                Some((definition.key_code, true))
            } else {
                None
            }
        })
}

#[derive(Clone, Copy)]
struct LayoutKey {
    definition: &'static KeyDefinition,
    keypad_digit: bool,
}

impl LayoutKey {
    fn find(key_code: u16) -> Option<Self> {
        let located = US_KEYBOARD_LAYOUT.iter().find(|definition| {
            definition.key_code == key_code
                || definition.key_code_without_location == Some(key_code)
        });
        if let Some(definition) = located {
            return Some(Self {
                definition,
                keypad_digit: false,
            });
        }
        US_KEYBOARD_LAYOUT
            .iter()
            .find(|definition| definition.shift_key_code == Some(key_code))
            .map(|definition| Self {
                definition,
                keypad_digit: true,
            })
    }

    fn key(self, modifiers: Modifiers) -> &'static str {
        let shifted = self.keypad_digit || modifiers.contains(Modifiers::SHIFT);
        match self.definition.shift_key {
            Some(shift_key) if shifted => shift_key,
            _ => self.definition.key,
        }
    }

    fn text(self, modifiers: Modifiers) -> &'static str {
        let key = self.key(modifiers);
        if key.chars().count() == 1 {
            key
        } else {
            self.definition.text.unwrap_or("")
        }
    }
}

fn key_event(
    kind: KeyEventKind,
    key_code: u16,
    modifiers: Modifiers,
    text: &str,
    unmodified_text: &str,
) -> KeyEvent {
    let layout = LayoutKey::find(key_code);
    let code = layout.map_or("", |layout| layout.definition.code);
    let commands = if kind == KeyEventKind::RawKeyDown {
        editing_commands(code, modifiers.bits())
    } else {
        Vec::new()
    };
    KeyEvent {
        kind,
        modifiers: modifiers.bits(),
        text: text.to_owned(),
        unmodified_text: unmodified_text.to_owned(),
        key: layout.map_or("", |layout| layout.key(modifiers)).to_owned(),
        code: code.to_owned(),
        windows_virtual_key_code: i64::from(key_code),
        location: 0,
        is_keypad: false,
        commands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(events: &[KeyEvent]) -> Vec<&'static str> {
        events.iter().map(|event| event.kind.wire_name()).collect()
    }

    fn typed(text: &str) -> Vec<KeyEvent> {
        convert_keys(text, &mut Modifiers::default(), true).expect("keys convert")
    }

    fn platform_modifiers() -> (Modifiers, Modifiers) {
        if cfg!(target_os = "macos") {
            (Modifiers::META, Modifiers::CTRL)
        } else {
            (Modifiers::CTRL, Modifiers::META)
        }
    }

    #[test]
    fn mixed_unicode_uses_layout_keys_and_falls_back_to_vk_zero() {
        let events = typed("aä😀");
        assert_eq!(kinds(&events), ["rawKeyDown", "char", "keyUp"].repeat(3));
        assert_eq!(
            (events[0].key.as_str(), events[0].code.as_str()),
            ("a", "KeyA")
        );
        assert_eq!(events[0].windows_virtual_key_code, 65);
        for (event, text) in events[3..].iter().zip(["ä", "ä", "ä", "😀", "😀", "😀"]) {
            assert_eq!(event.windows_virtual_key_code, 0);
            assert_eq!(
                (event.text.as_str(), event.unmodified_text.as_str()),
                (text, text)
            );
            assert!(event.key.is_empty() && event.code.is_empty());
        }
        let params = events[6].to_params();
        assert!(params.get("key").is_none() && params.get("code").is_none());
    }

    #[test]
    fn sticky_shift_uppercases_until_released() {
        let events = typed("\u{E008}ab");
        assert_eq!(
            kinds(&events),
            [
                "rawKeyDown",
                "rawKeyDown",
                "char",
                "keyUp",
                "rawKeyDown",
                "char",
                "keyUp",
                "keyUp"
            ]
        );
        assert_eq!(events[0].code, "ShiftLeft");
        assert_eq!(events[0].modifiers, 8);
        assert_eq!(
            (
                events[2].key.as_str(),
                events[2].text.as_str(),
                events[2].unmodified_text.as_str()
            ),
            ("A", "A", "a")
        );
        assert_eq!(events[5].text, "B");
        assert_eq!(
            (events[7].code.as_str(), events[7].modifiers),
            ("ShiftLeft", 0)
        );

        let mut sticky = Modifiers::NONE;
        let held = convert_keys("\u{E008}a", &mut sticky, false).expect("keys convert");
        assert_eq!(sticky, Modifiers::SHIFT);
        assert_eq!(kinds(&held), ["rawKeyDown", "rawKeyDown", "char", "keyUp"]);
    }

    #[test]
    fn shifted_characters_are_wrapped_in_shift() {
        let events = typed("A!");
        assert_eq!(
            kinds(&events),
            ["rawKeyDown", "rawKeyDown", "char", "keyUp", "keyUp"].repeat(2)
        );
        assert_eq!((events[0].key.as_str(), events[0].modifiers), ("Shift", 0));
        assert_eq!(
            (events[6].key.as_str(), events[6].code.as_str()),
            ("!", "Digit1")
        );
        assert_eq!(
            (events[6].unmodified_text.as_str(), events[6].modifiers),
            ("1", 8)
        );
    }

    #[test]
    fn backspace_shorthand_sends_no_char() {
        let events = typed("ab\u{8}c");
        assert_eq!(events.len(), 11);
        assert_eq!(kinds(&events[6..8]), ["rawKeyDown", "keyUp"]);
        assert_eq!(
            (events[6].key.as_str(), events[6].windows_virtual_key_code),
            ("Backspace", 8)
        );
        assert!(events[6].text.is_empty());
    }

    #[test]
    fn newline_presses_enter_with_carriage_return() {
        let events = typed("a\nb");
        assert_eq!(events.len(), 9);
        for event in &events[3..6] {
            assert_eq!(
                (event.key.as_str(), event.code.as_str()),
                ("Enter", "Enter")
            );
            assert_eq!(
                (event.text.as_str(), event.windows_virtual_key_code),
                ("\r", 13)
            );
        }
    }

    #[test]
    fn carriage_return_is_dropped() {
        let events = typed("a\rb");
        assert_eq!(events.len(), 6);
        assert_eq!(events[3].code, "KeyB");
    }

    #[test]
    fn lone_surrogates_are_invalid_arguments() {
        for units in [vec![0x61, 0xD800, 0x62], vec![0xDC00], vec![0x61, 0xD83D]] {
            let error =
                convert_utf16(units, &mut Modifiers::default(), true).expect_err("lone surrogate");
            assert!(
                matches!(&error, BrowserError::InvalidArgument { message } if message == "invalid surrogate in keys"),
                "{error}"
            );
        }
    }

    #[test]
    fn null_key_releases_sticky_modifiers() {
        let events = typed("\u{E009}\u{E008}\u{E000}x");
        assert_eq!(
            kinds(&events),
            [
                "rawKeyDown",
                "rawKeyDown",
                "keyUp",
                "keyUp",
                "rawKeyDown",
                "char",
                "keyUp"
            ]
        );
        assert_eq!(
            (events[1].modifiers, events[1].code.as_str()),
            (10, "ShiftLeft")
        );
        assert_eq!(
            (events[2].code.as_str(), events[2].modifiers),
            ("ShiftLeft", 0)
        );
        assert_eq!(
            (events[3].code.as_str(), events[3].modifiers),
            ("ControlLeft", 0)
        );
        assert_eq!((events[4].modifiers, events[5].text.as_str()), (0, "x"));
    }

    #[test]
    fn unknown_webdriver_key_is_an_invalid_argument() {
        let error =
            convert_keys("a\u{E02A}", &mut Modifiers::default(), true).expect_err("unknown key");
        assert!(
            error.to_string().contains("U+E02A at string index 1"),
            "{error}"
        );
    }

    #[test]
    fn webdriver_keys_map_to_their_keys() {
        let numpad = typed("\u{E01A}");
        assert_eq!(kinds(&numpad), ["rawKeyDown", "char", "keyUp"]);
        assert_eq!(
            (
                numpad[0].code.as_str(),
                numpad[0].key.as_str(),
                numpad[1].text.as_str()
            ),
            ("Numpad0", "0", "0")
        );
        assert_eq!(numpad[0].windows_virtual_key_code, 0x60);

        let delete = typed("\u{E017}");
        assert_eq!(kinds(&delete), ["rawKeyDown", "keyUp"]);
        assert_eq!(
            (delete[0].code.as_str(), delete[0].windows_virtual_key_code),
            ("Delete", 46)
        );

        let right_shift = typed("\u{E050}a");
        assert_eq!(
            (right_shift[0].code.as_str(), right_shift[1].text.as_str()),
            ("ShiftLeft", "A")
        );
    }

    #[test]
    fn named_keys_use_webdriver_code_points() {
        let expected = [
            (NamedKey::Enter, "Enter", 13),
            (NamedKey::Tab, "Tab", 9),
            (NamedKey::Escape, "Escape", 27),
            (NamedKey::Backspace, "Backspace", 8),
            (NamedKey::Delete, "Delete", 46),
            (NamedKey::ArrowUp, "ArrowUp", 38),
            (NamedKey::ArrowDown, "ArrowDown", 40),
            (NamedKey::ArrowLeft, "ArrowLeft", 37),
            (NamedKey::ArrowRight, "ArrowRight", 39),
            (NamedKey::Home, "Home", 36),
            (NamedKey::End, "End", 35),
            (NamedKey::PageUp, "PageUp", 33),
            (NamedKey::PageDown, "PageDown", 34),
            (NamedKey::Space, "Space", 32),
        ];
        for (named, code, key_code) in expected {
            let events = chord_events(Key::Named(named), Modifiers::NONE).expect("chord");
            assert_eq!(events[0].code, code, "{named:?}");
            assert_eq!(events[0].windows_virtual_key_code, key_code, "{named:?}");
        }
        let space = chord_events(Key::Named(NamedKey::Space), Modifiers::NONE).expect("chord");
        assert_eq!(
            (space[1].kind, space[1].text.as_str()),
            (KeyEventKind::Char, " ")
        );
    }

    #[test]
    fn editing_commands_follow_the_platform_command_key() {
        let (command, other) = platform_modifiers();
        let shift = command.bits() | Modifiers::SHIFT.bits();
        let expected = [
            ("KeyA", command.bits(), vec!["SelectAll"]),
            ("KeyC", command.bits(), vec!["Copy"]),
            ("KeyX", command.bits(), vec!["Cut"]),
            ("KeyY", command.bits(), vec!["Redo"]),
            ("KeyV", command.bits(), vec!["Paste"]),
            ("KeyV", shift, vec!["PasteAndMatchStyle"]),
            ("KeyZ", command.bits(), vec!["Undo"]),
            ("KeyZ", shift, vec!["Redo"]),
            ("KeyB", command.bits(), vec![""]),
            ("KeyA", other.bits(), vec![]),
            ("KeyA", Modifiers::NONE.bits(), vec![]),
        ];
        for (code, modifiers, commands) in expected {
            assert_eq!(
                editing_commands(code, modifiers),
                commands,
                "{code} {modifiers}"
            );
        }
    }

    #[test]
    fn chords_attach_commands_to_raw_key_down_only() {
        let (command, other) = platform_modifiers();
        let events = chord_events(Key::Char('a'), command).expect("chord");
        assert_eq!(
            kinds(&events),
            ["rawKeyDown", "rawKeyDown", "keyUp", "keyUp"]
        );
        assert_eq!(events[0].commands, [""]);
        assert_eq!(events[1].commands, ["SelectAll"]);
        assert!(events[1].text.is_empty() && events[2].commands.is_empty());
        assert_eq!(events[3].modifiers, 0);
        assert_eq!(
            events[1].to_params()["commands"],
            serde_json::json!(["SelectAll"])
        );

        let plain = chord_events(Key::Char('a'), other).expect("chord");
        assert!(plain.iter().all(|event| event.commands.is_empty()));
        assert!(plain.iter().all(|event| event.kind != KeyEventKind::Char));
    }

    #[test]
    fn chords_press_modifiers_in_order_and_release_them_all() {
        let mut modifiers = Modifiers::CTRL;
        modifiers.insert(Modifiers::SHIFT);
        let events = chord_events(Key::Char('z'), modifiers).expect("chord");
        let codes: Vec<&str> = events.iter().map(|event| event.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "ShiftLeft",
                "ControlLeft",
                "KeyZ",
                "KeyZ",
                "ShiftLeft",
                "ControlLeft"
            ]
        );
        assert_eq!(events[2].modifiers, 10);
        assert_eq!(events[2].key, "Z");
    }

    #[test]
    fn modifier_events_nest_down_and_up() {
        let mut modifiers = Modifiers::ALT;
        modifiers.insert(Modifiers::SHIFT);
        let down = modifier_events(modifiers, true);
        let up = modifier_events(modifiers, false);
        let summary = |events: &[KeyEvent]| {
            events
                .iter()
                .map(|event| (event.kind.wire_name(), event.key.clone(), event.modifiers))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            summary(&down),
            [
                ("rawKeyDown", "Shift".to_owned(), 8),
                ("rawKeyDown", "Alt".to_owned(), 9)
            ]
        );
        assert_eq!(
            summary(&up),
            [
                ("keyUp", "Alt".to_owned(), 8),
                ("keyUp", "Shift".to_owned(), 0)
            ]
        );
        assert_eq!(modifier_events(Modifiers::META, true)[0].code, "MetaLeft");
        assert!(modifier_events(Modifiers::NONE, false).is_empty());
    }

    #[test]
    fn to_params_sends_chromedriver_fields() {
        let event = &typed("a")[0];
        assert_eq!(
            event.to_params(),
            serde_json::json!({
                "type": "rawKeyDown",
                "modifiers": 0,
                "text": "a",
                "unmodifiedText": "a",
                "windowsVirtualKeyCode": 65,
                "code": "KeyA",
                "key": "a",
            })
        );
        let mut keypad = event.clone();
        keypad.location = KEYPAD_LOCATION;
        assert_eq!(keypad.to_params()["isKeypad"], true);
        assert!(keypad.to_params().get("location").is_none());
        keypad.location = 2;
        assert_eq!(keypad.to_params()["location"], 2);
    }

    #[test]
    fn vendored_table_has_111_us_codes() {
        let mut codes: Vec<&str> = US_KEYBOARD_LAYOUT
            .iter()
            .map(|definition| definition.code)
            .collect();
        assert_eq!(codes.len(), 111);
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 111);
    }

    #[test]
    fn printable_characters_round_trip_through_the_layout() {
        for definition in US_KEYBOARD_LAYOUT
            .iter()
            .filter(|definition| definition.location != Some(KEYPAD_LOCATION))
        {
            let layout = LayoutKey {
                definition,
                keypad_digit: false,
            };
            for shift in [Modifiers::NONE, Modifiers::SHIFT] {
                let text = layout.text(shift);
                let Some(character) = text.chars().next().filter(|c| *c != '\r') else {
                    continue;
                };
                let (key_code, needs_shift) = key_code_for_char(character).expect("mapped");
                assert_eq!(key_code, definition.key_code, "{text:?}");
                assert_eq!(
                    needs_shift,
                    shift == Modifiers::SHIFT && layout.text(Modifiers::NONE) != text
                );
                assert_eq!(
                    LayoutKey::find(key_code).map(|found| found.definition.code),
                    Some(definition.code)
                );
            }
        }
    }

    fn same_on_german_and_us_layouts(code: &str, shifted: bool) -> bool {
        match code {
            "Space" | "Enter" | "Tab" | "Backspace" | "ShiftLeft" => true,
            "KeyY" | "KeyZ" => false,
            "Period" => !shifted,
            _ => code.starts_with("Key") || (code.starts_with("Digit") && !shifted),
        }
    }

    fn assert_matches_golden(name: &str, actual: &KeyEvent, expected: &serde_json::Value) {
        let params = actual.to_params();
        for field in ["type", "modifiers", "text", "unmodifiedText"] {
            assert_eq!(params[field], expected[field], "{name}: {field}");
        }
        let shifted = expected["modifiers"].as_u64().unwrap_or(0) & 8 != 0;
        let code = expected["code"].as_str().unwrap_or("");
        if expected["windowsVirtualKeyCode"] == 0 || same_on_german_and_us_layouts(code, shifted) {
            for field in ["key", "code", "windowsVirtualKeyCode"] {
                assert_eq!(params.get(field), expected.get(field), "{name}: {field}");
            }
        }
    }

    #[test]
    fn chromedriver_golden_values_and_event_types() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/parity/golden/c_input.json"
        );
        let golden: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("golden readable"))
                .expect("golden is JSON");
        let mut compared = 0;
        for row in golden["rows"].as_array().expect("rows") {
            let name = row["case"]["name"].as_str().unwrap_or_default();
            let (Some(text), Some(expected)) = (
                row["case"]["text"].as_str(),
                row["wd"]["keyEvents"].as_array(),
            ) else {
                continue;
            };
            if expected.is_empty() {
                continue;
            }
            let actual = typed(text);
            assert_eq!(actual.len(), expected.len(), "{name}: event count");
            for (event, expected) in actual.iter().zip(expected) {
                assert_matches_golden(name, event, expected);
            }
            compared += 1;
        }
        assert_eq!(compared, 24);
    }
}
