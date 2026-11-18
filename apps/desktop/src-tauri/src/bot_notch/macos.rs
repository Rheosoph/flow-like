use super::layout::{PlatformMetrics, hardware_notch};
use block2::RcBlock;
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, ProtocolObject, Sel};
use objc2::{class, msg_send, sel};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSString};
use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;
use std::sync::OnceLock;

const STATUS_WINDOW_LEVEL: isize = 25;
const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
const STATIONARY: usize = 1 << 4;
const IGNORES_CYCLE: usize = 1 << 6;
const FULL_SCREEN_AUXILIARY: usize = 1 << 8;
const NONACTIVATING_PANEL: usize = 1 << 7;
/// The window server's type of a Space that holds a fullscreen window or a split view.
const FULLSCREEN_SPACE: i32 = 4;
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> *const c_void;
    fn CFUUIDCreateString(allocator: *const c_void, uuid: *const c_void) -> *const c_void;
    fn CFRelease(object: *const c_void);
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

thread_local! {
    static SPACE_OBSERVER: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>> =
        const { RefCell::new(None) };
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Size {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    origin: Point,
    size: Size,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct EdgeInsets {
    top: f64,
    left: f64,
    bottom: f64,
    right: f64,
}

// SAFETY: the layouts match CGPoint, CGSize, CGRect and NSEdgeInsets (all f64 fields).
unsafe impl Encode for Point {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl RefEncode for Point {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}
unsafe impl Encode for Size {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl RefEncode for Size {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}
unsafe impl Encode for Rect {
    const ENCODING: Encoding = Encoding::Struct(
        "CGRect",
        &[<Point as Encode>::ENCODING, <Size as Encode>::ENCODING],
    );
}
unsafe impl RefEncode for Rect {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}
unsafe impl Encode for EdgeInsets {
    const ENCODING: Encoding = Encoding::Struct(
        "NSEdgeInsets",
        &[f64::ENCODING, f64::ENCODING, f64::ENCODING, f64::ENCODING],
    );
}
unsafe impl RefEncode for EdgeInsets {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}

fn ns_window(window: &tauri::WebviewWindow) -> tauri::Result<*mut AnyObject> {
    window
        .ns_window()
        .map(|pointer| pointer.cast::<AnyObject>())
}

extern "C-unwind" fn takes_key(_: &AnyObject, _: Sel) -> Bool {
    Bool::YES
}

extern "C-unwind" fn never_main(_: &AnyObject, _: Sel) -> Bool {
    Bool::NO
}

/// An `NSPanel` that takes the keyboard for the prompt. It adds no storage of its own.
fn panel_class() -> Option<&'static AnyClass> {
    static CLASS: OnceLock<Option<&'static AnyClass>> = OnceLock::new();
    *CLASS.get_or_init(|| {
        let mut builder = ClassBuilder::new(c"FlowLikeBotNotchPanel", class!(NSPanel))?;
        // SAFETY: both selectors take no arguments and return BOOL.
        unsafe {
            builder.add_method(
                sel!(canBecomeKeyWindow),
                takes_key as extern "C-unwind" fn(_, _) -> _,
            );
            builder.add_method(
                sel!(canBecomeMainWindow),
                never_main as extern "C-unwind" fn(_, _) -> _,
            );
        }
        Some(builder.register())
    })
}

/// Must run on the main thread. Turns the window into a non-activating panel: macOS leaves a
/// regular app's plain window out of every fullscreen Space, whatever its collection behavior,
/// and a click on one would activate Flow-Like and leave that Space.
fn become_panel(ns_window: &AnyObject) {
    let Some(panel) = panel_class() else {
        tracing::warn!("Bot notch: the panel class is unavailable; fullscreen Spaces hide it");
        return;
    };
    let current = ns_window.class();
    if std::ptr::eq(current, panel) {
        return;
    }
    if panel.instance_size() > current.instance_size() {
        tracing::warn!(
            panel = panel.instance_size(),
            window = current.instance_size(),
            "Bot notch: the panel class outgrew the window; fullscreen Spaces hide it"
        );
        return;
    }
    // SAFETY: the panel class derives from NSWindow like the window's own class and needs no more
    // storage than the window was allocated with. `_setPreventsActivation:` is what a panel
    // created with this mask tells the window server; AppKit skips it for a mask set later.
    unsafe {
        objc2::ffi::object_setClass(std::ptr::from_ref(ns_window).cast_mut(), panel);
        let mask: usize = msg_send![ns_window, styleMask];
        let _: () = msg_send![ns_window, setStyleMask: mask | NONACTIVATING_PANEL];
        let _: () = msg_send![ns_window, setHidesOnDeactivate: false];
        if responds(ns_window, sel!(_setPreventsActivation:)) {
            let _: () = msg_send![ns_window, _setPreventsActivation: true];
        }
    }
}

/// Must run on the main thread. Level 25 sits above the menu bar; tao would reset it to floating
/// if `set_always_on_top` ran afterwards.
pub(super) fn configure_window(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let pointer = ns_window(window)?;
    // SAFETY: Tauri returns the live NSWindow of `window`; this runs on the main thread.
    let Some(ns_window) = (unsafe { pointer.as_ref() }) else {
        return Ok(());
    };
    become_panel(ns_window);
    unsafe {
        let _: () = msg_send![ns_window, setLevel: STATUS_WINDOW_LEVEL];
        let _: () = msg_send![
            ns_window,
            setCollectionBehavior: CAN_JOIN_ALL_SPACES | STATIONARY | IGNORES_CYCLE | FULL_SCREEN_AUXILIARY
        ];
        let _: () = msg_send![ns_window, setHasShadow: false];
        let _: () = msg_send![ns_window, setIgnoresMouseEvents: true];
    }
    Ok(())
}

/// Must run on the main thread. Orders the window front without activating the app.
pub(super) fn show_without_activating(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    configure_window(window)?;
    let pointer = ns_window(window)?;
    // SAFETY: as in `configure_window`.
    if let Some(ns_window) = unsafe { pointer.as_ref() } {
        unsafe {
            let _: () = msg_send![ns_window, orderFrontRegardless];
        }
    }
    Ok(())
}

fn responds(object: &AnyObject, selector: Sel) -> bool {
    unsafe { msg_send![object, respondsToSelector: selector] }
}

/// Must run on the main thread.
pub(super) fn screen_metrics() -> PlatformMetrics {
    // SAFETY: AppKit calls on the main thread with objects owned by the screen list.
    unsafe {
        let screens: Option<Retained<AnyObject>> = msg_send![class!(NSScreen), screens];
        let Some(screens) = screens else {
            return PlatformMetrics::default();
        };
        let count: usize = msg_send![&*screens, count];
        if count == 0 {
            return PlatformMetrics::default();
        }
        let screen: Option<Retained<AnyObject>> = msg_send![&*screens, objectAtIndex: 0usize];
        let Some(screen) = screen else {
            return PlatformMetrics::default();
        };
        metrics_of(&screen)
    }
}

unsafe fn metrics_of(screen: &AnyObject) -> PlatformMetrics {
    let frame: Rect = msg_send![screen, frame];
    let visible: Rect = msg_send![screen, visibleFrame];
    let menu_bar_height =
        ((frame.origin.y + frame.size.height) - (visible.origin.y + visible.size.height)).max(0.0);
    PlatformMetrics {
        menu_bar_height,
        notch: unsafe { notch_of(screen, frame.origin.x) },
        fullscreen: fullscreen_space(),
    }
}

unsafe fn notch_of(screen: &AnyObject, screen_min_x: f64) -> Option<super::layout::HardwareNotch> {
    if !responds(screen, sel!(safeAreaInsets))
        || !responds(screen, sel!(auxiliaryTopLeftArea))
        || !responds(screen, sel!(auxiliaryTopRightArea))
    {
        return None;
    }
    let insets: EdgeInsets = msg_send![screen, safeAreaInsets];
    let left: Rect = msg_send![screen, auxiliaryTopLeftArea];
    let right: Rect = msg_send![screen, auxiliaryTopRightArea];
    hardware_notch(
        screen_min_x,
        insets.top,
        left.origin.x + left.size.width,
        left.size.width,
        right.origin.x,
        right.size.width,
    )
}

/// Activates the app that owns a window, which also switches to its Space.
pub(super) fn activate_pid(pid: u32) {
    // SAFETY: NSRunningApplication class methods may be called from any thread.
    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![
            class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: pid as i32
        ];
        if let Some(app) = app {
            // NSApplicationActivateAllWindows | NSApplicationActivateIgnoringOtherApps
            let _: bool = msg_send![&*app, activateWithOptions: 3usize];
        }
    }
}

#[derive(Clone, Copy)]
struct SpaceApi {
    connection: unsafe extern "C" fn() -> i32,
    display_space: unsafe extern "C" fn(i32, *const c_void) -> u64,
    active_space: unsafe extern "C" fn(i32) -> u64,
    space_type: unsafe extern "C" fn(i32, u64) -> i32,
}

fn symbol(name: &CStr) -> Option<*mut c_void> {
    // SAFETY: `name` is NUL-terminated and the lookup only reads the loaded images.
    let address = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) };
    (!address.is_null()).then_some(address)
}

/// The window server's Space calls. They are not public, so they are looked up by name and a
/// macOS without them only loses the fullscreen behavior.
fn space_api() -> Option<SpaceApi> {
    static API: OnceLock<Option<SpaceApi>> = OnceLock::new();
    *API.get_or_init(|| {
        // SAFETY: the signatures are the ones these symbols have had since OS X 10.11.
        unsafe {
            Some(SpaceApi {
                connection: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> i32>(
                    symbol(c"CGSMainConnectionID")?,
                ),
                display_space: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(i32, *const c_void) -> u64,
                >(symbol(c"CGSManagedDisplayGetCurrentSpace")?),
                active_space: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(i32) -> u64>(
                    symbol(c"CGSGetActiveSpace")?,
                ),
                space_type: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(i32, u64) -> i32,
                >(symbol(c"CGSSpaceGetType")?),
            })
        }
    })
}

/// The Space the display with the menu bar shows, or the active one where displays share Spaces.
unsafe fn shown_space(api: SpaceApi, connection: i32) -> u64 {
    // SAFETY: the caller holds a valid connection; both objects are released after use.
    unsafe {
        let uuid = CGDisplayCreateUUIDFromDisplayID(CGMainDisplayID());
        let mut space = 0;
        if !uuid.is_null() {
            let name = CFUUIDCreateString(std::ptr::null(), uuid);
            CFRelease(uuid);
            if !name.is_null() {
                space = (api.display_space)(connection, name);
                CFRelease(name);
            }
        }
        if space == 0 {
            space = (api.active_space)(connection);
        }
        space
    }
}

/// Whether the display the notch lives on shows a fullscreen Space, where the menu bar is hidden.
pub(super) fn fullscreen_space() -> bool {
    let Some(api) = space_api() else {
        return false;
    };
    // SAFETY: the calls only read the window server's state for this process's connection.
    unsafe {
        let connection = (api.connection)();
        let space = shown_space(api, connection);
        space != 0 && (api.space_type)(connection, space) == FULLSCREEN_SPACE
    }
}

/// Must run on the main thread. Calls `changed` there each time the user lands on another Space;
/// a second call keeps the first watcher.
pub(super) fn watch_spaces(changed: impl Fn() + 'static) {
    if SPACE_OBSERVER.with(|slot| slot.borrow().is_some()) {
        return;
    }
    let block = RcBlock::new(move |_: NonNull<NSNotification>| changed());
    // SAFETY: AppKit objects used on the main thread; the name is the value of AppKit's
    // `NSWorkspaceActiveSpaceDidChangeNotification`. Without a queue the block runs on the
    // posting thread, which for workspace notifications is the main thread.
    let observer = unsafe {
        let workspace: Option<Retained<AnyObject>> =
            msg_send![class!(NSWorkspace), sharedWorkspace];
        let Some(workspace) = workspace else { return };
        let center: Option<Retained<NSNotificationCenter>> =
            msg_send![&*workspace, notificationCenter];
        let Some(center) = center else { return };
        let name = NSString::from_str("NSWorkspaceActiveSpaceDidChangeNotification");
        center.addObserverForName_object_queue_usingBlock(Some(&name), None, None, &block)
    };
    SPACE_OBSERVER.with(|slot| slot.replace(Some(observer)));
}
