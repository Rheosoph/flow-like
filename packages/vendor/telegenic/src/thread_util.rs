//! Worker-thread scheduling config and a join/wake handle.
//!
//! [`ThreadConfig`] only affects scheduling on Linux (SCHED_FIFO / affinity);
//! elsewhere applying a non-default config fails with `Unsupported`, which the
//! logged variant downgrades to a warning so the same code runs everywhere.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use snare::mio::Waker;

/// Thread priority and CPU affinity for an I/O worker.
///
/// - `priority < 1` → SCHED_OTHER (normal scheduling).
/// - `priority >= 1` → SCHED_FIFO with the given real-time priority.
/// - `cpu_affinity = Some(n)` pins the worker to logical CPU `n`.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct ThreadConfig {
    pub priority: i32,
    pub cpu_affinity: Option<usize>,
}

impl ThreadConfig {
    pub fn new(priority: i32, cpu_affinity: Option<usize>) -> Self {
        Self {
            priority,
            cpu_affinity,
        }
    }

    pub(crate) fn apply_logged(&self) {
        if self.priority < 1 && self.cpu_affinity.is_none() {
            return;
        }
        if let Err(e) = self.apply() {
            tracing::warn!("thread scheduling not applied: {e}");
        }
    }

    #[cfg(target_os = "linux")]
    fn apply(&self) -> io::Result<()> {
        unsafe {
            if let Some(cpu) = self.cpu_affinity {
                let ncpus = libc::sysconf(libc::_SC_NPROCESSORS_CONF) as usize;
                if cpu >= ncpus {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("CPU {cpu} out of range 0..{}", ncpus.saturating_sub(1)),
                    ));
                }
                let mut set: libc::cpu_set_t = std::mem::zeroed();
                libc::CPU_ZERO(&mut set);
                libc::CPU_SET(cpu, &mut set);
                let rc = libc::pthread_setaffinity_np(
                    libc::pthread_self(),
                    std::mem::size_of::<libc::cpu_set_t>(),
                    &set,
                );
                if rc != 0 {
                    return Err(io::Error::from_raw_os_error(rc));
                }
            }

            if self.priority >= 1 {
                let min = libc::sched_get_priority_min(libc::SCHED_FIFO);
                let max = libc::sched_get_priority_max(libc::SCHED_FIFO);
                if self.priority < min || self.priority > max {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "SCHED_FIFO priority {} out of range [{min}..={max}]",
                            self.priority
                        ),
                    ));
                }
                let mut param: libc::sched_param = std::mem::zeroed();
                param.sched_priority = self.priority;
                let rc =
                    libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &param);
                if rc != 0 {
                    return Err(io::Error::from_raw_os_error(rc));
                }
            }
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    fn apply(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "thread scheduling configuration is only supported on Linux",
        ))
    }
}

/// Owner/worker split handle: the owner can ask the worker to stop and wake it
/// from a blocking poll; the worker reports liveness. The owning side joins
/// the worker on drop; the non-owning twin handed into the thread cannot
/// accidentally tear it down.
#[derive(Debug)]
pub(crate) struct ThreadHandle {
    is_owner: bool,
    is_alive: Arc<AtomicBool>,
    should_die: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    waker: Option<Arc<Waker>>,
}

impl ThreadHandle {
    pub fn new() -> Self {
        Self {
            is_owner: true,
            is_alive: Arc::new(AtomicBool::new(true)),
            should_die: Arc::new(AtomicBool::new(false)),
            handle: None,
            waker: None,
        }
    }

    pub fn set_handle(&mut self, handle: JoinHandle<()>) {
        self.handle = Some(handle);
    }

    pub fn set_waker(&mut self, waker: Arc<Waker>) {
        self.waker = Some(waker);
    }

    pub fn wake(&self) -> io::Result<()> {
        match &self.waker {
            Some(w) => w.wake(),
            None => Ok(()),
        }
    }

    pub fn is_alive(&self) -> bool {
        self.is_alive.load(Ordering::Relaxed)
    }

    pub fn should_live(&self) -> bool {
        !self.should_die.load(Ordering::Relaxed)
    }

    pub fn has_died(&self) {
        self.is_alive.store(false, Ordering::Relaxed);
    }

    /// Ask the worker to exit at the next loop turn. Does **not** join — the
    /// owning `Drop` does that. Safe to call from any thread.
    pub fn request_stop(&self) {
        self.should_die.store(true, Ordering::Relaxed);
        let _ = self.wake();
    }

    /// Produce the non-owning twin to hand into the spawned thread.
    pub fn to_pass_in(&self) -> Self {
        Self {
            is_owner: false,
            is_alive: self.is_alive.clone(),
            should_die: self.should_die.clone(),
            handle: None,
            waker: self.waker.clone(),
        }
    }
}

impl Drop for ThreadHandle {
    fn drop(&mut self) {
        if !self.is_owner {
            return;
        }
        self.should_die.store(true, Ordering::Relaxed);
        let _ = self.wake();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
