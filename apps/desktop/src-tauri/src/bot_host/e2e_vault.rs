//! The OS keychain as the home of inbox key seals: the account bots this host runs and the
//! signed-in people of this device, each under its own service.

use flow_like_device_crypto::inbox::InboxKeyVault;
use zeroize::Zeroizing;

pub struct KeychainVault {
    #[cfg_attr(target_os = "android", allow(dead_code))]
    service: &'static str,
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    prefix: &'static str,
}

impl KeychainVault {
    /// The keys of the encryption bots this host runs, by bot id.
    pub const BOTS: Self = Self {
        service: "com.flow-like.bot-inbox-keys.v1",
        prefix: "bot-inbox-key:v1",
    };
    /// This device's keys of the people who signed in on it, by user sub.
    pub const PEOPLE: Self = Self {
        service: "com.flow-like.inbox-keys.v1",
        prefix: "inbox-key:v1",
    };
}

#[cfg(not(target_os = "android"))]
impl KeychainVault {
    fn entry(&self, owner: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(self.service, owner)
            .map_err(|_| "Cannot open the system credential store".to_owned())
    }
}

#[cfg(not(target_os = "android"))]
impl InboxKeyVault for KeychainVault {
    fn load(&self, owner: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        match self.entry(owner)?.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("The system credential store is unavailable or locked".into()),
        }
    }

    fn store(&self, owner: &str, secret: &[u8]) -> Result<(), String> {
        self.entry(owner)?
            .set_secret(secret)
            .map_err(|_| "Cannot save the inbox key".to_owned())
    }

    fn remove(&self, owner: &str) -> Result<(), String> {
        match self.entry(owner)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Cannot delete the inbox key".into()),
        }
    }
}

#[cfg(target_os = "android")]
impl InboxKeyVault for KeychainVault {
    fn load(&self, owner: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        crate::android_mobile::secure_read(&format!("{}:{owner}", self.prefix))
    }

    fn store(&self, owner: &str, secret: &[u8]) -> Result<(), String> {
        crate::android_mobile::secure_write(&format!("{}:{owner}", self.prefix), secret)
    }

    fn remove(&self, owner: &str) -> Result<(), String> {
        crate::android_mobile::secure_delete(&format!("{}:{owner}", self.prefix))
    }
}
