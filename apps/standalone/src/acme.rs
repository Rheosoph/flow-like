use crate::{certificate_requests, certificates, state::StateStore, vault};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like_device_protocol::{AcmeCertificateMetadata, AcmeEnvironment, SecretValue};
use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, Identifier, NewAccount, NewOrder, OrderStatus,
    RetryPolicy,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::RwLock,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub(crate) const SCHEMA: &str = "
CREATE TABLE certificate_acme (
    certificate_id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL,
    metadata_json TEXT,
    certificate_revision INTEGER NOT NULL,
    account_file TEXT,
    key_file TEXT,
    order_url TEXT,
    failures INTEGER NOT NULL DEFAULT 0
);
";

#[derive(Clone)]
struct Record {
    metadata: AcmeCertificateMetadata,
    certificate_revision: u64,
    account_file: Option<String>,
    key_file: Option<String>,
    order_url: Option<String>,
    failures: u32,
}

#[derive(Serialize, Deserialize)]
struct PendingKey {
    private_key: SecretValue,
    csr_der: String,
}

fn directory(environment: AcmeEnvironment) -> &'static str {
    match environment {
        AcmeEnvironment::LetsEncryptStaging => instant_acme::LetsEncrypt::Staging.url(),
        AcmeEnvironment::LetsEncryptProduction => instant_acme::LetsEncrypt::Production.url(),
    }
}

fn read_record(store: &StateStore, id: &str) -> Result<Record> {
    let value = store.connection.query_row(
        "SELECT metadata_json,certificate_revision,account_file,key_file,order_url,failures FROM certificate_acme WHERE certificate_id=?1 AND metadata_json IS NOT NULL",
        [id], |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).optional()?.context("ACME renewal is not configured")?;
    Ok(Record {
        metadata: serde_json::from_str(&value.0)?,
        certificate_revision: value.1,
        account_file: value.2,
        key_file: value.3,
        order_url: value.4,
        failures: value.5,
    })
}

pub(crate) fn list(store: &StateStore) -> Result<Vec<AcmeCertificateMetadata>> {
    store.connection.prepare("SELECT metadata_json FROM certificate_acme WHERE metadata_json IS NOT NULL ORDER BY certificate_id")?
        .query_map([], |row| row.get::<_, String>(0))?.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

/// The caller holds the management transaction. Disabling leaves the current service identity intact.
pub(crate) fn disable(store: &StateStore, id: &str) -> Result<()> {
    store.connection.execute("UPDATE certificate_acme SET revision=revision+1,metadata_json=NULL,account_file=NULL,key_file=NULL,order_url=NULL WHERE certificate_id=?1 AND metadata_json IS NOT NULL", [id])?;
    Ok(())
}

pub(crate) fn delete(store: &StateStore, id: &str, expected_revision: u64) -> Result<()> {
    let current = read_record(store, id)?;
    ensure!(
        current.metadata.revision == expected_revision,
        "ACME policy revision changed"
    );
    disable(store, id)
}

pub(crate) fn configure(
    store: &StateStore,
    id: &str,
    label: &str,
    expected_revision: u64,
    expected_certificate_revision: u64,
    dns_names: &[String],
    environment: AcmeEnvironment,
    http_bind: &str,
    terms_of_service_agreed: bool,
    now: i64,
) -> Result<AcmeCertificateMetadata> {
    ensure!(
        terms_of_service_agreed,
        "Accept the certificate authority terms before enabling ACME"
    );
    certificate_requests::check_revision(store, id, expected_certificate_revision)?;
    ensure!(
        !label.trim().is_empty() && label.len() <= 128 && !label.chars().any(char::is_control),
        "Certificate label must contain 1 to 128 printable bytes"
    );
    let (dns_names, _) = certificate_requests::canonical_names(dns_names, &[])?;
    ensure!(
        dns_names.iter().all(|name| name.contains('.')),
        "ACME requires public DNS names"
    );
    let bind: SocketAddr = http_bind
        .parse()
        .context("HTTP challenge listener must be an IP address and port")?;
    ensure!(
        bind.port() != 0 && !bind.ip().is_multicast(),
        "Invalid HTTP challenge listener"
    );
    let old: Option<(u64, bool)> = store.connection.query_row("SELECT revision,metadata_json IS NOT NULL FROM certificate_acme WHERE certificate_id=?1", [id], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    ensure!(
        old.as_ref()
            .filter(|value| value.1)
            .map_or(0, |value| value.0)
            == expected_revision,
        "ACME policy revision changed"
    );
    let revision = old
        .map_or(0, |value| value.0)
        .checked_add(1)
        .context("ACME policy revision exhausted")?;
    ensure!(
        revision < 9_007_199_254_740_991,
        "ACME policy revision exhausted"
    );
    let pending: bool = store.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM certificate_requests WHERE certificate_id=?1)",
        [id],
        |row| row.get(0),
    )?;
    ensure!(
        !pending,
        "Cancel the pending signing request before configuring ACME"
    );
    let count: usize = store.connection.query_row("SELECT COUNT(*) FROM (SELECT certificate_id FROM device_certificates WHERE metadata_json IS NOT NULL UNION SELECT certificate_id FROM certificate_acme WHERE metadata_json IS NOT NULL UNION SELECT certificate_id FROM certificate_requests UNION SELECT ?1)", [id], |row| row.get(0))?;
    ensure!(
        count <= flow_like_device_protocol::MAX_DEVICE_CERTIFICATES,
        "Device certificate limit reached"
    );
    crate::certificate_issuers::disable(store, id)?;
    let metadata = AcmeCertificateMetadata {
        certificate_id: id.into(),
        label: label.into(),
        revision,
        dns_names,
        environment,
        http_bind: bind.to_string(),
        next_attempt_at: now,
        last_renewed_at: None,
        last_error: None,
    };
    store.connection.execute("INSERT INTO certificate_acme(certificate_id,revision,metadata_json,certificate_revision) VALUES(?1,?2,?3,?4) ON CONFLICT(certificate_id) DO UPDATE SET revision=excluded.revision,metadata_json=excluded.metadata_json,certificate_revision=excluded.certificate_revision,account_file=NULL,key_file=NULL,order_url=NULL,failures=0",
        params![id, revision, serde_json::to_string(&metadata)?, expected_certificate_revision])?;
    Ok(metadata)
}

fn save_record(store: &StateStore, record: &Record) -> Result<()> {
    let updated = store.connection.execute("UPDATE certificate_acme SET metadata_json=?3,certificate_revision=?4,account_file=?5,key_file=?6,order_url=?7,failures=?8 WHERE certificate_id=?1 AND revision=?2 AND metadata_json IS NOT NULL",
        params![record.metadata.certificate_id, record.metadata.revision, serde_json::to_string(&record.metadata)?, record.certificate_revision, record.account_file, record.key_file, record.order_url, record.failures])?;
    ensure!(updated == 1, "ACME policy changed during issuance");
    Ok(())
}

fn persist(root: &Path, record: &Record, material: Option<&[u8]>) -> Result<Option<String>> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let transaction =
        Transaction::new_unchecked(&store.connection, TransactionBehavior::Immediate)?;
    ensure!(
        read_record(&store, &record.metadata.certificate_id)?
            .metadata
            .revision
            == record.metadata.revision,
        "ACME policy changed during issuance"
    );
    certificate_requests::check_revision(
        &store,
        &record.metadata.certificate_id,
        record.certificate_revision,
    )?;
    let file = if let Some(bytes) = material {
        let file = format!("{}.json", uuid::Uuid::new_v4());
        vault::write_new_private(&certificates::material_path(root, &file)?, bytes)?;
        Some(file)
    } else {
        None
    };
    // A new material file is linked in the same transaction, before garbage collection can see it.
    let updated = if record.account_file.is_none() && file.is_some() {
        Record {
            account_file: file.clone(),
            ..record.clone()
        }
    } else if record.key_file.is_none() && file.is_some() {
        Record {
            key_file: file.clone(),
            ..record.clone()
        }
    } else {
        record.clone()
    };
    save_record(&store, &updated)?;
    transaction.commit()?;
    Ok(file)
}

fn load_job(root: &Path, id: &str, revision: u64) -> Result<Record> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let record = read_record(&store, id)?;
    ensure!(record.metadata.revision == revision, "ACME policy changed");
    certificate_requests::check_revision(&store, id, record.certificate_revision)?;
    Ok(record)
}

async fn attempt(root: &Path, record: &mut Record) -> Result<()> {
    attempt_with_builder(root, record, Account::builder()?).await
}

async fn attempt_with_builder(
    root: &Path,
    record: &mut Record,
    builder: instant_acme::AccountBuilder,
) -> Result<()> {
    let listener = ChallengeServer::bind(&record.metadata.http_bind)
        .await
        .context("HTTP challenge listener could not start")?;
    let account = if let Some(file) = &record.account_file {
        let bytes = read_material(root, record, file)?;
        builder
            .from_credentials(serde_json::from_slice(&bytes)?)
            .await?
    } else {
        let (account, credentials) = builder
            .create(
                &NewAccount {
                    contact: &[],
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                directory(record.metadata.environment).into(),
                None,
            )
            .await?;
        record.account_file = persist(
            root,
            record,
            Some(&Zeroizing::new(serde_json::to_vec(&credentials)?)),
        )?;
        account
    };
    if record.key_file.is_none() {
        let key = Zeroizing::new(rcgen::KeyPair::generate()?);
        let mut parameters = rcgen::CertificateParams::new(record.metadata.dns_names.clone())?;
        parameters.distinguished_name = rcgen::DistinguishedName::new();
        let csr = parameters.serialize_request(&*key)?;
        let pending = PendingKey {
            private_key: SecretValue(key.serialize_pem()),
            csr_der: STANDARD.encode(csr.der()),
        };
        record.key_file = persist(
            root,
            record,
            Some(&Zeroizing::new(serde_json::to_vec(&pending)?)),
        )?;
    }
    let mut order = if let Some(url) = &record.order_url {
        account.order(url.clone()).await?
    } else {
        let names = record
            .metadata
            .dns_names
            .iter()
            .cloned()
            .map(Identifier::Dns)
            .collect::<Vec<_>>();
        let order = account.new_order(&NewOrder::new(&names)).await?;
        record.order_url = Some(order.url().to_owned());
        persist(root, record, None)?;
        order
    };
    if order.state().status == OrderStatus::Invalid {
        record.order_url = None;
        record.key_file = None;
        bail!("Certificate authority rejected the order");
    }
    if order.state().status == OrderStatus::Pending {
        let mut authorizations = order.authorizations();
        while let Some(authorization) = authorizations.next().await {
            let mut authorization = authorization?;
            match authorization.status {
                AuthorizationStatus::Valid => continue,
                AuthorizationStatus::Pending => (),
                _ => {
                    record.order_url = None;
                    record.key_file = None;
                    bail!("Certificate authority rejected domain authorization");
                }
            }
            let mut challenge = authorization
                .challenge(ChallengeType::Http01)
                .context("Certificate authority did not offer HTTP-01")?;
            listener
                .insert(&challenge.token, challenge.key_authorization().as_str())
                .await?;
            challenge.set_ready().await?;
        }
        if order.poll_ready(&RetryPolicy::default()).await? != OrderStatus::Ready {
            record.order_url = None;
            record.key_file = None;
            bail!("Certificate authority did not validate the domains");
        }
    }
    let bytes = read_material(
        root,
        record,
        record
            .key_file
            .as_deref()
            .context("Missing pending certificate key")?,
    )?;
    let key: PendingKey = serde_json::from_slice(&bytes)?;
    if order.state().status == OrderStatus::Ready {
        order.finalize_csr(&STANDARD.decode(&key.csr_der)?).await?;
    }
    let chain = order.poll_certificate(&RetryPolicy::default()).await?;
    finish(
        root,
        record,
        &chain,
        &key.private_key.0,
        crate::enrollment::unix_time()?,
    )?;
    Ok(())
}

fn read_material(root: &Path, record: &Record, file: &str) -> Result<Zeroizing<Vec<u8>>> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let transaction =
        Transaction::new_unchecked(&store.connection, TransactionBehavior::Immediate)?;
    let current = read_record(&store, &record.metadata.certificate_id)?;
    ensure!(
        current.metadata.revision == record.metadata.revision
            && (current.account_file.as_deref() == Some(file)
                || current.key_file.as_deref() == Some(file)),
        "ACME policy changed during issuance"
    );
    let bytes = vault::read_private(&certificates::material_path(root, file)?)?;
    transaction.commit()?;
    Ok(bytes)
}

fn finish(root: &Path, record: &mut Record, chain: &str, key: &str, now: i64) -> Result<()> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let transaction =
        Transaction::new_unchecked(&store.connection, TransactionBehavior::Immediate)?;
    let current = read_record(&store, &record.metadata.certificate_id)?;
    ensure!(
        current.metadata.revision == record.metadata.revision,
        "ACME policy changed during issuance"
    );
    let (checked, _) = certificates::validate_material(
        &record.metadata.certificate_id,
        &record.metadata.label,
        record.certificate_revision + 1,
        chain,
        key,
        now,
    )?;
    certificate_requests::validate_identity_types(chain)?;
    let (names, ips) =
        certificate_requests::canonical_names(&checked.dns_names, &checked.ip_addresses)?;
    ensure!(
        names == record.metadata.dns_names && ips.is_empty(),
        "Issued certificate does not match the approved names"
    );
    let metadata = certificates::put_issued(
        &store,
        root,
        &record.metadata.certificate_id,
        &record.metadata.label,
        record.certificate_revision,
        chain,
        key,
        now,
    )?;
    record.certificate_revision = metadata.revision;
    record.metadata.last_renewed_at = Some(now);
    record.metadata.last_error = None;
    // Renew after two thirds of the remaining lifetime; also works for short-lived certificates.
    record.metadata.next_attempt_at = now + ((metadata.not_after - now) * 2 / 3).max(60);
    record.failures = 0;
    record.key_file = None;
    record.order_url = None;
    save_record(&store, record)?;
    transaction.commit()?;
    Ok(())
}

fn record_failure(root: &Path, record: &mut Record, now: i64) -> Result<()> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let transaction =
        Transaction::new_unchecked(&store.connection, TransactionBehavior::Immediate)?;
    let mut current = read_record(&store, &record.metadata.certificate_id)?;
    ensure!(
        current.metadata.revision == record.metadata.revision,
        "ACME policy changed during issuance"
    );
    if record.order_url.is_none() && record.certificate_revision == current.certificate_revision {
        current.order_url = None;
        current.key_file = None;
    }
    current.failures = current.failures.saturating_add(1);
    current.metadata.next_attempt_at =
        now.saturating_add((300_i64.saturating_mul(1_i64 << current.failures.min(6))).min(21600));
    current.metadata.last_error = Some("ACME issuance failed. Check public DNS, inbound port 80, the challenge listener, and access to the certificate authority. Any installed certificate has been retained.".into());
    save_record(&store, &current)?;
    transaction.commit()?;
    Ok(())
}

pub async fn run(root: PathBuf, cancel: CancellationToken) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! { _ = cancel.cancelled() => return Ok(()), _ = interval.tick() => () }
        let jobs =
            (|| -> Result<_> { list(&StateStore::open(&root.join("management.sqlite"))?) })();
        let Ok(jobs) = jobs else {
            tracing::warn!("Unable to read ACME renewal policies");
            continue;
        };
        for job in jobs {
            let now = crate::enrollment::unix_time()?;
            if job.next_attempt_at > now {
                continue;
            }
            let Ok(mut record) = load_job(&root, &job.certificate_id, job.revision) else {
                continue;
            };
            let result = tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                result = tokio::time::timeout(Duration::from_secs(300), attempt(&root, &mut record)) => result,
            };
            if !matches!(result, Ok(Ok(()))) {
                // Remote error bodies may contain customer identifiers; expose a bounded local diagnosis.
                let _ = record_failure(&root, &mut record, crate::enrollment::unix_time()?);
            }
            let _ = certificates::collect_unused(&root);
        }
    }
}

struct ChallengeServer {
    values: Arc<RwLock<HashMap<String, String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ChallengeServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ChallengeServer {
    async fn bind(address: &str) -> Result<Self> {
        let listener = TcpListener::bind(address).await?;
        Self::start(listener)
    }

    fn start(listener: TcpListener) -> Result<Self> {
        let values = Arc::new(RwLock::new(HashMap::<String, String>::new()));
        let serving = values.clone();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = connections.join_next(), if !connections.is_empty() => (),
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break; };
                        if connections.len() >= 32 { drop(stream); continue; }
                        let serving = serving.clone();
                        connections.spawn(async move {
                            let _ = tokio::time::timeout(Duration::from_secs(5), serve_challenge(stream, serving)).await;
                        });
                    }
                }
            }
        });
        Ok(Self { values, task })
    }

    async fn insert(&self, token: &str, value: &str) -> Result<()> {
        ensure!(
            !token.is_empty()
                && token.len() <= 256
                && token
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
            "Invalid ACME challenge token"
        );
        ensure!(
            value.len() <= 1024 && value.is_ascii() && !value.chars().any(char::is_control),
            "Invalid ACME challenge response"
        );
        let mut values = self.values.write().await;
        ensure!(values.len() < 32, "Too many HTTP challenges");
        values.insert(format!("/.well-known/acme-challenge/{token}"), value.into());
        Ok(())
    }
}

async fn serve_challenge(
    mut stream: TcpStream,
    values: Arc<RwLock<HashMap<String, String>>>,
) -> Result<()> {
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 512];
    loop {
        let length = stream.read(&mut buffer).await?;
        if length == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..length]);
        ensure!(request.len() <= 4096, "HTTP challenge request too large");
        if request.windows(4).any(|part| part == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&request)?;
    let line = request.split("\r\n").next().unwrap_or_default();
    let parts = line.split(' ').collect::<Vec<_>>();
    let value =
        if parts.len() == 3 && parts[0] == "GET" && matches!(parts[2], "HTTP/1.0" | "HTTP/1.1") {
            values.read().await.get(parts[1]).cloned()
        } else {
            None
        };
    let (status, body) = value.map_or(("404 Not Found", String::new()), |value| ("200 OK", value));
    stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair};

    fn policy(store: &StateStore, id: &str, now: i64) -> Result<AcmeCertificateMetadata> {
        configure(
            store,
            id,
            "Public API",
            0,
            0,
            &["api.example.test".into()],
            AcmeEnvironment::LetsEncryptStaging,
            "127.0.0.1:8080",
            true,
            now,
        )
    }

    fn service(names: Vec<String>) -> Result<(String, String)> {
        let key = KeyPair::generate()?;
        let mut parameters = CertificateParams::new(names)?;
        parameters.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        Ok((parameters.self_signed(&key)?.pem(), key.serialize_pem()))
    }

    #[test]
    fn acme_policy_guards_consent_names_revisions_and_pending_requests() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = crate::enrollment::unix_time()?;
        let id = uuid::Uuid::new_v4().to_string();
        assert!(
            configure(
                &store,
                &id,
                "API",
                0,
                0,
                &["api.example.test".into()],
                AcmeEnvironment::LetsEncryptStaging,
                "127.0.0.1:8080",
                false,
                now
            )
            .is_err()
        );
        for name in ["localhost", "*.example.test", "127.0.0.1"] {
            assert!(
                configure(
                    &store,
                    &id,
                    "API",
                    0,
                    0,
                    &[name.into()],
                    AcmeEnvironment::LetsEncryptStaging,
                    "127.0.0.1:8080",
                    true,
                    now
                )
                .is_err()
            );
        }
        for bind in ["localhost:80", "127.0.0.1:0", "224.0.0.1:80"] {
            assert!(
                configure(
                    &store,
                    &id,
                    "API",
                    0,
                    0,
                    &["api.example.test".into()],
                    AcmeEnvironment::LetsEncryptStaging,
                    bind,
                    true,
                    now
                )
                .is_err()
            );
        }
        let first = policy(&store, &id, now)?;
        assert_eq!(first.revision, 1);
        assert!(policy(&store, &id, now).is_err());
        assert!(delete(&store, &id, 2).is_err());
        delete(&store, &id, 1)?;
        let second = policy(&store, &id, now)?;
        assert!(second.revision > first.revision);
        certificate_requests::create(
            &store,
            &root,
            &uuid::Uuid::new_v4().to_string(),
            &id,
            "Enterprise CSR",
            0,
            &["api.example.test".into()],
            &[],
            now,
        )?;
        assert!(list(&store)?.is_empty());
        assert!(policy(&store, &id, now).is_err());
        Ok(())
    }

    #[test]
    fn failed_acme_attempt_keeps_current_leaf_account_and_pending_key() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = crate::enrollment::unix_time()?;
        let id = uuid::Uuid::new_v4().to_string();
        policy(&store, &id, now)?;
        let mut record = read_record(&store, &id)?;
        let (chain, key) = service(vec!["api.example.test".into()])?;
        finish(&root, &mut record, &chain, &key, now)?;
        let original = certificates::metadata(&store, &id)?;
        record.account_file = persist(&root, &record, Some(b"private-account-credential"))?;
        record.key_file = persist(&root, &record, Some(b"private-pending-key"))?;
        record.order_url = Some("https://acme.example.test/order/1".into());
        persist(&root, &record, None)?;
        record_failure(&root, &mut record, now)?;
        certificates::collect_unused(&root)?;
        let failed = read_record(&store, &id)?;
        assert_eq!(failed.account_file, record.account_file);
        assert_eq!(failed.key_file, record.key_file);
        assert_eq!(failed.order_url, record.order_url);
        assert!(failed.metadata.last_error.is_some());
        assert!(failed.metadata.next_attempt_at > now);
        assert_eq!(certificates::metadata(&store, &id)?, original);
        assert_eq!(
            &*read_material(&root, &failed, failed.account_file.as_deref().unwrap())?,
            b"private-account-credential"
        );
        assert_eq!(std::fs::read_dir(root.join("certificates"))?.count(), 3);
        let json = serde_json::to_string(&failed.metadata)?;
        assert!(
            !json.contains("private-account-credential") && !json.contains("private-pending-key")
        );
        Ok(())
    }

    #[test]
    fn stale_acme_results_cannot_replace_a_manually_imported_certificate() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = crate::enrollment::unix_time()?;
        let id = uuid::Uuid::new_v4().to_string();
        policy(&store, &id, now)?;
        let mut record = read_record(&store, &id)?;
        let (chain, key) = service(vec!["api.example.test".into()])?;
        let (wrong_chain, wrong_key) = service(vec!["other.example.test".into()])?;
        assert!(finish(&root, &mut record, &wrong_chain, &wrong_key, now).is_err());
        assert!(certificates::metadata(&store, &id).is_err());
        let (manual_chain, manual_key) = service(vec!["api.example.test".into()])?;
        certificates::put(
            &store,
            &root,
            &id,
            "Enterprise imported",
            0,
            &manual_chain,
            &manual_key,
            now,
        )?;
        let imported = certificates::metadata(&store, &id)?;
        assert!(list(&store)?.is_empty());
        assert!(finish(&root, &mut record, &chain, &key, now).is_err());
        assert!(persist(&root, &record, Some(b"stale-account-key")).is_err());
        assert!(record_failure(&root, &mut record, now).is_err());
        assert_eq!(certificates::metadata(&store, &id)?, imported);
        Ok(())
    }

    async fn response(address: SocketAddr, request: &[u8]) -> Result<Vec<u8>> {
        let mut stream = TcpStream::connect(address).await?;
        stream.write_all(request).await?;
        let mut response = Vec::new();
        let _ =
            tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response)).await?;
        Ok(response)
    }

    #[tokio::test]
    async fn challenge_listener_serves_only_bounded_acme_tokens() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = ChallengeServer::start(listener)?;
        server
            .insert("token-1", "token-1.account-thumbprint")
            .await?;
        let good = response(
            address,
            b"GET /.well-known/acme-challenge/token-1 HTTP/1.1\r\nHost: api.example.test\r\n\r\n",
        )
        .await?;
        let good = String::from_utf8(good)?;
        assert!(good.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(good.ends_with("token-1.account-thumbprint"));
        for request in [
            b"GET / HTTP/1.1\r\nHost: api.example.test\r\n\r\n".as_slice(),
            b"POST /.well-known/acme-challenge/token-1 HTTP/1.1\r\nHost: api.example.test\r\n\r\n"
                .as_slice(),
            b"GET /.well-known/acme-challenge/%74oken-1 HTTP/1.1\r\nHost: api.example.test\r\n\r\n"
                .as_slice(),
        ] {
            assert!(
                response(address, request)
                    .await?
                    .starts_with(b"HTTP/1.1 404 Not Found\r\n")
            );
        }
        assert!(server.insert("../private", "value").await.is_err());
        assert!(
            server
                .insert("safe", "value\r\nInjected: header")
                .await
                .is_err()
        );
        assert!(server.insert(&"a".repeat(257), "value").await.is_err());
        assert!(server.insert("safe", &"a".repeat(1025)).await.is_err());
        for number in 2..=32 {
            server.insert(&format!("token-{number}"), "value").await?;
        }
        assert!(server.insert("over-limit", "value").await.is_err());
        let oversized = vec![b'a'; 4608];
        assert!(response(address, &oversized).await?.is_empty());
        Ok(())
    }

    #[derive(Default)]
    struct FakeAcmeState {
        accounts: usize,
        orders: usize,
        challenge_verified: bool,
        key_authorization: String,
        fail_finalize_once: bool,
        chain: Option<String>,
    }

    #[derive(Clone)]
    struct FakeAcme {
        state: Arc<tokio::sync::Mutex<FakeAcmeState>>,
        challenge_address: SocketAddr,
    }

    fn fake_order(state: &FakeAcmeState) -> serde_json::Value {
        serde_json::json!({
            "status": if state.chain.is_some() { "valid" } else if state.challenge_verified { "ready" } else { "pending" },
            "authorizations": ["https://acme.example.test/authorization"],
            "finalize": "https://acme.example.test/finalize",
            "certificate": state.chain.as_ref().map(|_| "https://acme.example.test/certificate"),
        })
    }

    impl instant_acme::HttpClient for FakeAcme {
        fn request(
            &self,
            request: axum::http::Request<instant_acme::BodyWrapper<bytes::Bytes>>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = std::result::Result<
                            instant_acme::BytesResponse,
                            instant_acme::Error,
                        >,
                    > + Send,
            >,
        > {
            let fake = self.clone();
            Box::pin(async move {
                async {
                    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
                    use sha2::{Digest, Sha256};
                    let path = request.uri().path().to_owned();
                    let bytes = axum::body::to_bytes(axum::body::Body::new(request.into_body()), 32 * 1024).await?;
                    let signed: serde_json::Value = if bytes.is_empty() { serde_json::json!({}) } else { serde_json::from_slice(&bytes)? };
                    let payload = signed.get("payload").and_then(serde_json::Value::as_str).unwrap_or_default();
                    let payload: serde_json::Value = if payload.is_empty() { serde_json::json!({}) } else { serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload)?)? };
                    let mut state = fake.state.lock().await;
                    let mut status = 200;
                    let mut location = None;
                    let body = match path.as_str() {
                        "/directory" => serde_json::json!({
                            "newNonce":"https://acme.example.test/nonce",
                            "newAccount":"https://acme.example.test/account",
                            "newOrder":"https://acme.example.test/new-order"
                        }).to_string(),
                        "/nonce" => String::new(),
                        "/account" => {
                            state.accounts += 1;
                            let protected: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(signed["protected"].as_str().context("Missing protected JWS")?)?)?;
                            let jwk = &protected["jwk"];
                            let canonical = format!("{{\"crv\":\"P-256\",\"kty\":\"EC\",\"x\":\"{}\",\"y\":\"{}\"}}", jwk["x"].as_str().unwrap(), jwk["y"].as_str().unwrap());
                            state.key_authorization = format!("test-token.{}", URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes())));
                            status = 201;
                            location = Some("https://acme.example.test/account/1");
                            "{}".into()
                        }
                        "/new-order" => {
                            assert_eq!(payload["identifiers"], serde_json::json!([{"type":"dns","value":"api.example.test"}]));
                            state.orders += 1;
                            status = 201;
                            location = Some("https://acme.example.test/order/1");
                            fake_order(&state).to_string()
                        }
                        "/authorization" => serde_json::json!({
                            "identifier":{"type":"dns","value":"api.example.test"},
                            "status":"pending",
                            "challenges":[{"type":"http-01","status":"pending","url":"https://acme.example.test/challenge","token":"test-token"}]
                        }).to_string(),
                        "/challenge" => {
                            let response = response(fake.challenge_address, b"GET /.well-known/acme-challenge/test-token HTTP/1.1\r\nHost: api.example.test\r\n\r\n").await?;
                            let response = String::from_utf8(response)?;
                            ensure!(response.starts_with("HTTP/1.1 200 OK\r\n") && response.ends_with(&state.key_authorization), "ACME challenge did not prove the account key");
                            state.challenge_verified = true;
                            serde_json::json!({"type":"http-01","status":"valid","url":"https://acme.example.test/challenge","token":"test-token"}).to_string()
                        }
                        "/order/1" => fake_order(&state).to_string(),
                        "/finalize" => {
                            ensure!(state.challenge_verified, "Finalized before HTTP domain validation");
                            if state.fail_finalize_once {
                                state.fail_finalize_once = false;
                                bail!("Simulated certificate authority outage");
                            }
                            let csr_der = URL_SAFE_NO_PAD.decode(payload["csr"].as_str().context("Missing finalization CSR")?)?;
                            let mut csr = rcgen::CertificateSigningRequestParams::from_der(&csr_der.as_slice().into())?;
                            csr.params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
                            let mut params = CertificateParams::default();
                            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
                            params.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
                            let issuer = rcgen::CertifiedIssuer::self_signed(params, KeyPair::generate()?)?;
                            state.chain = Some(csr.signed_by(&issuer)?.pem() + &issuer.pem());
                            fake_order(&state).to_string()
                        }
                        "/certificate" => state.chain.clone().context("Certificate not finalized")?,
                        _ => bail!("Unexpected ACME URL {path}"),
                    };
                    let mut response = axum::http::Response::builder().status(status).header("Replay-Nonce", "test-nonce");
                    if let Some(location) = location { response = response.header("Location", location); }
                    Ok::<_, anyhow::Error>(instant_acme::BytesResponse::from(response.body(instant_acme::BodyWrapper::<bytes::Bytes>::from(body.into_bytes()))?))
                }.await.map_err(|error| instant_acme::Error::Other(error.into_boxed_dyn_error()))
            })
        }
    }

    #[tokio::test]
    async fn acme_http_validation_and_issuance_resume_after_outage_with_same_account_and_key()
    -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(temp.path())?;
        let store = StateStore::open(&root.join("management.sqlite"))?;
        let now = crate::enrollment::unix_time()?;
        let reservation = TcpListener::bind("127.0.0.1:0").await?;
        let address = reservation.local_addr()?;
        drop(reservation);
        let id = uuid::Uuid::new_v4().to_string();
        configure(
            &store,
            &id,
            "Public API",
            0,
            0,
            &["api.example.test".into()],
            AcmeEnvironment::LetsEncryptStaging,
            &address.to_string(),
            true,
            now,
        )?;
        let fake = FakeAcme {
            state: Arc::new(tokio::sync::Mutex::new(FakeAcmeState {
                fail_finalize_once: true,
                ..Default::default()
            })),
            challenge_address: address,
        };
        let mut record = read_record(&store, &id)?;
        let failure = attempt_with_builder(
            &root,
            &mut record,
            Account::builder_with_http(Box::new(fake.clone())),
        )
        .await;
        assert!(failure.is_err());
        let stored = read_record(&store, &id)?;
        assert!(
            stored.account_file.is_some()
                && stored.key_file.is_some()
                && stored.order_url.is_some()
        );
        let account_file = stored.account_file.clone();
        let pending: PendingKey = serde_json::from_slice(&read_material(
            &root,
            &stored,
            stored.key_file.as_deref().unwrap(),
        )?)?;
        record_failure(&root, &mut record, now)?;
        assert!(certificates::metadata(&store, &id).is_err());
        // The completed failed attempt drops its challenge listener before a retry binds it.
        tokio::task::yield_now().await;
        let mut resumed = load_job(&root, &id, stored.metadata.revision)?;
        attempt_with_builder(
            &root,
            &mut resumed,
            Account::builder_with_http(Box::new(fake.clone())),
        )
        .await?;
        let complete = read_record(&store, &id)?;
        assert_eq!(complete.account_file, account_file);
        assert!(complete.key_file.is_none() && complete.order_url.is_none());
        assert!(complete.metadata.last_error.is_none());
        let state = fake.state.lock().await;
        assert_eq!(state.accounts, 1);
        assert_eq!(state.orders, 1);
        assert!(state.challenge_verified);
        let certificate = certificates::metadata(&store, &id)?;
        assert_eq!(certificate.revision, 1);
        assert_eq!(certificate.dns_names, ["api.example.test"]);
        assert_eq!(
            certificates::load_identity(&root, &id)?.private_key_pem.0,
            pending.private_key.0
        );
        certificates::collect_unused(&root)?;
        assert_eq!(std::fs::read_dir(root.join("certificates"))?.count(), 2);
        Ok(())
    }
}
