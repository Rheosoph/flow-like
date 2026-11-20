//! Keychain secrets stay in memory once read. macOS asks on every read it was not told to
//! "Always Allow", and one sign-in or bot turn reads the same item many times.

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard, PoisonError},
};
use zeroize::Zeroizing;

/// Secrets of one keychain service by account. Each keychain call runs under the lock, so
/// concurrent first reads ask once and a write never races a read of the same cache.
pub struct KeychainCache(Mutex<BTreeMap<String, Zeroizing<Vec<u8>>>>);

impl KeychainCache {
    pub const fn new() -> Self {
        Self(Mutex::new(BTreeMap::new()))
    }

    fn secrets(&self) -> MutexGuard<'_, BTreeMap<String, Zeroizing<Vec<u8>>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The secret of `account`; `read` asks the keychain only while none is kept. A missing
    /// secret or an error is not kept, so the next call asks again.
    pub fn read<E>(
        &self,
        account: &str,
        read: impl FnOnce() -> Result<Option<Zeroizing<Vec<u8>>>, E>,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, E> {
        let mut secrets = self.secrets();
        if let Some(secret) = secrets.get(account) {
            return Ok(Some(secret.clone()));
        }
        let secret = read()?;
        if let Some(secret) = &secret {
            secrets.insert(account.to_owned(), secret.clone());
        }
        Ok(secret)
    }

    /// Keeps `secret` once `write` saved it to the keychain.
    pub fn write<E>(
        &self,
        account: &str,
        secret: &[u8],
        write: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), E> {
        let mut secrets = self.secrets();
        secrets.remove(account);
        write()?;
        secrets.insert(account.to_owned(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }

    /// Drops the kept secret of `account` and runs `delete`.
    pub fn delete<E>(
        &self,
        account: &str,
        delete: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), E> {
        let mut secrets = self.secrets();
        secrets.remove(account);
        delete()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn secret(bytes: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        Some(Zeroizing::new(bytes.to_vec()))
    }

    #[test]
    fn a_secret_is_read_from_the_keychain_once() {
        let cache = KeychainCache::new();
        let reads = Cell::new(0);
        let read = || {
            reads.set(reads.get() + 1);
            Ok::<_, ()>(secret(b"key"))
        };
        assert_eq!(cache.read("a", read).unwrap().unwrap().as_slice(), b"key");
        assert_eq!(cache.read("a", read).unwrap().unwrap().as_slice(), b"key");
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn a_missing_secret_or_a_refused_read_is_asked_again() {
        let cache = KeychainCache::new();
        assert!(cache.read("a", || Ok::<_, ()>(None)).unwrap().is_none());
        assert!(cache.read("a", || Err::<Option<_>, _>("locked")).is_err());
        assert_eq!(
            cache
                .read("a", || Ok::<_, ()>(secret(b"key")))
                .unwrap()
                .unwrap()
                .as_slice(),
            b"key"
        );
    }

    #[test]
    fn a_write_replaces_the_kept_secret_only_when_it_was_saved() {
        let cache = KeychainCache::new();
        cache.read("a", || Ok::<_, ()>(secret(b"old"))).unwrap();
        assert!(cache.write("a", b"new", || Err("denied")).is_err());
        assert_eq!(
            cache
                .read("a", || Ok::<_, ()>(secret(b"keychain")))
                .unwrap()
                .unwrap()
                .as_slice(),
            b"keychain"
        );
        cache.write("a", b"new", || Ok::<_, ()>(())).unwrap();
        assert_eq!(
            cache
                .read("a", || Ok::<_, ()>(None))
                .unwrap()
                .unwrap()
                .as_slice(),
            b"new"
        );
    }

    #[test]
    fn a_deleted_secret_is_read_from_the_keychain_again() {
        let cache = KeychainCache::new();
        cache.read("a", || Ok::<_, ()>(secret(b"key"))).unwrap();
        cache.delete("a", || Ok::<_, ()>(())).unwrap();
        assert!(cache.read("a", || Ok::<_, ()>(None)).unwrap().is_none());
    }
}
