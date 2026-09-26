use crate::{state::StateStore, vault};
use anyhow::{Context, Result, bail, ensure};
use flow_like_device_protocol::{
    CertificateBinding, CertificateInventory, CertificateInventoryEntry, CertificateMetadata,
    MAX_CERTIFICATE_PEM_BYTES, MAX_DEVICE_CERTIFICATES, SecretValue, validate_certificate_id,
};
use rusqlite::{OptionalExtension, params};
use rustls::sign::CertifiedKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::Cursor,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use x509_parser::{extensions::GeneralName, prelude::*};
use zeroize::Zeroizing;

pub(crate) const SCHEMA: &str = "
CREATE TABLE device_certificates (
    certificate_id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    metadata_json TEXT,
    material_file TEXT,
    CHECK((metadata_json IS NULL) = (material_file IS NULL))
);
CREATE TABLE certificate_inventory (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    revision INTEGER NOT NULL CHECK(revision > 0)
);
INSERT INTO certificate_inventory(singleton,revision) VALUES(1,1);
";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateIdentity {
    pub revision: u64,
    pub certificate_chain_pem: SecretValue,
    pub private_key_pem: SecretValue,
    pub not_before: i64,
    pub not_after: i64,
}

fn directory(root: &Path) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    crate::config::ensure_unaliased_child(&root, "certificates")?;
    crate::supervisor::prepare_state_dir(&root.join("certificates"))
}

pub(crate) fn material_path(root: &Path, name: &str) -> Result<PathBuf> {
    let id = name
        .strip_suffix(".json")
        .context("Invalid certificate material filename")?;
    validate_certificate_id(id)?;
    let directory = directory(root)?;
    crate::config::ensure_unaliased_child(&directory, name)?;
    Ok(directory.join(name))
}

pub(crate) fn validate_material(
    certificate_id: &str,
    label: &str,
    revision: u64,
    chain_pem: &str,
    key_pem: &str,
    now: i64,
) -> Result<(CertificateMetadata, Arc<CertifiedKey>)> {
    validate_certificate_id(certificate_id)?;
    ensure!(
        !label.trim().is_empty() && label.len() <= 128 && !label.chars().any(char::is_control),
        "Certificate label must contain 1 to 128 printable bytes"
    );
    ensure!(
        !chain_pem.is_empty()
            && !key_pem.is_empty()
            && chain_pem.len().saturating_add(key_pem.len()) <= MAX_CERTIFICATE_PEM_BYTES,
        "Certificate chain and private key exceed the 12 KiB management limit"
    );
    let mut chain = Vec::new();
    for item in rustls_pemfile::read_all(&mut Cursor::new(chain_pem.as_bytes())) {
        match item.context("Invalid certificate PEM")? {
            rustls_pemfile::Item::X509Certificate(cert) => chain.push(cert),
            _ => bail!("Certificate chain must contain only CERTIFICATE blocks"),
        }
    }
    ensure!(
        !chain.is_empty() && chain.len() <= 8,
        "Certificate chain must contain 1 to 8 certificates"
    );
    let mut keys = Vec::new();
    for item in rustls_pemfile::read_all(&mut Cursor::new(key_pem.as_bytes())) {
        keys.push(match item.context("Invalid private key PEM")? {
            rustls_pemfile::Item::Pkcs1Key(key) => rustls::pki_types::PrivateKeyDer::Pkcs1(key),
            rustls_pemfile::Item::Pkcs8Key(key) => rustls::pki_types::PrivateKeyDer::Pkcs8(key),
            rustls_pemfile::Item::Sec1Key(key) => rustls::pki_types::PrivateKeyDer::Sec1(key),
            _ => bail!("Private key PEM must contain only one unencrypted private key"),
        });
    }
    ensure!(
        keys.len() == 1,
        "Exactly one unencrypted private key is required"
    );
    let parsed = chain
        .iter()
        .map(|der| {
            let (rest, cert) = X509Certificate::from_der(der.as_ref())
                .map_err(|_| anyhow::anyhow!("Invalid X.509 certificate"))?;
            ensure!(rest.is_empty(), "Trailing data in X.509 certificate");
            Ok(cert)
        })
        .collect::<Result<Vec<_>>>()?;
    let leaf = &parsed[0];
    for certificate in &parsed {
        certificate.extensions_map()?;
        for extension in certificate.extensions() {
            ensure!(
                !matches!(
                    extension.parsed_extension(),
                    x509_parser::extensions::ParsedExtension::ParseError { .. }
                ) && !(extension.critical && extension.parsed_extension().unsupported()),
                "Certificate has an invalid or unsupported critical extension"
            );
        }
        ensure!(
            certificate
                .extended_key_usage()?
                .is_none_or(|e| e.value.server_auth || e.value.any),
            "Certificate chain does not permit TLS server authentication"
        );
    }
    let not_before = parsed
        .iter()
        .map(|c| c.validity().not_before.timestamp())
        .max()
        .unwrap();
    let not_after = parsed
        .iter()
        .map(|c| c.validity().not_after.timestamp())
        .min()
        .unwrap();
    ensure!(
        not_before <= now && not_after > now,
        "Certificate chain is expired or not yet valid"
    );
    ensure!(
        !leaf.basic_constraints()?.is_some_and(|e| e.value.ca),
        "A CA certificate cannot be used as a service identity"
    );
    ensure!(
        leaf.extended_key_usage()?
            .is_none_or(|e| e.value.server_auth || e.value.any),
        "Certificate does not permit TLS server authentication"
    );
    ensure!(
        leaf.key_usage()?
            .is_none_or(|e| e.value.digital_signature()),
        "Certificate does not permit digital signatures"
    );
    for (index, pair) in parsed.windows(2).enumerate() {
        ensure!(
            pair[0].issuer() == pair[1].subject(),
            "Certificate chain is not in leaf-to-issuer order"
        );
        let constraints = pair[1]
            .basic_constraints()?
            .context("Certificate issuer has no CA constraints")?;
        ensure!(constraints.value.ca, "Certificate issuer is not a CA");
        ensure!(
            constraints
                .value
                .path_len_constraint
                .is_none_or(|max| index as u32 <= max),
            "Certificate issuer path length is exceeded"
        );
        ensure!(
            pair[1].key_usage()?.is_none_or(|e| e.value.key_cert_sign()),
            "Certificate issuer cannot sign certificates"
        );
        pair[0]
            .verify_signature(Some(pair[1].public_key()))
            .map_err(|_| anyhow::anyhow!("Certificate chain signature is invalid"))?;
    }
    let last = parsed.last().unwrap();
    if last.subject() == last.issuer() {
        last.verify_signature(None)
            .map_err(|_| anyhow::anyhow!("Self-signed certificate signature is invalid"))?;
    }
    let mut dns_names = BTreeSet::new();
    let mut ip_addresses = BTreeSet::new();
    if let Some(sans) = leaf.subject_alternative_name()? {
        ensure!(
            sans.value.general_names.len() <= 64,
            "Certificate has too many subject alternative names"
        );
        for san in &sans.value.general_names {
            match san {
                GeneralName::DNSName(name) => {
                    ensure!(
                        !name.is_empty()
                            && name.len() <= 253
                            && name.is_ascii()
                            && !name.chars().any(char::is_control),
                        "Invalid DNS subject alternative name"
                    );
                    dns_names.insert((*name).to_owned());
                }
                GeneralName::IPAddress(bytes) => {
                    let address = match bytes.len() {
                        4 => IpAddr::from(<[u8; 4]>::try_from(*bytes)?),
                        16 => IpAddr::from(<[u8; 16]>::try_from(*bytes)?),
                        _ => bail!("Invalid IP subject alternative name"),
                    };
                    ip_addresses.insert(address.to_string());
                }
                _ => (),
            }
        }
    }
    ensure!(
        !dns_names.is_empty() || !ip_addresses.is_empty(),
        "Service certificate must have a DNS name or IP subject alternative name"
    );
    let metadata = CertificateMetadata {
        certificate_id: certificate_id.into(),
        label: label.into(),
        revision,
        subject: leaf.subject().to_string(),
        issuer: leaf.issuer().to_string(),
        dns_names: dns_names.into_iter().collect(),
        ip_addresses: ip_addresses.into_iter().collect(),
        sha256_fingerprint: format!("{:x}", Sha256::digest(chain[0].as_ref())),
        not_before,
        not_after,
        binding_count: 0,
        bindings: vec![],
    };
    ensure!(
        serde_json::to_vec(&metadata)?.len() <= 8 * 1024,
        "Certificate metadata exceeds its encrypted response limit"
    );
    let key = CertifiedKey::from_der(
        chain,
        keys.pop().unwrap(),
        &rustls::crypto::ring::default_provider(),
    )
    .context("Unsupported or mismatched TLS private key")?;
    // Unknown key consistency is rejected, even if a provider would accept it.
    key.keys_match()
        .context("Certificate and private key do not match")?;
    Ok((metadata, Arc::new(key)))
}

pub fn inventory_revision(store: &StateStore) -> Result<u64> {
    Ok(store.connection.query_row(
        "SELECT revision FROM certificate_inventory WHERE singleton=1",
        [],
        |r| r.get(0),
    )?)
}

fn advance_inventory(store: &StateStore) -> Result<u64> {
    let next = inventory_revision(store)?
        .checked_add(1)
        .context("Certificate inventory revision exhausted")?;
    ensure!(
        next <= 9_007_199_254_740_991,
        "Certificate inventory revision exhausted"
    );
    store.connection.execute(
        "UPDATE certificate_inventory SET revision=?1 WHERE singleton=1",
        [next],
    )?;
    Ok(next)
}

pub fn metadata(store: &StateStore, id: &str) -> Result<CertificateMetadata> {
    validate_certificate_id(id)?;
    let json: String = store.connection.query_row("SELECT metadata_json FROM device_certificates WHERE certificate_id=?1 AND metadata_json IS NOT NULL", [id], |r| r.get(0)).optional()?.context("Unknown device certificate")?;
    let mut metadata: CertificateMetadata = serde_json::from_str(&json)?;
    metadata.bindings = bindings(store, id)?;
    metadata.binding_count = metadata.bindings.len();
    Ok(metadata)
}

pub fn list(store: &StateStore) -> Result<Vec<CertificateMetadata>> {
    let ids = store.connection.prepare("SELECT certificate_id FROM device_certificates WHERE metadata_json IS NOT NULL ORDER BY certificate_id")?
        .query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|id| metadata(store, id)).collect()
}

pub fn bindings(store: &StateStore, id: &str) -> Result<Vec<CertificateBinding>> {
    let mut found = BTreeSet::new();
    let mut inspect = |value: serde_json::Value| {
        if value
            .get("tls_certificate_id")
            .and_then(serde_json::Value::as_str)
            == Some(id)
        {
            if let (Some(placement), Some(project)) = (
                value.get("id").and_then(serde_json::Value::as_str),
                value.get("project_id").and_then(serde_json::Value::as_str),
            ) {
                found.insert((placement.to_owned(), project.to_owned()));
            }
        }
    };
    let mut statement = store.connection.prepare("SELECT config_json FROM placements UNION ALL SELECT previous_config_json FROM placement_rollouts WHERE state IN ('staged','validating','activating','rolling_back') UNION ALL SELECT candidate_config_json FROM placement_rollouts WHERE state IN ('staged','validating','activating','rolling_back')")?;
    for item in statement.query_map([], |r| r.get::<_, String>(0))? {
        inspect(serde_json::from_str(&item?)?);
    }
    Ok(found
        .into_iter()
        .map(|(placement_id, project_id)| CertificateBinding {
            placement_id,
            project_id,
            service: "https".into(),
        })
        .collect())
}

/// The caller holds the management write transaction, including its durable receipt.
pub(crate) fn put(
    store: &StateStore,
    root: &Path,
    id: &str,
    label: &str,
    expected_revision: u64,
    chain_pem: &str,
    key_pem: &str,
    now: i64,
) -> Result<CertificateMetadata> {
    let result = put_issued(
        store,
        root,
        id,
        label,
        expected_revision,
        chain_pem,
        key_pem,
        now,
    )?;
    crate::certificate_issuers::disable(store, id)?;
    crate::acme::disable(store, id)?;
    Ok(result)
}

pub(crate) fn put_issued(
    store: &StateStore,
    root: &Path,
    id: &str,
    label: &str,
    expected_revision: u64,
    chain_pem: &str,
    key_pem: &str,
    now: i64,
) -> Result<CertificateMetadata> {
    validate_certificate_id(id)?;
    let old: Option<(u64, bool)> = store.connection.query_row("SELECT revision,metadata_json IS NOT NULL FROM device_certificates WHERE certificate_id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    ensure!(
        old.as_ref().map_or(0, |v| v.0) == expected_revision,
        "Certificate revision changed"
    );
    ensure!(
        old.as_ref().is_none_or(|v| v.1),
        "Deleted certificate IDs cannot be reused"
    );
    let count: usize = store.connection.query_row(
        "SELECT COUNT(*) FROM (SELECT certificate_id FROM device_certificates WHERE metadata_json IS NOT NULL UNION SELECT certificate_id FROM certificate_requests UNION SELECT certificate_id FROM certificate_acme WHERE metadata_json IS NOT NULL UNION SELECT ?1)",
        [id],
        |r| r.get(0),
    )?;
    ensure!(
        count <= MAX_DEVICE_CERTIFICATES,
        "Device certificate limit reached"
    );
    let revision = expected_revision
        .checked_add(1)
        .context("Certificate revision exhausted")?;
    ensure!(
        revision <= 9_007_199_254_740_991,
        "Certificate revision exhausted"
    );
    let (mut metadata, _) = validate_material(id, label, revision, chain_pem, key_pem, now)?;
    let identity = CertificateIdentity {
        revision,
        certificate_chain_pem: SecretValue(chain_pem.into()),
        private_key_pem: SecretValue(key_pem.into()),
        not_before: metadata.not_before,
        not_after: metadata.not_after,
    };
    let file = format!("{}.json", uuid::Uuid::new_v4());
    let encoded = Zeroizing::new(serde_json::to_vec(&identity)?);
    // New immutable material is synced before the atomic SQLite pointer and receipt commit.
    // A rolled-back transaction leaves only an unreferenced file for collect_unused.
    vault::write_new_private(&material_path(root, &file)?, &encoded)?;
    store.connection.execute("INSERT INTO device_certificates(certificate_id,revision,metadata_json,material_file) VALUES(?1,?2,?3,?4) ON CONFLICT(certificate_id) DO UPDATE SET revision=excluded.revision,metadata_json=excluded.metadata_json,material_file=excluded.material_file", params![id, revision, serde_json::to_string(&metadata)?, file])?;
    advance_inventory(store)?;
    metadata.bindings = bindings(store, id)?;
    metadata.binding_count = metadata.bindings.len();
    Ok(metadata)
}

pub(crate) fn bounded_metadata(mut value: CertificateMetadata) -> Result<CertificateMetadata> {
    value.binding_count = value.bindings.len();
    value.bindings.truncate(32);
    while serde_json::to_vec(&value)?.len() > 12 * 1024 {
        ensure!(
            value.bindings.pop().is_some(),
            "Certificate metadata exceeds its encrypted response limit"
        );
    }
    Ok(value)
}

pub(crate) fn delete(store: &StateStore, id: &str, expected_revision: u64) -> Result<u64> {
    let current = metadata(store, id)?;
    ensure!(
        current.revision == expected_revision,
        "Certificate revision changed"
    );
    ensure!(
        current.bindings.is_empty(),
        "Certificate is in use; remove all placement and rollout bindings first"
    );
    crate::certificate_issuers::disable(store, id)?;
    store.connection.execute("UPDATE device_certificates SET metadata_json=NULL,material_file=NULL WHERE certificate_id=?1", [id])?;
    crate::acme::disable(store, id)?;
    advance_inventory(store)
}

pub fn validate_binding(store: &StateStore, root: &Path, id: &str, now: i64) -> Result<()> {
    let value = metadata(store, id)?;
    ensure!(
        value.not_before <= now && value.not_after > now,
        "Selected certificate is expired or not yet valid"
    );
    load_identity_from_store(store, root, id).map(|_| ())
}

fn load_identity_from_store(
    store: &StateStore,
    root: &Path,
    id: &str,
) -> Result<CertificateIdentity> {
    validate_certificate_id(id)?;
    let (json, file): (String, String) = store.connection.query_row("SELECT metadata_json,material_file FROM device_certificates WHERE certificate_id=?1 AND metadata_json IS NOT NULL", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?.context("Unknown device certificate")?;
    let metadata: CertificateMetadata = serde_json::from_str(&json)?;
    let bytes = vault::read_private(&material_path(root, &file)?)?;
    let identity: CertificateIdentity =
        serde_json::from_slice(&bytes).context("Invalid private certificate identity")?;
    ensure!(
        identity.revision == metadata.revision
            && identity.not_before == metadata.not_before
            && identity.not_after == metadata.not_after,
        "Certificate material revision mismatch"
    );
    let (checked, _) = validate_material(
        id,
        &metadata.label,
        identity.revision,
        &identity.certificate_chain_pem.0,
        &identity.private_key_pem.0,
        crate::enrollment::unix_time()?,
    )?;
    ensure!(
        checked == metadata,
        "Certificate material does not match its metadata"
    );
    Ok(identity)
}

pub fn load_identity(root: &Path, id: &str) -> Result<CertificateIdentity> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let transaction = rusqlite::Transaction::new_unchecked(
        &store.connection,
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let identity = load_identity_from_store(&store, root, id)?;
    transaction.commit()?;
    Ok(identity)
}

pub fn load_certified_key(root: &Path, id: &str) -> Result<(u64, Arc<CertifiedKey>)> {
    let identity = load_identity(root, id)?;
    let (_, key) = validate_material(
        id,
        "service",
        identity.revision,
        &identity.certificate_chain_pem.0,
        &identity.private_key_pem.0,
        crate::enrollment::unix_time()?,
    )?;
    Ok((identity.revision, key))
}

pub fn inventory(store: &StateStore, now: i64) -> Result<CertificateInventory> {
    let transaction = if store.connection.is_autocommit() {
        Some(rusqlite::Transaction::new_unchecked(
            &store.connection,
            rusqlite::TransactionBehavior::Deferred,
        )?)
    } else {
        None
    };
    let device_id = store
        .registration()?
        .map(|registration| registration.manifest.device_id)
        .unwrap_or_else(|| store.device_id().to_owned());
    let certificates = store.connection.prepare("SELECT metadata_json FROM device_certificates WHERE metadata_json IS NOT NULL ORDER BY certificate_id")?
        .query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?
        .iter().map(|value| serde_json::from_str::<CertificateMetadata>(value)).collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|item| CertificateInventoryEntry {
            certificate_id: item.certificate_id,
            revision: item.revision,
            fingerprint_sha256: item.sha256_fingerprint,
            not_after: item.not_after,
        })
        .collect();
    let inventory = CertificateInventory {
        version: 1,
        device_id,
        revision: inventory_revision(store)?,
        issued_at: now,
        certificates,
    };
    if let Some(transaction) = transaction {
        transaction.commit()?;
    }
    Ok(inventory)
}

/// Call after committing or rolling back a management command. Never unlink live material.
pub fn collect_unused(root: &Path) -> Result<()> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let transaction = rusqlite::Transaction::new_unchecked(
        &store.connection,
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let referenced = store
        .connection
        .prepare("SELECT material_file FROM device_certificates WHERE material_file IS NOT NULL UNION SELECT material_file FROM certificate_requests UNION SELECT material_file FROM certificate_issuers WHERE material_file IS NOT NULL UNION SELECT account_file FROM certificate_acme WHERE account_file IS NOT NULL UNION SELECT key_file FROM certificate_acme WHERE key_file IS NOT NULL")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<BTreeSet<_>, _>>()?;
    let directory = directory(root)?;
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !referenced.contains(&name)
            && name
                .strip_suffix(".json")
                .is_some_and(|id| validate_certificate_id(id).is_ok())
        {
            ensure!(
                entry.file_type()?.is_file(),
                "Certificate material must be a regular file"
            );
            std::fs::remove_file(entry.path())?;
        }
    }
    std::fs::File::open(directory)?.sync_all()?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{
        BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa,
        KeyPair, KeyUsagePurpose,
    };

    pub(crate) fn identity() -> Result<(String, String)> {
        let key = KeyPair::generate()?;
        let mut params =
            CertificateParams::new(vec!["service.example.test".into(), "127.0.0.1".into()])?;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        Ok((params.self_signed(&key)?.pem(), key.serialize_pem()))
    }

    #[test]
    fn service_identity_validates_key_usage_chain_and_effective_expiration() -> Result<()> {
        let now = crate::enrollment::unix_time()?;
        let id = uuid::Uuid::new_v4().to_string();
        let (chain, key) = identity()?;
        let (metadata, _) = validate_material(&id, "REST", 1, &chain, &key, now)?;
        assert_eq!(metadata.dns_names, ["service.example.test"]);
        assert_eq!(metadata.ip_addresses, ["127.0.0.1"]);
        assert_eq!(metadata.sha256_fingerprint.len(), 64);
        assert!(validate_material(&id, "REST", 1, &chain, &identity()?.1, now).is_err());
        assert!(validate_material("../escape", "REST", 1, &chain, &key, now).is_err());
        assert!(validate_material(&id, "REST", 1, &chain, &(key.clone() + &key), now).is_err());
        let key = KeyPair::generate()?;
        let mut params = CertificateParams::new(vec!["service.example.test".into()])?;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        assert!(
            validate_material(
                &id,
                "client-only",
                1,
                &params.self_signed(&key)?.pem(),
                &key.serialize_pem(),
                now
            )
            .is_err()
        );
        params.extended_key_usages.clear();
        params.not_after = rcgen::date_time_ymd(2000, 1, 1);
        assert!(
            validate_material(
                &id,
                "expired",
                1,
                &params.self_signed(&key)?.pem(),
                &key.serialize_pem(),
                now
            )
            .is_err()
        );
        params.not_before = rcgen::date_time_ymd(4095, 1, 1);
        params.not_after = rcgen::date_time_ymd(4096, 1, 1);
        assert!(
            validate_material(
                &id,
                "future",
                1,
                &params.self_signed(&key)?.pem(),
                &key.serialize_pem(),
                now
            )
            .is_err()
        );
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        ca_params.not_after = rcgen::date_time_ymd(2030, 1, 1);
        let ca_expiry = ca_params.not_after.unix_timestamp();
        let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate()?)?;
        let leaf =
            CertificateParams::new(vec!["service.example.test".into()])?.signed_by(&key, &ca)?;
        let chain = leaf.pem() + &ca.pem();
        assert_eq!(
            validate_material(&id, "chain", 1, &chain, &key.serialize_pem(), now)?
                .0
                .not_after,
            ca_expiry
        );
        assert!(
            validate_material(
                &id,
                "reversed",
                1,
                &(ca.pem() + &leaf.pem()),
                &key.serialize_pem(),
                now
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn certificate_rotation_is_atomic_private_and_revision_guarded() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = crate::enrollment::unix_time()?;
        let (chain, key) = identity()?;
        store.connection.execute_batch("BEGIN IMMEDIATE")?;
        let first = put(&store, &root, &id, "REST", 0, &chain, &key, now)?;
        store.connection.execute_batch("COMMIT")?;
        assert_eq!(load_identity(&root, &id)?.revision, 1);
        assert_eq!(inventory(&store, now)?.revision, 2);
        assert!(put(&store, &root, &id, "REST", 0, &chain, &key, now).is_err());
        let (next_chain, next_key) = identity()?;
        store.connection.execute_batch("BEGIN IMMEDIATE")?;
        put(&store, &root, &id, "REST", 1, &next_chain, &next_key, now)?;
        store.connection.execute_batch("ROLLBACK")?;
        collect_unused(&root)?;
        assert_eq!(load_identity(&root, &id)?.revision, 1);
        assert_eq!(
            metadata(&store, &id)?.sha256_fingerprint,
            first.sha256_fingerprint
        );
        assert_eq!(std::fs::read_dir(directory(&root)?)?.count(), 1);
        store.connection.execute_batch("BEGIN IMMEDIATE")?;
        put(&store, &root, &id, "REST", 1, &next_chain, &next_key, now)?;
        store.connection.execute_batch("COMMIT")?;
        collect_unused(&root)?;
        assert_eq!(load_identity(&root, &id)?.revision, 2);
        assert_ne!(
            metadata(&store, &id)?.sha256_fingerprint,
            first.sha256_fingerprint
        );
        assert_eq!(std::fs::read_dir(directory(&root)?)?.count(), 1);
        let saved: String = store.connection.query_row(
            "SELECT metadata_json FROM device_certificates",
            [],
            |r| r.get(0),
        )?;
        assert!(!saved.contains("PRIVATE KEY") && !saved.contains("BEGIN CERTIFICATE"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let entry = std::fs::read_dir(directory(&root)?)?.next().unwrap()?;
            assert_eq!(entry.metadata()?.permissions().mode() & 0o777, 0o600);
        }
        store.connection.execute_batch("BEGIN IMMEDIATE")?;
        delete(&store, &id, 2)?;
        store.connection.execute_batch("COMMIT")?;
        collect_unused(&root)?;
        assert!(load_identity(&root, &id).is_err());
        assert!(inventory(&store, now)?.certificates.is_empty());
        assert_eq!(inventory(&store, now)?.revision, 4);
        assert_eq!(std::fs::read_dir(directory(&root)?)?.count(), 0);
        assert!(put(&store, &root, &id, "reused", 2, &chain, &key, now).is_err());
        Ok(())
    }

    #[test]
    fn certificate_material_rejects_file_substitution_and_symlinks() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let id = uuid::Uuid::new_v4().to_string();
        let (chain, key) = identity()?;
        put(
            &store,
            &root,
            &id,
            "REST",
            0,
            &chain,
            &key,
            crate::enrollment::unix_time()?,
        )?;
        let file: String = store.connection.query_row(
            "SELECT material_file FROM device_certificates",
            [],
            |r| r.get(0),
        )?;
        let path = material_path(&root, &file)?;
        let bytes = vault::read_private(&path)?;
        let mut value: CertificateIdentity = serde_json::from_slice(&bytes)?;
        let (other_chain, other_key) = identity()?;
        value.certificate_chain_pem = SecretValue(other_chain);
        value.private_key_pem = SecretValue(other_key);
        std::fs::write(&path, serde_json::to_vec(&value)?)?;
        assert!(load_identity(&root, &id).is_err());
        #[cfg(unix)]
        {
            let other = root.join("outside.json");
            vault::write_new_private(&other, &bytes)?;
            std::fs::remove_file(&path)?;
            std::os::unix::fs::symlink(&other, &path)?;
            assert!(load_identity(&root, &id).is_err());
        }
        Ok(())
    }
}
