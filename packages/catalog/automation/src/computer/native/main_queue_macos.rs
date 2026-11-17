//! AppKit and Text Input Sources trap when called off the main thread. Automation runs on worker
//! threads, so calls that reach them hop onto the main queue here.

use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

type Job = Box<dyn FnOnce() + Send>;

unsafe extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(
        queue: *const DispatchQueue,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn pthread_main_np() -> i32;
}

extern "C" fn run_job(context: *mut c_void) {
    let job = unsafe { Box::from_raw(context.cast::<Job>()) };
    let _ = catch_unwind(AssertUnwindSafe(job));
}

/// Runs `work` on the main queue and waits for it. `None` when the main thread does not start it
/// within `timeout` or it panics. Work that has not started by then is dropped unrun, so a late
/// main thread never acts on a request its caller already gave up on.
pub(crate) fn on_main_queue<T, F>(timeout: Duration, work: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if unsafe { pthread_main_np() } != 0 {
        return Some(work());
    }
    let slot = Arc::new(Mutex::new(Some(work)));
    let claim = Arc::clone(&slot);
    let (sender, receiver) = mpsc::sync_channel(1);
    let job: Job = Box::new(move || {
        let work = claim.lock().ok().and_then(|mut work| work.take());
        if let Some(work) = work {
            let _ = sender.send(work());
        }
    });
    let context = Box::into_raw(Box::new(job)).cast::<c_void>();
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, context, run_job) };
    match receiver.recv_timeout(timeout) {
        Ok(value) => Some(value),
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let unstarted = slot.lock().ok().and_then(|mut work| work.take()).is_some();
            if unstarted {
                None
            } else {
                receiver.recv().ok()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn work_the_main_thread_never_starts_is_dropped_unrun() {
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        let answer = std::thread::spawn(move || {
            on_main_queue(Duration::from_millis(50), move || {
                flag.store(true, Ordering::SeqCst);
            })
        })
        .join()
        .expect("caller thread");
        assert!(answer.is_none());
        assert!(!ran.load(Ordering::SeqCst));
    }
}
