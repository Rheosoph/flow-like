use super::layout::{PlatformMetrics, hardware_notch};
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{class, msg_send, sel};

const STATUS_WINDOW_LEVEL: isize = 25;
const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
const STATIONARY: usize = 1 << 4;
const IGNORES_CYCLE: usize = 1 << 6;
const FULL_SCREEN_AUXILIARY: usize = 1 << 8;

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

/// Must run on the main thread. Level 25 sits above the menu bar; tao would reset it to floating
/// if `set_always_on_top` ran afterwards.
pub(super) fn configure_window(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let pointer = ns_window(window)?;
    // SAFETY: Tauri returns the live NSWindow of `window`; this runs on the main thread.
    let Some(ns_window) = (unsafe { pointer.as_ref() }) else {
        return Ok(());
    };
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
