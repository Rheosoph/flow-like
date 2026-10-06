use crate::{Error, Result};
use flow_like_device_crypto::controller::{
    CertifiedHandshake, UnlockedController, unlock_controller_vault,
};
use flow_like_device_protocol::Ed25519PublicKey;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::watch;
use zeroize::Zeroizing;

/// One device's controller authority, held only in this process. Locking or dropping it
/// zeroizes the seeds, and every tunnel certified by it closes.
pub struct ControllerKeys {
    device_id: String,
    controller: Mutex<Option<UnlockedController>>,
    locked: watch::Sender<bool>,
}

impl ControllerKeys {
    /// Opens the controller vault; the Argon2id derivation runs on a blocking thread.
    pub async fn unlock(
        device_id: impl Into<String>,
        password: Zeroizing<Vec<u8>>,
        vault: Vec<u8>,
    ) -> Result<Arc<Self>> {
        let device_id = device_id.into();
        let scope = device_id.clone();
        let opened = tokio::task::spawn_blocking(move || {
            unlock_controller_vault(&scope, &password, &vault).map_err(|error| format!("{error:#}"))
        })
        .await
        .map_err(|error| Error::Unlock {
            device_id: device_id.clone(),
            message: format!("the unlock task stopped: {error}"),
        })?;
        let controller = opened.map_err(|message| Error::Unlock {
            device_id: device_id.clone(),
            message,
        })?;
        Ok(Arc::new(Self {
            device_id,
            controller: Mutex::new(Some(controller)),
            locked: watch::channel(false).0,
        }))
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn is_locked(&self) -> bool {
        *self.locked.borrow()
    }

    /// The signing key the device pins for this controller.
    pub fn controller_key(&self) -> Result<Ed25519PublicKey> {
        self.with_controller(|controller| Ok(controller.public_bundle().controller_key))
    }

    pub fn lock(&self) {
        drop(self.slot().take());
        self.locked.send_replace(true);
    }

    pub(crate) fn watch_lock(&self) -> watch::Receiver<bool> {
        self.locked.subscribe()
    }

    /// A fresh Noise key under a certificate valid for `MANAGEMENT_SESSION_SECONDS` (300 s).
    pub(crate) fn begin_tunnel(
        &self,
        grant_id: &str,
        device_key: [u8; 32],
        now: i64,
    ) -> Result<CertifiedHandshake> {
        self.with_controller(|controller| {
            controller
                .begin_tunnel_noise(grant_id, device_key, now)
                .map_err(|error| {
                    Error::Invalid(format!(
                        "Could not certify a tunnel key for device {} and grant {grant_id}: {error:#}",
                        self.device_id
                    ))
                })
        })
    }

    fn with_controller<T>(
        &self,
        operation: impl FnOnce(&UnlockedController) -> Result<T>,
    ) -> Result<T> {
        match self.slot().as_ref() {
            Some(controller) => operation(controller),
            None => Err(Error::Locked {
                device_id: self.device_id.clone(),
            }),
        }
    }

    fn slot(&self) -> MutexGuard<'_, Option<UnlockedController>> {
        self.controller
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for ControllerKeys {
    fn drop(&mut self) {
        self.lock();
    }
}
