//! The write path: one insert per audited change, no read, no lock, no sequence.
//!
//! The record is hashed and MACed in memory. The hash covers salted commitments to
//! the personal values (IP, details), so those can expire later without breaking the
//! chain the audit worker builds from the record hashes.

use std::time::Duration;

use chrono::{DateTime, FixedOffset, Utc};
use flow_like_types::{Value, create_id, tokio};
use sea_orm::{
    ActiveEnum, ActiveValue::Set, ConnectionTrait, DbErr, EntityTrait, sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::db::{DbConflict, classify_db_err};
use crate::entity::{audit_record, sea_orm_active_enums::AuditActorType};

use super::crypto::{
    RecordFields, canonical_json, details_commitment, ip_commitment, once_id, random_salt,
    record_hash, record_mac, strip_nul, strip_nul_value,
};
use super::keys::entry_kid;
use super::level::RetentionClass;

/// Chain of changes that belong to no app or package.
pub const PLATFORM_CHAIN: &str = "platform";
/// Suffix of the chain that holds a scope's verbose-only records.
pub const ACTIVITY_SUFFIX: &str = "#activity";
/// Details larger than this are replaced by their size; audit details carry ids and
/// short codes, never payloads.
pub const MAX_DETAILS_BYTES: usize = 1024;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AuditRecordInput {
    pub actor_id: String,
    #[schema(value_type = String)]
    pub actor_type: AuditActorType,
    pub actor_ip: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    /// App or package id; `None` records on the platform chain.
    pub scope: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub details: Option<Value>,
}

impl AuditRecordInput {
    /// A change no authenticated caller made, such as one applied from a verified
    /// provider webhook or a background job. Recorded on the platform chain unless moved.
    pub fn system(actor_id: &str, action: &str, resource_type: &str, resource_id: &str) -> Self {
        Self {
            actor_id: actor_id.to_owned(),
            actor_type: AuditActorType::System,
            actor_ip: None,
            action: action.to_owned(),
            resource_type: resource_type.to_owned(),
            resource_id: resource_id.to_owned(),
            scope: None,
            details: None,
        }
    }

    pub fn on_scope(mut self, scope: &str) -> Self {
        self.scope = Some(scope.to_owned());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Chain the record lands on: the scope's chain, its activity chain for
    /// verbose-only actions, or the platform chain.
    pub fn chain_id(&self) -> String {
        chain_for(self.scope.as_deref(), &self.action)
    }
}

pub fn chain_for(scope: Option<&str>, action: &str) -> String {
    let base = scope
        .filter(|scope| !scope.is_empty())
        .unwrap_or(PLATFORM_CHAIN);
    match RetentionClass::of(action) {
        RetentionClass::Evidence => base.to_owned(),
        RetentionClass::Activity => format!("{base}{ACTIVITY_SUFFIX}"),
    }
}

/// Prefix of package chains. Package ids and app ids come from different namespaces, so
/// a package chain must never be readable as the chain of an app with the same id.
pub const PACKAGE_PREFIX: &str = "package:";

/// Scope of a package's records.
pub fn package_scope(package_id: &str) -> String {
    format!("{PACKAGE_PREFIX}{package_id}")
}

/// The app a chain belongs to, `None` for the platform and package chains (which only
/// platform administrators read).
pub fn chain_scope(chain_id: &str) -> Option<&str> {
    let base = chain_id.strip_suffix(ACTIVITY_SUFFIX).unwrap_or(chain_id);
    (base != PLATFORM_CHAIN && !base.starts_with(PACKAGE_PREFIX)).then_some(base)
}

pub fn chain_class(chain_id: &str) -> RetentionClass {
    if chain_id.ends_with(ACTIVITY_SUFFIX) {
        RetentionClass::Activity
    } else {
        RetentionClass::Evidence
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    Append,
    /// One record per (chain, action, resource): a repeated write is a no-op.
    Once,
}

/// Build the row for `input`. Separate from the insert so it is testable and so the
/// hash inputs are visible in one place.
pub fn build_record(
    mut input: AuditRecordInput,
    mode: WriteMode,
    entry_key: &[u8; 32],
    now: DateTime<Utc>,
) -> audit_record::ActiveModel {
    for text in [
        &mut input.actor_id,
        &mut input.action,
        &mut input.resource_type,
        &mut input.resource_id,
    ] {
        strip_nul(text);
    }
    if let Some(scope) = input.scope.as_mut() {
        strip_nul(scope);
    }
    let chain_id = input.chain_id();
    let id = match mode {
        WriteMode::Append => create_id(),
        WriteMode::Once => once_id(
            &chain_id,
            &input.action,
            &input.resource_type,
            &input.resource_id,
        ),
    };
    // Timestamps are hashed, so they carry the database's millisecond precision.
    let timestamp: DateTime<FixedOffset> = DateTime::from_timestamp_millis(now.timestamp_millis())
        .expect("current time fits in milliseconds")
        .fixed_offset();

    let actor_ip = input
        .actor_ip
        .take()
        .map(|mut ip| {
            strip_nul(&mut ip);
            ip
        })
        .filter(|ip| !ip.is_empty());
    let ip = actor_ip.map(|ip| {
        let salt = random_salt();
        (ip_commitment(&salt, &ip), salt, ip)
    });

    // JSON `null` is stored as no details, so the commitment triple stays all-or-nothing.
    let details = input
        .details
        .take()
        .filter(|details| !details.is_null())
        .map(|mut details| {
            strip_nul_value(&mut details);
            let size = canonical_json(&details).len();
            if size > MAX_DETAILS_BYTES {
                tracing::warn!(
                    action = %input.action,
                    size,
                    "audit details exceed {MAX_DETAILS_BYTES} bytes and were replaced by their size"
                );
                details = serde_json::json!({ "omitted_bytes": size });
            }
            let salt = random_salt();
            (details_commitment(&salt, &details), salt, details)
        });

    let actor_type = input.actor_type.to_value();
    let hash = record_hash(&RecordFields {
        id: &id,
        chain_id: &chain_id,
        timestamp_ms: timestamp.timestamp_millis(),
        actor_id: &input.actor_id,
        actor_type: &actor_type,
        action: &input.action,
        resource_type: &input.resource_type,
        resource_id: &input.resource_id,
        ip_commitment: ip.as_ref().map(|(commitment, _, _)| commitment.as_slice()),
        details_commitment: details
            .as_ref()
            .map(|(commitment, _, _)| commitment.as_slice()),
    });

    audit_record::ActiveModel {
        id: Set(id),
        chain_id: Set(chain_id),
        timestamp: Set(timestamp),
        actor_id: Set(input.actor_id),
        actor_type: Set(input.actor_type),
        action: Set(input.action),
        resource_type: Set(input.resource_type),
        resource_id: Set(input.resource_id),
        ip_commitment: Set(ip.as_ref().map(|(commitment, _, _)| commitment.to_vec())),
        details_commitment: Set(details
            .as_ref()
            .map(|(commitment, _, _)| commitment.to_vec())),
        actor_ip: Set(ip.as_ref().map(|(_, _, ip)| ip.clone())),
        ip_salt: Set(ip.as_ref().map(|(_, salt, _)| salt.to_vec())),
        details: Set(details.as_ref().map(|(_, _, details)| details.clone())),
        details_salt: Set(details.as_ref().map(|(_, salt, _)| salt.to_vec())),
        mac: Set(Some(record_mac(entry_key, &hash).to_vec())),
        // Only the worker may assign records to seals. Omit the column so the API
        // can use an INSERT grant that excludes sealId.
        seal_id: sea_orm::ActiveValue::NotSet,
        entry_kid: Set(Some(entry_kid(entry_key))),
    }
}

/// Attempts at the insert while a schema change has invalidated the session's catalog
/// (Aurora DSQL `OC001`). A failed single statement commits nothing and the built row
/// is reused, so the retry writes exactly what the first attempt would have.
const SCHEMA_CHANGE_ATTEMPTS: u32 = 3;
const SCHEMA_CHANGE_BACKOFF: Duration = Duration::from_millis(50);

/// Insert one record. A single statement: records never conflict with each other, so
/// any number of writers on the same chain proceed in parallel.
pub async fn write<C: ConnectionTrait>(
    db: &C,
    input: AuditRecordInput,
    mode: WriteMode,
) -> Result<(), DbErr> {
    let model = build_record(input, mode, super::keys::entry_key(), Utc::now());
    let mut attempt = 1;
    loop {
        let Err(error) = insert(db, model.clone(), mode).await else {
            return Ok(());
        };
        let Some(delay) = schema_change_backoff(attempt, classify_db_err(&error)) else {
            return Err(error);
        };
        tracing::warn!(attempt, %error, "audit record insert hit a schema change; retrying");
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

/// The pause before the next attempt, or `None` when `error` is not a stale catalog or
/// `attempt` was the last one.
fn schema_change_backoff(attempt: u32, conflict: Option<DbConflict>) -> Option<Duration> {
    (conflict == Some(DbConflict::SchemaChanged) && attempt < SCHEMA_CHANGE_ATTEMPTS)
        .then(|| SCHEMA_CHANGE_BACKOFF * attempt)
}

async fn insert<C: ConnectionTrait>(
    db: &C,
    model: audit_record::ActiveModel,
    mode: WriteMode,
) -> Result<(), DbErr> {
    let insert = audit_record::Entity::insert(model);
    match mode {
        WriteMode::Append => {
            insert.exec_without_returning(db).await?;
        }
        WriteMode::Once => {
            insert
                .on_conflict(
                    OnConflict::column(audit_record::Column::Id)
                        .do_nothing()
                        .to_owned(),
                )
                .exec_without_returning(db)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::crypto::{mac_matches, to_hash};

    fn input(action: &str, scope: Option<&str>) -> AuditRecordInput {
        AuditRecordInput {
            actor_id: "openid:user:".into(),
            actor_type: AuditActorType::User,
            actor_ip: Some("192.0.2.1".into()),
            action: action.into(),
            resource_type: "Board".into(),
            resource_id: "board-1".into(),
            scope: scope.map(str::to_owned),
            details: Some(serde_json::json!({ "count": 2 })),
        }
    }

    fn unwrap<T: Clone + Into<sea_orm::Value>>(value: &sea_orm::ActiveValue<T>) -> T {
        value.clone().unwrap()
    }

    #[test]
    fn chains_follow_scope_and_retention_class() {
        assert_eq!(chain_for(Some("app"), "board.update"), "app");
        assert_eq!(
            chain_for(Some("app"), "board.commands.execute"),
            "app#activity"
        );
        assert_eq!(chain_for(None, "pat.create"), "platform");
        assert_eq!(
            chain_for(Some(""), "api.request.attempt"),
            "platform#activity"
        );
        assert_eq!(chain_scope("app#activity"), Some("app"));
        assert_eq!(chain_scope("platform#activity"), None);
        assert_eq!(chain_class("app#activity"), RetentionClass::Activity);
        assert_eq!(chain_class("app"), RetentionClass::Evidence);
    }

    #[test]
    fn built_records_carry_a_valid_mac_over_their_commitments() {
        let key = [5; 32];
        let now = Utc::now();
        let model = build_record(
            input("board.update", Some("app")),
            WriteMode::Append,
            &key,
            now,
        );
        let id = unwrap(&model.id);
        let stored_ip = unwrap(&model.ip_commitment).unwrap();
        let stored_details = unwrap(&model.details_commitment).unwrap();
        let ip_salt = to_hash(&unwrap(&model.ip_salt).unwrap()).unwrap();
        let details_salt = to_hash(&unwrap(&model.details_salt).unwrap()).unwrap();
        assert_eq!(
            stored_ip,
            ip_commitment(&ip_salt, &unwrap(&model.actor_ip).unwrap()).to_vec()
        );
        assert_eq!(
            stored_details,
            details_commitment(&details_salt, &unwrap(&model.details).unwrap()).to_vec()
        );
        let timestamp = unwrap(&model.timestamp);
        assert_eq!(timestamp.timestamp_subsec_nanos() % 1_000_000, 0);
        let hash = record_hash(&RecordFields {
            id: &id,
            chain_id: "app",
            timestamp_ms: timestamp.timestamp_millis(),
            actor_id: "openid:user:",
            actor_type: "USER",
            action: "board.update",
            resource_type: "Board",
            resource_id: "board-1",
            ip_commitment: Some(&stored_ip),
            details_commitment: Some(&stored_details),
        });
        assert!(mac_matches(&key, &hash, &unwrap(&model.mac).unwrap()));
        assert!(model.seal_id.is_not_set());
        assert_eq!(unwrap(&model.entry_kid), Some(entry_kid(&key)));
    }

    #[test]
    fn once_records_share_an_id_and_appends_do_not() {
        let key = [6; 32];
        let now = Utc::now();
        let once = |_| {
            unwrap(
                &build_record(
                    input("execution.board.complete", Some("app")),
                    WriteMode::Once,
                    &key,
                    now,
                )
                .id,
            )
        };
        assert_eq!(once(1), once(2));
        let append = |_| {
            unwrap(
                &build_record(
                    input("board.update", Some("app")),
                    WriteMode::Append,
                    &key,
                    now,
                )
                .id,
            )
        };
        assert_ne!(append(1), append(2));
    }

    #[test]
    fn oversized_details_are_replaced_and_empty_ips_dropped() {
        let mut large = input("board.update", Some("app"));
        large.details = Some(serde_json::json!({ "text": "x".repeat(2000) }));
        large.actor_ip = Some(String::new());
        let model = build_record(large, WriteMode::Append, &[1; 32], Utc::now());
        let details = unwrap(&model.details).unwrap();
        assert!(details["omitted_bytes"].as_u64().unwrap() > MAX_DETAILS_BYTES as u64);
        assert!(unwrap(&model.actor_ip).is_none());
        assert!(unwrap(&model.ip_commitment).is_none());
    }

    #[test]
    fn nul_bytes_never_reach_the_row() {
        let mut dirty = input("board.update", Some("app"));
        dirty.resource_id = "board\u{0}1".into();
        dirty.details = Some(serde_json::json!({ "k\u{0}": "v\u{0}" }));
        let model = build_record(dirty, WriteMode::Append, &[1; 32], Utc::now());
        assert!(!unwrap(&model.resource_id).contains('\0'));
        assert!(!canonical_json(&unwrap(&model.details).unwrap()).contains("\\u0000"));
    }

    #[derive(Debug)]
    struct SqlState(&'static str);

    impl std::fmt::Display for SqlState {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for SqlState {}

    impl sea_orm::sqlx::error::DatabaseError for SqlState {
        fn message(&self) -> &str {
            "schema has been updated by another transaction"
        }

        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            Some(self.0.into())
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sea_orm::sqlx::error::ErrorKind {
            sea_orm::sqlx::error::ErrorKind::Other
        }
    }

    fn db_err(code: &'static str) -> DbErr {
        DbErr::Exec(sea_orm::RuntimeErr::SqlxError(std::sync::Arc::new(
            sea_orm::sqlx::Error::Database(Box::new(SqlState(code))),
        )))
    }

    #[test]
    fn only_a_schema_change_is_retried_and_only_a_bounded_number_of_times() {
        let stale = classify_db_err(&db_err("OC001"));
        assert_eq!(stale, Some(DbConflict::SchemaChanged));
        assert_eq!(schema_change_backoff(1, stale), Some(SCHEMA_CHANGE_BACKOFF));
        assert_eq!(
            schema_change_backoff(2, stale),
            Some(SCHEMA_CHANGE_BACKOFF * 2)
        );
        assert_eq!(schema_change_backoff(SCHEMA_CHANGE_ATTEMPTS, stale), None);
        for code in ["OC000", "23505", "42501"] {
            assert_eq!(
                schema_change_backoff(1, classify_db_err(&db_err(code))),
                None,
                "{code}"
            );
        }
        assert_eq!(
            schema_change_backoff(1, classify_db_err(&DbErr::Custom("x".into()))),
            None
        );
    }
}
