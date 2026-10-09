use super::*;
use std::sync::{Condvar, Mutex};

/// Propagate durable cancellation while the backend waits without progress callbacks.
pub(crate) struct BurnCancellationMonitor {
    stop: Arc<(Mutex<bool>, Condvar)>,
    reason: Arc<Mutex<Option<Error>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl BurnCancellationMonitor {
    pub(crate) fn start(
        control: &WorkerControl,
        cancellation: &flow_like_ml_burn::CancellationToken,
    ) -> Result<Self> {
        control.check_cancel()?;
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let reason = Arc::new(Mutex::new(None));
        let signal = stop.clone();
        let failure = reason.clone();
        let control = control.clone();
        let cancellation = cancellation.clone();
        let thread = std::thread::Builder::new()
            .name("inspection-cancellation".into())
            .spawn(move || {
                loop {
                    if let Err(error) = control.check_cancel() {
                        *failure.lock().unwrap_or_else(|error| error.into_inner()) = Some(error);
                        cancellation.cancel();
                        return;
                    }
                    let guard = signal.0.lock().unwrap_or_else(|error| error.into_inner());
                    let (guard, _) = signal
                        .1
                        .wait_timeout_while(guard, std::time::Duration::from_millis(50), |stop| {
                            !*stop
                        })
                        .unwrap_or_else(|error| error.into_inner());
                    if *guard {
                        return;
                    }
                }
            })?;
        Ok(Self {
            stop,
            reason,
            thread: Some(thread),
        })
    }

    /// Join before reading the reason so a deadline or lost lease remains distinguishable.
    pub(crate) fn finish(mut self) -> Option<Error> {
        self.shutdown();
        let reason = self
            .reason
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        reason
    }

    fn shutdown(&mut self) {
        *self
            .stop
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
        self.stop.1.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for BurnCancellationMonitor {
    fn drop(&mut self) {
        self.shutdown();
    }
}
