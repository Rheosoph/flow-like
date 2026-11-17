//! The keycode that types a character in the current keyboard layout. macOS lets only the main
//! queue read the layout (Text Input Sources assert it and trap otherwise), so the lookup runs
//! there and the input worker presses the raw keycode. enigo looks the layout up on the calling
//! thread, which crashed the app on the first character key a bot pressed.

use super::main_queue_macos::on_main_queue;
use std::{ffi::c_void, time::Duration};

const MAIN_QUEUE_TIMEOUT: Duration = Duration::from_secs(2);
/// `modifierKeyState` without and with Shift, as enigo passes them.
const MODIFIER_STATES: [u32; 2] = [0x100, 0x2_0102];
const UC_KEY_ACTION_DISPLAY: u16 = 3;
const UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK: u32 = 1;
const KEYCODE_COUNT: u16 = 128;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    static kTISPropertyUnicodeKeyLayoutData: *const c_void;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *mut c_void;
    fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> *mut c_void;
    fn TISGetInputSourceProperty(source: *mut c_void, key: *const c_void) -> *const c_void;
    fn LMGetKbdType() -> u8;
    #[allow(clippy::too_many_arguments)]
    fn UCKeyTranslate(
        layout: *const u8,
        virtual_key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        unicode: *mut u16,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFRelease(value: *const c_void);
}

/// The current layout's key data with the input source that owns it; the caller releases it.
unsafe fn layout_source() -> Option<(*mut c_void, *const u8)> {
    for copy in [
        TISCopyCurrentKeyboardLayoutInputSource,
        TISCopyCurrentASCIICapableKeyboardLayoutInputSource,
    ] {
        let source = unsafe { copy() };
        if source.is_null() {
            continue;
        }
        let data = unsafe { TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) };
        if !data.is_null() {
            return Some((source, unsafe { CFDataGetBytePtr(data) }));
        }
        unsafe { CFRelease(source.cast_const()) };
    }
    None
}

/// What `keycode` types under `modifiers` in `layout`.
unsafe fn typed(layout: *const u8, keyboard_type: u32, keycode: u16, modifiers: u32) -> String {
    let mut dead_key_state = 0;
    let mut chars = [0u16; 4];
    let mut length = 0;
    let status = unsafe {
        UCKeyTranslate(
            layout,
            keycode,
            UC_KEY_ACTION_DISPLAY,
            modifiers,
            keyboard_type,
            UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK,
            &raw mut dead_key_state,
            chars.len(),
            &raw mut length,
            chars.as_mut_ptr(),
        )
    };
    if status != 0 {
        return String::new();
    }
    String::from_utf16_lossy(&chars[..length.min(chars.len())])
}

/// Must run on the main queue. Prefers a key without Shift, then the lowest keycode, so a digit
/// comes from the main row rather than the keypad.
fn layout_keycode(character: char) -> Option<u16> {
    let wanted = character.to_string();
    let (source, layout) = unsafe { layout_source() }?;
    let keyboard_type = u32::from(unsafe { LMGetKbdType() });
    let found = MODIFIER_STATES.into_iter().find_map(|modifiers| {
        (0..KEYCODE_COUNT).find(|&keycode| {
            let text = unsafe { typed(layout, keyboard_type, keycode, modifiers) };
            text == wanted
        })
    });
    unsafe { CFRelease(source.cast_const()) };
    found
}

/// The keycode that types `character` in the current layout, read on the main queue.
pub(crate) fn keycode_for(character: char) -> Result<u16, String> {
    on_main_queue(MAIN_QUEUE_TIMEOUT, move || layout_keycode(character))
        .ok_or_else(|| {
            format!(
                "Could not press '{character}': the keyboard layout is read on the app's main thread, which did not answer within {} s",
                MAIN_QUEUE_TIMEOUT.as_secs()
            )
        })?
        .ok_or_else(|| {
            format!("Could not press '{character}': no key of the current keyboard layout types it")
        })
}
