use super::main_queue_macos::on_main_queue;
use super::*;
use crate::computer::accessibility::AccessibilityBounds;
use std::ffi::{CString, c_char, c_void};
use std::time::Duration;
type Ref = *const c_void;
const OWN_APP_TIMEOUT: Duration = Duration::from_secs(10);

/// Accessibility calls on Flow Like's own windows never leave the process: AppKit answers them on
/// the calling thread and traps off the main thread (a bot minimizing a Flow Like window crashed
/// the app). Other apps answer over IPC from any thread.
fn in_app<T: Send + 'static>(
    pid: u32,
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    if pid != std::process::id() {
        return work();
    }
    on_main_queue(OWN_APP_TIMEOUT, work).unwrap_or_else(|| {
        Err(anyhow!(
            "Flow Like's main thread did not take the accessibility request for its own window within {} s",
            OWN_APP_TIMEOUT.as_secs()
        ))
    })
}
#[repr(C)]
#[derive(Default, Copy, Clone)]
struct Point {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Default, Copy, Clone)]
struct Size {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Default)]
struct Rect {
    origin: Point,
    size: Size,
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(options: u32, relative_to: u32) -> Ref;
    fn CGRectMakeWithDictionaryRepresentation(dictionary: Ref, rect: *mut Rect) -> bool;
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> Ref;
    fn AXUIElementCreateSystemWide() -> Ref;
    fn AXUIElementCopyAttributeValue(element: Ref, attribute: Ref, value: *mut Ref) -> i32;
    fn AXUIElementSetAttributeValue(element: Ref, attribute: Ref, value: Ref) -> i32;
    fn AXUIElementCopyActionNames(element: Ref, actions: *mut Ref) -> i32;
    fn AXUIElementPerformAction(element: Ref, action: Ref) -> i32;
    fn AXUIElementSetMessagingTimeout(element: Ref, timeout: f32) -> i32;
    fn AXValueGetValue(value: Ref, kind: i32, output: *mut c_void) -> bool;
    fn AXValueGetTypeID() -> usize;
    fn AXValueCreate(kind: i32, value: *const c_void) -> Ref;
    fn AXIsProcessTrusted() -> bool;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFRetain(value: Ref) -> Ref;
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFEqual(a: Ref, b: Ref) -> bool;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: Ref, kind: i32, value: *mut c_void) -> bool;
    fn CFStringGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFBooleanGetValue(value: Ref) -> bool;
    fn CFBooleanGetTypeID() -> usize;
    fn CFStringCreateWithCString(allocator: Ref, text: *const c_char, encoding: u32) -> Ref;
    fn CFStringGetLength(value: Ref) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(value: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> bool;
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
    static kCFBooleanTrue: Ref;
    static kCFBooleanFalse: Ref;
}
struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CFRelease(self.0);
            }
        }
    }
}
fn string(s: &str) -> Result<Owned> {
    let text = CString::new(s)?;
    let value = unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), 0x08000100) };
    if value.is_null() {
        return Err(anyhow!("Cannot encode accessibility value"));
    }
    Ok(Owned(value))
}
fn attr(e: Ref, key: &str) -> Option<Owned> {
    let key = string(key).ok()?;
    let mut value = std::ptr::null();
    if unsafe { AXUIElementCopyAttributeValue(e, key.0, &mut value) } == 0 && !value.is_null() {
        Some(Owned(value))
    } else {
        None
    }
}
fn text(value: Ref) -> Option<String> {
    if value.is_null() || unsafe { CFGetTypeID(value) != CFStringGetTypeID() } {
        return None;
    }
    let size =
        unsafe { CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), 0x08000100) } + 1;
    if !(1..=1_048_576).contains(&size) {
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    if !unsafe { CFStringGetCString(value, buffer.as_mut_ptr().cast(), size, 0x08000100) } {
        return None;
    }
    let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}
fn attribute_text(e: Ref, key: &str) -> Option<String> {
    attr(e, key).and_then(|v| text(v.0))
}
fn array(value: Ref) -> Vec<Owned> {
    if value.is_null() || unsafe { CFGetTypeID(value) != CFArrayGetTypeID() } {
        return vec![];
    }
    (0..unsafe { CFArrayGetCount(value) }.min(2000))
        .filter_map(|i| {
            let value = unsafe { CFArrayGetValueAtIndex(value, i) };
            if value.is_null() {
                None
            } else {
                Some(Owned(unsafe { CFRetain(value) }))
            }
        })
        .collect()
}
fn children(e: Ref) -> Vec<Owned> {
    attr(e, "AXChildren")
        .map(|a| array(a.0))
        .unwrap_or_default()
}

fn element_bounds(element: Ref) -> Option<(f64, f64, f64, f64)> {
    let position = attr(element, "AXPosition")?;
    let size = attr(element, "AXSize")?;
    let mut point = Point::default();
    let mut dimensions = Size::default();
    if unsafe {
        CFGetTypeID(position.0) == AXValueGetTypeID()
            && CFGetTypeID(size.0) == AXValueGetTypeID()
            && AXValueGetValue(position.0, 1, (&mut point as *mut Point).cast())
            && AXValueGetValue(size.0, 2, (&mut dimensions as *mut Size).cast())
    } {
        Some((point.x, point.y, dimensions.width, dimensions.height))
    } else {
        None
    }
}

fn boolean(element: Ref, name: &str) -> bool {
    attr(element, name).is_some_and(|value| unsafe {
        CFGetTypeID(value.0) == CFBooleanGetTypeID() && CFBooleanGetValue(value.0)
    })
}

fn dictionary_value(dictionary: Ref, name: &str) -> Option<Ref> {
    if dictionary.is_null() || unsafe { CFGetTypeID(dictionary) != CFDictionaryGetTypeID() } {
        return None;
    }
    let key = string(name).ok()?;
    let value = unsafe { CFDictionaryGetValue(dictionary, key.0) };
    (!value.is_null()).then_some(value)
}

fn dictionary_number(dictionary: Ref, name: &str) -> Option<i64> {
    let value = dictionary_value(dictionary, name)?;
    let mut number = 0i64;
    if unsafe {
        CFGetTypeID(value) == CFNumberGetTypeID()
            && CFNumberGetValue(value, 4, (&mut number as *mut i64).cast())
    } {
        Some(number)
    } else {
        None
    }
}

fn dictionary_f64(dictionary: Ref, name: &str) -> Option<f64> {
    let value = dictionary_value(dictionary, name)?;
    let mut number = 0f64;
    if unsafe {
        CFGetTypeID(value) == CFNumberGetTypeID()
            && CFNumberGetValue(value, 6, (&mut number as *mut f64).cast())
    } {
        Some(number)
    } else {
        None
    }
}

const SYSTEM_OWNERS: &[&str] = &[
    "Window Server",
    "Dock",
    "SystemUIServer",
    "Control Center",
    "Notification Center",
    "loginwindow",
    "TextInputMenuAgent",
];

fn stacked_entry(entry: Ref) -> Option<StackedWindow> {
    if dictionary_number(entry, "kCGWindowLayer") != Some(0)
        || dictionary_f64(entry, "kCGWindowAlpha").is_some_and(|alpha| alpha <= 0.0)
    {
        return None;
    }
    let id = u32::try_from(dictionary_number(entry, "kCGWindowNumber")?).ok()?;
    let pid = u32::try_from(dictionary_number(entry, "kCGWindowOwnerPID")?)
        .ok()
        .filter(|pid| *pid > 0)?;
    let app_name = dictionary_value(entry, "kCGWindowOwnerName")
        .and_then(text)
        .unwrap_or_default();
    if SYSTEM_OWNERS.contains(&app_name.as_str()) {
        return None;
    }
    let mut rect = Rect::default();
    let bounds = dictionary_value(entry, "kCGWindowBounds")?;
    if !unsafe { CGRectMakeWithDictionaryRepresentation(bounds, &mut rect) } {
        return None;
    }
    let (width, height) = (rect.size.width, rect.size.height);
    if !usable_size(width, height) {
        return None;
    }
    Some(StackedWindow {
        id,
        pid,
        app_name,
        title: dictionary_value(entry, "kCGWindowName")
            .and_then(text)
            .filter(|title| !title.is_empty()),
        x: rect.origin.x,
        y: rect.origin.y,
        width,
        height,
    })
}

fn on_screen_entries() -> Result<Owned> {
    // OnScreenOnly | ExcludeDesktopElements; the list is ordered front to back.
    let metadata = Owned(unsafe { CGWindowListCopyWindowInfo(1 | (1 << 4), 0) });
    if metadata.0.is_null() {
        return Err(anyhow!(
            "Window metadata is unavailable in this desktop session"
        ));
    }
    Ok(metadata)
}

pub fn stacked_windows() -> Result<Vec<StackedWindow>> {
    let metadata = on_screen_entries()?;
    Ok(array(metadata.0)
        .iter()
        .filter_map(|entry| stacked_entry(entry.0))
        .collect())
}

/// The window exists, on screen or not: another Space and the Dock (minimized) count too.
pub fn window_exists(id: u32) -> bool {
    // kCGWindowListOptionIncludingWindow lists only that window, wherever it is.
    let metadata = Owned(unsafe { CGWindowListCopyWindowInfo(1 << 3, id) });
    !metadata.0.is_null()
        && array(metadata.0)
            .iter()
            .any(|entry| dictionary_number(entry.0, "kCGWindowNumber") == Some(i64::from(id)))
}

/// Runs `work` on the element with keyboard focus. The frontmost window's app is asked where it
/// answers: Flow Like's own fields on the main queue, like its windows (see `in_app`).
fn with_focused<T: Send + 'static>(
    work: impl FnOnce(Ref) -> Result<T> + Send + 'static,
) -> Result<T> {
    let pid = stacked_windows()?.first().map_or(0, |window| window.pid);
    in_app(pid, move || {
        let system = Owned(unsafe { AXUIElementCreateSystemWide() });
        if system.0.is_null() {
            return Err(anyhow!("Accessibility is unavailable"));
        }
        let focused = attr(system.0, "AXFocusedUIElement")
            .ok_or_else(|| anyhow!("No element has keyboard focus"))?;
        work(focused.0)
    })
}

/// The focused text element's whole text, or its selected text; None when it holds no text.
pub fn focused_text(selection: bool) -> Option<String> {
    let key = if selection {
        "AXSelectedText"
    } else {
        "AXValue"
    };
    with_focused(move |element| Ok(attribute_text(element, key)))
        .ok()
        .flatten()
}

/// Replaces the focused text element's text. Fails for elements without text and for fields
/// that refuse it (secure fields), so the caller can paste instead.
pub fn set_focused_text(text: &str) -> Result<()> {
    let text = text.to_string();
    with_focused(move |element| {
        if attribute_text(element, "AXValue").is_none() {
            return Err(anyhow!("The focused element holds no text"));
        }
        let value = string(&text)?;
        set(element, "AXValue", value.0)
    })
}

pub fn window_title(id: u32) -> Option<String> {
    let entry = stacked_windows().ok()?.into_iter().find(|w| w.id == id)?;
    let accessible = NativeWindow {
        id: entry.id,
        pid: entry.pid,
        app_name: entry.app_name.clone(),
        cg_title: entry.title.clone(),
        title: None,
        bounds: (entry.x, entry.y, entry.width, entry.height),
        focused: false,
        minimized: false,
        onscreen: true,
        transparent: false,
    };
    in_app(accessible.pid, move || {
        Ok(window_element(&accessible)
            .ok()
            .and_then(|(_, element)| attribute_text(element.0, "AXTitle")))
    })
    .ok()
    .flatten()
    .filter(|title| !title.is_empty())
    .or(entry.title)
}

struct AccessibleState {
    title: Option<String>,
    minimized: bool,
    focused: bool,
}

fn accessible_state(window: &NativeWindow) -> Option<AccessibleState> {
    let (app, element) = window_element(window).ok()?;
    Some(AccessibleState {
        title: attribute_text(element.0, "AXTitle"),
        minimized: boolean(element.0, "AXMinimized"),
        focused: boolean(app.0, "AXFrontmost")
            && attr(app.0, "AXFocusedWindow")
                .is_some_and(|focused| unsafe { CFEqual(focused.0, element.0) }),
    })
}

/// Reads every window's accessible state; Flow Like's own windows in one main-queue hop.
fn accessible_states(windows: &[NativeWindow]) -> Vec<Option<AccessibleState>> {
    let own_pid = std::process::id();
    let own: Vec<NativeWindow> = windows
        .iter()
        .filter(|window| window.pid == own_pid)
        .cloned()
        .collect();
    let mut own_states = if own.is_empty() {
        Vec::new()
    } else {
        in_app(own_pid, move || {
            Ok(own.iter().map(accessible_state).collect())
        })
        .unwrap_or_default()
    }
    .into_iter();
    windows
        .iter()
        .map(|window| {
            if window.pid == own_pid {
                own_states.next().flatten()
            } else {
                accessible_state(window)
            }
        })
        .collect()
}

/// Window identity and geometry do not require screen capture. Titles and state come from AX.
#[derive(Clone)]
pub struct NativeWindow {
    id: u32,
    pid: u32,
    app_name: String,
    cg_title: Option<String>,
    title: Option<String>,
    bounds: (f64, f64, f64, f64),
    focused: bool,
    minimized: bool,
    onscreen: bool,
    transparent: bool,
}

impl NativeWindow {
    pub fn all() -> Result<Vec<Self>> {
        if !unsafe { AXIsProcessTrusted() } {
            return Err(anyhow!(
                "Accessibility permission is required to enumerate windows"
            ));
        }
        // OptionAll is zero. ExcludeDesktopElements keeps minimized and off-screen windows.
        let metadata = Owned(unsafe { CGWindowListCopyWindowInfo(1 << 4, 0) });
        if metadata.0.is_null() {
            return Err(anyhow!(
                "Window metadata is unavailable in this desktop session"
            ));
        }
        let mut windows = Vec::new();
        for entry in array(metadata.0) {
            if dictionary_number(entry.0, "kCGWindowLayer") != Some(0) {
                continue;
            }
            let Some(id) = dictionary_number(entry.0, "kCGWindowNumber")
                .and_then(|value| u32::try_from(value).ok())
            else {
                continue;
            };
            let Some(pid) = dictionary_number(entry.0, "kCGWindowOwnerPID")
                .and_then(|value| u32::try_from(value).ok())
                .filter(|pid| *pid > 0)
            else {
                continue;
            };
            let Some(bounds) = dictionary_value(entry.0, "kCGWindowBounds") else {
                continue;
            };
            let mut rect = Rect::default();
            if !unsafe { CGRectMakeWithDictionaryRepresentation(bounds, &mut rect) }
                || rect.size.width <= 0.0
                || rect.size.height <= 0.0
            {
                continue;
            }
            windows.push(Self {
                id,
                pid,
                app_name: dictionary_value(entry.0, "kCGWindowOwnerName")
                    .and_then(text)
                    .unwrap_or_default(),
                cg_title: dictionary_value(entry.0, "kCGWindowName").and_then(text),
                title: None,
                bounds: (
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                ),
                focused: false,
                minimized: false,
                transparent: dictionary_number(entry.0, "kCGWindowAlpha") == Some(0),
                onscreen: dictionary_value(entry.0, "kCGWindowIsOnscreen").is_some_and(
                    |value| unsafe {
                        CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value)
                    },
                ),
            });
        }
        let states = accessible_states(&windows);
        for (window, state) in windows.iter_mut().zip(states) {
            if let Some(state) = state {
                window.title = state.title;
                window.minimized = state.minimized;
                window.focused = state.focused;
            }
        }
        Ok(windows)
    }
    pub fn id(&self) -> Result<u32> {
        Ok(self.id)
    }
    pub fn pid(&self) -> Result<u32> {
        Ok(self.pid)
    }
    pub fn app_name(&self) -> Result<String> {
        Ok(self.app_name.clone())
    }
    pub fn title(&self) -> Result<String> {
        self.title.clone().ok_or_else(|| {
            anyhow!(
                "Window {} has no uniquely resolved accessibility title",
                self.id
            )
        })
    }
    pub fn x(&self) -> Result<i32> {
        Ok(self.bounds.0.round() as i32)
    }
    pub fn y(&self) -> Result<i32> {
        Ok(self.bounds.1.round() as i32)
    }
    pub fn width(&self) -> Result<u32> {
        Ok(self.bounds.2.round() as u32)
    }
    pub fn height(&self) -> Result<u32> {
        Ok(self.bounds.3.round() as u32)
    }
    pub fn is_focused(&self) -> Result<bool> {
        Ok(self.focused)
    }
    pub fn is_minimized(&self) -> Result<bool> {
        Ok(self.minimized)
    }
    pub fn is_onscreen(&self) -> bool {
        self.onscreen
    }
    /// Helper surfaces that are not windows a person sees: invisible, tiny, nameless, or owned
    /// by system UI processes.
    pub fn is_ghost(&self) -> bool {
        self.transparent
            || self.bounds.2 < 2.0
            || self.bounds.3 < 2.0
            || (self.app_name.is_empty() && self.cg_title.as_deref().is_none_or(str::is_empty))
            || SYSTEM_OWNERS.contains(&self.app_name.as_str())
    }
    pub fn current_monitor(&self) -> Result<xcap::Monitor> {
        let mut best = None;
        let mut overlap = -1.0f64;
        for monitor in xcap::Monitor::all()? {
            let (x, y) = (monitor.x()? as f64, monitor.y()? as f64);
            let area = ((x + monitor.width()? as f64).min(self.bounds.0 + self.bounds.2)
                - x.max(self.bounds.0))
            .max(0.0)
                * ((y + monitor.height()? as f64).min(self.bounds.1 + self.bounds.3)
                    - y.max(self.bounds.1))
                .max(0.0);
            if area > overlap {
                overlap = area;
                best = Some(monitor);
            }
        }
        best.ok_or_else(|| anyhow!("No display is available for this window"))
    }
}
fn set(e: Ref, key: &str, value: Ref) -> Result<()> {
    let key = string(key)?;
    let result = unsafe { AXUIElementSetAttributeValue(e, key.0, value) };
    if result != 0 {
        return Err(anyhow!("Accessibility attribute rejected: {}", result));
    }
    Ok(())
}
fn perform(e: Ref, action: &str) -> Result<()> {
    let action = string(action)?;
    let result = unsafe { AXUIElementPerformAction(e, action.0) };
    if result != 0 {
        return Err(anyhow!("Accessibility action rejected: {}", result));
    }
    Ok(())
}
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

/// Brings the owning app forward, which makes macOS switch to the Space of its windows.
fn activate_app(pid: u32) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send, rc::Retained};
    let app: Option<Retained<AnyObject>> = unsafe {
        msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid as i32]
    };
    if let Some(app) = app {
        // NSApplicationActivateAllWindows | NSApplicationActivateIgnoringOtherApps
        let _: bool = unsafe { msg_send![&*app, activateWithOptions: 3usize] };
    }
}

fn reachable(w: &NativeWindow) -> Result<()> {
    let probe = w.clone();
    in_app(w.pid, move || window_element(&probe).map(drop))
}

/// Waits off the main thread, so an activation of Flow Like itself can take effect meanwhile.
fn reveal_across_spaces(w: &NativeWindow) -> Result<()> {
    let first = match reachable(w) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if w.onscreen {
        return Err(first);
    }
    let pid = w.pid;
    in_app(pid, move || {
        activate_app(pid);
        Ok(())
    })?;
    for _ in 0..8 {
        std::thread::sleep(Duration::from_millis(250));
        if reachable(w).is_ok() {
            return Ok(());
        }
    }
    Err(anyhow!(
        "Window {} of '{}' is on another Space or hidden and macOS does not expose it to Flow Like from here ({first}). Ask the owner to switch to that Space (Mission Control or Control+Arrow), or work with a window on the current Space.",
        w.id,
        w.app_name
    ))
}

fn window_element(w: &NativeWindow) -> Result<(Owned, Owned)> {
    if !unsafe { AXIsProcessTrusted() } {
        return Err(anyhow!("Accessibility permission is required"));
    }
    let app = Owned(unsafe { AXUIElementCreateApplication(w.pid()? as i32) });
    if app.0.is_null() {
        return Err(anyhow!("Application is no longer available"));
    }
    unsafe {
        AXUIElementSetMessagingTimeout(app.0, 2.0);
    }
    let windows = attr(app.0, "AXWindows")
        .ok_or_else(|| anyhow!("Application does not expose accessible windows"))?;
    let all = array(windows.0);
    let mut matches: Vec<_> = all
        .into_iter()
        .filter(|element| {
            let Some(bounds) = element_bounds(element.0) else {
                return false;
            };
            (bounds.0 - w.bounds.0).abs() <= 1.0
                && (bounds.1 - w.bounds.1).abs() <= 1.0
                && (bounds.2 - w.bounds.2).abs() <= 1.0
                && (bounds.3 - w.bounds.3).abs() <= 1.0
        })
        .collect();
    if matches.len() > 1 {
        if let Some(title) = w.cg_title.as_deref() {
            matches
                .retain(|element| attribute_text(element.0, "AXTitle").as_deref() == Some(title));
        }
    }
    if matches.len() != 1 {
        return Err(anyhow!(
            "Cannot uniquely identify accessible window {}",
            w.id()?
        ));
    }
    Ok((app, matches.remove(0)))
}
fn node(e: Ref) -> AccessibilityNode {
    let mut position = Point::default();
    let mut size = Size::default();
    let bounds = match (attr(e, "AXPosition"), attr(e, "AXSize")) {
        (Some(p), Some(s))
            if unsafe {
                CFGetTypeID(p.0) == AXValueGetTypeID()
                    && CFGetTypeID(s.0) == AXValueGetTypeID()
                    && AXValueGetValue(p.0, 1, (&mut position as *mut Point).cast())
                    && AXValueGetValue(s.0, 2, (&mut size as *mut Size).cast())
            } =>
        {
            Some(AccessibilityBounds {
                x: position.x.round() as i32,
                y: position.y.round() as i32,
                width: size.width.round() as i32,
                height: size.height.round() as i32,
            })
        }
        _ => None,
    };
    let mut actions = std::ptr::null();
    let actions =
        if unsafe { AXUIElementCopyActionNames(e, &mut actions) } == 0 && !actions.is_null() {
            let actions = Owned(actions);
            array(actions.0).iter().filter_map(|a| text(a.0)).collect()
        } else {
            vec![]
        };
    let role = attribute_text(e, "AXRole").unwrap_or_default();
    let value = attr(e, "AXValue").and_then(|v| scalar_text(v.0));
    let mut states = Vec::new();
    match attr(e, "AXEnabled").and_then(|v| boolean_value(v.0)) {
        Some(true) => states.push("enabled".to_string()),
        Some(false) => states.push("disabled".to_string()),
        None => {}
    }
    for key in ["AXFocused", "AXSelected", "AXExpanded"] {
        if boolean(e, key) {
            states.push(key.trim_start_matches("AX").to_lowercase());
        }
    }
    if matches!(role.as_str(), "AXCheckBox" | "AXRadioButton") {
        match value.as_deref() {
            Some("1") => states.push("checked".to_string()),
            Some("2") => states.push("mixed".to_string()),
            _ => {}
        }
    }
    let non_empty = |key: &str| attribute_text(e, key).filter(|text| !text.trim().is_empty());
    AccessibilityNode {
        role,
        name: non_empty("AXTitle")
            .or_else(|| non_empty("AXDescription"))
            .or_else(|| non_empty("AXPlaceholderValue")),
        value,
        description: attribute_text(e, "AXHelp"),
        bounds,
        states,
        actions,
        ..Default::default()
    }
}
fn boolean_value(value: Ref) -> Option<bool> {
    (!value.is_null() && unsafe { CFGetTypeID(value) == CFBooleanGetTypeID() })
        .then(|| unsafe { CFBooleanGetValue(value) })
}
fn scalar_text(value: Ref) -> Option<String> {
    if let Some(text) = text(value) {
        return Some(text);
    }
    if let Some(flag) = boolean_value(value) {
        return Some(flag.to_string());
    }
    let mut number = 0f64;
    if unsafe {
        CFGetTypeID(value) == CFNumberGetTypeID()
            && CFNumberGetValue(value, 13, (&mut number as *mut f64).cast())
    } && number.is_finite()
    {
        return Some(if number.fract() == 0.0 && number.abs() < 1e15 {
            format!("{}", number as i64)
        } else {
            number.to_string()
        });
    }
    None
}
fn walk(e: Ref, id: &str, path: Vec<usize>, depth: usize, budget: &mut usize) -> AccessibilityNode {
    *budget = budget.saturating_sub(1);
    let mut result = node(e);
    identify(&mut result, id, path.clone());
    if depth > 0 {
        for (i, child) in children(e).iter().enumerate() {
            if *budget == 0 {
                break;
            }
            let mut next = path.clone();
            next.push(i);
            result
                .children
                .push(walk(child.0, id, next, depth - 1, budget));
        }
    }
    result
}
pub async fn tree(id: &str, depth: usize) -> Result<AccessibilityNode> {
    let id = id.to_owned();
    tokio::task::spawn_blocking(move || {
        let w = select_window(&id, "", "", true)?.ok_or_else(|| anyhow!("Window not found"))?;
        in_app(w.pid, move || {
            let (_app, element) = window_element(&w)?;
            Ok(walk(element.0, &w.id.to_string(), vec![], depth, &mut 2000))
        })
    })
    .await?
}
pub async fn action(
    locator: &ElementLocator,
    action: &str,
    value: &str,
    cancellation: Option<flow_like_types::tokio_util::sync::CancellationToken>,
) -> Result<()> {
    let locator = locator.clone();
    let action = action.to_owned();
    let value = value.to_owned();
    tokio::task::spawn_blocking(move || {
        let w = select_window(&locator.window_id, "", "", true)?
            .ok_or_else(|| anyhow!("Window not found"))?;
        in_app(w.pid, move || {
            act(&w, &locator, &action, &value, cancellation)
        })
    })
    .await?
}
fn act(
    w: &NativeWindow,
    locator: &ElementLocator,
    action: &str,
    value: &str,
    cancellation: Option<flow_like_types::tokio_util::sync::CancellationToken>,
) -> Result<()> {
    let (_app, mut element) = window_element(w)?;
    for i in &locator.path {
        element = children(element.0)
            .into_iter()
            .nth(*i)
            .ok_or_else(|| anyhow!("Element no longer exists"))?;
    }
    let current = node(element.0);
    verify(&current, locator)?;
    if cancellation
        .as_ref()
        .is_some_and(|token| token.is_cancelled())
    {
        return Err(anyhow!("Automation cancelled"));
    }
    match action {
        "invoke" => perform(
            element.0,
            ["AXPress", "AXConfirm", "AXOpen", "AXPick"]
                .into_iter()
                .find(|name| current.actions.iter().any(|a| a == name))
                .unwrap_or("AXPress"),
        ),
        "focus" => set(element.0, "AXFocused", unsafe { kCFBooleanTrue }),
        "select" => set(element.0, "AXSelected", unsafe { kCFBooleanTrue }),
        "expand" => set(element.0, "AXExpanded", unsafe { kCFBooleanTrue }),
        "collapse" => set(element.0, "AXExpanded", unsafe { kCFBooleanFalse }),
        "set_value" => {
            let value = string(value)?;
            set(element.0, "AXValue", value.0)
        }
        _ => Err(anyhow!("Unsupported accessibility action: {}", action)),
    }
}
pub fn operate_window(
    w: &NativeWindow,
    operation: &str,
    bounds: Option<(i32, i32, i32, i32)>,
) -> Result<()> {
    if operation == "focus" {
        reveal_across_spaces(w)?;
    }
    let window = w.clone();
    let operation = operation.to_owned();
    in_app(w.pid, move || operate(&window, &operation, bounds))
}
fn operate(w: &NativeWindow, operation: &str, bounds: Option<(i32, i32, i32, i32)>) -> Result<()> {
    let (app, window) = window_element(w)?;
    match operation {
        "focus" => {
            set(app.0, "AXFrontmost", unsafe { kCFBooleanTrue })?;
            if attr(window.0, "AXMinimized").is_some_and(|v| unsafe {
                CFGetTypeID(v.0) == CFBooleanGetTypeID() && CFBooleanGetValue(v.0)
            }) {
                set(window.0, "AXMinimized", unsafe { kCFBooleanFalse })?;
            }
            perform(window.0, "AXRaise")
        }
        "minimize" => set(window.0, "AXMinimized", unsafe { kCFBooleanTrue }),
        "restore" => set(window.0, "AXMinimized", unsafe { kCFBooleanFalse }),
        "close" => {
            let button = attr(window.0, "AXCloseButton")
                .ok_or_else(|| anyhow!("Window has no close button"))?;
            perform(button.0, "AXPress")
        }
        "maximize" | "move_resize" => {
            let (x, y, width, height) = if operation == "maximize" {
                let monitor = w.current_monitor()?;
                (
                    monitor.x()?,
                    monitor.y()?,
                    i32::try_from(monitor.width()?)?,
                    i32::try_from(monitor.height()?)?,
                )
            } else {
                bounds.ok_or_else(|| anyhow!("Window bounds are required"))?
            };
            let p = Point {
                x: x as f64,
                y: y as f64,
            };
            let s = Size {
                width: width as f64,
                height: height as f64,
            };
            let p = Owned(unsafe { AXValueCreate(1, (&p as *const Point).cast()) });
            let s = Owned(unsafe { AXValueCreate(2, (&s as *const Size).cast()) });
            set(window.0, "AXPosition", p.0)?;
            set(window.0, "AXSize", s.0)
        }
        _ => Err(anyhow!("Unknown window operation")),
    }
}
