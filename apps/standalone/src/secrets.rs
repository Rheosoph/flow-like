use crate::{config::PlacementConfig, enrollment::unix_time, state::StateStore, supervisor, vault};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use rusqlite::{OptionalExtension, params};
use sha2::Sha256;
use std::{
    fs::File,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

fn key(root: &Path) -> Result<Zeroizing<[u8; 32]>> {
    let path = root.join("secrets.key");
    if !path.try_exists()? {
        let mut key = Zeroizing::new([0; 32]);
        OsRng.fill_bytes(key.as_mut());
        if let Err(error) = vault::write_new_private(&path, key.as_ref()) {
            if !path.try_exists()? {
                return Err(error);
            }
        }
    }
    let bytes = vault::read_private(&path)?;
    Ok(Zeroizing::new(
        bytes
            .as_slice()
            .try_into()
            .context("Invalid secret storage key")?,
    ))
}

/// A keyed journal digest prevents guessing a short secret from its public request hash.
pub fn request_digest(
    root: &Path,
    request: &flow_like_device_protocol::ManagementRequest,
) -> Result<String> {
    let encoded = Zeroizing::new(serde_json::to_vec(request)?);
    let key = key(root)?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.as_ref())?;
    mac.update(b"flow-like/secret-command/v1\0");
    mac.update(&encoded);
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

pub fn enqueue(
    store: &StateStore,
    root: &Path,
    operation: &str,
    placement: &str,
    revision: u64,
    name: &str,
    value: &str,
) -> Result<()> {
    crate::config::validate_id("secret name", name)?;
    ensure!(
        !value.is_empty() && value.len() <= 4096,
        "Secret must contain 1 to 4096 bytes"
    );
    let key = key(root)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|_| anyhow::anyhow!("Secret key initialization failed"))?;
    let mut nonce = [0; 24];
    OsRng.fill_bytes(&mut nonce);
    let aad = serde_json::to_vec(&(
        "flow-like/secret-operation/v1",
        operation,
        placement,
        revision,
        name,
    ))?;
    let mut sealed = nonce.to_vec();
    sealed.extend(
        cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: value.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("Secret encryption failed"))?,
    );
    store.connection.execute("INSERT INTO secret_operations(operation_id,placement_id,expected_revision,name,ciphertext,created_at,state) VALUES(?1,?2,?3,?4,?5,?6,'pending')",params![operation,placement,revision,name,sealed,unix_time()?])?;
    Ok(())
}

pub fn directory(config: &PlacementConfig) -> Result<PathBuf> {
    crate::config::validate_id("placement", &config.id)?;
    let mut path = config.project_path.canonicalize()?;
    for component in [".secrets", &config.id] {
        crate::config::ensure_unaliased_child(&path, component)?;
        let parent = path.clone();
        path.push(component);
        if path.try_exists()? {
            let metadata = std::fs::symlink_metadata(&path)?;
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "Secret directory must not be a symlink"
            );
        }
        path = supervisor::prepare_state_dir(&path)?;
        crate::config::ensure_unaliased_child(&parent, component)?;
    }
    Ok(path)
}

pub fn install(config: &PlacementConfig, name: &str, value: &[u8]) -> Result<()> {
    crate::config::validate_id("secret name", name)?;
    ensure!(
        !value.is_empty() && value.len() <= 4096,
        "Secret size exceeds its bound"
    );
    let directory = directory(config)?;
    let file_name = format!("{name}.secret");
    crate::config::ensure_unaliased_child(&directory, &file_name)?;
    let target = directory.join(&file_name);
    if target.try_exists()? {
        vault::read_private(&target)?;
    }
    let temporary = directory.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    vault::write_new_private(&temporary, value)?;
    if let Err(error) = std::fs::rename(&temporary, target) {
        let _ = std::fs::remove_file(temporary);
        return Err(error.into());
    }
    crate::config::ensure_unaliased_child(&directory, &file_name)?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

pub fn install_current(
    store: &StateStore,
    placement: &str,
    name: &str,
    value: &[u8],
) -> Result<()> {
    store.with_rollout_transaction(|| {
        store.require_no_active_rollout(placement)?;
        let record = store
            .get_placement(placement)?
            .context("Unknown placement")?;
        install(&serde_json::from_value(record.config)?, name, value)
    })
}

pub fn preserve_for_revision(old: &PlacementConfig, next: &PlacementConfig) -> Result<()> {
    ensure!(
        old.id == next.id && old.project_id == next.project_id,
        "Secret placement identity changed"
    );
    if old.project_path == next.project_path {
        return Ok(());
    }
    let source = old.project_path.join(".secrets").join(&old.id);
    let names: std::collections::BTreeSet<_> = next
        .secret_overrides
        .values()
        .chain(next.hosting.iter().map(|hosting| &hosting.auth_secret))
        .collect();
    let retained: std::collections::BTreeSet<_> = old
        .secret_overrides
        .values()
        .chain(old.hosting.iter().map(|hosting| &hosting.auth_secret))
        .collect();
    ensure!(
        names.len() <= 1025,
        "Placement secret count exceeds its bound"
    );
    if !source.try_exists()? {
        ensure!(
            names.is_disjoint(&retained),
            "Retained secrets are missing from the current placement"
        );
        return Ok(());
    }
    let source = directory(old)?;
    let destination = directory(next)?;
    for name in names {
        crate::config::validate_id("secret name", name)?;
        let file_name = format!("{name}.secret");
        crate::config::ensure_unaliased_child(&destination, &file_name)?;
        crate::config::ensure_unaliased_child(&source, &file_name)?;
        let target = destination.join(&file_name);
        if target.try_exists()? {
            vault::read_private(&target)?;
            if !retained.contains(name) {
                continue;
            }
        }
        let path = source.join(file_name);
        if !path.try_exists()? {
            ensure!(
                !retained.contains(name),
                "A retained secret is missing from the current placement"
            );
            continue;
        }
        // A revisited project snapshot may contain an older value. References retained
        // from the current placement follow its value, including manual rotations.
        let value = vault::read_private(&path)?;
        install(next, name, &value)?;
    }
    Ok(())
}

pub fn publish_one(root: &Path) -> Result<bool> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    store.connection.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<bool> {
        type Pending = (String, String, u64, String, Vec<u8>);
        let pending:Option<Pending>=store.connection.query_row("SELECT operation_id,placement_id,expected_revision,name,ciphertext FROM secret_operations WHERE state='pending' ORDER BY created_at,rowid LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((operation, placement, revision, name, ciphertext)) = pending else {
            return Ok(false);
        };
        let apply = (|| -> Result<()> {
            let record = store
                .get_placement(&placement)?
                .context("Placement removed")?;
            ensure!(
                record.config_revision == revision,
                "Placement changed before secret publication"
            );
            let config: PlacementConfig = serde_json::from_value(record.config)?;
            ensure!(ciphertext.len() >= 40, "Truncated secret operation");
            let key = key(root)?;
            let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
                .map_err(|_| anyhow::anyhow!("Secret key initialization failed"))?;
            let aad = serde_json::to_vec(&(
                "flow-like/secret-operation/v1",
                &operation,
                &placement,
                revision,
                &name,
            ))?;
            let plaintext = Zeroizing::new(
                cipher
                    .decrypt(
                        XNonce::from_slice(&ciphertext[..24]),
                        Payload {
                            msg: &ciphertext[24..],
                            aad: &aad,
                        },
                    )
                    .map_err(|_| anyhow::anyhow!("Secret authentication failed"))?,
            );
            install(&config, &name, &plaintext)
        })();
        let state = if apply.is_ok() { "completed" } else { "failed" };
        store.connection.execute(
            "UPDATE secret_operations SET state=?2,ciphertext=x'' WHERE operation_id=?1",
            params![operation, state],
        )?;
        store.connection.execute("UPDATE management_operations SET result_json=json_set(result_json,'$.state',?2,'$.result.secret',?2) WHERE operation_id=?1",params![operation,state])?;
        Ok(true)
    })();
    match result {
        Ok(value) => {
            store.connection.execute_batch("COMMIT")?;
            Ok(value)
        }
        Err(error) => {
            let _ = store.connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

pub async fn publish(root: PathBuf, cancel: CancellationToken) -> Result<()> {
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {_=cancel.cancelled()=>return Ok(()),_=tick.tick()=>()};
        publish_one(&root)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_directories_and_names_reject_case_aliases_without_replacing_values() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let config: PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"API","project_id":"project","deployment_id":"service",
            "revision":"v1","source":"offline","project_path":dir.path(),
            "events":[{"event_id":"event","event_version":[1,0,0],"board_version":[1,0,0]}]
        }))?;
        install(&config, "Token", b"original")?;
        let target = crate::config::private_secret_path(&config, "Token")?;
        assert!(install(&config, "token", b"replacement").is_err());
        assert!(crate::config::private_secret_path(&config, "token").is_err());
        let mut aliased = config.clone();
        aliased.id = "api".into();
        assert!(directory(&aliased).is_err());
        assert!(install(&aliased, "Token", b"replacement").is_err());
        assert!(crate::config::private_secret_path(&aliased, "Token").is_err());
        assert_eq!(&**vault::read_private(&target)?, b"original");
        install(&config, "Token", b"rotated")?;
        assert_eq!(&**vault::read_private(&target)?, b"rotated");
        Ok(())
    }

    #[test]
    fn revision_copy_preserves_only_referenced_secrets_and_keeps_new_values() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let old_path = crate::supervisor::prepare_state_dir(&root.join("old"))?;
        let next_path = crate::supervisor::prepare_state_dir(&root.join("next"))?;
        let old: PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"placement","project_id":"project","deployment_id":"service",
            "revision":"v1","source":"offline","project_path":old_path,
            "events":[{"event_id":"event","event_version":[1,0,0],"board_version":[1,0,0]}]
        }))?;
        install(&old, "needed", b"old")?;
        install(&old, "preserved", b"preserved")?;
        install(&old, "unused", b"unused")?;
        let mut next = old.clone();
        next.project_path = next_path;
        next.secret_overrides.insert("a".into(), "needed".into());
        next.secret_overrides.insert("b".into(), "preserved".into());
        install(&next, "needed", b"new")?;
        preserve_for_revision(&old, &next)?;
        let destination = directory(&next)?;
        assert_eq!(
            &**vault::read_private(&destination.join("needed.secret"))?,
            b"new"
        );
        assert_eq!(
            &**vault::read_private(&destination.join("preserved.secret"))?,
            b"preserved"
        );
        assert!(!destination.join("unused.secret").exists());
        Ok(())
    }

    #[test]
    fn revisiting_a_revision_keeps_current_retained_secret_values() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let old_path = crate::supervisor::prepare_state_dir(&root.join("old"))?;
        let current_path = crate::supervisor::prepare_state_dir(&root.join("current"))?;
        let mut old: PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"placement","project_id":"project","deployment_id":"service",
            "revision":"v1","source":"offline","project_path":old_path,
            "events":[{"event_id":"event","event_version":[1,0,0],"board_version":[1,0,0]}],
            "secret_overrides":{"credential":"token"}
        }))?;
        install(&old, "token", b"previous-token")?;
        let mut current = old.clone();
        current.project_path = current_path;
        current.revision = "v2".into();
        preserve_for_revision(&old, &current)?;
        install(&current, "token", b"rotated-token")?;
        old.revision = "v3".into();
        preserve_for_revision(&current, &old)?;
        assert_eq!(
            &**vault::read_private(&directory(&old)?.join("token.secret"))?,
            b"rotated-token"
        );
        std::fs::remove_file(directory(&current)?.join("token.secret"))?;
        assert!(preserve_for_revision(&current, &old).is_err());
        std::fs::remove_dir(directory(&current)?)?;
        assert!(preserve_for_revision(&current, &old).is_err());
        Ok(())
    }

    #[test]
    fn secret_queue_is_encrypted_scoped_and_recoverable() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let config: PlacementConfig = serde_json::from_value(
            serde_json::json!({"id":"placement","project_id":"project","deployment_id":"service","revision":"v1","source":"offline","project_path":root,"events":[{"event_id":"event","event_version":[1,0,0],"board_version":[1,0,0]}],"variables":{}}),
        )?;
        store.upsert_placement(
            "placement",
            &serde_json::to_value(&config)?,
            crate::state::DesiredState::Stopped,
        )?;
        enqueue(
            &store,
            &root,
            "operation",
            "placement",
            1,
            "listener",
            "test-listener-secret",
        )?;
        let ciphertext: Vec<u8> =
            store
                .connection
                .query_row("SELECT ciphertext FROM secret_operations", [], |r| r.get(0))?;
        assert!(!ciphertext.windows(20).any(|v| v == b"test-listener-secret"));
        assert!(publish_one(&root)?);
        assert!(!publish_one(&root)?);
        assert_eq!(
            &**vault::read_private(&root.join(".secrets/placement/listener.secret"))?,
            b"test-listener-secret"
        );
        assert!(install(&config, "../other", b"value").is_err());
        enqueue(
            &store,
            &root,
            "stale",
            "placement",
            2,
            "listener",
            "rejected",
        )?;
        publish_one(&root)?;
        assert_eq!(
            &**vault::read_private(&root.join(".secrets/placement/listener.secret"))?,
            b"test-listener-secret"
        );
        Ok(())
    }
}
