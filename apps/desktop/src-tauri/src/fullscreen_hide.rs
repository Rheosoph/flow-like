//! Hiding a window in native fullscreen leaves its Space behind as a black screen, so such a
//! window leaves fullscreen first and hides once AppKit reports the exit.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::time::Duration;

use block2::RcBlock;
use flow_like_types::tokio::sync::oneshot;
use flow_like_types::tokio::time::timeout;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::NSWindowDidExitFullScreenNotification;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};

/// The exit animation takes about a second; past this the window hides regardless.
const EXIT_TIMEOUT: Duration = Duration::from_secs(3);

thread_local! {
    static EXIT_OBSERVER: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>> =
        const { RefCell::new(None) };
}

pub(crate) async fn hide(window: &tauri::Window) -> tauri::Result<()> {
    if window.is_fullscreen()? {
        leave_fullscreen(window).await?;
    }
    window.hide()
}

async fn leave_fullscreen(window: &tauri::Window) -> tauri::Result<()> {
    let (exited, on_exit) = oneshot::channel();
    let target = window.clone();
    window.run_on_main_thread(move || {
        let label = target.label().to_owned();
        let left = observe_exit(&target, exited).and_then(|()| target.set_fullscreen(false));
        if let Err(error) = left {
            stop_observing();
            tracing::warn!(%error, window = label, "Leaving fullscreen before hiding failed");
        }
    })?;
    if timeout(EXIT_TIMEOUT, on_exit).await.is_err() {
        tracing::warn!(
            window = window.label(),
            "The window did not report leaving fullscreen in time; hiding it anyway"
        );
    }
    Ok(())
}

/// Runs on the main thread; the observer removes itself when the exit arrives.
fn observe_exit(window: &tauri::Window, exited: oneshot::Sender<()>) -> tauri::Result<()> {
    let ns_window = window.ns_window()?;
    // SAFETY: Tauri returns the live NSWindow of `window`, which outlives this call.
    let Some(ns_window) = (unsafe { ns_window.cast::<AnyObject>().as_ref() }) else {
        return Ok(());
    };
    let exited = Mutex::new(Some(exited));
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
        if let Some(exited) = exited.lock().ok().and_then(|mut exited| exited.take()) {
            let _ = exited.send(());
        }
        stop_observing();
    });
    stop_observing();
    // SAFETY: the name is AppKit's constant and the object is a live NSWindow. Without a queue
    // the block runs on the posting thread, which for window notifications is the main thread.
    let observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSWindowDidExitFullScreenNotification),
            Some(ns_window),
            None,
            &block,
        )
    };
    EXIT_OBSERVER.with(|slot| slot.replace(Some(observer)));
    Ok(())
}

fn stop_observing() {
    let Some(observer) = EXIT_OBSERVER.with(|slot| slot.take()) else {
        return;
    };
    let observer: &AnyObject = (*observer).as_ref();
    // SAFETY: `observer` is the token `addObserverForName:…` returned.
    unsafe { NSNotificationCenter::defaultCenter().removeObserver(observer) };
}
