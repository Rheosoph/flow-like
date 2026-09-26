use crate::{certificates, state::StateStore, vault};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::{
    CertificateMetadata, CertificateRequestPurpose, CertificateSigningRequest,
    MAX_DEVICE_CERTIFICATES, SecretValue, validate_certificate_id,
};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::IpAddr, path::Path};
use zeroize::Zeroizing;

pub(crate) const SCHEMA: &str = "
CREATE TABLE certificate_requests (
    request_id TEXT PRIMARY KEY NOT NULL,
    certificate_id TEXT UNIQUE NOT NULL,
    metadata_json TEXT NOT NULL,
    material_file TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
";
const REQUEST_LIFETIME: i64 = 30 * 24 * 60 * 60;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestMaterial {
    pub(crate) request: CertificateSigningRequest,
    pub(crate) private_key_pem: SecretValue,
}

pub(crate) fn canonical_names(
    dns_names: &[String],
    ip_addresses: &[String],
) -> Result<(Vec<String>, Vec<String>)> {
    ensure!(
        !dns_names.is_empty() || !ip_addresses.is_empty(),
        "Request at least one DNS name or IP address"
    );
    ensure!(
        dns_names.len().saturating_add(ip_addresses.len()) <= 32,
        "Certificate requests support at most 32 names"
    );
    let mut dns = BTreeSet::new();
    for name in dns_names {
        let name = name.to_ascii_lowercase();
        ensure!(
            !name.is_empty()
                && name.len() <= 253
                && name.parse::<IpAddr>().is_err()
                && name.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && label.as_bytes()[0].is_ascii_alphanumeric()
                        && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                        && label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                }),
            "Certificate DNS names must be ASCII hostnames without wildcards or trailing dots"
        );
        dns.insert(name);
    }
    let ips = ip_addresses
        .iter()
        .map(|address| address.parse::<IpAddr>().map(|address| address.to_string()))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .context("Invalid certificate IP address")?;
    ensure!(
        dns.iter().map(String::len).sum::<usize>() + ips.iter().map(String::len).sum::<usize>()
            <= 2048,
        "Certificate request names exceed 2 KiB"
    );
    Ok((dns.into_iter().collect(), ips.into_iter().collect()))
}

pub(crate) fn check_revision(store: &StateStore, id: &str, expected_revision: u64) -> Result<()> {
    validate_certificate_id(id)?;
    let current: Option<(u64, bool)> = store
        .connection
        .query_row(
            "SELECT revision, metadata_json IS NOT NULL FROM device_certificates WHERE certificate_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    ensure!(
        current.as_ref().map_or(0, |value| value.0) == expected_revision,
        "Certificate revision changed; create a new signing request"
    );
    ensure!(
        current.is_none_or(|value| value.1),
        "Deleted certificate IDs cannot be reused"
    );
    Ok(())
}

pub(crate) fn create(
    store: &StateStore,
    root: &Path,
    request_id: &str,
    certificate_id: &str,
    label: &str,
    expected_revision: u64,
    dns_names: &[String],
    ip_addresses: &[String],
    now: i64,
) -> Result<CertificateSigningRequest> {
    create_with_purpose(
        store,
        root,
        request_id,
        certificate_id,
        label,
        expected_revision,
        dns_names,
        ip_addresses,
        CertificateRequestPurpose::Service,
        None,
        now,
    )
}

pub(crate) fn create_with_purpose(
    store: &StateStore,
    root: &Path,
    request_id: &str,
    certificate_id: &str,
    label: &str,
    expected_revision: u64,
    dns_names: &[String],
    ip_addresses: &[String],
    purpose: CertificateRequestPurpose,
    leaf_lifetime_days: Option<u16>,
    now: i64,
) -> Result<CertificateSigningRequest> {
    validate_certificate_id(request_id)?;
    check_revision(store, certificate_id, expected_revision)?;
    ensure!(
        !label.trim().is_empty() && label.len() <= 128 && !label.chars().any(char::is_control),
        "Certificate label must contain 1 to 128 printable bytes"
    );
    let (dns_names, ip_addresses) = canonical_names(dns_names, ip_addresses)?;
    store.connection.execute(
        "DELETE FROM certificate_requests WHERE expires_at<=?1",
        [now],
    )?;
    let count: usize =
        store
            .connection
            .query_row("SELECT COUNT(*) FROM certificate_requests", [], |r| {
                r.get(0)
            })?;
    ensure!(
        count < MAX_DEVICE_CERTIFICATES,
        "Pending certificate request limit reached"
    );
    let exists: bool = store.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM certificate_requests WHERE certificate_id=?1 OR request_id=?2)",
        params![certificate_id, request_id], |r| r.get(0))?;
    ensure!(
        !exists,
        "Cancel the existing signing request before creating another"
    );
    let count: usize = store.connection.query_row(
        "SELECT COUNT(*) FROM (SELECT certificate_id FROM device_certificates WHERE metadata_json IS NOT NULL UNION SELECT certificate_id FROM certificate_requests UNION SELECT certificate_id FROM certificate_acme WHERE metadata_json IS NOT NULL UNION SELECT ?1)",
        [certificate_id], |r| r.get(0))?;
    ensure!(
        count <= MAX_DEVICE_CERTIFICATES,
        "Device certificate limit reached"
    );
    let key = Zeroizing::new(KeyPair::generate()?);
    let mut parameters = CertificateParams::new(
        dns_names
            .iter()
            .chain(&ip_addresses)
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    parameters.distinguished_name = DistinguishedName::new();
    parameters.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    parameters.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    if purpose == CertificateRequestPurpose::Issuer {
        parameters.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        parameters.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    }
    let csr_pem = parameters.serialize_request(&*key)?.pem()?;
    let request = CertificateSigningRequest {
        request_id: request_id.into(),
        purpose,
        leaf_lifetime_days,
        certificate_id: certificate_id.into(),
        label: label.into(),
        expected_revision,
        dns_names,
        ip_addresses,
        csr_pem,
        created_at: now,
        expires_at: now
            .checked_add(REQUEST_LIFETIME)
            .context("Invalid request timestamp")?,
    };
    ensure!(
        serde_json::to_vec(&request)?.len() <= 10 * 1024,
        "Certificate request exceeds management response limit"
    );
    let material = RequestMaterial {
        request: request.clone(),
        private_key_pem: SecretValue(key.serialize_pem()),
    };
    let file = format!("{}.json", uuid::Uuid::new_v4());
    vault::write_new_private(
        &certificates::material_path(root, &file)?,
        &Zeroizing::new(serde_json::to_vec(&material)?),
    )?;
    store.connection.execute(
        "INSERT INTO certificate_requests(request_id,certificate_id,metadata_json,material_file,expires_at) VALUES(?1,?2,?3,?4,?5)",
        params![request_id, certificate_id, serde_json::to_string(&request)?, file, request.expires_at])?;
    if purpose == CertificateRequestPurpose::Service {
        crate::certificate_issuers::disable(store, certificate_id)?;
        crate::acme::disable(store, certificate_id)?;
    }
    Ok(request)
}

pub(crate) fn list(store: &StateStore) -> Result<Vec<CertificateSigningRequest>> {
    let encoded = store
        .connection
        .prepare("SELECT metadata_json FROM certificate_requests ORDER BY request_id")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    encoded
        .iter()
        .map(|item| serde_json::from_str(item).map_err(Into::into))
        .collect()
}

pub(crate) fn delete(store: &StateStore, request_id: &str) -> Result<()> {
    validate_certificate_id(request_id)?;
    ensure!(
        store.connection.execute(
            "DELETE FROM certificate_requests WHERE request_id=?1",
            [request_id]
        )? == 1,
        "Unknown certificate signing request"
    );
    Ok(())
}

pub(crate) fn install(
    store: &StateStore,
    root: &Path,
    request_id: &str,
    chain_pem: &str,
    now: i64,
) -> Result<CertificateMetadata> {
    let material = load(store, root, request_id, now)?;
    let request = &material.request;
    ensure!(
        request.purpose == CertificateRequestPurpose::Service,
        "Issuer requests must be installed as a renewal authority"
    );
    let (checked, _) = certificates::validate_material(
        &request.certificate_id,
        &request.label,
        request
            .expected_revision
            .checked_add(1)
            .context("Certificate revision exhausted")?,
        chain_pem,
        &material.private_key_pem.0,
        now,
    )?;
    validate_identity_types(chain_pem)?;
    let names = canonical_names(&checked.dns_names, &checked.ip_addresses)?;
    ensure!(
        names.0 == request.dns_names && names.1 == request.ip_addresses,
        "Signed certificate names must exactly match the signing request"
    );
    let metadata = certificates::put(
        store,
        root,
        &request.certificate_id,
        &request.label,
        request.expected_revision,
        chain_pem,
        &material.private_key_pem.0,
        now,
    )?;
    delete(store, request_id)?;
    Ok(metadata)
}

pub(crate) fn validate_identity_types(chain_pem: &str) -> Result<()> {
    let chain = rustls_pemfile::certs(&mut std::io::Cursor::new(chain_pem.as_bytes()))
        .collect::<std::io::Result<Vec<_>>>()?;
    use x509_parser::prelude::FromDer;
    let leaf_der = chain.first().context("Missing service certificate")?;
    let (_, leaf) = x509_parser::certificate::X509Certificate::from_der(leaf_der.as_ref())
        .map_err(|_| anyhow::anyhow!("Invalid service certificate"))?;
    if let Some(names) = leaf.subject_alternative_name()? {
        ensure!(
            names.value.general_names.iter().all(|name| matches!(
                name,
                x509_parser::extensions::GeneralName::DNSName(_)
                    | x509_parser::extensions::GeneralName::IPAddress(_)
            )),
            "Signed certificate contains unrequested identity types"
        );
    }
    Ok(())
}

pub(crate) fn load(
    store: &StateStore,
    root: &Path,
    request_id: &str,
    now: i64,
) -> Result<RequestMaterial> {
    validate_certificate_id(request_id)?;
    let (json, file): (String, String) = store
        .connection
        .query_row(
            "SELECT metadata_json,material_file FROM certificate_requests WHERE request_id=?1",
            [request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .context("Unknown certificate signing request")?;
    let request: CertificateSigningRequest = serde_json::from_str(&json)?;
    ensure!(
        request.expires_at > now,
        "Certificate signing request has expired; create a new request"
    );
    check_revision(store, &request.certificate_id, request.expected_revision)?;
    let material: RequestMaterial = serde_json::from_slice(&vault::read_private(
        &certificates::material_path(root, &file)?,
    )?)?;
    ensure!(
        material.request == request,
        "Certificate request private material mismatch"
    );
    Ok(material)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{BasicConstraints, CertificateSigningRequestParams, CertifiedIssuer, IsCa};

    fn sign(request: &CertificateSigningRequest, names: Option<Vec<String>>) -> Result<String> {
        let mut parameters = CertificateParams::default();
        parameters.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        parameters.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let issuer = CertifiedIssuer::self_signed(parameters, KeyPair::generate()?)?;
        let mut csr = CertificateSigningRequestParams::from_pem(&request.csr_pem)?;
        if let Some(names) = names {
            csr.params.subject_alt_names = CertificateParams::new(names)?.subject_alt_names;
        }
        Ok(csr.signed_by(&issuer)?.pem() + &issuer.pem())
    }

    fn request(
        store: &StateStore,
        root: &Path,
        certificate_id: &str,
        revision: u64,
        now: i64,
    ) -> Result<CertificateSigningRequest> {
        create(
            store,
            root,
            &uuid::Uuid::new_v4().to_string(),
            certificate_id,
            "REST",
            revision,
            &["API.Example.Test".into()],
            &["127.0.0.1".into()],
            now,
        )
    }

    #[test]
    fn generated_service_key_stays_private_and_signed_chain_installs_atomically() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = crate::enrollment::unix_time()?;
        let id = uuid::Uuid::new_v4().to_string();
        let request = request(&store, &root, &id, 0, now)?;
        assert_eq!(request.dns_names, ["api.example.test"]);
        assert_eq!(list(&store)?, [request.clone()]);
        let public_json = serde_json::to_string(&request)?;
        assert!(!public_json.contains("PRIVATE KEY"));
        let saved: String = store.connection.query_row(
            "SELECT metadata_json FROM certificate_requests",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(saved, public_json);
        certificates::collect_unused(&root)?;
        assert_eq!(std::fs::read_dir(root.join("certificates"))?.count(), 1);
        let chain = sign(&request, None)?;
        store.connection.execute_batch("BEGIN IMMEDIATE")?;
        let issued = install(&store, &root, &request.request_id, &chain, now)?;
        store.connection.execute_batch("ROLLBACK")?;
        certificates::collect_unused(&root)?;
        assert!(certificates::metadata(&store, &id).is_err());
        assert_eq!(list(&store)?, [request.clone()]);
        store.connection.execute_batch("BEGIN IMMEDIATE")?;
        let installed = install(&store, &root, &request.request_id, &chain, now)?;
        store.connection.execute_batch("COMMIT")?;
        certificates::collect_unused(&root)?;
        assert_eq!(installed, issued);
        assert_eq!(installed.revision, 1);
        assert!(list(&store)?.is_empty());
        assert_eq!(certificates::load_identity(&root, &id)?.revision, 1);
        assert_eq!(std::fs::read_dir(root.join("certificates"))?.count(), 1);
        Ok(())
    }

    #[test]
    fn installation_rejects_wrong_key_broadened_names_expiry_and_stale_revision() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = crate::enrollment::unix_time()?;
        let id = uuid::Uuid::new_v4().to_string();
        let pending = request(&store, &root, &id, 0, now)?;
        let wrong_key = rcgen::generate_simple_self_signed(vec![
            "api.example.test".into(),
            "127.0.0.1".into(),
        ])?;
        assert!(
            install(
                &store,
                &root,
                &pending.request_id,
                &wrong_key.cert.pem(),
                now
            )
            .is_err()
        );
        let broadened = sign(
            &pending,
            Some(vec![
                "api.example.test".into(),
                "127.0.0.1".into(),
                "other.example.test".into(),
            ]),
        )?;
        assert!(install(&store, &root, &pending.request_id, &broadened, now).is_err());
        let chain = sign(&pending, None)?;
        assert!(
            install(
                &store,
                &root,
                &pending.request_id,
                &chain,
                pending.expires_at
            )
            .is_err()
        );
        certificates::put(
            &store,
            &root,
            &id,
            "Imported",
            0,
            &wrong_key.cert.pem(),
            &wrong_key.signing_key.serialize_pem(),
            now,
        )?;
        assert!(install(&store, &root, &pending.request_id, &chain, now).is_err());
        assert_eq!(certificates::metadata(&store, &id)?.label, "Imported");
        assert!(request(&store, &root, &id, 1, now).is_err());
        delete(&store, &pending.request_id)?;
        certificates::collect_unused(&root)?;
        assert_eq!(std::fs::read_dir(root.join("certificates"))?.count(), 1);
        Ok(())
    }

    #[test]
    fn canonical_names_reject_ambiguous_or_unbounded_identities() -> Result<()> {
        let names = canonical_names(
            &["A.example".into(), "a.example".into()],
            &["2001:db8:0:0::1".into()],
        )?;
        assert_eq!(
            names,
            (vec!["a.example".into()], vec!["2001:db8::1".into()])
        );
        for value in [
            "*.example.test",
            "a.example.",
            "bad..name",
            "-bad.name",
            "127.0.0.1",
            "ümlaut.example",
            "foo/bar",
            " foo.example",
        ] {
            assert!(
                canonical_names(&[value.into()], &[]).is_err(),
                "accepted {value}"
            );
        }
        assert!(canonical_names(&[], &[]).is_err());
        assert!(canonical_names(&[], &["127.0.0.0/24".into()]).is_err());
        assert!(canonical_names(&vec!["a.test".into(); 33], &[]).is_err());
        Ok(())
    }

    #[test]
    fn expired_requests_are_replaced_without_reusing_their_private_keys() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = crate::enrollment::unix_time()?;
        let id = uuid::Uuid::new_v4().to_string();
        let first = request(&store, &root, &id, 0, now)?;
        let replacement = request(&store, &root, &id, 0, first.expires_at)?;
        assert_ne!(first.csr_pem, replacement.csr_pem);
        assert_ne!(first.request_id, replacement.request_id);
        certificates::collect_unused(&root)?;
        assert_eq!(std::fs::read_dir(root.join("certificates"))?.count(), 1);
        Ok(())
    }
}
