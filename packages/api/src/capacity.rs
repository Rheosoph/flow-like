//! Current storage and project occupancy. Monthly quotas live in `quota`.
//!
//! A payer row serializes admission. Project provenance is written by the server
//! at admission and survives editing, source retirement and deletion.

use flow_like_storage::object_store::ObjectStoreExt;
use sea_orm::{ConnectionTrait, DatabaseTransaction, Statement};

use crate::{
    entity::sea_orm_active_enums::Visibility,
    error::{ApiError, QuotaLimitDetails},
    state::AppState,
};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityUsage {
    pub storage_bytes: i64,
    pub reserved_storage_bytes: i64,
    pub project_count: i64,
}

pub fn counts_as_project(visibility: &Visibility, public_fork_source: Option<&str>) -> bool {
    public_fork_source.is_none()
        && matches!(visibility, Visibility::Private | Visibility::Prototype)
}

pub fn public_fork_source(visibility: &Visibility, source_id: &str) -> Option<String> {
    matches!(
        visibility,
        Visibility::Public | Visibility::PublicRequestAccess
    )
    .then(|| source_id.to_owned())
}

/// COALESCE arms that resolve the payer of app `$1`, shared so every statement
/// charges the same account.
const APP_PAYER_ARMS: &str = r#"(SELECT "payerId" FROM "ProjectCapacity" WHERE "appId" = $1),
             (SELECT m."userId" FROM "Membership" m JOIN "App" a ON a."ownerRoleId" = m."roleId" AND a."id" = m."appId" WHERE a."id" = $1 ORDER BY m."userId" LIMIT 1)"#;

/// Resolve the retained owner snapshot before consulting live memberships. An app's
/// user-scoped objects are included in its owner's storage, just like shared files.
pub async fn payer_for_app<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
) -> Result<Option<String>, ApiError> {
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            format!(r#"SELECT COALESCE({APP_PAYER_ARMS}) AS "payerId""#),
            [app_id.into()],
        ))
        .await?;
    row.map(|row| row.try_get("", "payerId").map_err(ApiError::from))
        .transpose()
        .map(Option::flatten)
}

struct StorageAccount {
    payer_id: String,
    plan: String,
    max_total_size: i64,
    /// Absent until the account baseline is initialized.
    usage: Option<CapacityUsage>,
}

/// The payer of a storage write, falling back to the caller, with its plan and
/// initialized occupancy in one statement.
async fn storage_account(
    state: &AppState,
    app_id: &str,
    fallback_payer: &str,
) -> Result<StorageAccount, ApiError> {
    let row = state
        .db
        .query_one_raw(Statement::from_sql_and_values(
            state.db.get_database_backend(),
            format!(
                r#"SELECT u.id,u.tier,c."storageBytes",c."reservedStorageBytes",c."projectCount"
        FROM (SELECT COALESCE({APP_PAYER_ARMS}, $2) AS id) p
        JOIN "User" u ON u.id = p.id
        LEFT JOIN "AccountCapacity" c ON c."payerId" = u.id AND c.initialized = true"#
            ),
            [app_id.into(), fallback_payer.into()],
        ))
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let plan = row.try_get::<String>("", "tier")?.to_uppercase();
    let max_total_size = state
        .platform_config
        .tiers
        .get(&plan)
        .map(|tier| tier.max_total_size)
        .ok_or_else(|| ApiError::internal(format!("missing entitlement for {plan}")))?;
    let usage = match row.try_get::<Option<i64>>("", "storageBytes")? {
        Some(storage_bytes) => Some(CapacityUsage {
            storage_bytes,
            reserved_storage_bytes: row.try_get("", "reservedStorageBytes")?,
            project_count: row.try_get("", "projectCount")?,
        }),
        None => None,
    };
    Ok(StorageAccount {
        payer_id: row.try_get("", "id")?,
        plan,
        max_total_size,
        usage,
    })
}

const BASELINE_PAGE: usize = 250;

fn baseline_pending() -> ApiError {
    ApiError::usage_refresh_pending()
}

/// Advance one indexed membership page. Partial totals stay unavailable for
/// admission; a retained project row marks which stored bytes already contribute.
async fn baseline_page(
    txn: &DatabaseTransaction,
    payer_id: &str,
    batch: usize,
) -> Result<(CapacityUsage, bool), ApiError> {
    let row = txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        r#"INSERT INTO "AccountCapacity" ("payerId") VALUES ($1)
        ON CONFLICT ("payerId") DO UPDATE SET "updatedAt"=now()
        RETURNING "storageBytes","reservedStorageBytes","projectCount",initialized,"baselineAppId""#,
        [payer_id.into()])).await?.ok_or_else(||ApiError::internal("capacity account disappeared"))?;
    let mut usage = CapacityUsage {
        storage_bytes: row.try_get("", "storageBytes")?,
        reserved_storage_bytes: row.try_get("", "reservedStorageBytes")?,
        project_count: row.try_get("", "projectCount")?,
    };
    if row.try_get::<bool>("", "initialized")? {
        return Ok((usage, true));
    }
    let mut cursor: String = row.try_get("", "baselineAppId")?;
    let rows = txn.query_all_raw(Statement::from_sql_and_values(txn.get_database_backend(),r#"
        SELECT m."appId",a.id AS "ownedAppId",a."totalSize",
          COALESCE(a.visibility IN ('PRIVATE','PROTOTYPE') AND NOT COALESCE(s.visibility IN ('PUBLIC','PUBLIC_REQUEST_ACCESS'),false),false) AS counted,
          CASE WHEN s.visibility IN ('PUBLIC','PUBLIC_REQUEST_ACCESS') THEN s.id ELSE NULL END AS source
        FROM (SELECT "appId","roleId" FROM "Membership" WHERE "userId"=$1 AND "appId">$2 ORDER BY "appId" LIMIT $3) m
        LEFT JOIN "App" a ON a.id=m."appId" AND a."ownerRoleId"=m."roleId"
        LEFT JOIN "App" s ON s.id=a."forkedFrom" AND EXISTS(SELECT 1 FROM "ForkJob" j WHERE j."destAppId"=a.id AND j.policy->>'kind'='online_copy')
        ORDER BY m."appId""#,[payer_id.into(),cursor.clone().into(),((batch.clamp(1,BASELINE_PAGE)+1) as i64).into()])).await?;
    let ready = rows.len() <= batch;
    let mut projects = std::collections::HashMap::new();
    let mut insert_values = Vec::new();
    let mut placeholders = Vec::new();
    for row in rows.into_iter().take(batch) {
        cursor = row.try_get("", "appId")?;
        let Some(app_id) = row.try_get::<Option<String>>("", "ownedAppId")? else {
            continue;
        };
        let counted: bool = row.try_get("", "counted")?;
        let source: Option<String> = row.try_get("", "source")?;
        let size = row.try_get::<i64>("", "totalSize")?.max(0);
        projects.insert(app_id.clone(), (size, counted));
        let offset = insert_values.len();
        placeholders.push(format!(
            "(${},${},${},${},true)",
            offset + 1,
            offset + 2,
            offset + 3,
            offset + 4
        ));
        insert_values.extend([
            app_id.into(),
            payer_id.into(),
            counted.into(),
            source.into(),
        ]);
    }
    if !placeholders.is_empty() {
        let inserted = txn.query_all_raw(Statement::from_sql_and_values(txn.get_database_backend(),
            format!(r#"INSERT INTO "ProjectCapacity" ("appId","payerId",counted,"publicForkSourceId",active) VALUES {} ON CONFLICT ("appId") DO NOTHING RETURNING "appId""#, placeholders.join(",")), insert_values)).await?;
        for row in inserted {
            let app_id: String = row.try_get("", "appId")?;
            if let Some((size, counted)) = projects.get(&app_id) {
                usage.storage_bytes = usage
                    .storage_bytes
                    .checked_add(*size)
                    .ok_or_else(|| ApiError::internal("storage total overflow"))?;
                usage.project_count += i64::from(*counted);
            }
        }
    }
    txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        r#"UPDATE "AccountCapacity" SET "storageBytes"=$2,"projectCount"=$3,initialized=$4,"baselineAppId"=$5 WHERE "payerId"=$1"#,
        [payer_id.into(),usage.storage_bytes.into(),usage.project_count.into(),ready.into(),cursor.into()])).await?;
    Ok((usage, ready))
}

async fn initialized_usage<C: ConnectionTrait>(
    db: &C,
    payer_id: &str,
) -> Result<Option<CapacityUsage>, ApiError> {
    let row = db.query_one_raw(Statement::from_sql_and_values(db.get_database_backend(),
        r#"SELECT "storageBytes","reservedStorageBytes","projectCount" FROM "AccountCapacity" WHERE "payerId"=$1 AND initialized=true"#,
        [payer_id.into()])).await?;
    row.map(|row| {
        Ok(CapacityUsage {
            storage_bytes: row.try_get("", "storageBytes")?,
            reserved_storage_bytes: row.try_get("", "reservedStorageBytes")?,
            project_count: row.try_get("", "projectCount")?,
        })
    })
    .transpose()
}

/// Callers prepare outside their mutation transaction so a pending response does
/// not roll back baseline progress. Small new accounts initialize immediately.
pub async fn prepare_account(state: &AppState, payer_id: &str) -> Result<(), ApiError> {
    if initialized_usage(&state.db, payer_id).await?.is_some() {
        return Ok(());
    }
    let ready = state
        .transaction(|txn| {
            let payer = payer_id.to_owned();
            Box::pin(async move {
                Ok::<_, ApiError>(baseline_page(txn, &payer, BASELINE_PAGE).await?.1)
            })
        })
        .await?;
    if ready {
        Ok(())
    } else {
        Err(baseline_pending())
    }
}

pub async fn lock_account(
    txn: &DatabaseTransaction,
    payer_id: &str,
) -> Result<CapacityUsage, ApiError> {
    let (usage, ready) = baseline_page(txn, payer_id, BASELINE_PAGE).await?;
    if ready {
        Ok(usage)
    } else {
        Err(baseline_pending())
    }
}

pub async fn usage(state: &AppState, payer_id: &str) -> Result<CapacityUsage, ApiError> {
    usage_with_db(&state.db, state.db_dialect, payer_id).await
}

pub(crate) async fn usage_with_db(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    payer_id: &str,
) -> Result<CapacityUsage, ApiError> {
    if let Some(usage) = initialized_usage(db, payer_id).await? {
        return Ok(usage);
    }
    let (usage, ready) = crate::db::retry_transaction(
        db,
        dialect,
        None,
        &crate::db::RetryPolicy::default(),
        |txn| {
            let payer = payer_id.to_owned();
            Box::pin(async move { baseline_page(txn, &payer, BASELINE_PAGE).await })
        },
    )
    .await?;
    if ready {
        Ok(usage)
    } else {
        Err(baseline_pending())
    }
}

pub async fn prepare_pending(state: &AppState) -> Result<u64, ApiError> {
    let rows=state.db.query_all_raw(Statement::from_string(state.db.get_database_backend(),r#"SELECT "payerId" FROM "AccountCapacity" WHERE initialized=false ORDER BY "updatedAt","payerId" LIMIT 10"#)).await?;
    let started = std::time::Instant::now();
    let mut ready = 0;
    for row in rows {
        if started.elapsed() >= std::time::Duration::from_secs(5) {
            break;
        }
        let payer: String = row.try_get("", "payerId")?;
        ready += u64::from(
            state
                .transaction(|txn| {
                    let payer = payer.clone();
                    Box::pin(async move {
                        Ok::<_, ApiError>(baseline_page(txn, &payer, BASELINE_PAGE).await?.1)
                    })
                })
                .await?,
        );
    }
    Ok(ready)
}

async fn fence_plan(txn: &DatabaseTransaction, payer: &str, plan: &str) -> Result<(), ApiError> {
    crate::db::coordination::coordinate(txn, "account-quota", &[payer]).await?;
    let row = txn
        .query_one_raw(Statement::from_sql_and_values(
            txn.get_database_backend(),
            r#"SELECT tier FROM "User" WHERE id=$1"#,
            [payer.into()],
        ))
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if row.try_get::<String>("", "tier")?.to_uppercase() != plan {
        return Err(ApiError::conflict(
            "Your plan changed. Retry the operation with your current allowance.",
        ));
    }
    Ok(())
}

pub async fn summary(state: &AppState, payer_id: &str) -> Result<(i64, i64), ApiError> {
    let usage = usage(state, payer_id).await?;
    Ok((usage.storage_bytes, usage.project_count))
}

fn capacity_error(
    resource: &str,
    payer_id: &str,
    plan: &str,
    used: i64,
    reserved: i64,
    requested: i64,
    limit: i64,
) -> ApiError {
    ApiError::quota_exceeded(QuotaLimitDetails {
        required_model_tier: None,
        resource: resource.into(),
        scope: "account".into(),
        payer_id: payer_id.into(),
        plan: plan.into(),
        used,
        reserved,
        requested,
        limit,
        unit: if resource == "storage_bytes" {
            "bytes"
        } else {
            "projects"
        }
        .into(),
        period_end: None,
        actions: vec![
            "upgrade".into(),
            if resource == "storage_bytes" {
                "delete_files"
            } else {
                "delete_projects"
            }
            .into(),
            "use_local".into(),
        ],
    })
}

/// Call inside the transaction that creates the app or durable fork job.
pub async fn admit_project(
    txn: &DatabaseTransaction,
    payer_id: &str,
    app_id: &str,
    visibility: &Visibility,
    source: Option<&str>,
    plan: &str,
    limit: i64,
) -> Result<(), ApiError> {
    flow_like_db::coordination::app_capacity(txn, app_id).await?;
    fence_plan(txn, payer_id, plan).await?;
    let usage = lock_account(txn, payer_id).await?;
    if let Some(row) = txn
        .query_one_raw(Statement::from_sql_and_values(
            txn.get_database_backend(),
            r#"SELECT "payerId", "active" FROM "ProjectCapacity" WHERE "appId" = $1"#,
            [app_id.into()],
        ))
        .await?
    {
        if row.try_get::<String>("", "payerId")? != payer_id
            || !row.try_get::<bool>("", "active")?
        {
            return Err(ApiError::forbidden(
                "This project allocation is no longer available.",
            ));
        }
        return Ok(());
    }
    let counted = counts_as_project(visibility, source);
    if crate::quota::enforcing() && counted && limit >= 0 && usage.project_count >= limit {
        return Err(capacity_error(
            "projects",
            payer_id,
            plan,
            usage.project_count,
            0,
            1,
            limit,
        ));
    }
    txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        r#"INSERT INTO "ProjectCapacity" ("appId", "payerId", "counted", "publicForkSourceId") VALUES ($1,$2,$3,$4)"#,
        [app_id.into(), payer_id.into(), counted.into(), source.map(str::to_owned).into()])).await?;
    if counted {
        txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
            r#"UPDATE "AccountCapacity" SET "projectCount" = "projectCount" + 1, "updatedAt" = now() WHERE "payerId" = $1"#,
            [payer_id.into()])).await?;
    }
    Ok(())
}

/// Deletion releases the slot once, while preserving the payer for late S3 events.
pub async fn release_project(state: &AppState, app_id: &str) -> Result<(), ApiError> {
    if let Some(payer) = payer_for_app(&state.db, app_id).await? {
        prepare_account(state, &payer).await?;
    }
    state.transaction(|txn| {
        let app_id = app_id.to_owned();
        Box::pin(async move {
            flow_like_db::coordination::app_capacity(txn, &app_id).await?;
            let Some(payer_id) = payer_for_app(txn, &app_id).await? else { return Ok(()); };
            lock_account(txn, &payer_id).await?;
            let row = txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                r#"UPDATE "ProjectCapacity" SET "active" = false WHERE "appId" = $1 AND "active" = true RETURNING "counted""#,
                [app_id.into()])).await?;
            if row.map(|r| r.try_get::<bool>("", "counted")).transpose()?.unwrap_or(false) {
                txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                    r#"UPDATE "AccountCapacity" SET "projectCount" = GREATEST(0,"projectCount" - 1), "updatedAt" = now() WHERE "payerId" = $1"#,
                    [payer_id.into()])).await?;
            }
            Ok(())
        })
    }).await
}

/// Publication can release a counted slot; taking a public project private needs admission.
pub async fn set_visibility(
    txn: &DatabaseTransaction,
    app_id: &str,
    visibility: &Visibility,
    plan: &str,
    limit: i64,
) -> Result<(), ApiError> {
    flow_like_db::coordination::app_capacity(txn, app_id).await?;
    let Some(payer_id) = payer_for_app(txn, app_id).await? else {
        return Ok(());
    };
    fence_plan(txn, &payer_id, plan).await?;
    let usage = lock_account(txn, &payer_id).await?;
    let row = txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        r#"SELECT "counted", "publicForkSourceId", "active" FROM "ProjectCapacity" WHERE "appId" = $1"#,
        [app_id.into()])).await?.ok_or_else(|| ApiError::internal("project capacity missing"))?;
    let counted = row.try_get::<bool>("", "active")?
        && counts_as_project(
            visibility,
            row.try_get::<Option<String>>("", "publicForkSourceId")?
                .as_deref(),
        );
    let old: bool = row.try_get("", "counted")?;
    if old == counted {
        return Ok(());
    }
    if crate::quota::enforcing() && counted && limit >= 0 && usage.project_count >= limit {
        return Err(capacity_error(
            "projects",
            &payer_id,
            plan,
            usage.project_count,
            0,
            1,
            limit,
        ));
    }
    txn.execute_raw(Statement::from_sql_and_values(
        txn.get_database_backend(),
        r#"UPDATE "ProjectCapacity" SET "counted" = $2 WHERE "appId" = $1"#,
        [app_id.into(), counted.into()],
    ))
    .await?;
    txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        r#"UPDATE "AccountCapacity" SET "projectCount" = GREATEST(0,"projectCount" + $2), "updatedAt" = now() WHERE "payerId" = $1"#,
        [payer_id.into(), (if counted {1_i64} else {-1_i64}).into()])).await?;
    Ok(())
}

/// Move current occupancy with ownership. Existing operations retain their admitted
/// payer; unfinished work and issued upload grants must settle before a transfer.
pub async fn transfer_project(
    txn: &DatabaseTransaction,
    app_id: &str,
    new_payer: &str,
    plan: &str,
    project_limit: i64,
    storage_limit: i64,
) -> Result<(), ApiError> {
    flow_like_db::coordination::app_capacity(txn, app_id).await?;
    let old_payer = payer_for_app(txn, app_id)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if old_payer == new_payer {
        return Ok(());
    }
    let mut payers = [old_payer.as_str(), new_payer];
    payers.sort_unstable();
    for payer in payers {
        crate::db::coordination::coordinate(txn, "account-quota", &[payer]).await?;
    }
    fence_plan(txn, new_payer, plan).await?;
    for payer in payers {
        lock_account(txn, payer).await?;
    }
    for query in [
        r#"SELECT id FROM "StorageUploadGrant" WHERE "appId" = $1 LIMIT 1"#,
        r#"SELECT id FROM "QuotaOperation" WHERE "appId" = $1 AND status != 'finalized' LIMIT 1"#,
        r#"SELECT "destAppId" FROM "ForkJob" WHERE "destAppId" = $1 AND status NOT IN ('DONE','FAILED') LIMIT 1"#,
    ] {
        if txn
            .query_one_raw(Statement::from_sql_and_values(
                txn.get_database_backend(),
                query,
                [app_id.into()],
            ))
            .await?
            .is_some()
        {
            return Err(ApiError::conflict(
                "This project has unfinished cloud work or issued upload grants. Retry the ownership transfer after the work finishes and the grants expire.",
            ));
        }
    }
    let project = txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
        r#"SELECT p.counted,p.active,a."totalSize" FROM "ProjectCapacity" p JOIN "App" a ON a.id=p."appId" WHERE p."appId"=$1"#,[app_id.into()])).await?.ok_or(ApiError::NOT_FOUND)?;
    if !project.try_get::<bool>("", "active")? {
        return Err(ApiError::conflict("This project is being deleted."));
    }
    let counted = i64::from(project.try_get::<bool>("", "counted")?);
    let bytes = project.try_get::<i64>("", "totalSize")?.max(0);
    let usage = lock_account(txn, new_payer).await?;
    if crate::quota::enforcing()
        && counted > 0
        && project_limit >= 0
        && counted > project_limit.saturating_sub(usage.project_count)
    {
        return Err(capacity_error(
            "projects",
            new_payer,
            plan,
            usage.project_count,
            0,
            counted,
            project_limit,
        ));
    }
    if crate::quota::enforcing()
        && storage_limit >= 0
        && bytes
            > storage_limit
                .saturating_sub(usage.storage_bytes)
                .saturating_sub(usage.reserved_storage_bytes)
    {
        return Err(capacity_error(
            "storage_bytes",
            new_payer,
            plan,
            usage.storage_bytes,
            usage.reserved_storage_bytes,
            bytes,
            storage_limit,
        ));
    }
    for (payer, direction) in [(old_payer.as_str(), -1_i64), (new_payer, 1_i64)] {
        txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
            r#"UPDATE "AccountCapacity" SET "projectCount"=GREATEST(0,"projectCount"+$2),"storageBytes"=GREATEST(0,"storageBytes"+$3),"updatedAt"=now() WHERE "payerId"=$1"#,
            [payer.into(),(direction*counted).into(),(direction*bytes).into()])).await?;
    }
    txn.execute_raw(Statement::from_sql_and_values(
        txn.get_database_backend(),
        r#"UPDATE "ProjectCapacity" SET "payerId"=$2 WHERE "appId"=$1"#,
        [app_id.into(), new_payer.into()],
    ))
    .await?;
    Ok(())
}

/// Stops issuing further writes at capacity. Existing cloud credentials and URLs
/// remain valid until expiry, so this is an eventual storage gate, not a byte reservation.
pub async fn check_storage_write(
    state: &AppState,
    app_id: &str,
    fallback_payer: &str,
    requested_bytes: i64,
) -> Result<(), ApiError> {
    if requested_bytes < 0 {
        return Err(ApiError::bad_request("Upload size cannot be negative."));
    }
    let account = storage_account(state, app_id, fallback_payer).await?;
    let usage = match account.usage {
        Some(initialized) => initialized,
        None => usage(state, &account.payer_id).await?,
    };
    let limit = account.max_total_size;
    let occupied = usage
        .storage_bytes
        .saturating_add(usage.reserved_storage_bytes);
    if crate::quota::enforcing()
        && limit >= 0
        && (occupied >= limit || requested_bytes > limit.saturating_sub(occupied))
    {
        return Err(capacity_error(
            "storage_bytes",
            &account.payer_id,
            &account.plan,
            usage.storage_bytes,
            usage.reserved_storage_bytes,
            requested_bytes,
            limit,
        ));
    }
    Ok(())
}

fn object_id(bucket: &str, key: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(bucket.as_bytes());
    hasher.update(&[0]);
    hasher.update(key.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Reserve all issued S3 byte bounds in the same account transaction. A cancelled
/// HTTP request retains its grants. The tracker moves occupied bytes between the
/// reservation and stored-data totals as objects appear, change or are deleted.
pub async fn reserve_uploads(
    state: &AppState,
    app_id: &str,
    fallback_payer: &str,
    uploads: Vec<(String, String, u64)>,
    expires_at: chrono::DateTime<chrono::FixedOffset>,
) -> Result<(), ApiError> {
    if uploads.is_empty() {
        return Ok(());
    }
    let StorageAccount {
        payer_id,
        plan,
        max_total_size,
        usage,
    } = storage_account(state, app_id, fallback_payer).await?;
    if usage.is_none() {
        prepare_account(state, &payer_id).await?;
    }
    state
        .transaction(|txn| {
            let payer_id = payer_id.clone();
            let app_id = app_id.to_owned();
            let uploads = uploads.clone();
            let plan = plan.clone();
            Box::pin(async move {
                reserve_uploads_in(
                    txn,
                    &payer_id,
                    &app_id,
                    uploads,
                    expires_at,
                    &plan,
                    max_total_size,
                )
                .await
            })
        })
        .await
}

async fn reserve_uploads_in(
    txn: &DatabaseTransaction,
    payer_id: &str,
    app_id: &str,
    uploads: Vec<(String, String, u64)>,
    expires_at: chrono::DateTime<chrono::FixedOffset>,
    plan: &str,
    limit: i64,
) -> Result<(), ApiError> {
    flow_like_db::coordination::app_capacity(txn, app_id).await?;
    if payer_for_app(txn, app_id)
        .await?
        .as_deref()
        .is_some_and(|payer| payer != payer_id)
    {
        return Err(ApiError::conflict(
            "Project ownership changed. Retry the upload.",
        ));
    }
    fence_plan(txn, payer_id, plan).await?;
    let usage = lock_account(txn, &payer_id).await?;
    let mut objects = std::collections::BTreeMap::<String, (String, String, i64)>::new();
    for (bucket, key, bytes) in uploads {
        let bytes =
            i64::try_from(bytes).map_err(|_| ApiError::bad_request("Upload size is too large."))?;
        let entry = objects
            .entry(object_id(&bucket, &key))
            .or_insert((bucket, key, bytes));
        entry.2 = entry.2.max(bytes);
    }
    let objects: Vec<_> = objects.into_iter().collect();
    let mut extra = 0_i64;
    for chunk in objects.chunks(250) {
        let ids: Vec<sea_orm::Value> = chunk.iter().map(|(id, _)| id.clone().into()).collect();
        let ids_sql = (1..=ids.len())
            .map(|index| format!("(${index}::text)"))
            .collect::<Vec<_>>()
            .join(",");
        let rows = txn.query_all_raw(Statement::from_sql_and_values(txn.get_database_backend(),
            format!(r#"SELECT ids.id,g."maxBytes",g."reservedBytes",g."payerId",COALESCE(f.size,0) AS size
            FROM (VALUES {ids_sql}) AS ids(id)
            LEFT JOIN "StorageUploadGrant" g ON g.id=ids.id
            LEFT JOIN "FileAccountingObject" f ON f.id=ids.id"#), ids)).await?;
        let mut previous = std::collections::HashMap::new();
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let owner: Option<String> = row.try_get("", "payerId")?;
            if owner.as_deref().is_some_and(|owner| owner != payer_id) {
                return Err(ApiError::forbidden(
                    "The storage object belongs to another account.",
                ));
            }
            previous.insert(
                id,
                (
                    row.try_get::<Option<i64>>("", "maxBytes")?.unwrap_or(0),
                    row.try_get::<Option<i64>>("", "reservedBytes")?
                        .unwrap_or(0),
                    row.try_get::<i64>("", "size")?,
                ),
            );
        }
        let mut values = Vec::<sea_orm::Value>::new();
        let mut placeholders = Vec::new();
        for (id, (bucket, key, bytes)) in chunk {
            let (old_max, old_reserved, accounted) = previous
                .get(id)
                .copied()
                .ok_or_else(|| ApiError::internal("Upload accounting snapshot is missing"))?;
            let max_bytes = old_max.max(*bytes);
            let reserved = max_bytes.saturating_sub(accounted).max(0);
            extra = extra
                .checked_add(reserved - old_reserved)
                .ok_or_else(|| ApiError::bad_request("Upload batch is too large."))?;
            let offset = values.len();
            placeholders.push(format!(
                "({})",
                (1..=8)
                    .map(|index| format!("${}", offset + index))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
            values.extend([
                id.clone().into(),
                payer_id.into(),
                app_id.into(),
                bucket.clone().into(),
                key.clone().into(),
                max_bytes.into(),
                reserved.into(),
                expires_at.into(),
            ]);
        }
        txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
            format!(r#"INSERT INTO "StorageUploadGrant" (id,"payerId","appId",bucket,"objectKey","maxBytes","reservedBytes","expiresAt") VALUES {}
            ON CONFLICT (id) DO UPDATE SET "maxBytes"=EXCLUDED."maxBytes","reservedBytes"=EXCLUDED."reservedBytes","expiresAt"=GREATEST("StorageUploadGrant"."expiresAt",EXCLUDED."expiresAt"),"updatedAt"=now()"#, placeholders.join(",")), values)).await?;
    }
    let occupied = usage
        .storage_bytes
        .saturating_add(usage.reserved_storage_bytes);
    if crate::quota::enforcing() && limit >= 0 && extra > limit.saturating_sub(occupied) {
        return Err(capacity_error(
            "storage_bytes",
            &payer_id,
            &plan,
            usage.storage_bytes,
            usage.reserved_storage_bytes,
            extra,
            limit,
        ));
    }
    txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
    r#"UPDATE "AccountCapacity" SET "reservedStorageBytes" = GREATEST(0,"reservedStorageBytes" + $2), "updatedAt" = now() WHERE "payerId" = $1"#,
    [payer_id.into(), extra.into()])).await?;
    Ok(())
}

/// Bounded reconciliation after grant expiry. A grace period covers uploads that
/// started while a URL was valid. Unknown storage state retains its reservation.
/// This is eventual enforcement: S3 cannot revoke a request already in flight.
pub async fn sweep_upload_grants(state: &AppState) -> Result<u64, ApiError> {
    let cutoff = chrono::Utc::now().fixed_offset() - chrono::Duration::hours(24);
    let rows = state.db.query_all_raw(Statement::from_sql_and_values(state.db.get_database_backend(),
        r#"SELECT g."id",g."payerId",g."bucket",g."objectKey",g."expiresAt",p.active FROM "StorageUploadGrant" g LEFT JOIN "ProjectCapacity" p ON p."appId"=g."appId" WHERE g."expiresAt" < $1 ORDER BY g."expiresAt" LIMIT 100"#, [cutoff.into()])).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let credentials = state.master_credentials().await?;
    let Some(current_bucket) =
        crate::routes::app::data::upload_policy::content_bucket(credentials.as_ref())
    else {
        return Ok(0);
    };
    let store = credentials.to_store(false).await?.as_generic();
    let mut released = 0_u64;
    let started = std::time::Instant::now();
    for row in rows {
        if started.elapsed() >= std::time::Duration::from_secs(5) {
            break;
        }
        if row.try_get::<String>("", "bucket")? != current_bucket {
            continue;
        }
        let id: String = row.try_get("", "id")?;
        let payer: String = row.try_get("", "payerId")?;
        let key: String = row.try_get("", "objectKey")?;
        let expires: chrono::DateTime<chrono::FixedOffset> = row.try_get("", "expiresAt")?;
        let actual = match flow_like_types::tokio::time::timeout(
            std::time::Duration::from_secs(2),
            store.head(&flow_like_storage::Path::from(key.clone())),
        )
        .await
        {
            Ok(Ok(meta)) => i64::try_from(meta.size)
                .map_err(|_| ApiError::internal("storage object too large"))?,
            Ok(Err(flow_like_storage::object_store::Error::NotFound { .. })) => 0,
            _ => continue,
        };
        if row.try_get::<Option<bool>>("", "active")? == Some(false) && actual > 0 {
            // A valid form may have recreated this file after app deletion.
            // Keep its reservation until the delete event reconciles to zero.
            let _ = flow_like_types::tokio::time::timeout(
                std::time::Duration::from_secs(2),
                store.delete(&flow_like_storage::Path::from(key)),
            )
            .await;
            continue;
        }
        released += u64::from(state.transaction(|txn| {
            let id = id.clone(); let payer = payer.clone();
            Box::pin(async move {
                lock_account(txn, &payer).await?;
                let accounted = txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                    r#"SELECT "size" FROM "FileAccountingObject" WHERE "id" = $1"#, [id.clone().into()])).await?.map(|r| r.try_get::<i64>("", "size")).transpose()?.unwrap_or(0);
                if actual != accounted { return Ok::<bool, ApiError>(false); }
                let deleted = txn.query_one_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                    r#"DELETE FROM "StorageUploadGrant" WHERE "id" = $1 AND "expiresAt" = $2 RETURNING "reservedBytes""#, [id.into(), expires.into()])).await?;
                let Some(deleted) = deleted else { return Ok(false) };
                let reserved: i64 = deleted.try_get("", "reservedBytes")?;
                txn.execute_raw(Statement::from_sql_and_values(txn.get_database_backend(),
                    r#"UPDATE "AccountCapacity" SET "reservedStorageBytes" = GREATEST(0,"reservedStorageBytes" - $2), "updatedAt" = now() WHERE "payerId" = $1"#, [payer.into(), reserved.into()])).await?;
                Ok(true)
            })
        }).await?);
    }
    Ok(released)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DatabaseBackend, DatabaseConnection, TransactionTrait};

    #[test]
    fn only_server_verified_public_forks_are_exempt() {
        assert!(counts_as_project(&Visibility::Private, None));
        assert!(counts_as_project(&Visibility::Prototype, None));
        for visibility in [
            Visibility::Public,
            Visibility::PublicRequestAccess,
            Visibility::Offline,
        ] {
            assert!(!counts_as_project(&visibility, None));
        }
        for visibility in [Visibility::Private, Visibility::Prototype] {
            assert!(!counts_as_project(
                &visibility,
                Some("purchased-public-source")
            ));
        }
        assert_eq!(
            public_fork_source(&Visibility::Public, "source"),
            Some("source".into())
        );
        assert_eq!(
            public_fork_source(&Visibility::PublicRequestAccess, "source"),
            Some("source".into())
        );
        assert_eq!(public_fork_source(&Visibility::Private, "source"), None);
        assert_eq!(public_fork_source(&Visibility::Offline, "source"), None);
    }

    #[tokio::test]
    async fn postgres_project_admission_serializes_and_preserves_fork_provenance() {
        use sea_orm::sqlx::postgres::{PgConnectOptions, PgPoolOptions};
        use std::str::FromStr;
        let Ok(url) = std::env::var("FLOW_LIKE_TEST_DATABASE_URL") else {
            eprintln!("skipping capacity integration test: FLOW_LIKE_TEST_DATABASE_URL is unset");
            return;
        };
        let admin = Database::connect(&url).await.unwrap();
        let schema = format!(
            "capacity_{}",
            flow_like_types::create_id().replace('-', "_")
        );
        admin
            .execute_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("CREATE SCHEMA {schema}"),
            ))
            .await
            .unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect_with(
                PgConnectOptions::from_str(&url)
                    .unwrap()
                    .options([("search_path", schema.as_str())]),
            )
            .await
            .unwrap();
        let db = DatabaseConnection::from(pool);
        for statement in [
            r#"CREATE TABLE "MutationLock" (id BIGINT PRIMARY KEY, "updatedAt" TIMESTAMPTZ DEFAULT now())"#,
            r#"CREATE TABLE "QuotaOperation" (id TEXT PRIMARY KEY, "appId" TEXT, status TEXT)"#,
            r#"CREATE TABLE "User" (id TEXT PRIMARY KEY,tier TEXT)"#,
            r#"INSERT INTO "User" VALUES ('payer','FREE'),('recipient','FREE')"#,
            r#"CREATE TABLE "App" ("id" TEXT PRIMARY KEY, "ownerRoleId" TEXT, "visibility" TEXT, "forkedFrom" TEXT, "totalSize" BIGINT)"#,
            r#"CREATE TABLE "Membership" ("userId" TEXT, "roleId" TEXT, "appId" TEXT)"#,
            r#"CREATE TABLE "ForkJob" ("destAppId" TEXT, "policy" JSONB, status TEXT DEFAULT 'DONE')"#,
            r#"CREATE TABLE "FileAccountingObject" ("id" TEXT PRIMARY KEY, "size" BIGINT)"#,
        ] {
            db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, statement))
                .await
                .unwrap();
        }
        for statement in
            include_str!("../prisma/migrations/20260913120002_account_capacity/migration.sql")
                .split(';')
                .map(str::trim)
                .filter(|sql| !sql.is_empty())
        {
            db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, statement))
                .await
                .unwrap();
        }
        for statement in [
            r#"INSERT INTO "App" VALUES ('public-source',NULL,'PUBLIC',NULL,0),('original','owner','PRIVATE',NULL,10),('verified-fork','owner','PRIVATE','public-source',10),('unverified-fork','owner','PRIVATE','public-source',10)"#,
            r#"INSERT INTO "Membership" VALUES ('payer','owner','original'),('payer','owner','verified-fork'),('payer','owner','unverified-fork')"#,
            r#"INSERT INTO "ForkJob" ("destAppId",policy) VALUES ('verified-fork','{"kind":"online_copy"}')"#,
        ] {
            db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, statement))
                .await
                .unwrap();
        }
        let txn = db.begin().await.unwrap();
        let usage = lock_account(&txn, "payer").await.unwrap();
        assert_eq!(usage.project_count, 2);
        assert_eq!(usage.storage_bytes, 30);
        txn.commit().await.unwrap();
        let held = db.begin().await.unwrap();
        lock_account(&held, "payer").await.unwrap();
        let fast = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            initialized_usage(&db, "payer"),
        )
        .await
        .expect("initialized usage must not wait for the writer lock")
        .unwrap()
        .unwrap();
        assert_eq!((fast.storage_bytes, fast.project_count), (30, 2));
        held.rollback().await.unwrap();

        let results = futures::future::join_all((0..8).map(|i| {
            let db = db.clone();
            async move {
                let txn = db.begin().await.unwrap();
                let result = admit_project(
                    &txn,
                    "payer",
                    &format!("parallel-{i}"),
                    &Visibility::Private,
                    None,
                    "FREE",
                    3,
                )
                .await;
                if result.is_ok() {
                    txn.commit().await.unwrap();
                } else {
                    txn.rollback().await.unwrap();
                }
                result
            }
        }))
        .await;
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .all(|err| err.public_code() == "PLAN_LIMIT_EXCEEDED")
        );

        let txn = db.begin().await.unwrap();
        admit_project(
            &txn,
            "payer",
            "purchased-fork",
            &Visibility::Private,
            Some("public-source"),
            "FREE",
            0,
        )
        .await
        .unwrap();
        admit_project(
            &txn,
            "payer",
            "purchased-fork",
            &Visibility::Private,
            Some("public-source"),
            "FREE",
            0,
        )
        .await
        .unwrap();
        txn.execute_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"DELETE FROM "App" WHERE "id" = 'public-source'"#,
        ))
        .await
        .unwrap();
        set_visibility(&txn, "purchased-fork", &Visibility::Prototype, "FREE", 0)
            .await
            .unwrap();
        assert_eq!(lock_account(&txn, "payer").await.unwrap().project_count, 3);
        txn.commit().await.unwrap();

        let expiry = (chrono::Utc::now() + chrono::Duration::minutes(15)).fixed_offset();
        let uploads = futures::future::join_all((0..8).map(|i| {
            let db = db.clone();
            async move {
                let txn = db.begin().await.unwrap();
                let result = reserve_uploads_in(
                    &txn,
                    "payer",
                    "original",
                    vec![("bucket".into(), format!("parallel-{i}"), 100)],
                    expiry,
                    "FREE",
                    130,
                )
                .await;
                if result.is_ok() {
                    txn.commit().await.unwrap();
                } else {
                    txn.rollback().await.unwrap();
                }
                (i, result)
            }
        }))
        .await;
        assert_eq!(
            uploads.iter().filter(|(_, result)| result.is_ok()).count(),
            1
        );
        assert!(
            uploads
                .iter()
                .filter_map(|(_, result)| result.as_ref().err())
                .all(|err| err.public_code() == "PLAN_LIMIT_EXCEEDED")
        );
        let winner = uploads.iter().find(|(_, result)| result.is_ok()).unwrap().0;
        let key = format!("parallel-{winner}");
        let txn = db.begin().await.unwrap();
        // A response lost after commit may be retried at full capacity. Repeated
        // keys in the same request and a smaller form do not create headroom.
        reserve_uploads_in(
            &txn,
            "payer",
            "original",
            vec![
                ("bucket".into(), key.clone(), 100),
                ("bucket".into(), key.clone(), 50),
            ],
            expiry,
            "FREE",
            130,
        )
        .await
        .unwrap();
        assert_eq!(
            lock_account(&txn, "payer")
                .await
                .unwrap()
                .reserved_storage_bytes,
            100
        );
        txn.commit().await.unwrap();
        let txn = db.begin().await.unwrap();
        let denied = reserve_uploads_in(
            &txn,
            "payer",
            "original",
            vec![("bucket".into(), "batch-must-roll-back".into(), 1)],
            expiry,
            "FREE",
            130,
        )
        .await
        .unwrap_err();
        assert_eq!(denied.public_code(), "PLAN_LIMIT_EXCEEDED");
        txn.rollback().await.unwrap();
        assert!(
            db.query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT id FROM "StorageUploadGrant" WHERE id = $1"#,
                [object_id("bucket", "batch-must-roll-back").into()]
            ))
            .await
            .unwrap()
            .is_none()
        );
        let txn = db.begin().await.unwrap();
        assert_eq!(
            transfer_project(&txn, "original", "recipient", "FREE", 10, 1000)
                .await
                .unwrap_err()
                .public_code(),
            "CONFLICT"
        );
        txn.rollback().await.unwrap();
        let txn = db.begin().await.unwrap();
        assert_eq!(
            transfer_project(&txn, "unverified-fork", "recipient", "FREE", 0, 1000)
                .await
                .unwrap_err()
                .public_code(),
            "PLAN_LIMIT_EXCEEDED"
        );
        txn.rollback().await.unwrap();
        let txn = db.begin().await.unwrap();
        assert_eq!(
            transfer_project(&txn, "verified-fork", "recipient", "FREE", 0, 9)
                .await
                .unwrap_err()
                .public_code(),
            "PLAN_LIMIT_EXCEEDED"
        );
        txn.rollback().await.unwrap();
        let txn = db.begin().await.unwrap();
        transfer_project(&txn, "verified-fork", "recipient", "FREE", 0, 10)
            .await
            .unwrap();
        transfer_project(&txn, "unverified-fork", "recipient", "FREE", 1, 20)
            .await
            .unwrap();
        let old_usage = lock_account(&txn, "payer").await.unwrap();
        let new_usage = lock_account(&txn, "recipient").await.unwrap();
        assert_eq!((old_usage.storage_bytes, old_usage.project_count), (10, 2));
        assert_eq!((new_usage.storage_bytes, new_usage.project_count), (20, 1));
        assert_eq!(
            payer_for_app(&txn, "verified-fork")
                .await
                .unwrap()
                .as_deref(),
            Some("recipient")
        );
        txn.commit().await.unwrap();
        db.execute_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"UPDATE "User" SET tier='PREMIUM' WHERE id='recipient'"#,
        ))
        .await
        .unwrap();
        let txn = db.begin().await.unwrap();
        let changed_plan = reserve_uploads_in(
            &txn,
            "recipient",
            "verified-fork",
            vec![("bucket".into(), "after-downgrade".into(), 1)],
            expiry,
            "FREE",
            1000,
        )
        .await
        .unwrap_err();
        assert_eq!(changed_plan.public_code(), "CONFLICT");
        txn.rollback().await.unwrap();

        for query in [
            r#"INSERT INTO "User" VALUES ('batch','MAX')"#,
            r#"INSERT INTO "App" VALUES ('batch-app','batch-owner','PRIVATE',NULL,0)"#,
            r#"INSERT INTO "Membership" VALUES ('batch','batch-owner','batch-app')"#,
        ] {
            db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, query))
                .await
                .unwrap();
        }
        let txn = db.begin().await.unwrap();
        lock_account(&txn, "batch").await.unwrap();
        let mut batch_uploads: Vec<_> = (0..100)
            .map(|index| ("bucket".to_owned(), format!("batch-{index}"), 10))
            .collect();
        batch_uploads.extend([
            ("bucket".into(), "batch-0".into(), 12),
            ("bucket".into(), "batch-0".into(), 4),
        ]);
        for _ in 0..2 {
            reserve_uploads_in(
                &txn,
                "batch",
                "batch-app",
                batch_uploads.clone(),
                expiry,
                "MAX",
                1002,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            lock_account(&txn, "batch")
                .await
                .unwrap()
                .reserved_storage_bytes,
            1002
        );
        let grants = txn
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                r#"SELECT count(*) AS n FROM "StorageUploadGrant" WHERE "payerId"='batch'"#,
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(grants.try_get::<i64>("", "n").unwrap(), 100);
        txn.commit().await.unwrap();
        let txn = db.begin().await.unwrap();
        let denied_batch = reserve_uploads_in(
            &txn,
            "batch",
            "batch-app",
            vec![
                ("bucket".into(), "batch-0".into(), 13),
                ("bucket".into(), "new".into(), 10),
            ],
            expiry,
            "MAX",
            1002,
        )
        .await
        .unwrap_err();
        assert_eq!(denied_batch.public_code(), "PLAN_LIMIT_EXCEEDED");
        txn.rollback().await.unwrap();
        assert_eq!(
            initialized_usage(&db, "batch")
                .await
                .unwrap()
                .unwrap()
                .reserved_storage_bytes,
            1002
        );

        for query in [
            r#"INSERT INTO "User" VALUES ('bulk','MAX')"#,
            r#"INSERT INTO "App" (id,"ownerRoleId",visibility,"totalSize") SELECT 'bulk-'||LPAD(n::text,5,'0'),'bulk-owner','PRIVATE',1 FROM generate_series(1,3005) n"#,
            r#"INSERT INTO "Membership" SELECT 'bulk','bulk-owner',id FROM "App" WHERE id LIKE 'bulk-%'"#,
        ] {
            db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, query))
                .await
                .unwrap();
        }
        let txn = db.begin().await.unwrap();
        let (partial, ready) = baseline_page(&txn, "bulk", BASELINE_PAGE).await.unwrap();
        assert!(!ready);
        assert_eq!((partial.storage_bytes, partial.project_count), (250, 250));
        txn.commit().await.unwrap();
        assert!(initialized_usage(&db, "bulk").await.unwrap().is_none());
        let txn = db.begin().await.unwrap();
        let pending = admit_project(
            &txn,
            "bulk",
            "new-before-ready",
            &Visibility::Private,
            None,
            "MAX",
            -1,
        )
        .await
        .unwrap_err();
        assert_eq!(pending.public_code(), "USAGE_REFRESH_PENDING");
        txn.rollback().await.unwrap();
        // Simulate S3 changes on an already-baselined app and one still ahead of
        // the cursor. Only the former has contributed to the partial counter.
        for query in [
            r#"UPDATE "App" SET "totalSize"=2 WHERE id='bulk-00001'"#,
            r#"UPDATE "AccountCapacity" SET "storageBytes"="storageBytes"+1 WHERE "payerId"='bulk'"#,
            r#"UPDATE "App" SET "totalSize"=7 WHERE id='bulk-03005'"#,
        ] {
            db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, query))
                .await
                .unwrap();
        }
        let mut pages = 1;
        loop {
            let txn = db.begin().await.unwrap();
            let (usage, ready) = baseline_page(&txn, "bulk", BASELINE_PAGE).await.unwrap();
            txn.commit().await.unwrap();
            pages += 1;
            if ready {
                assert_eq!((usage.storage_bytes, usage.project_count), (3012, 3005));
                break;
            }
            assert!(pages < 20, "baseline must make bounded forward progress");
        }
        assert_eq!(pages, 13);
        db.close().await.unwrap();
        admin
            .execute_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("DROP SCHEMA {schema} CASCADE"),
            ))
            .await
            .unwrap();
    }
}
