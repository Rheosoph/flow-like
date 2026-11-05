//! Object state and aggregate totals share one transaction. Tombstones preserve event ordering.
//! A notification batch commits one transaction per app chunk, so the app's coordination row and
//! its App, User and AccountCapacity totals are written once per chunk with the summed delta.

use flow_like_db::{retry_transaction, DbDialect, RetryPolicy};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr, QueryResult, Statement, Value,
};
use std::collections::{btree_map, BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

/// DSQL caps a transaction at 3,000 modified rows. An object modifies at most five: its
/// accounting row, its grant, its user's total and its own fallback payer's account twice;
/// the chunk adds the app's lock and total rows.
const MAX_OBJECTS_PER_TRANSACTION: usize = 500;
const MAX_SAMPLE_ROUNDS: usize = 8;
const MAX_CONCURRENT_READS: usize = 32;

/// A chunk carries many objects, so a lost commit race is worth a longer wait than a single
/// object's: the budget outlasts several competing commits on the same app. Delays stay within
/// one second, the range of the sub-second jitter source in `flow_like_db`.
const CHUNK_RETRY: RetryPolicy = RetryPolicy {
    max_attempts: 12,
    base_delay: Duration::from_millis(20),
    max_delay: Duration::from_secs(1),
    max_total: Duration::from_secs(20),
    idempotent: true,
};

pub type Failure = Arc<DbErr>;

#[derive(Clone, Debug)]
pub struct Observation {
    pub bucket: String,
    pub key: String,
    pub app_id: String,
    pub user_id: Option<String>,
    pub sequencer: String,
    pub legacy_size: i64,
}

impl Observation {
    fn id(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.bucket.as_bytes());
        hasher.update(&[0]);
        hasher.update(self.key.as_bytes());
        hasher.finalize().to_hex().to_string()
    }
}

struct Accounted {
    size: i64,
    sequencer: String,
    payer_id: Option<String>,
}

impl Accounted {
    fn from_row(row: &QueryResult) -> Result<Self, DbErr> {
        Ok(Self {
            size: row.try_get("", "size")?,
            sequencer: row.try_get("", "sequencer")?,
            payer_id: row.try_get("", "payerId")?,
        })
    }
}

/// One object of a batch at its newest observation, and the input positions it answers for.
struct Tracked {
    id: String,
    observation: Observation,
    positions: Vec<usize>,
    outcome: Option<Result<(), Failure>>,
}

/// A pending object with its size sampled after `snapshot` was read.
struct Sampled {
    tracked: usize,
    id: String,
    observation: Observation,
    snapshot: Option<(i64, String)>,
    size: i64,
}

struct Write<'a> {
    object: &'a Sampled,
    previous: Option<&'a Accounted>,
    payer: Option<String>,
    delta: i64,
    /// Whether the payer's account moves: bytes changed or the object has an upload grant.
    settles: bool,
}

fn non_negative(size: i64) -> Result<i64, DbErr> {
    if size < 0 {
        return Err(DbErr::Custom("object size must not be negative".into()));
    }
    Ok(size)
}

fn add_delta(total: i64, delta: i64) -> Result<i64, DbErr> {
    total
        .checked_add(delta)
        .ok_or_else(|| DbErr::Custom(format!("object size delta overflow: {total} + {delta}")))
}

pub fn normalize_sequencer(value: &str) -> Result<String, String> {
    if value.is_empty() || !value.bytes().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("S3 event sequencer must be a nonempty hexadecimal number".into());
    }
    let trimmed = value.trim_start_matches('0');
    Ok(if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.to_ascii_lowercase()
    })
}

fn newer_than(candidate: &str, previous: &str) -> bool {
    previous.is_empty()
        || candidate.len() > previous.len()
        || (candidate.len() == previous.len() && candidate > previous)
}

fn parameters(count: usize) -> String {
    (1..=count)
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// `VALUES` rows cast per column, so PostgreSQL and Aurora DSQL infer the same types.
fn typed_rows(count: usize, casts: &[&str]) -> String {
    (0..count)
        .map(|row| {
            let columns = casts
                .iter()
                .enumerate()
                .map(|(column, cast)| format!("${}::{cast}", row * casts.len() + column + 1))
                .collect::<Vec<_>>();
            format!("({})", columns.join(","))
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn ids_of(rows: &[QueryResult]) -> Result<HashSet<String>, DbErr> {
    rows.iter().map(|row| row.try_get("", "id")).collect()
}

async fn accounted<C: ConnectionTrait>(
    connection: &C,
    ids: &[&str],
) -> Result<HashMap<String, Accounted>, DbErr> {
    let rows = connection
        .query_all_raw(Statement::from_sql_and_values(
            connection.get_database_backend(),
            format!(
                r#"SELECT "id", "size", "sequencer", "payerId" FROM "FileAccountingObject" WHERE "id" IN ({})"#,
                parameters(ids.len())
            ),
            ids.iter().map(|id| Value::from(*id)),
        ))
        .await?;
    rows.iter()
        .map(|row| Ok((row.try_get("", "id")?, Accounted::from_row(row)?)))
        .collect()
}

async fn grants(
    txn: &DatabaseTransaction,
    ids: &[&str],
) -> Result<HashMap<String, (i64, i64)>, DbErr> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = txn
        .query_all_raw(Statement::from_sql_and_values(
            txn.get_database_backend(),
            format!(
                r#"SELECT "id", "maxBytes", "reservedBytes" FROM "StorageUploadGrant" WHERE "id" IN ({})"#,
                parameters(ids.len())
            ),
            ids.iter().map(|id| Value::from(*id)),
        ))
        .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get("", "id")?,
                (
                    row.try_get("", "maxBytes")?,
                    row.try_get("", "reservedBytes")?,
                ),
            ))
        })
        .collect()
}

async fn app_payer(txn: &DatabaseTransaction, app_id: &str) -> Result<Option<String>, DbErr> {
    Ok(txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        r#"SELECT COALESCE((SELECT "payerId" FROM "ProjectCapacity" WHERE "appId" = $1),
            (SELECT m."userId" FROM "Membership" m JOIN "App" a ON a."ownerRoleId" = m."roleId" AND a."id" = m."appId" WHERE a."id" = $1 ORDER BY m."userId" LIMIT 1)) AS "payerId""#,
        [app_id.into()])).await?.map(|row| row.try_get::<Option<String>>("", "payerId")).transpose()?.flatten())
}

/// New rows are written in their final state; the legacy baseline is what their delta starts from.
async fn insert_objects(
    txn: &DatabaseTransaction,
    writes: &[&Write<'_>],
) -> Result<HashSet<String>, DbErr> {
    if writes.is_empty() {
        return Ok(HashSet::new());
    }
    let mut values = Vec::<Value>::with_capacity(writes.len() * 8);
    for write in writes {
        let observation = &write.object.observation;
        values.extend([
            write.object.id.clone().into(),
            observation.bucket.clone().into(),
            observation.key.clone().into(),
            observation.app_id.clone().into(),
            observation.user_id.clone().into(),
            write.object.size.into(),
            observation.sequencer.clone().into(),
            write.payer.clone().into(),
        ]);
    }
    let rows = txn.query_all_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        format!(r#"INSERT INTO "FileAccountingObject" ("id", "bucket", "objectKey", "appId", "userId", "size", "sequencer", "payerId", "updatedAt")
            SELECT v."id", v."bucket", v."objectKey", v."appId", v."userId", v."size", v."sequencer", v."payerId", now()
            FROM (VALUES {}) AS v("id", "bucket", "objectKey", "appId", "userId", "size", "sequencer", "payerId")
            ON CONFLICT ("id") DO NOTHING RETURNING "id""#,
            typed_rows(writes.len(), &["text", "text", "text", "text", "text", "bigint", "text", "text"])),
        values)).await?;
    ids_of(&rows)
}

/// Each row moves only from the state its size sample was taken against.
async fn update_objects(
    txn: &DatabaseTransaction,
    writes: &[&Write<'_>],
) -> Result<HashSet<String>, DbErr> {
    let mut values = Vec::<Value>::with_capacity(writes.len() * 6);
    for write in writes {
        let Some(previous) = write.previous else {
            continue;
        };
        values.extend([
            write.object.id.clone().into(),
            write.object.size.into(),
            write.object.observation.sequencer.clone().into(),
            write.payer.clone().into(),
            previous.size.into(),
            previous.sequencer.clone().into(),
        ]);
    }
    if values.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = txn.query_all_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        format!(r#"UPDATE "FileAccountingObject" AS f SET "size" = v."size", "sequencer" = v."sequencer", "payerId" = v."payerId", "updatedAt" = now()
            FROM (VALUES {}) AS v("id", "size", "sequencer", "payerId", "oldSize", "oldSequencer")
            WHERE f."id" = v."id" AND f."size" = v."oldSize" AND f."sequencer" = v."oldSequencer" RETURNING f."id""#,
            typed_rows(values.len() / 6, &["text", "bigint", "text", "text", "bigint", "text"])),
        values)).await?;
    ids_of(&rows)
}

/// The ids whose rows this transaction wrote; a row that moved since its snapshot is skipped.
async fn write_objects(
    txn: &DatabaseTransaction,
    writes: &[Write<'_>],
) -> Result<HashSet<String>, DbErr> {
    let (fresh, existing): (Vec<&Write>, Vec<&Write>) =
        writes.iter().partition(|write| write.previous.is_none());
    let mut written = insert_objects(txn, &fresh).await?;
    written.extend(update_objects(txn, &existing).await?);
    Ok(written)
}

async fn update_grants(
    txn: &DatabaseTransaction,
    reservations: &[(&str, i64)],
) -> Result<(), DbErr> {
    if reservations.is_empty() {
        return Ok(());
    }
    let values = reservations
        .iter()
        .flat_map(|(id, reserved)| [Value::from(*id), Value::from(*reserved)])
        .collect::<Vec<_>>();
    txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        format!(r#"UPDATE "StorageUploadGrant" AS g SET "reservedBytes" = v."reservedBytes", "updatedAt" = now()
            FROM (VALUES {}) AS v("id", "reservedBytes") WHERE g."id" = v."id""#,
            typed_rows(reservations.len(), &["text", "bigint"])),
        values)).await?;
    Ok(())
}

/// Objects already at a sequencer at least as new are settled; the rest are written only when
/// their row still matches the snapshot their size was sampled against.
fn plan_writes<'a>(
    objects: &'a [Sampled],
    rows: &'a HashMap<String, Accounted>,
    app_payer: Option<&str>,
) -> Result<(Vec<usize>, Vec<Write<'a>>), DbErr> {
    let mut settled = Vec::new();
    let mut writes = Vec::new();
    for object in objects {
        let previous = rows.get(&object.id);
        if previous.is_some_and(|row| !newer_than(&object.observation.sequencer, &row.sequencer)) {
            settled.push(object.tracked);
            continue;
        }
        let unchanged = match (&object.snapshot, previous) {
            (Some((size, sequencer)), Some(row)) => {
                *size == row.size && *sequencer == row.sequencer
            }
            (None, None) => true,
            _ => false,
        };
        if !unchanged {
            continue;
        }
        let old_size = previous.map_or(object.observation.legacy_size, |row| row.size);
        let delta = object.size.checked_sub(old_size).ok_or_else(|| {
            DbErr::Custom(format!(
                "object size delta overflow: {} - {old_size}",
                object.size
            ))
        })?;
        writes.push(Write {
            object,
            previous,
            payer: app_payer
                .map(str::to_owned)
                .or_else(|| previous.and_then(|row| row.payer_id.clone())),
            delta,
            settles: false,
        });
    }
    Ok((settled, writes))
}

/// An event that moves no bytes and has no upload grant changes nothing on the account. Grants
/// are only issued under the app lock, so none can appear before commit; an existing grant is
/// read again under the payer lock when the account is settled.
async fn lock_payers(txn: &DatabaseTransaction, writes: &mut [Write<'_>]) -> Result<(), DbErr> {
    let idle = writes
        .iter()
        .filter(|write| write.payer.is_some() && write.delta == 0)
        .map(|write| write.object.id.as_str())
        .collect::<Vec<_>>();
    let granted = grants(txn, &idle).await?;
    for write in writes.iter_mut() {
        write.settles =
            write.payer.is_some() && (write.delta != 0 || granted.contains_key(&write.object.id));
    }
    let payers = writes
        .iter()
        .filter(|write| write.settles)
        .filter_map(|write| write.payer.as_deref())
        .collect::<BTreeSet<_>>();
    for payer in payers {
        // Lock before changing App.totalSize: initialization reads that total under the same
        // payer lock and must see either the before or after state.
        txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
            r#"INSERT INTO "AccountCapacity" ("payerId") VALUES ($1) ON CONFLICT ("payerId") DO UPDATE SET "updatedAt"=now()"#,
            [payer.into()])).await?;
    }
    Ok(())
}

async fn update_account(
    txn: &DatabaseTransaction,
    app_id: &str,
    payer: &str,
    stored: i64,
    reserved: Option<i64>,
) -> Result<(), DbErr> {
    match reserved {
        // One write settles both totals; stored bytes still move only for a retained project.
        Some(reserved) => {
            txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                r#"UPDATE "AccountCapacity" SET "storageBytes" = CASE WHEN EXISTS(SELECT 1 FROM "ProjectCapacity" p WHERE p."appId"=$3 AND p."payerId"=$1) THEN GREATEST(0,"storageBytes" + $2) ELSE "storageBytes" END, "reservedStorageBytes" = GREATEST(0,"reservedStorageBytes" + $4), "updatedAt" = now() WHERE "payerId" = $1"#,
                [payer.into(), stored.into(), app_id.into(), reserved.into()])).await?;
        }
        None if stored != 0 => {
            txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                r#"UPDATE "AccountCapacity" SET "storageBytes" = GREATEST(0,"storageBytes" + $2), "updatedAt" = now() WHERE "payerId" = $1 AND EXISTS(SELECT 1 FROM "ProjectCapacity" p WHERE p."appId"=$3 AND p."payerId"=$1)"#,
                [payer.into(), stored.into(), app_id.into()])).await?;
        }
        None => {}
    }
    Ok(())
}

/// Moves each payer's stored bytes by its objects' summed delta and re-derives their grants'
/// reservations from the new sizes, in one account write per payer.
async fn settle_accounts(
    txn: &DatabaseTransaction,
    app_id: &str,
    written: &[&Write<'_>],
) -> Result<(), DbErr> {
    let settling = written
        .iter()
        .filter(|write| write.settles)
        .collect::<Vec<_>>();
    let ids = settling
        .iter()
        .map(|write| write.object.id.as_str())
        .collect::<Vec<_>>();
    let current = grants(txn, &ids).await?;
    let mut accounts = BTreeMap::<&str, (i64, Option<i64>)>::new();
    let mut reservations = Vec::new();
    for write in settling {
        let Some(payer) = write.payer.as_deref() else {
            continue;
        };
        let (stored, reserved) = accounts.entry(payer).or_insert((0, None));
        *stored = add_delta(*stored, write.delta)?;
        if let Some((max_bytes, old_reserved)) = current.get(&write.object.id) {
            let now_reserved = max_bytes.saturating_sub(write.object.size).max(0);
            reservations.push((write.object.id.as_str(), now_reserved));
            *reserved = Some(add_delta(
                reserved.unwrap_or(0),
                now_reserved - old_reserved,
            )?);
        }
    }
    update_grants(txn, &reservations).await?;
    for (payer, (stored, reserved)) in accounts {
        update_account(txn, app_id, payer, stored, reserved).await?;
    }
    Ok(())
}

/// A late storage event can outlive its app or user. Updating zero rows is valid:
/// the retained object tombstone still prevents a duplicate event being counted.
async fn add_totals(
    txn: &DatabaseTransaction,
    app_id: &str,
    written: &[&Write<'_>],
) -> Result<(), DbErr> {
    let mut app_delta = 0_i64;
    let mut users = BTreeMap::<&str, i64>::new();
    for write in written {
        app_delta = add_delta(app_delta, write.delta)?;
        if let Some(user_id) = write.object.observation.user_id.as_deref() {
            let total = users.entry(user_id).or_insert(0);
            *total = add_delta(*total, write.delta)?;
        }
    }
    if app_delta != 0 {
        txn.execute_raw(Statement::from_sql_and_values(
            txn.get_database_backend(),
            r#"UPDATE "App" SET "totalSize" = "totalSize" + $2 WHERE "id" = $1"#,
            [app_id.into(), app_delta.into()],
        ))
        .await?;
    }
    for (user_id, delta) in users.into_iter().filter(|(_, delta)| *delta != 0) {
        txn.execute_raw(Statement::from_sql_and_values(
            txn.get_database_backend(),
            r#"UPDATE "User" SET "totalSize" = "totalSize" + $2 WHERE "id" = $1"#,
            [user_id.into(), delta.into()],
        ))
        .await?;
    }
    Ok(())
}

/// Commits every object whose row still matches its sample. Returns the tracked indices that
/// are settled: written now, or already at a sequencer at least as new.
async fn commit_chunk(
    txn: &DatabaseTransaction,
    app_id: &str,
    objects: &[Sampled],
) -> Result<Vec<usize>, DbErr> {
    flow_like_db::coordination::app_capacity(txn, app_id).await?;
    let ids = objects
        .iter()
        .map(|object| object.id.as_str())
        .collect::<Vec<_>>();
    let rows = accounted(txn, &ids).await?;
    let payer = app_payer(txn, app_id).await?;
    let (mut settled, mut writes) = plan_writes(objects, &rows, payer.as_deref())?;
    lock_payers(txn, &mut writes).await?;
    let written_ids = write_objects(txn, &writes).await?;
    let written = writes
        .iter()
        .filter(|write| written_ids.contains(&write.object.id))
        .collect::<Vec<_>>();
    settle_accounts(txn, app_id, &written).await?;
    add_totals(txn, app_id, &written).await?;
    settled.extend(written.iter().map(|write| write.object.tracked));
    Ok(settled)
}

/// Runs `read` for each listed object, a bounded number at a time. Results follow `indices`.
async fn read_sizes<R, Fut>(
    tracked: &[Tracked],
    indices: &[usize],
    read: &R,
) -> Vec<Result<i64, DbErr>>
where
    R: Fn(&Observation) -> Fut,
    Fut: Future<Output = Result<i64, DbErr>> + Send + 'static,
{
    let mut sizes = indices
        .iter()
        .map(|&index| {
            Err(DbErr::Custom(format!(
                "object size read for {} did not run",
                tracked[index].observation.key
            )))
        })
        .collect::<Vec<_>>();
    let mut tasks = tokio::task::JoinSet::new();
    let mut slots = HashMap::new();
    let mut queued = indices.iter().enumerate();
    loop {
        while tasks.len() < MAX_CONCURRENT_READS {
            let Some((slot, &index)) = queued.next() else {
                break;
            };
            slots.insert(tasks.spawn(read(&tracked[index].observation)).id(), slot);
        }
        let Some(joined) = tasks.join_next_with_id().await else {
            return sizes;
        };
        match joined {
            Ok((task, size)) => sizes[slots[&task]] = size,
            Err(error) => {
                let slot = slots[&error.id()];
                sizes[slot] = Err(DbErr::Custom(format!(
                    "object size read for {} failed: {error}",
                    tracked[indices[slot]].observation.key
                )));
            }
        }
    }
}

fn fail_pending(tracked: &mut [Tracked], error: DbErr) {
    let failure = Arc::new(error);
    for object in tracked.iter_mut().filter(|object| object.outcome.is_none()) {
        object.outcome = Some(Err(failure.clone()));
    }
}

/// Pending objects with the accounting snapshot their size is sampled after. Objects whose row
/// is already at a sequencer at least as new are settled without sampling.
async fn unsettled(
    db: &DatabaseConnection,
    tracked: &mut [Tracked],
) -> Result<Vec<(usize, Option<(i64, String)>)>, DbErr> {
    let pending = (0..tracked.len())
        .filter(|&index| tracked[index].outcome.is_none())
        .collect::<Vec<_>>();
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let ids = pending
        .iter()
        .map(|&index| tracked[index].id.as_str())
        .collect::<Vec<_>>();
    let snapshots = accounted(db, &ids).await?;
    let mut candidates = Vec::with_capacity(pending.len());
    for index in pending {
        let snapshot = snapshots
            .get(&tracked[index].id)
            .map(|row| (row.size, row.sequencer.clone()));
        if snapshot.as_ref().is_some_and(|(_, sequencer)| {
            !newer_than(&tracked[index].observation.sequencer, sequencer)
        }) {
            tracked[index].outcome = Some(Ok(()));
        } else {
            candidates.push((index, snapshot));
        }
    }
    Ok(candidates)
}

/// Pairs each object with its sampled size; an object whose read fails is failed on its own.
async fn sample<R, Fut>(
    tracked: &mut [Tracked],
    candidates: Vec<(usize, Option<(i64, String)>)>,
    read_current_size: &R,
) -> Vec<Sampled>
where
    R: Fn(&Observation) -> Fut,
    Fut: Future<Output = Result<i64, DbErr>> + Send + 'static,
{
    let indices = candidates
        .iter()
        .map(|(index, _)| *index)
        .collect::<Vec<_>>();
    let sizes = read_sizes(tracked, &indices, read_current_size).await;
    let mut sampled = Vec::with_capacity(candidates.len());
    for ((index, snapshot), size) in candidates.into_iter().zip(sizes) {
        match size.and_then(non_negative) {
            Ok(size) => sampled.push(Sampled {
                tracked: index,
                id: tracked[index].id.clone(),
                observation: tracked[index].observation.clone(),
                snapshot,
                size,
            }),
            Err(error) => tracked[index].outcome = Some(Err(Arc::new(error))),
        }
    }
    sampled
}

/// Imports the legacy baseline of objects that have no accounting row yet.
async fn read_baselines<R, Fut>(
    tracked: &mut [Tracked],
    candidates: &mut Vec<(usize, Option<(i64, String)>)>,
    read_legacy_size: &R,
) where
    R: Fn(&Observation) -> Fut,
    Fut: Future<Output = Result<i64, DbErr>> + Send + 'static,
{
    let absent = candidates
        .iter()
        .filter(|(_, snapshot)| snapshot.is_none())
        .map(|(index, _)| *index)
        .collect::<Vec<_>>();
    let sizes = read_sizes(tracked, &absent, read_legacy_size).await;
    for (index, size) in absent.into_iter().zip(sizes) {
        match size.and_then(non_negative) {
            Ok(size) => tracked[index].observation.legacy_size = size,
            Err(error) => tracked[index].outcome = Some(Err(Arc::new(error))),
        }
    }
    candidates.retain(|(index, _)| tracked[*index].outcome.is_none());
}

/// The body re-runs from the sampled inputs on every attempt and only a committed attempt's
/// writes count, so a retried chunk never adds an object twice.
async fn commit(
    db: &DatabaseConnection,
    dialect: DbDialect,
    app: &Arc<str>,
    objects: Arc<[Sampled]>,
) -> Result<Vec<usize>, DbErr> {
    retry_transaction::<_, Vec<usize>, DbErr>(db, dialect, None, &CHUNK_RETRY, |txn| {
        let objects = objects.clone();
        let app = app.clone();
        Box::pin(async move { commit_chunk(txn, &app, &objects).await })
    })
    .await
}

/// Sample storage without holding an app or account transaction. A changed object snapshot
/// rejects the sample, so an earlier HEAD cannot overwrite a later event; only the changed
/// objects are sampled again. The legacy baseline is read only when the first snapshot finds
/// no accounting row.
async fn account_chunk<L, LFut, F, Fut>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    app_id: &str,
    tracked: &mut [Tracked],
    read_legacy_size: &L,
    read_current_size: &F,
) where
    L: Fn(&Observation) -> LFut,
    LFut: Future<Output = Result<i64, DbErr>> + Send + 'static,
    F: Fn(&Observation) -> Fut,
    Fut: Future<Output = Result<i64, DbErr>> + Send + 'static,
{
    let app: Arc<str> = app_id.into();
    for round in 0..MAX_SAMPLE_ROUNDS {
        let mut candidates = match unsettled(db, tracked).await {
            Ok(candidates) if candidates.is_empty() => return,
            Ok(candidates) => candidates,
            Err(error) => return fail_pending(tracked, error),
        };
        if round == 0 {
            read_baselines(tracked, &mut candidates, read_legacy_size).await;
        }
        let objects = sample(tracked, candidates, read_current_size).await;
        if objects.is_empty() {
            continue;
        }
        match commit(db, dialect, &app, objects.into()).await {
            Ok(settled) => {
                for index in settled {
                    tracked[index].outcome = Some(Ok(()));
                }
            }
            Err(error) => return fail_pending(tracked, error),
        }
        tokio::task::yield_now().await;
    }
    fail_pending(
        tracked,
        DbErr::Custom("Object changed repeatedly while accounting; retry the notification".into()),
    );
}

/// Objects by app, each once at its newest sequencer: the size sampled for it reflects every
/// older event on the same key. Observations that cannot be accounted are failed in `results`.
fn group(
    observations: &[Observation],
    results: &mut [Result<(), Failure>],
) -> BTreeMap<String, Vec<Tracked>> {
    let mut objects = BTreeMap::<(String, String), Tracked>::new();
    for (position, observation) in observations.iter().enumerate() {
        if let Err(error) = non_negative(observation.legacy_size) {
            results[position] = Err(Arc::new(error));
            continue;
        }
        match objects.entry((observation.app_id.clone(), observation.id())) {
            btree_map::Entry::Vacant(slot) => {
                let id = slot.key().1.clone();
                slot.insert(Tracked {
                    id,
                    observation: observation.clone(),
                    positions: vec![position],
                    outcome: None,
                });
            }
            btree_map::Entry::Occupied(mut slot) => {
                let object = slot.get_mut();
                object.positions.push(position);
                if newer_than(&observation.sequencer, &object.observation.sequencer) {
                    object.observation = observation.clone();
                }
            }
        }
    }
    let mut apps = BTreeMap::<String, Vec<Tracked>>::new();
    for ((app_id, _), object) in objects {
        apps.entry(app_id).or_default().push(object);
    }
    apps
}

/// Accounts a notification batch with one transaction per app chunk. Returns, for each
/// observation, the failure that left it unaccounted; a failed chunk fails only its own objects.
pub async fn apply_batch<L, LFut, F, Fut>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    observations: &[Observation],
    read_legacy_size: L,
    read_current_size: F,
) -> Vec<Result<(), Failure>>
where
    L: Fn(&Observation) -> LFut,
    LFut: Future<Output = Result<i64, DbErr>> + Send + 'static,
    F: Fn(&Observation) -> Fut,
    Fut: Future<Output = Result<i64, DbErr>> + Send + 'static,
{
    let mut results = vec![Ok(()); observations.len()];
    for (app_id, mut tracked) in group(observations, &mut results) {
        for chunk in tracked.chunks_mut(MAX_OBJECTS_PER_TRANSACTION) {
            account_chunk(
                db,
                dialect,
                &app_id,
                chunk,
                &read_legacy_size,
                &read_current_size,
            )
            .await;
        }
        for object in tracked {
            if let Some(Err(failure)) = object.outcome {
                for position in object.positions {
                    results[position] = Err(failure.clone());
                }
            }
        }
    }
    results
}

#[cfg(test)]
async fn apply(
    db: &DatabaseConnection,
    dialect: DbDialect,
    observation: Observation,
    size: i64,
) -> Result<(), DbErr> {
    apply_current(db, dialect, observation, move || async move { Ok(size) }).await
}

#[cfg(test)]
async fn apply_current<F, Fut>(
    db: &DatabaseConnection,
    dialect: DbDialect,
    observation: Observation,
    read_current_size: F,
) -> Result<(), DbErr>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<i64, DbErr>> + Send + 'static,
{
    let legacy_size = observation.legacy_size;
    let mut results = apply_batch(
        db,
        dialect,
        std::slice::from_ref(&observation),
        move |_| async move { Ok(legacy_size) },
        |_| read_current_size(),
    )
    .await;
    results.pop().unwrap_or(Ok(())).map_err(|failure| {
        Arc::try_unwrap(failure).unwrap_or_else(|shared| DbErr::Custom(shared.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_sequencers_compare_as_hex_numbers_with_arbitrary_width() {
        assert_eq!(normalize_sequencer("0000AF").unwrap(), "af");
        assert!(newer_than("100", "ff"));
        assert!(!newer_than("ff", "100"));
        assert!(!newer_than("af", "af"));
        assert!(newer_than("0", ""));
        assert!(normalize_sequencer("").is_err());
        assert!(normalize_sequencer("xyz").is_err());
    }

    #[test]
    fn identity_includes_bucket_and_does_not_exceed_index_limits() {
        let mut row = Observation {
            bucket: "a".into(),
            key: "x".repeat(1024),
            app_id: "app".into(),
            user_id: None,
            sequencer: "1".into(),
            legacy_size: 0,
        };
        let first = row.id();
        row.bucket = "b".into();
        assert_ne!(first, row.id());
        assert_eq!(first.len(), 64);
    }

    fn observed(key: &str, sequencer: &str) -> Observation {
        let parts = key.split('/').collect::<Vec<_>>();
        let (user_id, app_id) = match parts.as_slice() {
            ["users", user, "apps", app, ..] => (Some((*user).to_owned()), (*app).to_owned()),
            [_, app, ..] => (None, (*app).to_owned()),
            _ => panic!("test key {key} names no app"),
        };
        Observation {
            bucket: "bucket".into(),
            key: key.into(),
            app_id,
            user_id,
            sequencer: sequencer.into(),
            legacy_size: 0,
        }
    }

    #[test]
    fn batches_group_by_app_and_keep_each_object_once_at_its_newest_sequencer() {
        let observations = [
            observed("apps/b/x", "1"),
            observed("users/u/apps/a/y", "2"),
            observed("users/u/apps/a/y", "10"),
            observed("users/u/apps/a/y", "f"),
            observed("apps/a/z", "1"),
            Observation {
                legacy_size: -1,
                ..observed("apps/a/negative", "1")
            },
        ];
        let mut results: Vec<Result<(), Failure>> = vec![Ok(()); observations.len()];
        let apps = group(&observations, &mut results);
        assert_eq!(apps.keys().collect::<Vec<_>>(), ["a", "b"]);
        let a = &apps["a"];
        assert_eq!(a.len(), 2);
        let y = a
            .iter()
            .find(|object| object.observation.key == "users/u/apps/a/y")
            .unwrap();
        assert_eq!(y.observation.sequencer, "10");
        assert_eq!(y.positions, [1, 2, 3]);
        assert_eq!(apps["b"][0].positions, [0]);
        assert!(results[..5].iter().all(Result::is_ok));
        assert!(results[5].is_err());
    }

    #[test]
    fn values_rows_are_typed_and_numbered_across_rows() {
        assert_eq!(parameters(3), "$1,$2,$3");
        assert_eq!(
            typed_rows(2, &["text", "bigint"]),
            "($1::text,$2::bigint),($3::text,$4::bigint)"
        );
    }

    /// A unique schema with the accounting tables in the disposable PostgreSQL database named
    /// by FLOW_LIKE_TEST_DATABASE_URL.
    struct Fixture {
        admin: DatabaseConnection,
        db: DatabaseConnection,
        schema: String,
    }

    impl Fixture {
        async fn create(prefix: &str) -> Option<Self> {
            use sea_orm::{
                sqlx::postgres::{PgConnectOptions, PgPoolOptions},
                Database,
            };
            use std::str::FromStr;
            let Ok(url) = std::env::var("FLOW_LIKE_TEST_DATABASE_URL") else {
                eprintln!(
                    "skipping PostgreSQL accounting test: FLOW_LIKE_TEST_DATABASE_URL is unset"
                );
                return None;
            };
            let admin = Database::connect(&url).await.unwrap();
            let schema = format!(
                "{prefix}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            sql(&admin, &format!("CREATE SCHEMA {schema}")).await;
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect_with(
                    PgConnectOptions::from_str(&url)
                        .unwrap()
                        .options([("search_path", schema.as_str())]),
                )
                .await
                .unwrap();
            let db = DatabaseConnection::from(pool);
            for table in [
                r#"CREATE TABLE "MutationLock" (id BIGINT PRIMARY KEY, "updatedAt" TIMESTAMPTZ DEFAULT now())"#,
                r#"CREATE TABLE "App" (id TEXT PRIMARY KEY, "ownerRoleId" TEXT, "totalSize" BIGINT NOT NULL)"#,
                r#"CREATE TABLE "User" (id TEXT PRIMARY KEY, "totalSize" BIGINT NOT NULL)"#,
                r#"CREATE TABLE "FileAccountingObject" (id TEXT PRIMARY KEY, bucket TEXT, "objectKey" TEXT, "appId" TEXT, "userId" TEXT, "payerId" TEXT, size BIGINT, sequencer TEXT, "updatedAt" TIMESTAMPTZ)"#,
                r#"CREATE TABLE "Membership" ("userId" TEXT, "appId" TEXT, "roleId" TEXT)"#,
                r#"CREATE TABLE "ProjectCapacity" ("appId" TEXT PRIMARY KEY, "payerId" TEXT)"#,
                r#"CREATE TABLE "AccountCapacity" ("payerId" TEXT PRIMARY KEY, "storageBytes" BIGINT DEFAULT 0, "reservedStorageBytes" BIGINT DEFAULT 0, "initialized" BOOLEAN DEFAULT false, "updatedAt" TIMESTAMPTZ)"#,
                r#"CREATE TABLE "StorageUploadGrant" ("id" TEXT PRIMARY KEY, "maxBytes" BIGINT, "reservedBytes" BIGINT, "updatedAt" TIMESTAMPTZ)"#,
            ] {
                sql(&db, table).await;
            }
            Some(Self { admin, db, schema })
        }

        async fn drop_schema(self) {
            self.db.close().await.unwrap();
            sql(&self.admin, &format!("DROP SCHEMA {} CASCADE", self.schema)).await;
        }
    }

    async fn sql(db: &DatabaseConnection, sql: &str) {
        db.execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            sql,
        ))
        .await
        .unwrap();
    }

    async fn value(db: &DatabaseConnection, sql: &str) -> i64 {
        db.query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            sql,
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "n")
        .unwrap()
    }

    /// Checks actual rollback and replay behavior, including legacy import, one object at a time.
    #[tokio::test]
    async fn postgres_accounting_rollbacks_replays_and_ordering() {
        use sea_orm::DatabaseBackend;
        let Some(fixture) = Fixture::create("file_accounting").await else {
            return;
        };
        let db = fixture.db.clone();
        for statement in [
            r#"ALTER TABLE "App" ADD CONSTRAINT fail_overwrite CHECK ("totalSize" <= 100)"#,
            r#"INSERT INTO "App" VALUES ('app','owner-role',100)"#,
            r#"INSERT INTO "User" VALUES ('user',100)"#,
            r#"INSERT INTO "Membership" VALUES ('user','app','owner-role')"#,
            r#"INSERT INTO "AccountCapacity" ("payerId", "storageBytes", "initialized") VALUES ('user',100,true)"#,
            r#"INSERT INTO "ProjectCapacity" VALUES ('app','user')"#,
        ] {
            sql(&db, statement).await;
        }
        let observation = Observation {
            bucket: "bucket".into(),
            key: "users/user/apps/app/file".into(),
            app_id: "app".into(),
            user_id: Some("user".into()),
            sequencer: "2".into(),
            legacy_size: 100,
        };
        assert!(apply(&db, DbDialect::Postgres, observation.clone(), 150)
            .await
            .is_err());
        assert_eq!(
            value(
                &db,
                r#"SELECT count(*)::BIGINT AS n FROM "FileAccountingObject""#
            )
            .await,
            0
        );
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await,
            100
        );
        sql(&db, r#"ALTER TABLE "App" DROP CONSTRAINT fail_overwrite"#).await;
        apply(&db, DbDialect::Postgres, observation.clone(), 150)
            .await
            .unwrap();
        apply(&db, DbDialect::Postgres, observation.clone(), 150)
            .await
            .unwrap();
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await,
            150
        );
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "User""#).await,
            150
        );
        let stale = Observation {
            sequencer: "1".into(),
            ..observation.clone()
        };
        apply(&db, DbDialect::Postgres, stale, 50).await.unwrap();
        assert_eq!(
            value(&db, r#"SELECT size AS n FROM "FileAccountingObject""#).await,
            150
        );

        sql(
            &db,
            r#"ALTER TABLE "App" ADD CONSTRAINT fail_delete CHECK ("totalSize" > 0)"#,
        )
        .await;
        let deletion = Observation {
            sequencer: "3".into(),
            ..observation.clone()
        };
        assert!(apply(&db, DbDialect::Postgres, deletion.clone(), 0)
            .await
            .is_err());
        assert_eq!(
            value(&db, r#"SELECT size AS n FROM "FileAccountingObject""#).await,
            150
        );
        sql(&db, r#"ALTER TABLE "App" DROP CONSTRAINT fail_delete"#).await;
        apply(&db, DbDialect::Postgres, deletion.clone(), 0)
            .await
            .unwrap();
        apply(&db, DbDialect::Postgres, deletion.clone(), 0)
            .await
            .unwrap();
        assert_eq!(value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await, 0);
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "User""#).await,
            0
        );

        let recreated = Observation {
            sequencer: "4".into(),
            ..observation.clone()
        };
        let (first, duplicate) = tokio::join!(
            apply(&db, DbDialect::Postgres, recreated.clone(), 200),
            apply(&db, DbDialect::Postgres, recreated, 200)
        );
        first.unwrap();
        duplicate.unwrap();
        apply(&db, DbDialect::Postgres, deletion, 0).await.unwrap();
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await,
            200
        );
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "User""#).await,
            200
        );

        // A slow HEAD holds no app/account lock. A newer event whose earlier
        // sample loses a race must resample before committing its larger sequencer.
        use std::sync::{
            atomic::{AtomicI64, AtomicUsize, Ordering},
            Arc,
        };
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let current_size = Arc::new(AtomicI64::new(200));
        let samples = Arc::new(AtomicUsize::new(0));
        let first = {
            let db = db.clone();
            let entered = entered.clone();
            let release = release.clone();
            let current_size = current_size.clone();
            let samples = samples.clone();
            let event = Observation {
                sequencer: "6".into(),
                ..observation.clone()
            };
            tokio::spawn(async move {
                apply_current(&db, DbDialect::Postgres, event, move || {
                    let entered = entered.clone();
                    let release = release.clone();
                    let current_size = current_size.clone();
                    let samples = samples.clone();
                    async move {
                        let captured = current_size.load(Ordering::SeqCst);
                        if samples.fetch_add(1, Ordering::SeqCst) == 0 {
                            entered.notify_one();
                            release.notified().await;
                        }
                        Ok(captured)
                    }
                })
                .await
            })
        };
        entered.notified().await;
        // Both coordination locks remain available while the first HEAD waits.
        use sea_orm::TransactionTrait;
        let unrelated = db.begin().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            flow_like_db::coordination::app_capacity(&unrelated, "app")
                .await
                .unwrap();
            unrelated
                .execute_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    r#"UPDATE "AccountCapacity" SET "updatedAt"=now() WHERE "payerId"='user'"#,
                ))
                .await
                .unwrap();
        })
        .await
        .expect("HEAD must not hold broad SQL locks");
        unrelated.commit().await.unwrap();
        current_size.store(300, Ordering::SeqCst);
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            apply(
                &db,
                DbDialect::Postgres,
                Observation {
                    sequencer: "5".into(),
                    ..observation.clone()
                },
                300,
            ),
        )
        .await
        .expect("another event can finish during HEAD")
        .unwrap();
        release.notify_one();
        first.await.unwrap().unwrap();
        assert_eq!(
            samples.load(Ordering::SeqCst),
            2,
            "stale HEAD must be sampled again"
        );
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await,
            300
        );
        let next = Observation {
            sequencer: "7".into(),
            ..observation
        };
        assert!(
            apply_current(&db, DbDialect::Postgres, next.clone(), || async {
                Err(DbErr::Custom("S3 unavailable".into()))
            })
            .await
            .is_err()
        );
        assert_eq!(
            value(&db, r#"SELECT size AS n FROM "FileAccountingObject""#).await,
            300
        );
        apply(&db, DbDialect::Postgres, next, 400).await.unwrap();
        assert_eq!(
            value(&db, r#"SELECT "totalSize" AS n FROM "App""#).await,
            400
        );
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId" = 'user'"#
            )
            .await,
            400
        );
        // An object's payer survives deletion of the app and its membership.
        sql(&db, r#"DELETE FROM "Membership""#).await;
        sql(&db, r#"DELETE FROM "App""#).await;
        let late = Observation {
            bucket: "bucket".into(),
            key: "users/user/apps/app/file".into(),
            app_id: "app".into(),
            user_id: Some("user".into()),
            sequencer: "8".into(),
            legacy_size: 0,
        };
        apply(&db, DbDialect::Postgres, late.clone(), 0)
            .await
            .unwrap();
        apply(&db, DbDialect::Postgres, late, 0).await.unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId" = 'user'"#
            )
            .await,
            0
        );
        let replayed = Observation {
            bucket: "bucket".into(),
            key: "users/user/apps/app/file".into(),
            app_id: "app".into(),
            user_id: Some("user".into()),
            sequencer: "9".into(),
            legacy_size: 0,
        };
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            r#"INSERT INTO "StorageUploadGrant" ("id","maxBytes","reservedBytes") VALUES ($1,400,400)"#, [replayed.id().into()])).await.unwrap();
        sql(
            &db,
            r#"UPDATE "AccountCapacity" SET "reservedStorageBytes" = 400"#,
        )
        .await;
        // A retained project already contributes while the account baseline is
        // incomplete. New events must update that partial total immediately.
        sql(&db, r#"UPDATE "AccountCapacity" SET initialized=false"#).await;
        apply(&db, DbDialect::Postgres, replayed.clone(), 400)
            .await
            .unwrap();
        assert_eq!(
            value(&db, r#"SELECT "storageBytes" AS n FROM "AccountCapacity""#).await,
            400
        );
        sql(&db, r#"UPDATE "AccountCapacity" SET initialized=true"#).await;
        apply(&db, DbDialect::Postgres, replayed.clone(), 400)
            .await
            .unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" + "reservedStorageBytes" AS n FROM "AccountCapacity""#
            )
            .await,
            400
        );
        assert_eq!(
            value(
                &db,
                r#"SELECT "reservedStorageBytes" AS n FROM "AccountCapacity""#
            )
            .await,
            0
        );
        apply(
            &db,
            DbDialect::Postgres,
            Observation {
                sequencer: "a".into(),
                ..replayed.clone()
            },
            0,
        )
        .await
        .unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" + "reservedStorageBytes" AS n FROM "AccountCapacity""#
            )
            .await,
            400
        );
        assert_eq!(
            value(
                &db,
                r#"SELECT "reservedStorageBytes" AS n FROM "AccountCapacity""#
            )
            .await,
            400
        );
        apply(
            &db,
            DbDialect::Postgres,
            Observation {
                sequencer: "b".into(),
                ..replayed
            },
            400,
        )
        .await
        .unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" + "reservedStorageBytes" AS n FROM "AccountCapacity""#
            )
            .await,
            400
        );
        // HEAD may run during an ownership transfer, but the SQL commit resolves
        // the current payer after acquiring the shared app lock.
        let transfer = db.begin().await.unwrap();
        flow_like_db::coordination::app_capacity(&transfer, "app")
            .await
            .unwrap();
        let sampled = Arc::new(tokio::sync::Notify::new());
        let observer = {
            let db = db.clone();
            let sampled = sampled.clone();
            tokio::spawn(async move {
                apply_current(
                    &db,
                    DbDialect::Postgres,
                    Observation {
                        bucket: "bucket".into(),
                        key: "users/user/apps/app/file".into(),
                        app_id: "app".into(),
                        user_id: Some("user".into()),
                        sequencer: "c".into(),
                        legacy_size: 0,
                    },
                    move || {
                        let sampled = sampled.clone();
                        async move {
                            sampled.notify_one();
                            Ok(450)
                        }
                    },
                )
                .await
                .unwrap();
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), sampled.notified())
            .await
            .expect("HEAD can finish while ownership transfer holds the app lock");
        for statement in [
            r#"DELETE FROM "StorageUploadGrant""#,
            r#"UPDATE "ProjectCapacity" SET "payerId"='recipient' WHERE "appId"='app'"#,
            r#"UPDATE "AccountCapacity" SET "storageBytes"=0 WHERE "payerId"='user'"#,
            r#"INSERT INTO "AccountCapacity" ("payerId","storageBytes",initialized) VALUES ('recipient',400,true)"#,
        ] {
            transfer
                .execute_raw(Statement::from_string(DatabaseBackend::Postgres, statement))
                .await
                .unwrap();
        }
        transfer.commit().await.unwrap();
        observer.await.unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='recipient'"#
            )
            .await,
            450
        );
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='user'"#
            )
            .await,
            0
        );
        // An older delete sampled before a recreation cannot erase the new bytes.
        let delete_entered = Arc::new(tokio::sync::Notify::new());
        let delete_release = Arc::new(tokio::sync::Notify::new());
        let stale_delete = {
            let db = db.clone();
            let entered = delete_entered.clone();
            let release = delete_release.clone();
            tokio::spawn(async move {
                apply_current(
                    &db,
                    DbDialect::Postgres,
                    Observation {
                        bucket: "bucket".into(),
                        key: "users/user/apps/app/file".into(),
                        app_id: "app".into(),
                        user_id: Some("user".into()),
                        sequencer: "d".into(),
                        legacy_size: 0,
                    },
                    move || {
                        let entered = entered.clone();
                        let release = release.clone();
                        async move {
                            entered.notify_one();
                            release.notified().await;
                            Ok(0)
                        }
                    },
                )
                .await
                .unwrap();
            })
        };
        delete_entered.notified().await;
        apply(
            &db,
            DbDialect::Postgres,
            Observation {
                bucket: "bucket".into(),
                key: "users/user/apps/app/file".into(),
                app_id: "app".into(),
                user_id: Some("user".into()),
                sequencer: "e".into(),
                legacy_size: 0,
            },
            500,
        )
        .await
        .unwrap();
        delete_release.notify_one();
        stale_delete.await.unwrap();
        assert_eq!(
            value(&db, r#"SELECT size AS n FROM "FileAccountingObject""#).await,
            500
        );
        assert_eq!(
            value(
                &db,
                r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='recipient'"#
            )
            .await,
            500
        );
        let final_event = Observation {
            bucket: "bucket".into(),
            key: "users/user/apps/app/file".into(),
            app_id: "app".into(),
            user_id: Some("user".into()),
            sequencer: "e".into(),
            legacy_size: 0,
        };
        apply_current(&db, DbDialect::Postgres, final_event.clone(), || async {
            Err(DbErr::Custom("duplicate events must not call HEAD".into()))
        })
        .await
        .unwrap();
        // Continual same-size updates still invalidate old samples through their
        // sequencer. Stop after a bounded number and leave the notification retryable.
        let changing_db = db.clone();
        let sample_count = Arc::new(AtomicUsize::new(0));
        let observed_samples = sample_count.clone();
        let changed = apply_current(
            &db,
            DbDialect::Postgres,
            Observation {
                sequencer: "ff".into(),
                ..final_event
            },
            move || {
                let db = changing_db.clone();
                let sample_count = sample_count.clone();
                async move {
                    let next = 15 + sample_count.fetch_add(1, Ordering::SeqCst);
                    db.execute_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        r#"UPDATE "FileAccountingObject" SET sequencer=$1"#,
                        [format!("{next:x}").into()],
                    ))
                    .await?;
                    Ok(600)
                }
            },
        )
        .await
        .unwrap_err();
        assert!(changed.to_string().contains("retry the notification"));
        assert_eq!(observed_samples.load(Ordering::SeqCst), 8);
        assert_eq!(
            value(&db, r#"SELECT size AS n FROM "FileAccountingObject""#).await,
            500
        );
        // An event that moves no bytes and has no grant commits without the payer lock.
        let payer_held = db.begin().await.unwrap();
        payer_held
            .execute_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                r#"UPDATE "AccountCapacity" SET "updatedAt"=now() WHERE "payerId"='recipient'"#,
            ))
            .await
            .unwrap();
        let empty = Observation {
            bucket: "bucket".into(),
            key: "users/user/apps/app/empty".into(),
            app_id: "app".into(),
            user_id: Some("user".into()),
            sequencer: "1".into(),
            legacy_size: 0,
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            apply(&db, DbDialect::Postgres, empty.clone(), 0),
        )
        .await
        .expect("an event that changes nothing on the account must not wait for the payer lock")
        .unwrap();
        payer_held.rollback().await.unwrap();
        // The same event on a granted object still settles its reservation under that lock.
        let granted = Observation {
            key: "users/user/apps/app/granted".into(),
            ..empty
        };
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            r#"INSERT INTO "StorageUploadGrant" ("id","maxBytes","reservedBytes") VALUES ($1,100,40)"#, [granted.id().into()])).await.unwrap();
        let reserved_before = value(
            &db,
            r#"SELECT "reservedStorageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='recipient'"#,
        )
        .await;
        apply(&db, DbDialect::Postgres, granted, 0).await.unwrap();
        assert_eq!(
            value(
                &db,
                r#"SELECT "reservedStorageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='recipient'"#
            )
            .await,
            reserved_before + 60
        );
        fixture.drop_schema().await;
    }

    #[tokio::test]
    async fn postgres_upload_grant_settles_on_recipient_after_ownership_transfer() {
        use sea_orm::{DatabaseBackend, TransactionTrait};
        let Some(fixture) = Fixture::create("file_accounting_transfer_grant").await else {
            return;
        };
        let db = fixture.db.clone();
        for statement in [
            r#"ALTER TABLE "StorageUploadGrant" ADD COLUMN "payerId" TEXT"#,
            r#"INSERT INTO "App" VALUES ('app','owner-role',40)"#,
            r#"INSERT INTO "User" VALUES ('owner',40)"#,
            r#"INSERT INTO "ProjectCapacity" VALUES ('app','owner')"#,
            r#"INSERT INTO "AccountCapacity" ("payerId","storageBytes","reservedStorageBytes",initialized) VALUES ('owner',40,60,true),('recipient',10,20,true)"#,
        ] {
            sql(&db, statement).await;
        }
        let upload = Observation {
            bucket: "bucket".into(),
            key: "users/owner/apps/app/file".into(),
            app_id: "app".into(),
            user_id: Some("owner".into()),
            sequencer: "2".into(),
            legacy_size: 40,
        };
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            r#"INSERT INTO "FileAccountingObject" (id,bucket,"objectKey","appId","userId","payerId",size,sequencer) VALUES ($1,'bucket','users/owner/apps/app/file','app','owner','owner',40,'1')"#,
            [upload.id().into()])).await.unwrap();
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            r#"INSERT INTO "StorageUploadGrant" (id,"payerId","maxBytes","reservedBytes") VALUES ($1,'owner',100,60)"#,
            [upload.id().into()])).await.unwrap();

        // HEAD finishes before transfer commits. Settlement must resolve the new
        // payer under the app lock, even though the grant retains its original payer.
        let transfer = db.begin().await.unwrap();
        flow_like_db::coordination::app_capacity(&transfer, "app")
            .await
            .unwrap();
        let sampled = Arc::new(tokio::sync::Notify::new());
        let observer = {
            let db = db.clone();
            let sampled = sampled.clone();
            let upload = upload.clone();
            tokio::spawn(async move {
                apply_current(&db, DbDialect::Postgres, upload, move || {
                    let sampled = sampled.clone();
                    async move {
                        sampled.notify_one();
                        Ok(70)
                    }
                })
                .await
                .unwrap();
            })
        };
        tokio::time::timeout(Duration::from_secs(1), sampled.notified())
            .await
            .expect("HEAD must finish while transfer holds the app lock");
        for statement in [
            r#"UPDATE "ProjectCapacity" SET "payerId"='recipient' WHERE "appId"='app'"#,
            r#"UPDATE "AccountCapacity" SET "storageBytes"=0,"reservedStorageBytes"=0 WHERE "payerId"='owner'"#,
            r#"UPDATE "AccountCapacity" SET "storageBytes"=50,"reservedStorageBytes"=80 WHERE "payerId"='recipient'"#,
        ] {
            transfer
                .execute_raw(Statement::from_string(DatabaseBackend::Postgres, statement))
                .await
                .unwrap();
        }
        transfer.commit().await.unwrap();
        observer.await.unwrap();

        for (sequencer, actual, expected_stored, expected_reserved) in [
            ("2", 70, 80, 50),
            ("3", 0, 10, 120),
            ("3", 0, 10, 120),
            ("4", 100, 110, 20),
        ] {
            apply(
                &db,
                DbDialect::Postgres,
                Observation {
                    sequencer: sequencer.into(),
                    ..upload.clone()
                },
                actual,
            )
            .await
            .unwrap();
            assert_eq!(
                value(&db, r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='recipient'"#).await,
                expected_stored
            );
            assert_eq!(
                value(&db, r#"SELECT "reservedStorageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='recipient'"#).await,
                expected_reserved
            );
            assert_eq!(
                value(&db, r#"SELECT "storageBytes" + "reservedStorageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='owner'"#).await,
                0
            );
        }
        assert_eq!(
            value(
                &db,
                r#"SELECT count(*) AS n FROM "StorageUploadGrant" WHERE "payerId"='owner'"#
            )
            .await,
            1
        );
        fixture.drop_schema().await;
    }

    /// Two apps, a stale sequencer, an object created and deleted before the batch ran, an
    /// unreadable object and a lost commit race in one batch: one committed transaction per
    /// app, every object counted once, and failures confined to their own objects.
    #[tokio::test]
    async fn postgres_batches_commit_each_app_once_and_fail_only_their_own_objects() {
        use sea_orm::DatabaseBackend;
        use std::sync::Mutex;
        let Some(fixture) = Fixture::create("file_accounting_batch").await else {
            return;
        };
        let db = fixture.db.clone();
        for statement in [
            r#"INSERT INTO "App" VALUES ('a','role-a',10),('b','role-b',0),('c','role-c',0)"#,
            r#"INSERT INTO "User" VALUES ('u',0),('v',0)"#,
            r#"INSERT INTO "ProjectCapacity" VALUES ('a','payer-a'),('b','payer-b')"#,
            r#"INSERT INTO "AccountCapacity" ("payerId","storageBytes",initialized) VALUES ('payer-a',10,true),('payer-b',0,true)"#,
            r#"CREATE TABLE "WriteLog" (txid BIGINT, "table" TEXT)"#,
            r#"CREATE SEQUENCE app_conflicts"#,
            r#"CREATE FUNCTION log_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN INSERT INTO "WriteLog" VALUES (txid_current(), TG_TABLE_NAME); RETURN NULL; END $$"#,
            // nextval survives rollback, so only the test's first App update loses a commit race.
            r#"CREATE FUNCTION conflict_once() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF nextval('app_conflicts') = 1 THEN RAISE EXCEPTION 'injected commit race' USING ERRCODE = '40001'; END IF; RETURN NULL; END $$"#,
            r#"CREATE TRIGGER app_conflict AFTER UPDATE ON "App" FOR EACH STATEMENT EXECUTE FUNCTION conflict_once()"#,
            r#"CREATE TRIGGER app_writes AFTER UPDATE ON "App" FOR EACH STATEMENT EXECUTE FUNCTION log_write()"#,
            r#"CREATE TRIGGER lock_writes AFTER INSERT OR UPDATE ON "MutationLock" FOR EACH STATEMENT EXECUTE FUNCTION log_write()"#,
        ] {
            sql(&db, statement).await;
        }
        let old = observed("apps/a/old", "3");
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            r#"INSERT INTO "FileAccountingObject" (id, bucket, "objectKey", "appId", size, sequencer, "payerId") VALUES ($1,'bucket','apps/a/old','a',10,'5','payer-a')"#,
            [old.id().into()])).await.unwrap();
        let sizes = HashMap::from([
            ("users/u/apps/a/one", 100),
            ("users/u/apps/a/two", 50),
            ("users/u/apps/a/gone", 0),
            ("users/v/apps/b/three", 70),
            ("apps/a/four", 5),
            ("apps/b/more", 30),
        ]);
        let heads = Mutex::new(Vec::<String>::new());
        let head = |observation: &Observation| {
            heads.lock().unwrap().push(observation.key.clone());
            let size = sizes.get(observation.key.as_str()).copied();
            let key = observation.key.clone();
            async move { size.ok_or_else(|| DbErr::Custom(format!("S3 HEAD of {key} failed"))) }
        };
        let no_legacy = |_: &Observation| async { Ok::<_, DbErr>(0) };
        let total = |sql: &'static str| value(&db, sql);

        let batch = [
            observed("users/u/apps/a/one", "1"),
            observed("users/u/apps/a/two", "1"),
            old,
            observed("users/v/apps/b/three", "1"),
            observed("users/u/apps/a/gone", "1"),
            observed("users/u/apps/a/gone", "2"),
            observed("apps/b/unreadable", "1"),
        ];
        let results = apply_batch(&db, DbDialect::Postgres, &batch, no_legacy, &head).await;
        assert!(results[..6].iter().all(Result::is_ok), "{results:?}");
        assert!(results[6]
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("apps/b/unreadable"));
        let mut sampled = heads.lock().unwrap().clone();
        sampled.sort();
        assert_eq!(
            sampled,
            [
                "apps/b/unreadable",
                "users/u/apps/a/gone",
                "users/u/apps/a/one",
                "users/u/apps/a/two",
                "users/v/apps/b/three"
            ]
        );
        assert_eq!(
            total(r#"SELECT "totalSize" AS n FROM "App" WHERE id='a'"#).await,
            160
        );
        assert_eq!(
            total(r#"SELECT "totalSize" AS n FROM "App" WHERE id='b'"#).await,
            70
        );
        assert_eq!(
            total(r#"SELECT "totalSize" AS n FROM "User" WHERE id='u'"#).await,
            150
        );
        assert_eq!(
            total(r#"SELECT "totalSize" AS n FROM "User" WHERE id='v'"#).await,
            70
        );
        assert_eq!(
            total(r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='payer-a'"#)
                .await,
            160
        );
        assert_eq!(
            total(r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='payer-b'"#)
                .await,
            70
        );
        assert_eq!(total(r#"SELECT count(*) AS n FROM "FileAccountingObject" WHERE "objectKey"='users/u/apps/a/gone' AND size=0 AND sequencer='2' AND "payerId"='payer-a'"#).await, 1);
        assert_eq!(total(r#"SELECT size AS n FROM "FileAccountingObject" WHERE "objectKey"='apps/a/old' AND sequencer='5'"#).await, 10);
        assert_eq!(total(r#"SELECT count(*) AS n FROM "FileAccountingObject" WHERE "objectKey"='apps/b/unreadable'"#).await, 0);
        assert_eq!(
            total(r#"SELECT count(DISTINCT txid) AS n FROM "WriteLog""#).await,
            2
        );
        assert_eq!(
            total(r#"SELECT count(*) AS n FROM "WriteLog" WHERE "table"='App'"#).await,
            2
        );
        // App a lost one commit race and its retry counted every object once.
        assert_eq!(
            total(r#"SELECT last_value AS n FROM app_conflicts"#).await,
            3
        );

        // A chunk that cannot commit fails only its app's objects; redelivering the whole
        // batch afterwards adds nothing for objects that were already counted.
        sql(
            &db,
            r#"ALTER TABLE "App" ADD CONSTRAINT b_frozen CHECK (id <> 'b' OR "totalSize" <= 70)"#,
        )
        .await;
        let redelivered = [
            observed("users/u/apps/a/one", "1"),
            observed("apps/a/four", "1"),
            observed("apps/b/more", "1"),
        ];
        let results = apply_batch(&db, DbDialect::Postgres, &redelivered, no_legacy, &head).await;
        assert!(results[0].is_ok() && results[1].is_ok(), "{results:?}");
        assert!(results[2].is_err());
        assert_eq!(
            total(r#"SELECT "totalSize" AS n FROM "App" WHERE id='a'"#).await,
            165
        );
        assert_eq!(
            total(r#"SELECT "totalSize" AS n FROM "App" WHERE id='b'"#).await,
            70
        );
        assert_eq!(
            total(r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='payer-b'"#)
                .await,
            70
        );
        assert_eq!(total(r#"SELECT count(*) AS n FROM "FileAccountingObject" WHERE "objectKey"='apps/b/more'"#).await, 0);
        sql(&db, r#"ALTER TABLE "App" DROP CONSTRAINT b_frozen"#).await;
        let results = apply_batch(&db, DbDialect::Postgres, &redelivered, no_legacy, &head).await;
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        assert_eq!(
            total(r#"SELECT "totalSize" AS n FROM "App" WHERE id='a'"#).await,
            165
        );
        assert_eq!(
            total(r#"SELECT "totalSize" AS n FROM "App" WHERE id='b'"#).await,
            100
        );
        assert_eq!(
            total(r#"SELECT "storageBytes" AS n FROM "AccountCapacity" WHERE "payerId"='payer-b'"#)
                .await,
            100
        );

        // A burst larger than one transaction's row budget splits into bounded chunks.
        sql(&db, r#"TRUNCATE "WriteLog""#).await;
        let burst = (0..=MAX_OBJECTS_PER_TRANSACTION)
            .map(|index| observed(&format!("apps/c/{index}"), "1"))
            .collect::<Vec<_>>();
        let results = apply_batch(&db, DbDialect::Postgres, &burst, no_legacy, |_| async {
            Ok(1)
        })
        .await;
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(
            total(r#"SELECT "totalSize" AS n FROM "App" WHERE id='c'"#).await,
            burst.len() as i64
        );
        assert_eq!(
            total(r#"SELECT count(*) AS n FROM "WriteLog" WHERE "table"='App'"#).await,
            2
        );
        fixture.drop_schema().await;
    }
}
