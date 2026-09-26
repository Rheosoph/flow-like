use crate::{certificate_requests, certificates, state::StateStore, vault};
use anyhow::{Context, Result, ensure};
use flow_like_device_crypto::certificate_authority::{self, SignedCertificateChain};
use flow_like_device_protocol::{
    CertificateIssuerMetadata, CertificateRequestPurpose, CertificateSigningRequest, SecretValue,
    validate_certificate_id,
};
use rcgen::{
    CertificateParams, DistinguishedName, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub(crate) const SCHEMA: &str = "
CREATE TABLE certificate_issuers (
    certificate_id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    metadata_json TEXT,
    material_file TEXT,
    certificate_revision INTEGER,
    failures INTEGER NOT NULL DEFAULT 0,
    CHECK((metadata_json IS NULL) = (material_file IS NULL)),
    CHECK((metadata_json IS NULL) = (certificate_revision IS NULL))
);
";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssuerMaterial {
    certificate_id: String,
    revision: u64,
    certificate_chain_pem: SecretValue,
    private_key_pem: SecretValue,
}

pub(crate) fn create_request(
    store: &StateStore,
    root: &Path,
    request_id: &str,
    certificate_id: &str,
    expected_revision: u64,
    dns_names: &[String],
    ip_addresses: &[String],
    leaf_lifetime_days: u16,
    now: i64,
) -> Result<CertificateSigningRequest> {
    ensure!(
        (1..=397).contains(&leaf_lifetime_days),
        "Certificate lifetime must be 1 to 397 days"
    );
    let current = certificates::metadata(store, certificate_id)?;
    let names = certificate_requests::canonical_names(dns_names, ip_addresses)?;
    let current_names =
        certificate_requests::canonical_names(&current.dns_names, &current.ip_addresses)?;
    ensure!(
        names == current_names,
        "Renewal authority names must match the current service certificate"
    );
    for a in &names.0 {
        ensure!(
            !names
                .0
                .iter()
                .any(|b| a != b && a.ends_with(&format!(".{b}"))),
            "Delegated issuer names cannot contain overlapping parent and child hostnames"
        );
    }
    certificate_requests::create_with_purpose(
        store,
        root,
        request_id,
        certificate_id,
        &current.label,
        expected_revision,
        &names.0,
        &names.1,
        CertificateRequestPurpose::Issuer,
        Some(leaf_lifetime_days),
        now,
    )
}

pub(crate) fn list(store: &StateStore) -> Result<Vec<CertificateIssuerMetadata>> {
    let encoded = store.connection.prepare("SELECT metadata_json FROM certificate_issuers WHERE metadata_json IS NOT NULL ORDER BY certificate_id")?
        .query_map([], |row| row.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
    encoded
        .iter()
        .map(|item| serde_json::from_str(item).map_err(Into::into))
        .collect()
}

pub(crate) fn metadata(
    store: &StateStore,
    certificate_id: &str,
) -> Result<CertificateIssuerMetadata> {
    validate_certificate_id(certificate_id)?;
    let encoded: String = store.connection.query_row("SELECT metadata_json FROM certificate_issuers WHERE certificate_id=?1 AND metadata_json IS NOT NULL", [certificate_id], |row| row.get(0)).optional()?.context("Unknown certificate renewal authority")?;
    Ok(serde_json::from_str(&encoded)?)
}

pub(crate) fn install(
    store: &StateStore,
    root: &Path,
    request_id: &str,
    chain_pem: &str,
    now: i64,
) -> Result<CertificateIssuerMetadata> {
    let material = certificate_requests::load(store, root, request_id, now)?;
    let request = &material.request;
    ensure!(
        request.purpose == CertificateRequestPurpose::Issuer,
        "Expected a device issuing authority request"
    );
    let lifetime = request
        .leaf_lifetime_days
        .context("Missing renewal certificate lifetime")?;
    let (_, not_after) = certificate_authority::validate_device_issuer_chain(
        chain_pem,
        &material.private_key_pem.0,
        &request.dns_names,
        &request.ip_addresses,
        now,
    )?;
    ensure!(
        not_after > now + 3600,
        "Renewal authority must remain valid for at least one hour"
    );
    let current = certificates::metadata(store, &request.certificate_id)?;
    let previous: Option<u64> = store
        .connection
        .query_row(
            "SELECT revision FROM certificate_issuers WHERE certificate_id=?1",
            [&request.certificate_id],
            |row| row.get(0),
        )
        .optional()?;
    let revision = previous
        .unwrap_or(0)
        .checked_add(1)
        .context("Issuer revision exhausted")?;
    ensure!(
        revision <= 9_007_199_254_740_991,
        "Issuer revision exhausted"
    );
    let policy = CertificateIssuerMetadata {
        certificate_id: request.certificate_id.clone(),
        revision,
        dns_names: request.dns_names.clone(),
        ip_addresses: request.ip_addresses.clone(),
        leaf_lifetime_days: lifetime,
        not_after,
        last_renewed_at: None,
        next_renewal_at: renewal_at(current.not_before, current.not_after, now)
            .min(now + i64::from(lifetime) * 86_400 * 2 / 3)
            .min(not_after - 3600),
        last_error: None,
    };
    let identity = IssuerMaterial {
        certificate_id: request.certificate_id.clone(),
        revision,
        certificate_chain_pem: SecretValue(chain_pem.into()),
        private_key_pem: material.private_key_pem,
    };
    let file = format!("{}.json", uuid::Uuid::new_v4());
    vault::write_new_private(
        &certificates::material_path(root, &file)?,
        &Zeroizing::new(serde_json::to_vec(&identity)?),
    )?;
    store.connection.execute("INSERT INTO certificate_issuers(certificate_id,revision,metadata_json,material_file,certificate_revision) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(certificate_id) DO UPDATE SET revision=excluded.revision,metadata_json=excluded.metadata_json,material_file=excluded.material_file,certificate_revision=excluded.certificate_revision,failures=0", params![policy.certificate_id, revision, serde_json::to_string(&policy)?, file, current.revision])?;
    certificate_requests::delete(store, request_id)?;
    crate::acme::disable(store, &policy.certificate_id)?;
    Ok(policy)
}

fn renewal_at(not_before: i64, not_after: i64, now: i64) -> i64 {
    // One third of the issued lifetime remains for disconnected devices to retry.
    let lifetime = not_after.saturating_sub(not_before).max(1);
    (not_after - (lifetime / 3).clamp(3600, 30 * 24 * 3600)).max(now)
}

/// Explicit imports, deletion and revocation invalidate the old delegated signer.
pub(crate) fn disable(store: &StateStore, certificate_id: &str) -> Result<()> {
    let old: Option<u64> = store.connection.query_row("SELECT revision FROM certificate_issuers WHERE certificate_id=?1 AND metadata_json IS NOT NULL", [certificate_id], |row| row.get(0)).optional()?;
    if let Some(revision) = old {
        let revision = revision
            .checked_add(1)
            .context("Issuer revision exhausted")?;
        ensure!(
            revision <= 9_007_199_254_740_991,
            "Issuer revision exhausted"
        );
        store.connection.execute("UPDATE certificate_issuers SET revision=?2,metadata_json=NULL,material_file=NULL,certificate_revision=NULL,failures=0 WHERE certificate_id=?1", params![certificate_id, revision])?;
    }
    // A revoked delegation cannot be resurrected by an earlier outstanding request.
    store.connection.execute("DELETE FROM certificate_requests WHERE certificate_id=?1 AND json_extract(metadata_json,'$.purpose')='issuer'", [certificate_id])?;
    Ok(())
}

pub(crate) fn delete(
    store: &StateStore,
    certificate_id: &str,
    expected_revision: u64,
) -> Result<()> {
    ensure!(
        metadata(store, certificate_id)?.revision == expected_revision,
        "Renewal authority revision changed"
    );
    disable(store, certificate_id)
}

fn renew(
    store: &StateStore,
    root: &Path,
    policy: &mut CertificateIssuerMetadata,
    now: i64,
) -> Result<()> {
    ensure!(
        policy.not_after > now + 3600,
        "Certificate renewal authority is expired or expires within one hour"
    );
    let (file, expected_revision): (String, u64) = store.connection.query_row("SELECT material_file,certificate_revision FROM certificate_issuers WHERE certificate_id=?1 AND revision=?2 AND metadata_json IS NOT NULL", params![policy.certificate_id, policy.revision], |row| Ok((row.get(0)?, row.get(1)?)))?;
    certificate_requests::check_revision(store, &policy.certificate_id, expected_revision)?;
    let current = certificates::metadata(store, &policy.certificate_id)?;
    let identity: IssuerMaterial = serde_json::from_slice(&vault::read_private(
        &certificates::material_path(root, &file)?,
    )?)?;
    ensure!(
        identity.certificate_id == policy.certificate_id && identity.revision == policy.revision,
        "Renewal authority material does not match policy"
    );
    let key = Zeroizing::new(KeyPair::generate()?);
    let mut parameters = CertificateParams::new(
        policy
            .dns_names
            .iter()
            .chain(&policy.ip_addresses)
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    parameters.distinguished_name = DistinguishedName::new();
    parameters.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    parameters.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let csr = certificate_authority::CertificateSigningRequest {
        csr_pem: parameters.serialize_request(&*key)?.pem()?,
        dns_names: policy.dns_names.clone(),
        ip_addresses: policy.ip_addresses.clone(),
        validity_days: u32::from(policy.leaf_lifetime_days),
    };
    let SignedCertificateChain {
        certificate_chain_pem,
        not_before,
        not_after,
    } = certificate_authority::sign_csr_with_device_issuer(
        &identity.certificate_chain_pem.0,
        &identity.private_key_pem.0,
        &csr,
        now,
    )?;
    ensure!(
        not_after > now + 3600 && not_after <= policy.not_after,
        "Renewed certificate has an invalid lifetime"
    );
    let key_pem = Zeroizing::new(key.serialize_pem());
    let issued = certificates::put_issued(
        store,
        root,
        &policy.certificate_id,
        &current.label,
        expected_revision,
        &certificate_chain_pem,
        &key_pem,
        now,
    )?;
    policy.last_renewed_at = Some(now);
    policy.next_renewal_at = renewal_at(not_before, not_after, now).max(now + 60);
    policy.last_error = None;
    store.connection.execute("UPDATE certificate_issuers SET metadata_json=?2,certificate_revision=?3,failures=0 WHERE certificate_id=?1", params![policy.certificate_id, serde_json::to_string(policy)?, issued.revision])?;
    Ok(())
}

pub fn renew_due(root: &Path, now: i64) -> Result<usize> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let candidates = list(&store)?;
    let mut renewed = 0;
    for candidate in candidates {
        if candidate.next_renewal_at > now {
            continue;
        }
        let transaction = rusqlite::Transaction::new_unchecked(
            &store.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let Ok(mut policy) = metadata(&store, &candidate.certificate_id) else {
            continue;
        };
        if policy.next_renewal_at > now {
            continue;
        }
        store
            .connection
            .execute_batch("SAVEPOINT certificate_renewal")?;
        match renew(&store, root, &mut policy, now) {
            Ok(()) => {
                store
                    .connection
                    .execute_batch("RELEASE certificate_renewal")?;
                renewed += 1;
            }
            Err(_) => {
                store.connection.execute_batch(
                    "ROLLBACK TO certificate_renewal; RELEASE certificate_renewal",
                )?;
                policy = metadata(&store, &policy.certificate_id)?;
                let failures: u32 = store.connection.query_row(
                    "SELECT failures FROM certificate_issuers WHERE certificate_id=?1",
                    [&policy.certificate_id],
                    |r| r.get(0),
                )?;
                let failures = failures.saturating_add(1).min(16);
                policy.last_error = Some(if policy.not_after <= now + 3600 { "Issuing authority has expired or expires within one hour. Install a new delegation." } else { "Automatic certificate renewal failed. Check the issuing authority and device state." }.into());
                policy.next_renewal_at = now.saturating_add((60_i64 << failures.min(6)).min(3600));
                store.connection.execute("UPDATE certificate_issuers SET metadata_json=?2,failures=?3 WHERE certificate_id=?1", params![policy.certificate_id, serde_json::to_string(&policy)?, failures])?;
            }
        }
        transaction.commit()?;
    }
    store.connection.execute(
        "DELETE FROM certificate_requests WHERE expires_at<=?1",
        [now],
    )?;
    certificates::collect_unused(root)?;
    Ok(renewed)
}

pub async fn run(root: PathBuf, cancellation: CancellationToken) {
    loop {
        let path = root.clone();
        let result =
            tokio::task::spawn_blocking(move || renew_due(&path, crate::enrollment::unix_time()?))
                .await;
        if !matches!(result, Ok(Ok(_))) {
            tracing::warn!("Device certificate renewal pass failed");
        }
        tokio::select! {
            _ = cancellation.cancelled() => return,
            _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use certificate_authority::{CertificateAuthoritySpec, CertificateAuthorityVault};

    const PASSWORD: &[u8] = b"local-owner-test-password";

    fn authority(now: i64) -> Result<CertificateAuthorityVault> {
        certificate_authority::create_certificate_authority_vault(
            &CertificateAuthoritySpec {
                account_binding: "test-owner".into(),
                authority_id: uuid::Uuid::new_v4().to_string(),
                label: "Example enterprise".into(),
                dns_suffixes: vec!["example.test".into()],
                ip_addresses: vec!["127.0.0.1".into()],
                validity_days: 365,
            },
            PASSWORD,
            now,
        )
    }

    fn initial(store: &StateStore, root: &Path, id: &str, now: i64) -> Result<()> {
        let identity = rcgen::generate_simple_self_signed(vec![
            "api.example.test".into(),
            "127.0.0.1".into(),
        ])?;
        certificates::put(
            store,
            root,
            id,
            "API",
            0,
            &identity.cert.pem(),
            &identity.signing_key.serialize_pem(),
            now,
        )?;
        Ok(())
    }

    fn delegate(
        store: &StateStore,
        root: &Path,
        id: &str,
        revision: u64,
        vault: &CertificateAuthorityVault,
        now: i64,
    ) -> Result<CertificateIssuerMetadata> {
        let request = create_request(
            store,
            root,
            &uuid::Uuid::new_v4().to_string(),
            id,
            revision,
            &["api.example.test".into()],
            &["127.0.0.1".into()],
            30,
            now,
        )?;
        let signed = certificate_authority::sign_device_certificate_issuer(
            "test-owner",
            &vault.public_bundle.authority_id,
            PASSWORD,
            &vault.vault,
            &certificate_authority::CertificateSigningRequest {
                csr_pem: request.csr_pem.clone(),
                dns_names: request.dns_names.clone(),
                ip_addresses: request.ip_addresses.clone(),
                validity_days: 180,
            },
            now,
        )?;
        install(
            store,
            root,
            &request.request_id,
            &signed.certificate_chain_pem,
            now,
        )
    }

    fn force_due(store: &StateStore, policy: &CertificateIssuerMetadata, now: i64) -> Result<()> {
        let mut due = policy.clone();
        due.next_renewal_at = now;
        store.connection.execute(
            "UPDATE certificate_issuers SET metadata_json=?2 WHERE certificate_id=?1",
            params![due.certificate_id, serde_json::to_string(&due)?],
        )?;
        Ok(())
    }

    #[test]
    fn owner_delegation_renews_locally_and_manual_import_stops_it() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = crate::enrollment::unix_time()?;
        let id = uuid::Uuid::new_v4().to_string();
        initial(&store, &root, &id, now)?;
        let original = certificates::metadata(&store, &id)?;
        let vault = authority(now)?;
        let policy = delegate(&store, &root, &id, 1, &vault, now)?;
        assert_eq!(certificates::metadata(&store, &id)?, original);
        assert!(certificate_requests::list(&store)?.is_empty());
        let serialized = serde_json::to_string(&policy)?;
        assert!(!serialized.contains("PRIVATE KEY") && !serialized.contains("BEGIN CERTIFICATE"));
        let file: String = store.connection.query_row(
            "SELECT material_file FROM certificate_issuers",
            [],
            |row| row.get(0),
        )?;
        let issuer: IssuerMaterial = serde_json::from_slice(&vault::read_private(
            &certificates::material_path(&root, &file)?,
        )?)?;
        force_due(&store, &policy, now)?;
        assert_eq!(renew_due(&root, now)?, 1);
        let leaf = certificates::metadata(&store, &id)?;
        assert_eq!(leaf.revision, 2);
        assert_eq!(leaf.dns_names, ["api.example.test"]);
        assert_eq!(leaf.ip_addresses, ["127.0.0.1"]);
        assert_ne!(leaf.sha256_fingerprint, original.sha256_fingerprint);
        assert!(leaf.not_after <= now + 30 * 86_400 && leaf.not_after <= policy.not_after);
        let service_identity = certificates::load_identity(&root, &id)?;
        assert_ne!(service_identity.private_key_pem.0, issuer.private_key_pem.0);
        let renewed = metadata(&store, &id)?;
        assert_eq!(renewed.revision, policy.revision);
        assert_eq!(renewed.last_renewed_at, Some(now));
        assert!(renewed.last_error.is_none());
        assert_eq!(renew_due(&root, now + 1)?, 0);
        let imported = rcgen::generate_simple_self_signed(vec!["api.example.test".into()])?;
        certificates::put(
            &store,
            &root,
            &id,
            "Enterprise replacement",
            2,
            &imported.cert.pem(),
            &imported.signing_key.serialize_pem(),
            now,
        )?;
        assert!(list(&store)?.is_empty());
        assert_eq!(renew_due(&root, now + 31 * 86_400)?, 0);
        assert_eq!(
            certificates::metadata(&store, &id)?.label,
            "Enterprise replacement"
        );
        assert_eq!(std::fs::read_dir(root.join("certificates"))?.count(), 1);
        Ok(())
    }

    #[test]
    fn failed_renewal_preserves_leaf_and_records_backoff_then_recovers() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = crate::enrollment::unix_time()?;
        let id = uuid::Uuid::new_v4().to_string();
        initial(&store, &root, &id, now)?;
        let original = certificates::metadata(&store, &id)?;
        let vault = authority(now)?;
        let policy = delegate(&store, &root, &id, 1, &vault, now)?;
        force_due(&store, &policy, now)?;
        let file: String = store.connection.query_row(
            "SELECT material_file FROM certificate_issuers",
            [],
            |row| row.get(0),
        )?;
        let path = certificates::material_path(&root, &file)?;
        let bytes = vault::read_private(&path)?;
        std::fs::write(&path, b"damaged")?;
        assert_eq!(renew_due(&root, now)?, 0);
        assert_eq!(certificates::metadata(&store, &id)?, original);
        let failed = metadata(&store, &id)?;
        assert!(failed.last_error.is_some());
        assert!(failed.next_renewal_at >= now + 60);
        assert_eq!(renew_due(&root, now + 1)?, 0);
        std::fs::write(&path, &bytes)?;
        assert_eq!(renew_due(&root, failed.next_renewal_at)?, 1);
        let recovered = metadata(&store, &id)?;
        assert!(recovered.last_error.is_none());
        assert_eq!(certificates::metadata(&store, &id)?.revision, 2);
        Ok(())
    }

    #[test]
    fn revoked_delegation_keeps_leaf_and_cannot_reinstall_pending_issuer() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = crate::enrollment::unix_time()?;
        let id = uuid::Uuid::new_v4().to_string();
        initial(&store, &root, &id, now)?;
        let original = certificates::metadata(&store, &id)?;
        let vault = authority(now)?;
        let policy = delegate(&store, &root, &id, 1, &vault, now)?;
        create_request(
            &store,
            &root,
            &uuid::Uuid::new_v4().to_string(),
            &id,
            1,
            &["api.example.test".into()],
            &["127.0.0.1".into()],
            30,
            now,
        )?;
        assert!(delete(&store, &id, policy.revision + 1).is_err());
        delete(&store, &id, policy.revision)?;
        assert!(list(&store)?.is_empty());
        assert!(certificate_requests::list(&store)?.is_empty());
        assert_eq!(certificates::metadata(&store, &id)?, original);
        assert_eq!(renew_due(&root, now + 3600)?, 0);
        assert_eq!(std::fs::read_dir(root.join("certificates"))?.count(), 1);
        Ok(())
    }
}
