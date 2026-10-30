use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
};
use zeroize::Zeroizing;

const SERVICE: &str = "com.flow-like.native-auth.v1";
const MAX_STORED_BYTES: u64 = 512 * 1024;

fn validate_length(bytes: &[u8]) -> Result<(), String> {
    // XChaCha20-Poly1305 adds a 24-byte nonce and a 16-byte authentication tag.
    if bytes.len().saturating_add(40) > MAX_STORED_BYTES as usize {
        return Err("Authentication session exceeds the encrypted storage limit".into());
    }
    Ok(())
}

fn path(hub: &str) -> std::path::PathBuf {
    crate::settings::execution_authority_dir()
        .join("sessions")
        .join(format!("{}.bin", blake3::hash(hub.as_bytes()).to_hex()))
}

// Keep both ciphertext and its Keychain key available after the first device unlock.
// This pins the storage contract independently of the app's default Data Protection class.
#[cfg(all(not(test), target_os = "ios"))]
fn set_background_file_protection(path: &std::path::Path) -> std::io::Result<()> {
    use objc2::{rc::autoreleasepool, runtime::AnyObject};
    use objc2_foundation::{
        NSDictionary, NSFileManager, NSFileProtectionCompleteUntilFirstUserAuthentication,
        NSFileProtectionKey, NSString,
    };
    let path = path.to_str().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid authentication file path",
        )
    })?;
    autoreleasepool(|_| {
        // SAFETY: These Foundation constants are NSString objects. NSFileProtectionKey's
        // documented value type is NSFileProtectionType, an NSString alias.
        let attributes = unsafe {
            NSDictionary::<NSString, AnyObject>::from_slices(
                &[NSFileProtectionKey],
                &[NSFileProtectionCompleteUntilFirstUserAuthentication.as_ref()],
            )
        };
        // SAFETY: The dictionary contains exactly the documented file protection attribute.
        unsafe {
            NSFileManager::defaultManager()
                .setAttributes_ofItemAtPath_error(&attributes, &NSString::from_str(path))
        }
        .map_err(|_| {
            std::io::Error::other("Cannot configure background authentication file protection")
        })
    })
}

#[cfg(all(not(test), not(target_os = "android")))]
fn cipher(hub: &str, create: bool) -> Result<XChaCha20Poly1305, String> {
    let entry = keyring::Entry::new(SERVICE, blake3::hash(hub.as_bytes()).to_hex().as_str())
        .map_err(|_| "Cannot open the system credential store")?;
    let secret = match entry.get_secret() {
        Ok(secret) => Zeroizing::new(secret),
        Err(keyring::Error::NoEntry) if create => {
            let secret = Zeroizing::new(XChaCha20Poly1305::generate_key(&mut OsRng).to_vec());
            #[cfg(not(target_os = "ios"))]
            entry
                .set_secret(&secret)
                .map_err(|_| "Cannot save the authentication encryption key")?;
            #[cfg(target_os = "ios")]
            {
                use security_framework::{
                    access_control::{ProtectionMode, SecAccessControl},
                    passwords::{PasswordOptions, set_generic_password_options},
                };
                let account = blake3::hash(hub.as_bytes()).to_hex().to_string();
                let mut options = PasswordOptions::new_generic_password(SERVICE, &account);
                // Background execution can resume after the first device unlock. The key
                // remains on this device and never enters an iCloud keychain backup.
                let access = SecAccessControl::create_with_protection(
                    Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
                    0,
                )
                .map_err(|_| "Cannot configure background authentication storage")?;
                options.set_access_control(access);
                set_generic_password_options(&secret, options)
                    .map_err(|_| "Cannot save the authentication encryption key")?;
            }
            secret
        }
        Err(_) => return Err("The system credential store is unavailable or locked".into()),
    };
    XChaCha20Poly1305::new_from_slice(&secret)
        .map_err(|_| "Invalid authentication encryption key".into())
}

#[cfg(all(not(test), not(target_os = "android")))]
pub(super) async fn read(hub: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    let hub = hub.to_owned();
    tokio::task::spawn_blocking(move || {
        let path = path(&hub);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("Cannot read the saved authentication session".into()),
        };
        if !metadata.file_type().is_file() || metadata.len() > MAX_STORED_BYTES {
            return Err("Invalid saved authentication file".into());
        }
        #[cfg(target_os = "ios")]
        set_background_file_protection(&path)
            .map_err(|_| "Cannot configure background authentication file protection")?;
        let encrypted =
            std::fs::read(path).map_err(|_| "Cannot read the saved authentication session")?;
        if encrypted.len() < 24 {
            return Err("Invalid saved authentication file".into());
        }
        let plaintext = cipher(&hub, false)?
            .decrypt(
                XNonce::from_slice(&encrypted[..24]),
                Payload {
                    msg: &encrypted[24..],
                    aad: hub.as_bytes(),
                },
            )
            .map_err(|_| "Cannot decrypt the saved authentication session")?;
        Ok(Some(Zeroizing::new(plaintext)))
    })
    .await
    .map_err(|_| "Authentication storage worker failed")?
}

#[cfg(all(not(test), not(target_os = "android")))]
pub(super) async fn write(hub: &str, bytes: Vec<u8>) -> Result<(), String> {
    validate_length(&bytes)?;
    let hub = hub.to_owned();
    let bytes = Zeroizing::new(bytes);
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let path = path(&hub);
        let dir = path.parent().ok_or("Invalid authentication directory")?;
        std::fs::create_dir_all(dir).map_err(|_| "Cannot create authentication directory")?;
        #[cfg(target_os = "ios")]
        {
            set_background_file_protection(&crate::settings::execution_authority_dir())
                .map_err(|_| "Cannot configure authentication directory protection")?;
            set_background_file_protection(dir)
                .map_err(|_| "Cannot configure authentication directory protection")?;
        }
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let encrypted = cipher(&hub, !path.exists())?
            .encrypt(
                &nonce,
                Payload {
                    msg: &bytes,
                    aad: hub.as_bytes(),
                },
            )
            .map_err(|_| "Cannot encrypt authentication session")?;
        let temp = dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            #[cfg(target_os = "ios")]
            set_background_file_protection(&temp)?;
            file.write_all(&nonce)?;
            file.write_all(&encrypted)?;
            file.sync_all()?;
            std::fs::rename(&temp, &path)?;
            #[cfg(unix)]
            std::fs::File::open(dir)?.sync_all()?;
            Ok::<_, std::io::Error>(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result.map_err(|_| "Cannot commit authentication session".into())
    })
    .await
    .map_err(|_| "Authentication storage worker failed")?
}

// Android has no OS keyring backend in this dependency. Its running process can own
// refresh, but it must not silently fall back to writing plaintext login grants.
#[cfg(all(not(test), target_os = "android"))]
pub(super) async fn read(_: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    Ok(None)
}
#[cfg(all(not(test), target_os = "android"))]
pub(super) async fn write(_: &str, bytes: Vec<u8>) -> Result<(), String> {
    validate_length(&bytes)
}

#[cfg(test)]
static SAVED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>> =
    std::sync::LazyLock::new(std::sync::Mutex::default);
#[cfg(test)]
pub(super) async fn read(hub: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    Ok(SAVED.lock().unwrap().get(hub).cloned().map(Zeroizing::new))
}
#[cfg(test)]
pub(super) async fn write(hub: &str, bytes: Vec<u8>) -> Result<(), String> {
    validate_length(&bytes)?;
    SAVED.lock().unwrap().insert(hub.to_owned(), bytes);
    Ok(())
}

#[cfg(test)]
#[tokio::test]
async fn oversized_ciphertext_is_rejected_before_replacing_the_saved_grant() {
    let key = "auth-storage-size-regression";
    write(key, b"existing-grant".to_vec()).await.unwrap();
    assert!(
        write(key, vec![0; MAX_STORED_BYTES as usize])
            .await
            .is_err()
    );
    assert_eq!(
        read(key).await.unwrap().unwrap().as_slice(),
        b"existing-grant"
    );
    write(key, vec![0; MAX_STORED_BYTES as usize - 40])
        .await
        .unwrap();
}
