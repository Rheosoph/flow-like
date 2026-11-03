//! The OS keychain as the home of encryption bots' inbox key seals.

use flow_like_device_crypto::inbox::InboxKeyVault;
use zeroize::Zeroizing;

#[cfg(not(target_os = "android"))]
const KEYCHAIN_SERVICE: &str = "com.flow-like.bot-inbox-keys.v1";

pub struct KeychainVault;

#[cfg(not(target_os = "android"))]
impl KeychainVault {
    fn entry(owner: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(KEYCHAIN_SERVICE, owner)
            .map_err(|_| "Cannot open the system credential store".to_owned())
    }
}

#[cfg(not(target_os = "android"))]
impl InboxKeyVault for KeychainVault {
    fn load(&self, owner: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        match Self::entry(owner)?.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("The system credential store is unavailable or locked".into()),
        }
    }

    fn store(&self, owner: &str, secret: &[u8]) -> Result<(), String> {
        Self::entry(owner)?
            .set_secret(secret)
            .map_err(|_| "Cannot save the inbox key".to_owned())
    }

    fn remove(&self, owner: &str) -> Result<(), String> {
        match Self::entry(owner)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Cannot delete the inbox key".into()),
        }
    }
}

#[cfg(target_os = "android")]
impl InboxKeyVault for KeychainVault {
    fn load(&self, _: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        Err("Encrypted chats need a system keychain, which this device doesn't offer".into())
    }

    fn store(&self, _: &str, _: &[u8]) -> Result<(), String> {
        self.load("").map(drop)
    }

    fn remove(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}
