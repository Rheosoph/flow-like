use crate::{CancellationToken, Error, Result};
use burn::{
    module::{Module, ModuleVisitor, Param},
    tensor::{Bool, Int, Tensor},
};
use std::sync::{Mutex, MutexGuard, TryLockError};

// Burn's device RNG is shared across workers and model families in this process.
// A phase includes seeding and every random operation that depends on that seed.
static RNG: Mutex<()> = Mutex::new(());

pub(crate) fn lock(cancellation: Option<&CancellationToken>) -> Result<MutexGuard<'static, ()>> {
    let Some(cancellation) = cancellation else {
        return Ok(RNG.lock().unwrap_or_else(|error| error.into_inner()));
    };
    loop {
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        match RNG.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
            Err(TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
        }
    }
}

pub(crate) fn materialize(model: &impl Module) {
    struct Materialize;
    impl ModuleVisitor for Materialize {
        fn visit_float<const D: usize>(&mut self, param: &Param<Tensor<D>>) {
            let _ = param.val().into_data();
        }
        fn visit_int<const D: usize>(&mut self, param: &Param<Tensor<D, Int>>) {
            let _ = param.val().into_data();
        }
        fn visit_bool<const D: usize>(&mut self, param: &Param<Tensor<D, Bool>>) {
            let _ = param.val().into_data();
        }
    }
    model.visit(&mut Materialize);
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;
    #[test]
    fn cancellation_interrupts_a_waiting_rng_phase() {
        let _test = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let guard = lock(None).unwrap();
        let cancellation = CancellationToken::new();
        let worker_cancel = cancellation.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let cancelled = matches!(lock(Some(&worker_cancel)), Err(Error::Cancelled));
            tx.send(cancelled).unwrap();
        });
        cancellation.cancel();
        assert!(rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap());
        drop(guard);
        worker.join().unwrap();
    }
}
