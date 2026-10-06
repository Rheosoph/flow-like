//! On-device model hosting: content-addressed model store, acquisition, runtimes and serving.

/// Locks a std mutex; data left by a holder that panicked is still consistent here.
macro_rules! lock {
    ($mutex:expr) => {
        $mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    };
}

pub mod acquire;
pub mod db;
pub mod engines;
pub mod fetch;
#[cfg(feature = "runtime")]
pub mod gateway;
#[cfg(feature = "runtime")]
pub mod host;
pub mod recommend;
#[cfg(feature = "runtime")]
pub mod router;
pub mod runtime;
pub mod stats;
pub mod store;
pub mod supervisor;
pub mod system;

#[cfg(test)]
pub(crate) mod test_server;
