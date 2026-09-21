//! Audit trail integration tests against a disposable PostgreSQL database.
//!
//! Run with AUDIT_TEST_DATABASE_URL pointing at an empty database and `cargo test -p
//! flow-like-api --test audit_integrity -- --ignored`. The first test creates the audit
//! tables from the migration and refuses a database that already has them.
//!
//! The audit worker is process-wide: one lease, one epoch timeline, one archive per
//! month. The tests therefore run one at a time on their own chains, share a clock that
//! only moves forward so epochs stay in time order, share the archive bucket, and
//! restore every row they tamper with so a later tick can still archive the month.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, PoisonError};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use flow_like::hub::{AuditConfig, AuditRetention};
use flow_like_api::{
    audit::{
        crypto::{self, EpochFields, Hash, RecordFields, SealFields, ZERO_HASH},
        export, keys, merkle,
        record::{self, ACTIVITY_SUFFIX, AuditRecordInput, WriteMode, build_record},
        signer::{self, AuditSigner, LocalSigner, SharedSigner},
        verify::{self, ChainReport, HeadCheck, INVALID_SEAL_ID},
        wire::{EpochLine, Line, RecordLine, SealLine, WatermarkLine},
        worker::{self, AuditWorkerContext, TickReport, bucket},
    },
    db::DbDialect,
    entity::{
        audit_archive, audit_entry, audit_epoch, audit_record, audit_seal, audit_watermark,
        sea_orm_active_enums::AuditActorType,
    },
};
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{ObjectStoreExt, memory::InMemory};
use flow_like_types::create_id;
use p256::ecdsa::{
    Signature, SigningKey, VerifyingKey,
    signature::{Signer, Verifier},
};
use sea_orm::{
    ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, Statement,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, MutexGuard, OnceCell};

const DATABASE_URL_ENV: &str = "AUDIT_TEST_DATABASE_URL";
const KID: &str = "audit-integrity-test";
const MIGRATION: &str =
    include_str!("../prisma/migrations/20260919120001_audit_seals/migration.sql");

/// Tables the worker reads besides its own: the lease row, the legacy trail it
/// exports, and the AI Act assessments that lengthen activity retention.
const SUPPORT_TABLES: &str = r#"
CREATE TABLE "AuditWorkerLease" (
    "id" BIGINT PRIMARY KEY,
    "updatedAt" TIMESTAMPTZ(3) NOT NULL DEFAULT now(),
    "owner" TEXT,
    "expiresAt" TIMESTAMPTZ(3)
);
CREATE TABLE "AuditEntry" (
    "id" TEXT PRIMARY KEY, "sequence" BIGINT NOT NULL,
    "timestamp" TIMESTAMPTZ(3) NOT NULL, "actorId" TEXT NOT NULL,
    "actorType" TEXT NOT NULL, "actorIp" TEXT, "action" TEXT NOT NULL,
    "resourceType" TEXT NOT NULL, "resourceId" TEXT NOT NULL, "chainId" TEXT,
    "summary" TEXT NOT NULL, "details" JSONB, "entryHash" TEXT NOT NULL,
    "prevHash" TEXT NOT NULL, "signature" TEXT, "kid" TEXT
);
CREATE TABLE "AiActAssessment" (
    "id" TEXT PRIMARY KEY, "appId" TEXT NOT NULL, "version" INTEGER NOT NULL DEFAULT 1,
    "status" TEXT NOT NULL DEFAULT 'DRAFT',
    "riskCategory" TEXT NOT NULL DEFAULT 'UNDETERMINED',
    "conformityScore" INTEGER, "conformityBand" TEXT, "answers" JSONB NOT NULL,
    "signals" JSONB, "transparencyObligations" JSONB, "responsibleUserId" TEXT,
    "responsibleName" TEXT, "responsibleEmail" TEXT, "submittedAt" TIMESTAMPTZ(3),
    "reviewedById" TEXT, "reviewedAt" TIMESTAMPTZ(3), "reviewNote" TEXT,
    "createdAt" TIMESTAMPTZ(3) NOT NULL DEFAULT now(), "updatedAt" TIMESTAMPTZ(3) NOT NULL,
    UNIQUE ("appId", "version")
);
"#;

static SERIAL: Mutex<()> = Mutex::const_new(());
static SCHEMA: OnceCell<()> = OnceCell::const_new();
static CLOCK: std::sync::Mutex<Option<DateTime<Utc>>> = std::sync::Mutex::new(None);

struct Harness {
    db: DatabaseConnection,
    dialect: DbDialect,
    _serial: MutexGuard<'static, ()>,
}

async fn harness() -> Harness {
    let serial = SERIAL.lock().await;
    let url = std::env::var(DATABASE_URL_ENV)
        .expect("set AUDIT_TEST_DATABASE_URL to a disposable PostgreSQL database");
    let mut options = ConnectOptions::new(url);
    options.max_connections(16).sqlx_logging(false);
    let db = Database::connect(options)
        .await
        .expect("connect to the audit test database");
    let setup = &db;
    SCHEMA
        .get_or_init(move || async move {
            setup
                .execute_unprepared(MIGRATION)
                .await
                .expect("create the audit tables; the database must be empty");
            setup
                .execute_unprepared(SUPPORT_TABLES)
                .await
                .expect("create the lease, legacy and AI Act tables");
            let entry_key = STANDARD.encode([7u8; 32]);
            keys::init_entry_key(Some(entry_key.as_str()), None).expect("fixed audit entry key");
            signer::register_verifying_key(KID, test_signer().verifying_key())
                .expect("register the test audit key");
        })
        .await;
    let dialect = DbDialect::resolve(None, &db).await;
    Harness {
        db,
        dialect,
        _serial: serial,
    }
}

fn test_signer() -> LocalSigner {
    LocalSigner::new(
        SigningKey::from_slice(&[41; 32]).expect("valid P-256 scalar"),
        Some(KID.into()),
    )
}

/// Seal every pending record at the next tick.
fn retention() -> AuditRetention {
    // The shared test clock only moves forward, so values would otherwise expire in
    // tests that run after one that jumped ahead; expiry is tested with its own window.
    AuditRetention {
        seal_after_seconds: 0,
        epoch_interval_seconds: 0,
        ip_days: 36_500,
        activity_days: 36_500,
        high_risk_activity_days: 36_500,
        ..AuditRetention::default()
    }
}

fn worker_context(
    harness: &Harness,
    retention: AuditRetention,
    bucket: Option<Arc<FlowLikeStore>>,
) -> AuditWorkerContext {
    let signer: SharedSigner = Arc::new(test_signer());
    signed_worker_context(harness, retention, bucket, signer)
}

fn signed_worker_context(
    harness: &Harness,
    retention: AuditRetention,
    bucket: Option<Arc<FlowLikeStore>>,
    signer: SharedSigner,
) -> AuditWorkerContext {
    AuditWorkerContext::new(
        harness.db.clone(),
        harness.dialect,
        AuditConfig {
            retention,
            ..AuditConfig::default()
        },
        Some(signer),
        bucket,
        None,
    )
}

fn memory_bucket() -> Arc<FlowLikeStore> {
    // Archive rows and their trusted receipts share the process-wide test timeline.
    static BUCKET: OnceLock<Arc<FlowLikeStore>> = OnceLock::new();
    BUCKET
        .get_or_init(|| Arc::new(FlowLikeStore::Memory(Arc::new(InMemory::new()))))
        .clone()
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn retained_checkpoint_rejects_chain_tail_deletion_after_worker_restart() {
    use flow_like_api::audit::worker::checkpoint::{Checkpoint, verify_retained};
    let harness = harness().await;
    let db = &harness.db;
    let store = memory_bucket();
    let context = worker_context(&harness, retention(), Some(store.clone())).with_checkpoints();
    let app = create_id();
    insert_records(db, &app, "board.update", clock(Duration::seconds(1)), 2).await;
    // A new day guarantees this checkpoint includes the new chain.
    let now = clock(Duration::days(1));
    tick(&context, now).await;
    let key = Path::from(format!("checkpoints/{}.json", now.format("%Y/%m/%d")));
    let checkpoint: Checkpoint = serde_json::from_slice(&object(&store, &key).await).unwrap();
    assert!(checkpoint.chains.contains_key(&app));
    verify_retained(db, &checkpoint).await.unwrap();
    let tail = seals(db, &app).await.pop().unwrap();
    // Keeping the stored hash must not hide changes to the retained seal's fields.
    exec(
        db,
        r#"UPDATE "AuditSeal" SET "recordCount" = $2 WHERE "id" = $1"#,
        vec![tail.id.clone().into(), (tail.record_count + 1).into()],
    )
    .await;
    let altered = audit_seal::Entity::find_by_id(tail.id.clone())
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(altered.hash, tail.hash);
    let altered_result = verify_retained(db, &checkpoint).await;
    exec(
        db,
        r#"UPDATE "AuditSeal" SET "recordCount" = $2 WHERE "id" = $1"#,
        vec![tail.id.clone().into(), tail.record_count.into()],
    )
    .await;
    assert!(
        altered_result.is_err(),
        "a retained seal with changed fields and its original hash was accepted"
    );
    verify_retained(db, &checkpoint).await.unwrap();
    audit_seal::Entity::delete_by_id(tail.id.clone())
        .exec(db)
        .await
        .unwrap();
    // The epoch still exists, so the old epoch-only head would match.
    assert_eq!(
        verify::check_head(
            db,
            checkpoint.epoch.seq,
            &unhex(&checkpoint.epoch.hash),
            None
        )
        .await
        .unwrap(),
        HeadCheck::Matches
    );
    assert!(verify_retained(db, &checkpoint).await.is_err());
    let restarted = worker_context(&harness, retention(), Some(store)).with_checkpoints();
    assert!(
        worker::tick(&restarted, clock(Duration::seconds(1)))
            .await
            .is_err()
    );
    // Other integration tests share this database; restore the deliberately deleted row.
    use sea_orm::IntoActiveModel;
    audit_seal::Entity::insert(tail.into_active_model())
        .exec_without_returning(db)
        .await
        .unwrap();
    verify_retained(db, &checkpoint).await.unwrap();
}

/// The worker's clock, `step` after the latest time any test used. Every test shares
/// one epoch timeline whose sequence follows time, so the clock never moves back.
fn clock(step: Duration) -> DateTime<Utc> {
    let mut guard = CLOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let base = guard.map_or_else(Utc::now, |last| last.max(Utc::now()));
    let next = DateTime::from_timestamp_millis((base + step).timestamp_millis())
        .expect("test time in range");
    *guard = Some(next);
    next
}

fn clock_at(target: DateTime<Utc>) {
    let mut guard = CLOCK.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(
        guard.is_none_or(|last| target >= last),
        "the test clock cannot move back to {target}"
    );
    *guard = Some(target);
}

async fn tick(context: &AuditWorkerContext, now: DateTime<Utc>) -> TickReport {
    let report = worker::tick(context, now).await.expect("audit worker tick");
    assert!(!report.skipped, "another audit worker holds the lease");
    assert!(
        report.failed_steps.is_empty(),
        "audit worker steps failed at {now}: {:?}",
        report.failed_steps
    );
    report
}

fn input(scope: &str, action: &str, resource_id: &str) -> AuditRecordInput {
    AuditRecordInput {
        actor_id: "audit-test-user".into(),
        actor_type: AuditActorType::User,
        actor_ip: Some("192.0.2.10".into()),
        action: action.into(),
        resource_type: "AuditTest".into(),
        resource_id: resource_id.into(),
        scope: Some(scope.into()),
        details: Some(json!({ "resource": resource_id, "count": 2 })),
    }
}

/// Records built as the API builds them, one millisecond apart from `at`.
async fn insert_records(
    db: &DatabaseConnection,
    scope: &str,
    action: &str,
    at: DateTime<Utc>,
    count: usize,
) -> Vec<String> {
    let mut ids = Vec::with_capacity(count);
    for index in 0..count {
        let model = build_record(
            input(scope, action, &format!("resource-{index}")),
            WriteMode::Append,
            keys::entry_key(),
            at + Duration::milliseconds(index as i64),
        );
        ids.push(model.id.clone().unwrap());
        audit_record::Entity::insert(model)
            .exec_without_returning(db)
            .await
            .expect("insert audit record");
    }
    ids
}

async fn exec(db: &DatabaseConnection, sql: &str, values: Vec<sea_orm::Value>) -> u64 {
    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        values,
    ))
    .await
    .unwrap_or_else(|error| panic!("{sql} failed: {error}"))
    .rows_affected()
}

async fn records(db: &DatabaseConnection, chain_id: &str) -> Vec<audit_record::Model> {
    audit_record::Entity::find()
        .filter(audit_record::Column::ChainId.eq(chain_id))
        .order_by_asc(audit_record::Column::Timestamp)
        .order_by_asc(audit_record::Column::Id)
        .all(db)
        .await
        .expect("read audit records")
}

async fn members(db: &DatabaseConnection, seal_id: &str) -> Vec<audit_record::Model> {
    audit_record::Entity::find()
        .filter(audit_record::Column::SealId.eq(seal_id))
        .all(db)
        .await
        .expect("read sealed records")
}

async fn seals(db: &DatabaseConnection, chain_id: &str) -> Vec<audit_seal::Model> {
    audit_seal::Entity::find()
        .filter(audit_seal::Column::ChainId.eq(chain_id))
        .order_by_asc(audit_seal::Column::Seq)
        .all(db)
        .await
        .expect("read audit seals")
}

async fn epoch(db: &DatabaseConnection, seq: i64) -> audit_epoch::Model {
    audit_epoch::Entity::find_by_id(seq)
        .one(db)
        .await
        .expect("read audit epoch")
        .unwrap_or_else(|| panic!("epoch {seq} exists"))
}

async fn verify_full(db: &DatabaseConnection, chain_id: &str) -> ChainReport {
    verify::verify_chain(db, chain_id, true)
        .await
        .expect("verify chain")
}

async fn assert_valid(db: &DatabaseConnection, chain_id: &str) {
    let report = verify_full(db, chain_id).await;
    assert!(report.valid, "{report:?}");
    let timeline = verify::verify_epochs(db, true)
        .await
        .expect("verify epochs");
    assert!(timeline.valid, "{timeline:?}");
}

async fn assert_broken(db: &DatabaseConnection, chain_id: &str, seq: i64, tampering: &str) {
    let report = verify_full(db, chain_id).await;
    assert!(!report.valid, "{tampering} went unnoticed: {report:?}");
    assert_eq!(
        report.first_broken_seal,
        Some(seq),
        "{tampering}: {report:?}"
    );
    let incremental = verify::verify_chain(db, chain_id, false)
        .await
        .expect("incremental verification after a failed full check");
    assert!(
        !incremental.valid,
        "{tampering} was forgotten: {incremental:?}"
    );
    assert_eq!(incremental.first_broken_seal, Some(seq), "{incremental:?}");
}

async fn set_seq(db: &DatabaseConnection, seal_id: &str, seq: i64) {
    exec(
        db,
        r#"UPDATE "AuditSeal" SET "seq" = $2 WHERE "id" = $1"#,
        vec![seal_id.into(), seq.into()],
    )
    .await;
}

async fn object(store: &FlowLikeStore, key: &Path) -> Vec<u8> {
    store
        .as_generic()
        .get(key)
        .await
        .unwrap_or_else(|error| panic!("reading {key} failed: {error}"))
        .bytes()
        .await
        .unwrap_or_else(|error| panic!("reading {key} failed: {error}"))
        .to_vec()
}

fn first_of_next_month(date: NaiveDate) -> NaiveDate {
    let (year, month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1).expect("valid month")
}

fn unhex(text: &str) -> Vec<u8> {
    hex::decode(text).unwrap_or_else(|_| panic!("{text} is hex"))
}

fn unhex32(text: &str) -> Hash {
    crypto::to_hash(&unhex(text)).unwrap_or_else(|| panic!("{text} is a 32-byte hash"))
}

fn parse_lines(bytes: &[u8]) -> Vec<Line> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).expect("audit NDJSON line"))
        .collect()
}

/// Standard ECDSA P-256 / SHA-256 verification with the 32 hash bytes as the message,
/// which is what an offline verifier does with the operator's public key.
fn check_signature(kid: &str, digest: &Hash, signature_b64: &str) {
    assert_eq!(kid, KID);
    let raw = STANDARD.decode(signature_b64).expect("base64 signature");
    let signature = Signature::from_slice(&raw).expect("raw 64-byte r || s signature");
    test_signer()
        .verifying_key()
        .verify(digest, &signature)
        .expect("signature verifies with the audit public key");
}

fn record_line_hash(record: &RecordLine) -> Hash {
    let ip = record.ip_commitment.as_deref().map(unhex);
    let details = record.details_commitment.as_deref().map(unhex);
    if let (Some(value), Some(salt), Some(commitment)) = (&record.actor_ip, &record.ip_salt, &ip) {
        assert_eq!(
            crypto::ip_commitment(&unhex32(salt), value).as_slice(),
            commitment.as_slice(),
            "record {} IP",
            record.id
        );
    }
    if let (Some(value), Some(salt), Some(commitment)) =
        (&record.details, &record.details_salt, &details)
    {
        assert_eq!(
            crypto::details_commitment(&unhex32(salt), value).as_slice(),
            commitment.as_slice(),
            "record {} details",
            record.id
        );
    }
    crypto::record_hash(&RecordFields {
        id: &record.id,
        chain_id: &record.chain_id,
        timestamp_ms: record.timestamp_ms,
        actor_id: &record.actor_id,
        actor_type: &record.actor_type,
        action: &record.action,
        resource_type: &record.resource_type,
        resource_id: &record.resource_id,
        ip_commitment: ip.as_deref(),
        details_commitment: details.as_deref(),
    })
}

fn seal_line_hash(seal: &SealLine) -> Hash {
    crypto::seal_hash(&SealFields {
        id: &seal.id,
        chain_id: &seal.chain_id,
        seq: seal.seq,
        class: &seal.class,
        prev_hash: &unhex(&seal.prev_hash),
        record_count: i64::from(seal.record_count),
        records_root: &unhex(&seal.records_root),
        first_at_ms: seal.first_at_ms,
        last_at_ms: seal.last_at_ms,
        sealed_at_ms: seal.sealed_at_ms,
    })
}

fn epoch_line_hash(epoch: &EpochLine) -> Hash {
    crypto::epoch_hash(&EpochFields {
        seq: epoch.seq,
        prev_hash: &unhex(&epoch.prev_hash),
        seal_count: i64::from(epoch.seal_count),
        seals_root: &unhex(&epoch.seals_root),
        created_at_ms: epoch.created_at_ms,
        kid: &epoch.kid,
    })
}

fn check_epoch_line(epoch: &EpochLine) {
    let hash = epoch_line_hash(epoch);
    assert_eq!(hex::encode(hash), epoch.hash, "epoch {} hash", epoch.seq);
    check_signature(&epoch.kid, &hash, &epoch.signature);
}

#[derive(Default)]
struct OfflineChain {
    seals: Vec<SealLine>,
    records: Vec<RecordLine>,
}

/// Everything a holder of an export or an archive can check without the database:
/// record commitments and hashes, seal roots and hashes, seal links, epoch inclusion
/// proofs, epoch hashes, links and signatures, and watermark signatures.
fn verify_offline(lines: &[Line]) -> HashMap<String, OfflineChain> {
    let mut epochs = BTreeMap::new();
    let mut watermarks: HashMap<String, WatermarkLine> = HashMap::new();
    let mut seal_lines = Vec::new();
    let mut sealed: HashMap<String, Vec<RecordLine>> = HashMap::new();
    for line in lines {
        match line {
            Line::Epoch(epoch) => {
                check_epoch_line(epoch);
                epochs.insert(epoch.seq, epoch.clone());
            }
            Line::Seal(seal) => seal_lines.push(seal.clone()),
            Line::Record(record) => match record.seal_id.as_deref() {
                Some(INVALID_SEAL_ID) => {}
                Some(seal_id) => sealed
                    .entry(seal_id.to_owned())
                    .or_default()
                    .push(record.clone()),
                None => panic!("record {} travels without a seal", record.id),
            },
            Line::Watermark(watermark) => {
                let leaf = crypto::watermark_hash(
                    &watermark.chain_id,
                    watermark.seq,
                    &unhex(&watermark.hash),
                    watermark.pruned_at_ms,
                );
                let root = unhex32(&watermark.batch_root);
                let proof: Vec<Hash> = watermark
                    .batch_proof
                    .iter()
                    .map(|hash| unhex32(hash))
                    .collect();
                assert!(
                    merkle::verify_inclusion(
                        &leaf,
                        watermark.batch_index as u64,
                        watermark.batch_size as u64,
                        &proof,
                        &root
                    ),
                    "watermark of {} is part of its batch",
                    watermark.chain_id
                );
                let batch = crypto::watermark_batch_hash(
                    &root,
                    i64::from(watermark.batch_size),
                    watermark.pruned_at_ms,
                    &watermark.kid,
                );
                check_signature(&watermark.kid, &batch, &watermark.signature);
                watermarks.insert(watermark.chain_id.clone(), watermark.clone());
            }
        }
    }
    let ordered: Vec<&EpochLine> = epochs.values().collect();
    for pair in ordered.windows(2) {
        if pair[1].seq == pair[0].seq + 1 {
            assert_eq!(
                pair[1].prev_hash, pair[0].hash,
                "epoch {} link",
                pair[1].seq
            );
        }
    }

    let mut chains: HashMap<String, OfflineChain> = HashMap::new();
    for seal in seal_lines {
        let mut records = sealed.remove(&seal.id).unwrap_or_default();
        records.sort_by(|left, right| {
            (left.timestamp_ms, left.id.as_bytes()).cmp(&(right.timestamp_ms, right.id.as_bytes()))
        });
        let label = format!("{} seal {}", seal.chain_id, seal.seq);
        assert_eq!(records.len(), seal.record_count as usize, "{label} records");
        assert!(
            records
                .iter()
                .all(|record| record.chain_id == seal.chain_id)
        );
        let leaves: Vec<Hash> = records.iter().map(record_line_hash).collect();
        assert_eq!(
            hex::encode(merkle::root(&leaves)),
            seal.records_root,
            "{label} root"
        );
        assert_eq!(
            records.first().map(|record| record.timestamp_ms),
            Some(seal.first_at_ms)
        );
        assert_eq!(
            records.last().map(|record| record.timestamp_ms),
            Some(seal.last_at_ms)
        );
        let class = if seal.chain_id.ends_with(ACTIVITY_SUFFIX) {
            "activity"
        } else {
            "evidence"
        };
        assert_eq!(seal.class, class, "{label} class");
        let hash = seal_line_hash(&seal);
        assert_eq!(hex::encode(hash), seal.hash, "{label} hash");
        let epoch_seq = seal
            .epoch_seq
            .unwrap_or_else(|| panic!("{label} is anchored"));
        let epoch = epochs
            .get(&epoch_seq)
            .unwrap_or_else(|| panic!("{label}: epoch {epoch_seq} travels with its seals"));
        let proof: Vec<Hash> = seal.epoch_proof.iter().map(|hash| unhex32(hash)).collect();
        let index = seal
            .epoch_index
            .unwrap_or_else(|| panic!("{label} epoch index"));
        assert!(
            merkle::verify_inclusion(
                &hash,
                index as u64,
                epoch.seal_count as u64,
                &proof,
                &unhex32(&epoch.seals_root),
            ),
            "{label} is part of epoch {epoch_seq}"
        );
        let chain = chains.entry(seal.chain_id.clone()).or_default();
        chain.records.extend(records);
        chain.seals.push(seal);
    }
    assert!(
        sealed.is_empty(),
        "records without their seal: {:?}",
        sealed.keys().collect::<Vec<_>>()
    );

    for (chain_id, chain) in &mut chains {
        chain.seals.sort_by_key(|seal| seal.seq);
        let mut previous = watermarks
            .get(chain_id)
            .map(|watermark| (watermark.seq, watermark.hash.clone()));
        if previous.is_none() && chain.seals.first().is_some_and(|seal| seal.seq == 1) {
            previous = Some((0, hex::encode(ZERO_HASH)));
        }
        for seal in &chain.seals {
            if let Some((seq, hash)) = &previous
                && seal.seq == seq + 1
            {
                assert_eq!(&seal.prev_hash, hash, "{chain_id} seal {} link", seal.seq);
            }
            previous = Some((seal.seq, seal.hash.clone()));
        }
    }
    chains
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn concurrent_writers_seal_into_one_verified_chain() {
    let harness = harness().await;
    let db = &harness.db;
    let app = create_id();

    let mut writers = tokio::task::JoinSet::new();
    for index in 0..48 {
        let db = db.clone();
        let app = app.clone();
        writers.spawn(async move {
            record::write(
                &db,
                input(&app, "board.update", &format!("board-{index}")),
                WriteMode::Append,
            )
            .await
        });
    }
    while let Some(result) = writers.join_next().await {
        result
            .expect("writer task")
            .expect("writers on one chain never conflict");
    }
    let pending = records(db, &app).await;
    assert_eq!(pending.len(), 48);
    assert!(
        pending
            .iter()
            .all(|record| record.seal_id.is_none() && record.mac.is_some())
    );
    let report = verify_full(db, &app).await;
    assert_eq!(
        (
            report.pending_records,
            report.pending_invalid,
            report.seals_checked
        ),
        (48, 0, 0),
        "{report:?}"
    );

    let context = worker_context(
        &harness,
        AuditRetention {
            max_records_per_seal: 10,
            ..retention()
        },
        None,
    );
    let ticked = tick(&context, clock(Duration::seconds(1))).await;
    assert!(
        ticked.sealed_records >= 48 && ticked.epochs >= 1,
        "{ticked:?}"
    );

    let chain = seals(db, &app).await;
    assert_eq!(
        chain.iter().map(|seal| seal.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert!(chain.iter().all(|seal| seal.record_count <= 10
        && seal.class == "evidence"
        && seal.epoch_seq.is_some()));
    assert!(
        records(db, &app)
            .await
            .iter()
            .all(|record| record.seal_id.is_some() && record.mac.is_none())
    );

    let report = verify_full(db, &app).await;
    assert!(report.valid, "{report:?}");
    assert_eq!(
        (
            report.seals_checked,
            report.records_checked,
            report.pending_records,
            report.unanchored_seals,
            report.redacted_values,
        ),
        (5, 48, 0, 0, 0),
        "{report:?}"
    );
    assert_eq!(report.latest_seal_seq, Some(5));
    let incremental = verify::verify_chain(db, &app, false)
        .await
        .expect("incremental verify");
    assert!(incremental.valid, "{incremental:?}");

    let timeline = verify::verify_epochs(db, true)
        .await
        .expect("verify epochs");
    assert!(timeline.valid, "{timeline:?}");
    let latest = epoch(db, timeline.latest_epoch_seq.expect("an epoch exists")).await;
    assert_eq!(
        verify::check_head(db, latest.seq, &latest.hash, None)
            .await
            .unwrap(),
        HeadCheck::Matches
    );
    assert_eq!(
        verify::check_head(db, latest.seq, &[0; 32], None)
            .await
            .unwrap(),
        HeadCheck::Differs
    );
    assert_eq!(
        verify::check_head(db, latest.seq + 1_000, &latest.hash, None)
            .await
            .unwrap(),
        HeadCheck::Differs
    );
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn details_normalised_by_jsonb_still_verify() {
    let harness = harness().await;
    let db = &harness.db;
    let app = create_id();
    let values = [
        json!({"v": -0.0}),
        json!({"v": 1e18}),
        json!({"v": 1e-7}),
        json!({"v": 1.2345678901234567e30}),
        json!({"v": f64::MIN_POSITIVE}),
        json!({"v": f64::MAX}),
        json!({"v": 9_007_199_254_740_993_u64}),
        json!({"v": [1.0, null, {"z": 2, "a": true}], "text": "tab\tquote\"é\u{0}"}),
        Value::Null,
    ];
    let count = values.len() as u64;
    for (index, details) in values.into_iter().enumerate() {
        let mut entry = input(&app, "board.update", &format!("numeric-{index}"));
        entry.details = Some(details);
        record::write(db, entry, WriteMode::Append)
            .await
            .expect("write record");
    }
    tick(
        &worker_context(&harness, retention(), None),
        clock(Duration::seconds(1)),
    )
    .await;
    let report = verify_full(db, &app).await;
    assert!(report.valid, "{report:?}");
    assert_eq!(
        (report.records_checked, report.pending_invalid),
        (count, 0),
        "{report:?}"
    );
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn once_only_writes_keep_one_record_per_resource() {
    let harness = harness().await;
    let db = &harness.db;
    let app = create_id();
    let action = "execution.board.complete";
    let chain_id = record::chain_for(Some(app.as_str()), action);
    assert_eq!(chain_id, format!("{app}{ACTIVITY_SUFFIX}"));

    let mut writers = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let db = db.clone();
        let app = app.clone();
        writers.spawn(async move {
            record::write(&db, input(&app, action, "run-1"), WriteMode::Once).await
        });
    }
    while let Some(result) = writers.join_next().await {
        result
            .expect("writer task")
            .expect("a repeated once-only write is a no-op");
    }
    let rows = records(db, &chain_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].id,
        crypto::once_id(&chain_id, action, "AuditTest", "run-1")
    );
    record::write(db, input(&app, action, "run-2"), WriteMode::Once)
        .await
        .expect("another resource gets its own record");

    tick(
        &worker_context(&harness, retention(), None),
        clock(Duration::seconds(1)),
    )
    .await;
    record::write(db, input(&app, action, "run-1"), WriteMode::Once)
        .await
        .expect("a sealed once-only record stays the only one");
    let report = verify_full(db, &chain_id).await;
    assert!(report.valid, "{report:?}");
    assert_eq!(
        (report.records_checked, report.pending_records),
        (2, 0),
        "{report:?}"
    );
    assert!(
        seals(db, &chain_id)
            .await
            .iter()
            .all(|seal| seal.class == "activity")
    );
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn sealed_tampering_is_reported_at_the_first_broken_seal() {
    let harness = harness().await;
    let db = &harness.db;
    let app = create_id();
    insert_records(db, &app, "board.update", clock(Duration::seconds(1)), 12).await;
    let context = worker_context(
        &harness,
        AuditRetention {
            max_records_per_seal: 4,
            ..retention()
        },
        None,
    );
    tick(&context, clock(Duration::seconds(1))).await;
    let chain = seals(db, &app).await;
    assert_eq!(chain.len(), 3);
    assert_valid(db, &app).await;

    let victim = members(db, &chain[1].id).await.remove(0);
    let victim_id: sea_orm::Value = victim.id.clone().into();

    exec(
        db,
        r#"UPDATE "AuditRecord" SET "resourceId" = 'tampered' WHERE "id" = $1"#,
        vec![victim_id.clone()],
    )
    .await;
    assert_broken(db, &app, 2, "an edited sealed record").await;
    exec(
        db,
        r#"UPDATE "AuditRecord" SET "resourceId" = $2 WHERE "id" = $1"#,
        vec![victim_id.clone(), victim.resource_id.clone().into()],
    )
    .await;
    assert_valid(db, &app).await;

    exec(
        db,
        r#"UPDATE "AuditRecord" SET "actorIp" = '198.51.100.7' WHERE "id" = $1"#,
        vec![victim_id.clone()],
    )
    .await;
    assert_broken(db, &app, 2, "an edited IP address").await;
    exec(
        db,
        r#"UPDATE "AuditRecord" SET "actorIp" = $2 WHERE "id" = $1"#,
        vec![victim_id.clone(), victim.actor_ip.clone().into()],
    )
    .await;
    assert_valid(db, &app).await;

    audit_record::Entity::delete_by_id(victim.id.clone())
        .exec(db)
        .await
        .expect("delete sealed record");
    assert_broken(db, &app, 2, "a deleted sealed record").await;
    audit_record::Entity::insert(audit_record::ActiveModel::from(victim.clone()))
        .exec_without_returning(db)
        .await
        .expect("restore sealed record");
    assert_valid(db, &app).await;

    set_seq(db, &chain[0].id, -1).await;
    set_seq(db, &chain[1].id, 1).await;
    set_seq(db, &chain[0].id, 2).await;
    assert_broken(db, &app, 1, "reordered seals").await;
    set_seq(db, &chain[1].id, -1).await;
    set_seq(db, &chain[0].id, 1).await;
    set_seq(db, &chain[1].id, 2).await;
    assert_valid(db, &app).await;

    let tail = chain.last().unwrap();
    audit_seal::Entity::delete_by_id(tail.id.clone())
        .exec(db)
        .await
        .unwrap();
    for full in [false, true, false] {
        let missing = verify::verify_chain(db, &app, full).await.unwrap();
        assert!(
            !missing.valid,
            "a deleted cached boundary went unnoticed: {missing:?}"
        );
        assert_eq!(missing.first_broken_seal, Some(tail.seq));
    }
    audit_seal::Entity::insert(audit_seal::ActiveModel::from(tail.clone()))
        .exec_without_returning(db)
        .await
        .unwrap();
    assert_valid(db, &app).await;

    let later_app = create_id();
    insert_records(
        db,
        &later_app,
        "board.update",
        clock(Duration::seconds(1)),
        1,
    )
    .await;
    tick(&context, clock(Duration::seconds(1))).await;
    assert_valid(db, &app).await;
    let epoch_tail = epoch(db, seals(db, &later_app).await[0].epoch_seq.unwrap()).await;
    let anchor = epoch(db, chain[0].epoch_seq.expect("anchored seal")).await;
    assert!(
        epoch_tail.seq > anchor.seq,
        "corrupt an epoch before the cached boundary"
    );
    let forged: Signature = SigningKey::from_slice(&[42; 32])
        .expect("valid P-256 scalar")
        .sign(&anchor.hash);
    exec(
        db,
        r#"UPDATE "AuditEpoch" SET "signature" = $2 WHERE "seq" = $1"#,
        vec![anchor.seq.into(), forged.to_bytes().to_vec().into()],
    )
    .await;
    assert_broken(db, &app, 1, "a forged epoch signature").await;
    let timeline = verify::verify_epochs(db, true)
        .await
        .expect("verify epochs");
    assert_eq!(
        (timeline.valid, timeline.first_broken_epoch),
        (false, Some(anchor.seq)),
        "{timeline:?}"
    );
    let incremental = verify::verify_epochs(db, false).await.unwrap();
    assert_eq!(
        (incremental.valid, incremental.first_broken_epoch),
        (false, Some(anchor.seq)),
        "a known invalid epoch signature was forgotten: {incremental:?}"
    );
    exec(
        db,
        r#"UPDATE "AuditEpoch" SET "signature" = $2 WHERE "seq" = $1"#,
        vec![anchor.seq.into(), anchor.signature.clone().into()],
    )
    .await;
    assert_valid(db, &app).await;

    audit_epoch::Entity::delete_by_id(epoch_tail.seq)
        .exec(db)
        .await
        .unwrap();
    for full in [false, true, false] {
        let missing = verify::verify_epochs(db, full).await.unwrap();
        assert!(
            !missing.valid,
            "a deleted cached epoch went unnoticed: {missing:?}"
        );
        assert_eq!(missing.first_broken_epoch, Some(epoch_tail.seq));
    }
    audit_epoch::Entity::insert(audit_epoch::ActiveModel::from(epoch_tail))
        .exec_without_returning(db)
        .await
        .unwrap();
    assert_valid(db, &app).await;
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn tampered_pending_records_are_quarantined() {
    let harness = harness().await;
    let db = &harness.db;
    let app = create_id();
    let ids = insert_records(db, &app, "board.update", clock(Duration::seconds(1)), 3).await;
    exec(
        db,
        r#"UPDATE "AuditRecord" SET "resourceId" = 'tampered' WHERE "id" = $1"#,
        vec![ids[1].clone().into()],
    )
    .await;
    exec(
        db,
        r#"UPDATE "AuditRecord" SET "mac" = NULL WHERE "id" = $1"#,
        vec![ids[2].clone().into()],
    )
    .await;
    let pending = verify_full(db, &app).await;
    assert!(!pending.valid, "{pending:?}");
    assert_eq!(
        (pending.pending_records, pending.pending_invalid),
        (3, 2),
        "{pending:?}"
    );

    let report = tick(
        &worker_context(&harness, retention(), None),
        clock(Duration::seconds(1)),
    )
    .await;
    assert!(report.quarantined >= 2, "{report:?}");
    for id in &ids[1..] {
        let record = audit_record::Entity::find_by_id(id.clone())
            .one(db)
            .await
            .expect("read record")
            .expect("quarantined records are kept");
        assert_eq!(record.seal_id.as_deref(), Some(INVALID_SEAL_ID));
    }
    let sealed = verify_full(db, &app).await;
    assert!(!sealed.valid, "{sealed:?}");
    assert_eq!(
        (
            sealed.records_checked,
            sealed.pending_records,
            sealed.pending_invalid,
            sealed.first_broken_seal,
        ),
        (1, 0, 2, None),
        "{sealed:?}"
    );

    for id in &ids[1..] {
        audit_record::Entity::delete_by_id(id.clone())
            .exec(db)
            .await
            .expect("remove the quarantined fixture");
    }
    assert_valid(db, &app).await;
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn expired_personal_values_are_redacted_not_broken() {
    let harness = harness().await;
    let db = &harness.db;
    let app = create_id();
    insert_records(db, &app, "board.update", clock(Duration::seconds(1)), 4).await;
    let context = worker_context(
        &harness,
        AuditRetention {
            ip_days: 1,
            details_days: Some(2),
            ..retention()
        },
        None,
    );
    tick(&context, clock(Duration::seconds(1))).await;
    let seal = seals(db, &app).await.remove(0);
    assert!(seal.ip_pending && seal.details_pending, "{seal:?}");
    let fresh = verify_full(db, &app).await;
    assert!(fresh.valid && fresh.redacted_values == 0, "{fresh:?}");

    let report = tick(&context, clock(Duration::days(1) + Duration::hours(1))).await;
    assert!(report.expired_values >= 4, "{report:?}");
    let rows = records(db, &app).await;
    assert!(rows.iter().all(|record| record.actor_ip.is_none()
        && record.ip_salt.is_none()
        && record.ip_commitment.is_some()
        && record.details.is_some()));
    let report = verify_full(db, &app).await;
    assert!(report.valid, "an expired IP is not tampering: {report:?}");
    assert_eq!(report.redacted_values, 4);

    tick(&context, clock(Duration::days(1))).await;
    let rows = records(db, &app).await;
    assert!(rows.iter().all(|record| record.details.is_none()
        && record.details_salt.is_none()
        && record.details_commitment.is_some()));
    let report = verify_full(db, &app).await;
    assert!(
        report.valid,
        "expired details are not tampering: {report:?}"
    );
    assert_eq!(report.redacted_values, 8);
    let seal = seals(db, &app).await.remove(0);
    assert!(!seal.ip_pending && !seal.details_pending, "{seal:?}");
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn enabling_details_expiry_covers_existing_and_legacy_seals() {
    let harness = harness().await;
    let db = &harness.db;
    let app = create_id();
    insert_records(db, &app, "board.update", clock(Duration::seconds(1)), 3).await;
    let context = worker_context(&harness, retention(), None);
    tick(&context, clock(Duration::seconds(1))).await;
    let original = seals(db, &app).await.remove(0);
    assert!(
        original.details_pending,
        "raw details must be tracked without an expiry policy"
    );

    // Reproduce flags written by workers predating the policy-independent flag.
    exec(
        db,
        r#"UPDATE "AuditSeal" SET "detailsPending" = false WHERE "id" = $1"#,
        vec![original.id.clone().into()],
    )
    .await;
    let finite = worker_context(
        &harness,
        AuditRetention {
            details_days: Some(1),
            ..retention()
        },
        None,
    );
    tick(&finite, clock(Duration::days(2))).await;
    let rows = records(db, &app).await;
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|record| record.details.is_none()
        && record.details_salt.is_none()
        && record.details_commitment.is_some()));
    let after = seals(db, &app).await.remove(0);
    assert!(!after.details_pending);
    assert_eq!(after.hash, original.hash);
    assert_valid(db, &app).await;
}

async fn assess(db: &DatabaseConnection, app_id: &str, version: i32, risk_category: &str) {
    exec(
        db,
        r#"INSERT INTO "AiActAssessment" ("id", "appId", "version", "status", "riskCategory", "answers", "updatedAt") VALUES ($1, $2, $3, 'APPROVED', $4, '[]'::jsonb, now())"#,
        vec![
            create_id().into(),
            app_id.into(),
            version.into(),
            risk_category.into(),
        ],
    )
    .await;
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn activity_prune_honours_the_high_risk_window() {
    let harness = harness().await;
    let db = &harness.db;
    let normal = create_id();
    let high_risk = create_id();
    assess(db, &normal, 1, "HIGH").await;
    assess(db, &normal, 2, "LIMITED").await;
    assess(db, &high_risk, 1, "MINIMAL").await;
    assess(db, &high_risk, 2, "HIGH").await;
    let normal_activity = format!("{normal}{ACTIVITY_SUFFIX}");
    let high_risk_activity = format!("{high_risk}{ACTIVITY_SUFFIX}");

    let start = clock(Duration::seconds(1));
    for app in [&normal, &high_risk] {
        insert_records(db, app, "execution.board.start", start, 3).await;
        insert_records(db, app, "board.update", start, 1).await;
    }
    let context = worker_context(
        &harness,
        AuditRetention {
            activity_days: 1,
            high_risk_activity_days: 3,
            ..retention()
        },
        None,
    );
    tick(&context, clock(Duration::seconds(1))).await;
    let pruned_seal = seals(db, &normal_activity).await.remove(0);
    assert_eq!(pruned_seal.class, "activity");
    assert_valid(db, &normal_activity).await;
    assert_valid(db, &high_risk_activity).await;

    let report = tick(&context, clock(Duration::days(2))).await;
    assert!(
        report.pruned_seals >= 1 && report.pruned_records >= 3,
        "{report:?}"
    );
    assert!(seals(db, &normal_activity).await.is_empty());
    assert!(records(db, &normal_activity).await.is_empty());
    let watermark = audit_watermark::Entity::find_by_id(normal_activity.clone())
        .one(db)
        .await
        .expect("read watermark")
        .expect("pruning leaves a watermark");
    assert_eq!(
        (watermark.seq, &watermark.hash, watermark.kid.as_str()),
        (pruned_seal.seq, &pruned_seal.hash, KID)
    );
    let report = verify_full(db, &normal_activity).await;
    assert!(report.valid, "{report:?}");
    assert_eq!(
        (
            report.pruned_before_seq,
            report.latest_seal_seq,
            report.empty
        ),
        (Some(1), None, false),
        "{report:?}"
    );
    assert_eq!(
        seals(db, &high_risk_activity).await.len(),
        1,
        "a high-risk AI system keeps its activity for the longer window"
    );
    for app in [&normal, &high_risk] {
        assert_eq!(
            records(db, app).await.len(),
            1,
            "evidence is never pruned without an archive"
        );
    }

    tick(&context, clock(Duration::days(2))).await;
    assert!(seals(db, &high_risk_activity).await.is_empty());
    let report = verify_full(db, &high_risk_activity).await;
    assert!(
        report.valid && report.pruned_before_seq == Some(1),
        "{report:?}"
    );

    insert_records(
        db,
        &normal,
        "execution.board.start",
        clock(Duration::seconds(1)),
        2,
    )
    .await;
    tick(&context, clock(Duration::seconds(1))).await;
    let continued = seals(db, &normal_activity).await;
    assert_eq!(continued.len(), 1);
    assert_eq!(
        (continued[0].seq, &continued[0].prev_hash),
        (2, &watermark.hash),
        "the next seal links to the watermark"
    );
    let report = verify_full(db, &normal_activity).await;
    assert!(report.valid, "{report:?}");
    assert_eq!(
        (
            report.pruned_before_seq,
            report.latest_seal_seq,
            report.records_checked
        ),
        (Some(1), Some(2), 2),
        "{report:?}"
    );
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn export_pages_verify_offline() {
    let harness = harness().await;
    let db = &harness.db;
    let app = create_id();
    insert_records(db, &app, "board.update", clock(Duration::seconds(1)), 5).await;
    let context = worker_context(
        &harness,
        AuditRetention {
            max_records_per_seal: 2,
            ..retention()
        },
        None,
    );
    tick(&context, clock(Duration::seconds(1))).await;
    let pending = insert_records(db, &app, "board.update", clock(Duration::seconds(1)), 1).await;

    let page = export::page(db, &app, 0, 100, 5_000)
        .await
        .expect("export page");
    assert_eq!(page.last_seq, Some(3));
    let lines = parse_lines(&page.body);
    assert!(!lines.iter().any(|line| matches!(line, Line::Watermark(_))));
    let chains = verify_offline(&lines);
    let chain = &chains[&app];
    assert_eq!(
        chain.seals.iter().map(|seal| seal.seq).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(chain.records.len(), 5);
    assert!(
        chain.records.iter().all(|record| record.id != pending[0]
            && record.actor_ip.is_some()
            && record.details.is_some()),
        "exports carry sealed records with their raw values"
    );

    let rest = export::page(db, &app, 2, 100, 5_000)
        .await
        .expect("export page");
    assert_eq!(rest.last_seq, Some(3));
    let rest = verify_offline(&parse_lines(&rest.body));
    assert_eq!(
        rest[&app]
            .seals
            .iter()
            .map(|seal| seal.seq)
            .collect::<Vec<_>>(),
        vec![3]
    );
    let bounded = export::page(db, &app, 0, 1, 5_000)
        .await
        .expect("export page");
    assert_eq!(bounded.last_seq, Some(1));
    let done = export::page(db, &app, 3, 100, 5_000)
        .await
        .expect("export page");
    assert_eq!(done.last_seq, None);
    assert!(
        !parse_lines(&done.body)
            .iter()
            .any(|line| matches!(line, Line::Seal(_) | Line::Record(_)))
    );
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn closed_month_is_archived_then_pruned_behind_a_watermark() {
    let harness = harness().await;
    let db = &harness.db;
    let store = memory_bucket();
    let context = worker_context(
        &harness,
        AuditRetention {
            archive_grace_days: 0,
            evidence_hot_days: 1,
            ..retention()
        },
        Some(store.clone()),
    );
    let app = create_id();
    insert_records(db, &app, "board.update", clock(Duration::seconds(1)), 6).await;
    let sealed_at = clock(Duration::seconds(1));
    let report = tick(&context, sealed_at).await;
    assert!(report.head_written, "{report:?}");

    let sealed = seals(db, &app).await;
    let last = sealed.last().expect("the chain is sealed").clone();
    let anchor = epoch(db, sealed[0].epoch_seq.expect("anchored seal")).await;
    let head: Value =
        serde_json::from_slice(&object(&store, &bucket::head_key(sealed_at.date_naive())).await)
            .expect("head JSON");
    let head: EpochLine = serde_json::from_value(head["epoch"].clone()).expect("head epoch");
    check_epoch_line(&head);
    assert!(head.seq >= anchor.seq);

    let month = anchor.created_at.with_timezone(&Utc).date_naive();
    let period = bucket::period_of(month);
    let next_month = first_of_next_month(month);
    clock_at(next_month.and_hms_opt(0, 0, 0).expect("midnight").and_utc() + Duration::days(2));
    // Each tick archives the oldest closed month; earlier months of the shared clock
    // may come first.
    let mut manifest_row = None;
    for _ in 0..24 {
        tick(&context, clock(Duration::seconds(1))).await;
        manifest_row = audit_archive::Entity::find_by_id((period.clone(), 0))
            .one(db)
            .await
            .expect("read archive rows");
        if manifest_row.is_some() {
            break;
        }
    }
    let manifest_row = manifest_row.unwrap_or_else(|| panic!("{period} was never archived"));
    assert_eq!(manifest_row.kid.as_deref(), Some(KID));

    let manifest_bytes = object(&store, &bucket::archive_manifest_key(&period)).await;
    assert_eq!(
        Sha256::digest(&manifest_bytes).as_slice(),
        manifest_row.sha256.as_slice()
    );
    let manifest: Value = serde_json::from_slice(&manifest_bytes).expect("manifest JSON");
    assert_eq!(manifest["format"], "flow-like.audit-archive/v1");
    assert_eq!(manifest["period"], period.as_str());
    assert_eq!(
        manifest["retain_until"],
        format!("{}-12-31", month.year() + 3).as_str()
    );
    assert_eq!(
        (&manifest["raw_ip"], &manifest["raw_details"]),
        (&json!(false), &json!(true))
    );
    let mut unsigned = manifest.clone();
    let signature = unsigned
        .as_object_mut()
        .expect("manifest object")
        .remove("signature")
        .expect("signed manifest");
    check_signature(
        manifest["kid"].as_str().expect("manifest kid"),
        &crypto::manifest_hash(&unsigned),
        signature.as_str().expect("base64 signature"),
    );

    let mut lines = Vec::new();
    for part in manifest["parts"].as_array().expect("manifest parts") {
        let key = Path::from(part["key"].as_str().expect("part key"));
        let bytes = object(&store, &key).await;
        assert_eq!(
            hex::encode(Sha256::digest(&bytes)),
            part["sha256"].as_str().expect("part digest")
        );
        lines.extend(parse_lines(
            &zstd::stream::decode_all(bytes.as_slice()).expect("zstd archive part"),
        ));
    }
    assert!(!lines.iter().any(|line| matches!(line, Line::Watermark(_))));
    let epoch_seqs: Vec<i64> = lines
        .iter()
        .filter_map(|line| match line {
            Line::Epoch(epoch) => Some(epoch.seq),
            _ => None,
        })
        .collect();
    assert_eq!(
        (epoch_seqs.first(), epoch_seqs.last()),
        (
            manifest["epochs"]["first"].as_i64().as_ref(),
            manifest["epochs"]["last"].as_i64().as_ref()
        )
    );
    let archived = verify_offline(&lines);
    let chain = &archived[&app];
    assert_eq!(
        chain
            .seals
            .iter()
            .map(|seal| seal.hash.clone())
            .collect::<Vec<_>>(),
        sealed
            .iter()
            .map(|seal| hex::encode(&seal.hash))
            .collect::<Vec<_>>()
    );
    assert_eq!(chain.records.len(), 6);
    assert!(
        chain
            .records
            .iter()
            .all(|record| record.actor_ip.is_none() && record.details.is_some()),
        "archives never carry raw IPs"
    );

    assert!(seals(db, &app).await.is_empty());
    assert!(records(db, &app).await.is_empty());
    let watermark = audit_watermark::Entity::find_by_id(app.clone())
        .one(db)
        .await
        .expect("read watermark")
        .expect("pruning leaves a watermark");
    assert_eq!((watermark.seq, &watermark.hash), (last.seq, &last.hash));
    let report = verify_full(db, &app).await;
    assert!(report.valid, "{report:?}");
    assert_eq!(
        (report.pruned_before_seq, report.seals_checked, report.empty),
        (Some(last.seq), 0, false),
        "{report:?}"
    );
    let timeline = verify::verify_epochs(db, true)
        .await
        .expect("verify epochs");
    assert!(timeline.valid, "{timeline:?}");
    let retained = verify::check_head(db, head.seq, &unhex(&head.hash), None)
        .await
        .expect("check head");
    assert!(
        matches!(retained, HeadCheck::Matches | HeadCheck::Archived),
        "{retained:?}"
    );

    insert_records(db, &app, "board.update", clock(Duration::seconds(1)), 2).await;
    tick(&context, clock(Duration::seconds(1))).await;
    let report = verify_full(db, &app).await;
    assert!(report.valid, "{report:?}");
    assert_eq!(
        (
            report.pruned_before_seq,
            report.latest_seal_seq,
            report.records_checked
        ),
        (Some(last.seq), Some(last.seq + 1), 2),
        "{report:?}"
    );

    let page = export::page(db, &app, 0, 100, 5_000)
        .await
        .expect("export page");
    let lines = parse_lines(&page.body);
    assert!(
        matches!(lines.first(), Some(Line::Watermark(first)) if first.chain_id == app && first.seq == last.seq),
        "a page that starts below the watermark opens with it"
    );
    let exported = verify_offline(&lines);
    assert_eq!(exported[&app].seals[0].seq, last.seq + 1);
    assert_eq!(page.last_seq, Some(last.seq + 1));
}

/// Signs like the audit key and counts every request, standing in for a billed key
/// service.
struct CountingSigner {
    inner: LocalSigner,
    requests: Arc<AtomicUsize>,
}

#[flow_like_types::async_trait]
impl AuditSigner for CountingSigner {
    fn kid(&self) -> &str {
        self.inner.kid()
    }

    fn verifying_key(&self) -> VerifyingKey {
        self.inner.verifying_key()
    }

    async fn sign(&self, digest: &Hash) -> flow_like_types::Result<signer::RawSignature> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.inner.sign(digest).await
    }
}

fn counting_context(
    harness: &Harness,
    retention: AuditRetention,
    requests: &Arc<AtomicUsize>,
) -> AuditWorkerContext {
    signed_worker_context(
        harness,
        retention,
        None,
        Arc::new(CountingSigner {
            inner: test_signer(),
            requests: requests.clone(),
        }),
    )
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn key_service_requests_stay_bounded() {
    let harness = harness().await;
    let db = &harness.db;
    let requests = Arc::new(AtomicUsize::new(0));
    let signed = || requests.load(Ordering::SeqCst);
    let apps: Vec<String> = (0..25).map(|_| create_id()).collect();
    let start = clock(Duration::seconds(1));
    for app in &apps {
        insert_records(db, app, "execution.board.start", start, 4).await;
    }

    let sealing = counting_context(
        &harness,
        AuditRetention {
            epoch_interval_seconds: 300,
            ..retention()
        },
        &requests,
    );
    let sealed = tick(&sealing, clock(Duration::seconds(1))).await;
    assert!(sealed.seals >= 25, "{sealed:?}");
    assert_eq!(
        (sealed.epochs, signed()),
        (0, 0),
        "sealed records wait for the epoch interval under their seal MAC: {sealed:?}"
    );
    for app in &apps {
        assert_valid(db, &format!("{app}{ACTIVITY_SUFFIX}")).await;
    }

    let anchored = tick(&sealing, clock(Duration::seconds(301))).await;
    assert!(anchored.anchored_seals >= 25, "{anchored:?}");
    assert_eq!(
        (anchored.epochs, signed()),
        (1, 1),
        "one epoch signature covers every chain's seals: {anchored:?}"
    );
    let quiet = tick(&sealing, clock(Duration::seconds(30))).await;
    assert_eq!((quiet.epochs, signed()), (0, 1), "{quiet:?}");

    let pruning = counting_context(
        &harness,
        AuditRetention {
            activity_days: 1,
            high_risk_activity_days: 1,
            ..retention()
        },
        &requests,
    );
    let pruned = tick(&pruning, clock(Duration::days(2))).await;
    assert_eq!(
        signed(),
        2,
        "one signature covers every chain's watermark: {pruned:?}"
    );
    let mut signatures = Vec::new();
    for app in &apps {
        let chain = format!("{app}{ACTIVITY_SUFFIX}");
        assert!(seals(db, &chain).await.is_empty(), "{chain} is pruned");
        let watermark = audit_watermark::Entity::find_by_id(chain.clone())
            .one(db)
            .await
            .expect("read watermark")
            .expect("pruning leaves a watermark");
        assert!(watermark.batch_size >= 25);
        signatures.push(watermark.signature);
        assert!(verify_full(db, &chain).await.valid);
    }
    assert!(signatures.windows(2).all(|pair| pair[0] == pair[1]));

    let steady = tick(&pruning, clock(Duration::minutes(10))).await;
    assert_eq!(
        signed(),
        2,
        "an idle worker makes no key-service request: {steady:?}"
    );
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn a_rewritten_unanchored_seal_is_reported_and_never_signed() {
    let harness = harness().await;
    let db = &harness.db;
    let forged_app = create_id();
    let honest_app = create_id();
    let start = clock(Duration::seconds(1));
    insert_records(db, &forged_app, "board.update", start, 3).await;
    insert_records(db, &honest_app, "board.update", start, 3).await;
    let context = worker_context(
        &harness,
        AuditRetention {
            epoch_interval_seconds: 300,
            ..retention()
        },
        None,
    );
    tick(&context, clock(Duration::seconds(1))).await;

    // Rewrite a sealed record and recompute the seal's root and hash, as someone with
    // database access but without the entry key could.
    let mut forged_records = records(db, &forged_app).await;
    forged_records[0].resource_id = "forged".into();
    let mut ordered = forged_records.clone();
    verify::sort_records(&mut ordered);
    let leaves: Vec<Hash> = ordered
        .iter()
        .map(|record| verify::check_record(record).expect("record hashes").0)
        .collect();
    let mut seal = seals(db, &forged_app).await.remove(0);
    seal.records_root = merkle::root(&leaves).to_vec();
    seal.hash = verify::seal_hash_of(&seal).to_vec();
    exec(
        db,
        r#"UPDATE "AuditRecord" SET "resourceId" = 'forged' WHERE "id" = $1"#,
        vec![forged_records[0].id.clone().into()],
    )
    .await;
    exec(
        db,
        r#"UPDATE "AuditSeal" SET "recordsRoot" = $2, "hash" = $3 WHERE "id" = $1"#,
        vec![
            seal.id.clone().into(),
            seal.records_root.clone().into(),
            seal.hash.clone().into(),
        ],
    )
    .await;

    let report = verify_full(db, &forged_app).await;
    assert!(!report.valid, "{report:?}");
    assert_eq!(report.first_broken_seal, Some(seal.seq), "{report:?}");
    assert_eq!(
        report.problem.as_deref(),
        Some("unanchored seal fails its MAC"),
        "{report:?}"
    );

    tick(&context, clock(Duration::seconds(301))).await;
    assert!(
        seals(db, &forged_app).await[0].epoch_seq.is_none(),
        "the forged seal is never signed"
    );
    assert!(seals(db, &honest_app).await[0].epoch_seq.is_some());
    assert_valid(db, &honest_app).await;
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn legacy_entries_and_late_writers_are_exported_before_deletion() {
    let harness = harness().await;
    let db = &harness.db;
    let ids: Vec<String> = (0..3).map(|_| create_id()).collect();
    for (index, id) in ids.iter().enumerate() {
        exec(
            db,
            r#"INSERT INTO "AuditEntry" ("id", "sequence", "timestamp", "actorId", "actorType", "action", "resourceType", "resourceId", "summary", "entryHash", "prevHash") VALUES ($1, $2, now(), 'legacy-user', 'USER', 'app.update', 'App', $1, 'legacy entry', 'hash', 'prev')"#,
            vec![id.clone().into(), (index as i64 + 1).into()],
        )
        .await;
    }
    let store = memory_bucket();
    let context = worker_context(&harness, retention(), Some(store.clone()));
    let mut deleted = 0;
    for _ in 0..4 {
        deleted += tick(&context, clock(Duration::seconds(1)))
            .await
            .legacy_exported;
        if audit_entry::Entity::find()
            .count(db)
            .await
            .expect("count legacy entries")
            == 0
        {
            break;
        }
    }
    assert!(deleted >= 3, "legacy rows deleted: {deleted}");

    let row = audit_archive::Entity::find_by_id(("legacy".to_owned(), 0))
        .one(db)
        .await
        .expect("read archive rows")
        .expect("the legacy export is recorded");
    assert_eq!(row.record_count, 3);
    assert_eq!(row.object_key, bucket::legacy_key().to_string());
    let bytes = object(&store, &bucket::legacy_key()).await;
    assert_eq!(Sha256::digest(&bytes).as_slice(), row.sha256.as_slice());
    let text = String::from_utf8(zstd::stream::decode_all(bytes.as_slice()).expect("zstd"))
        .expect("UTF-8 NDJSON");
    let exported: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("legacy JSON line"))
        .collect();
    assert_eq!(exported.len(), 3);
    for id in &ids {
        assert!(exported.iter().any(|entry| entry["id"] == id.as_str()));
    }

    // An older API replica can append after the first export during a rolling upgrade.
    let late_id = create_id();
    exec(
        db,
        r#"INSERT INTO "AuditEntry" ("id", "sequence", "timestamp", "actorId", "actorType", "action", "resourceType", "resourceId", "summary", "entryHash", "prevHash") VALUES ($1, 4, now(), 'legacy-user', 'USER', 'app.update', 'App', $1, 'late entry', 'hash', 'prev')"#,
        vec![late_id.clone().into()],
    ).await;
    tick(&context, clock(Duration::seconds(1))).await;
    assert!(
        audit_entry::Entity::find_by_id(&late_id)
            .one(db)
            .await
            .unwrap()
            .is_none()
    );
    let late_archive = audit_archive::Entity::find()
        .filter(audit_archive::Column::Period.eq("legacy"))
        .filter(audit_archive::Column::Part.gt(0))
        .order_by_desc(audit_archive::Column::Part)
        .one(db)
        .await
        .unwrap()
        .expect("late entries need their own archive");
    let bytes = object(&store, &Path::from(late_archive.object_key)).await;
    assert_eq!(
        Sha256::digest(&bytes).as_slice(),
        late_archive.sha256.as_slice()
    );
    let text = String::from_utf8(zstd::stream::decode_all(bytes.as_slice()).unwrap()).unwrap();
    assert!(
        text.lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|entry| entry["id"] == late_id)
    );
}
