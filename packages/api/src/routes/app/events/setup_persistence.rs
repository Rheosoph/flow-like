use super::{PersistError, PersistInputs, PreparedRegistrations, find_event_setup};
use crate::{
    db::{
        DEFAULT_WRITE_CHUNK, DSQL_MAX_ROWS_PER_TRANSACTION, DbDialect, batch::DEFAULT_WRITE_BYTES,
    },
    entity::{event, event_remote_auth, event_remote_registration},
    execution::variant::STABLE_VARIANT,
    state::AppState,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect, TryIntoModel, sea_query::OnConflict,
};
use std::collections::HashMap;

type RouteKey = (String, Option<String>, String);

fn route_key(row: &event_remote_registration::Model) -> RouteKey {
    (
        row.kind.clone(),
        row.method.as_ref().map(|m| m.to_uppercase()),
        row.path.clone(),
    )
}

struct Replacement {
    registrations: Vec<event_remote_registration::ActiveModel>,
    auths: Vec<event_remote_auth::ActiveModel>,
    remove_registrations: Vec<String>,
    remove_auths: Vec<String>,
    bytes: usize,
}

impl Replacement {
    fn rows(&self) -> usize {
        // The Event lock and both serving pointers also participate in the write set.
        self.registrations.len()
            + self.auths.len()
            + self.remove_registrations.len()
            + self.remove_auths.len()
            + 4
    }

    fn validate(&self, dialect: DbDialect) -> Result<(), PersistError> {
        if dialect.bounded_transactions() {
            for row in &self.registrations {
                let row = row.clone().try_into_model()?;
                validate_index_key(&[
                    &row.app_id,
                    &row.event_id,
                    &row.event_version,
                    &row.variant,
                    &row.kind,
                    row.method.as_deref().unwrap_or_default(),
                    &row.path,
                ])?;
                if encoded_size(&row)? > 2 * 1024 * 1024 - 8192 {
                    return Err(PersistError::Budget("A route configuration exceeds the database's 2 MiB row limit. Reduce its schema or configuration. The previous setup remains active.".into()));
                }
                for json in [row.schema_json.as_ref(), row.extras_json.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    validate_json_column(json)?;
                }
            }
            for row in &self.auths {
                let row = row.clone().try_into_model()?;
                validate_index_key(&[
                    &row.app_id,
                    &row.event_id,
                    &row.event_version,
                    &row.variant,
                    &row.node_id,
                    &row.kind,
                ])?;
                validate_json_column(&row.config_json)?;
            }
        }
        if dialect.bounded_transactions()
            && (self.rows() > DSQL_MAX_ROWS_PER_TRANSACTION || self.bytes > DEFAULT_WRITE_BYTES)
        {
            return Err(PersistError::Budget(format!(
                "This setup changes {} rows and approximately {} bytes, exceeding the database's atomic setup budget ({} rows, {} bytes). Split the routes across multiple events. The previous setup remains active.",
                self.rows(),
                self.bytes,
                DSQL_MAX_ROWS_PER_TRANSACTION,
                DEFAULT_WRITE_BYTES,
            )));
        }
        Ok(())
    }
}

fn validate_index_key(parts: &[&str]) -> Result<(), PersistError> {
    // Reserve space for key encoding in DSQL's 1 KiB secondary-index limit.
    if parts.iter().map(|part| part.len()).sum::<usize>() > 1024 - 128 {
        return Err(PersistError::Budget("A route path, resource URI or node identifier exceeds the database's indexed-key limit. Shorten it before running setup again. The previous setup remains active.".into()));
    }
    Ok(())
}

fn validate_json_column(value: &serde_json::Value) -> Result<(), PersistError> {
    if serde_json::to_vec(value)
        .map_err(flow_like_types::Error::from)?
        .len()
        > 1024 * 1024 - 4096
    {
        return Err(PersistError::Budget(
            "A route schema or configuration exceeds the database's 1 MiB column limit. Reduce that configuration before running setup again. The previous setup remains active.".into(),
        ));
    }
    Ok(())
}

fn encoded_size(row: &impl serde::Serialize) -> Result<usize, PersistError> {
    // Include row and index overhead in addition to the JSON representation.
    Ok(serde_json::to_vec(row)
        .map_err(flow_like_types::Error::from)?
        .len()
        + 1024)
}

fn reconcile(
    prepared: &PreparedRegistrations,
    registrations: Vec<event_remote_registration::Model>,
    auths: Vec<event_remote_auth::Model>,
) -> Result<Replacement, PersistError> {
    let mut result = Replacement {
        registrations: Vec::new(),
        auths: Vec::new(),
        remove_registrations: Vec::new(),
        remove_auths: Vec::new(),
        bytes: 8192,
    };
    let mut old_auths: HashMap<(String, String), Vec<event_remote_auth::Model>> = HashMap::new();
    for row in auths {
        old_auths
            .entry((row.node_id.clone(), row.kind.clone()))
            .or_default()
            .push(row);
    }
    let mut auth_ids = HashMap::new();
    for active in &prepared.auths {
        let mut row = active.clone().try_into_model()?;
        let generated_id = row.id.clone();
        if let Some(old) = old_auths
            .get_mut(&(row.node_id.clone(), row.kind.clone()))
            .and_then(Vec::pop)
        {
            row.id = old.id;
            row.created_at = old.created_at;
        }
        auth_ids.insert(generated_id, row.id.clone());
        result.bytes += encoded_size(&row)?;
        // An existing auth row is updated in place so its registrations remain attached.
        result.auths.push(row.into_active_model().reset_all());
    }

    let mut old_routes: HashMap<RouteKey, Vec<event_remote_registration::Model>> = HashMap::new();
    for row in registrations {
        old_routes.entry(route_key(&row)).or_default().push(row);
    }
    for active in &prepared.registrations {
        let mut row = active.clone().try_into_model()?;
        if let Some(id) = row.auth_id.as_mut() {
            if let Some(actual) = auth_ids.get(id) {
                *id = actual.clone();
            }
        }
        if let Some(old) = old_routes.get_mut(&route_key(&row)).and_then(Vec::pop) {
            row.id = old.id.clone();
            row.created_at = old.created_at;
            if row == old {
                continue;
            }
        }
        result.bytes += encoded_size(&row)?;
        result
            .registrations
            .push(row.into_active_model().reset_all());
    }
    for row in old_routes.into_values().flatten() {
        result.bytes += encoded_size(&row)?;
        result.remove_registrations.push(row.id);
    }
    for row in old_auths.into_values().flatten() {
        result.bytes += encoded_size(&row)?;
        result.remove_auths.push(row.id);
    }
    Ok(result)
}

pub(super) async fn replace_registration_rows(
    txn: &DatabaseTransaction,
    state: &AppState,
    inputs: &PersistInputs,
) -> Result<(usize, usize), PersistError> {
    let existing = event_remote_registration::Entity::find()
        .filter(event_remote_registration::Column::AppId.eq(&inputs.app_id))
        .filter(event_remote_registration::Column::EventId.eq(&inputs.event_id))
        .filter(event_remote_registration::Column::EventVersion.eq(&inputs.event_version))
        .filter(event_remote_registration::Column::Variant.eq(&inputs.variant))
        .all(txn)
        .await?;
    let existing_auths = event_remote_auth::Entity::find()
        .filter(event_remote_auth::Column::AppId.eq(&inputs.app_id))
        .filter(event_remote_auth::Column::EventId.eq(&inputs.event_id))
        .filter(event_remote_auth::Column::EventVersion.eq(&inputs.event_version))
        .filter(event_remote_auth::Column::Variant.eq(&inputs.variant))
        .all(txn)
        .await?;
    let replacement = reconcile(&inputs.prepared, existing, existing_auths)?;
    replacement.validate(state.db_dialect)?;

    for ids in replacement.remove_registrations.chunks(DEFAULT_WRITE_CHUNK) {
        event_remote_registration::Entity::delete_many()
            .filter(event_remote_registration::Column::Id.is_in(ids.to_vec()))
            .exec(txn)
            .await?;
    }
    for rows in replacement.auths.chunks(100) {
        event_remote_auth::Entity::insert_many(rows.to_vec())
            .on_conflict(
                OnConflict::column(event_remote_auth::Column::Id)
                    .update_columns([
                        event_remote_auth::Column::ConfigJson,
                        event_remote_auth::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(txn)
            .await?;
    }
    for rows in replacement.registrations.chunks(100) {
        event_remote_registration::Entity::insert_many(rows.to_vec())
            .on_conflict(
                OnConflict::column(event_remote_registration::Column::Id)
                    .update_columns([
                        event_remote_registration::Column::NodeId,
                        event_remote_registration::Column::SchemaJson,
                        event_remote_registration::Column::ExtrasJson,
                        event_remote_registration::Column::AuthId,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(txn)
            .await?;
    }
    // Reassign registration references before removing obsolete auth rows, avoiding cascades.
    for ids in replacement.remove_auths.chunks(DEFAULT_WRITE_CHUNK) {
        event_remote_auth::Entity::delete_many()
            .filter(event_remote_auth::Column::Id.is_in(ids.to_vec()))
            .exec(txn)
            .await?;
    }
    Ok((
        inputs.prepared.registrations.len(),
        inputs.prepared.auths.len(),
    ))
}

pub(super) async fn prune_registration_versions(
    state: &AppState,
    inputs: &PersistInputs,
    previous_version: Option<&str>,
) -> Result<(), PersistError> {
    // Each page obtains the same Event lock as setup before reading serving pointers.
    // Re-reading those pointers prevents a concurrent setup from losing its active rows.
    for _ in 0..256 {
        let removed = state
            .transaction(|txn| {
                let app_id = inputs.app_id.clone();
                let event_id = inputs.event_id.clone();
                let variant = inputs.variant.clone();
                let written = inputs.event_version.clone();
                let previous = previous_version.map(ToOwned::to_owned);
                Box::pin(async move {
                    let Some(event) = event::Entity::find_by_id(&event_id)
                        .filter(event::Column::AppId.eq(&app_id))
                        .lock_exclusive()
                        .one(txn)
                        .await?
                    else {
                        return Ok::<_, PersistError>(0);
                    };
                    let live = find_event_setup(txn, &app_id, &event_id, &variant)
                        .await?
                        .map(|row| row.event_version)
                        .or_else(|| {
                            (variant == STABLE_VARIANT)
                                .then_some(event.last_setup_version)
                                .flatten()
                        });
                    let protected: Vec<String> = [Some(written), previous, live]
                        .into_iter()
                        .flatten()
                        .collect();
                    let rows = event_remote_registration::Entity::find()
                        .filter(event_remote_registration::Column::AppId.eq(&app_id))
                        .filter(event_remote_registration::Column::EventId.eq(&event_id))
                        .filter(event_remote_registration::Column::Variant.eq(&variant))
                        .filter(
                            event_remote_registration::Column::EventVersion
                                .is_not_in(protected.clone()),
                        )
                        .order_by_asc(event_remote_registration::Column::Id)
                        .limit(32)
                        .all(txn)
                        .await?;
                    let mut bytes = 8192;
                    let mut ids = Vec::new();
                    for row in rows {
                        let size = encoded_size(&row)?;
                        if !ids.is_empty() && bytes + size > DEFAULT_WRITE_BYTES {
                            break;
                        }
                        bytes += size;
                        ids.push(row.id);
                    }
                    if !ids.is_empty() {
                        return Ok(event_remote_registration::Entity::delete_many()
                            .filter(event_remote_registration::Column::Id.is_in(ids))
                            .exec(txn)
                            .await?
                            .rows_affected);
                    }
                    // All obsolete registrations are gone, so deleting their auth rows cannot
                    // expand the transaction through ON DELETE SET NULL.
                    let rows = event_remote_auth::Entity::find()
                        .filter(event_remote_auth::Column::AppId.eq(&app_id))
                        .filter(event_remote_auth::Column::EventId.eq(&event_id))
                        .filter(event_remote_auth::Column::Variant.eq(&variant))
                        .filter(event_remote_auth::Column::EventVersion.is_not_in(protected))
                        .order_by_asc(event_remote_auth::Column::Id)
                        .limit(32)
                        .all(txn)
                        .await?;
                    let mut bytes = 8192;
                    let mut ids = Vec::new();
                    for row in rows {
                        let size = encoded_size(&row)?;
                        if !ids.is_empty() && bytes + size > DEFAULT_WRITE_BYTES {
                            break;
                        }
                        bytes += size;
                        ids.push(row.id);
                    }
                    if ids.is_empty() {
                        return Ok(0);
                    }
                    Ok(event_remote_auth::Entity::delete_many()
                        .filter(event_remote_auth::Column::Id.is_in(ids))
                        .exec(txn)
                        .await?
                        .rows_affected)
                })
            })
            .await?;
        if removed == 0 {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Set;

    fn registration(index: usize, prefix: &str) -> event_remote_registration::ActiveModel {
        let now = chrono::DateTime::from_timestamp_millis(1_700_000_000_000)
            .unwrap()
            .fixed_offset();
        event_remote_registration::ActiveModel {
            id: Set(format!("{prefix}-{index}")),
            app_id: Set("app".into()),
            event_id: Set("event".into()),
            event_version: Set("1.0.0".into()),
            variant: Set("stable".into()),
            kind: Set("rest_fn".into()),
            method: Set(Some("GET".into())),
            path: Set(format!("/route/{index}")),
            node_id: Set(Some("handler".into())),
            schema_json: Set(None),
            extras_json: Set(None),
            auth_id: Set(None),
            created_at: Set(now),
        }
    }

    #[test]
    fn rerunning_sixteen_hundred_routes_reuses_rows_instead_of_doubling_writes() {
        let old = (0..1600)
            .map(|i| registration(i, "old").try_into_model().unwrap())
            .collect();
        let prepared = PreparedRegistrations {
            registrations: (0..1600)
                .map(|i| {
                    let mut row = registration(i, "new");
                    row.node_id = Set(Some("new-handler".into()));
                    row
                })
                .collect(),
            auths: Vec::new(),
        };
        let replacement = reconcile(&prepared, old, Vec::new()).unwrap();
        assert_eq!(replacement.registrations.len(), 1600);
        assert!(replacement.remove_registrations.is_empty());
        assert_eq!(
            replacement.registrations[0].id.clone().take().unwrap(),
            "old-0"
        );
        replacement.validate(DbDialect::Dsql).unwrap();
    }

    #[test]
    fn unchanged_routes_do_not_consume_the_write_budget() {
        let old = (0..1600)
            .map(|i| registration(i, "old").try_into_model().unwrap())
            .collect();
        let prepared = PreparedRegistrations {
            registrations: (0..1600).map(|i| registration(i, "new")).collect(),
            auths: Vec::new(),
        };
        let replacement = reconcile(&prepared, old, Vec::new()).unwrap();
        assert!(replacement.registrations.is_empty());
        assert!(replacement.remove_registrations.is_empty());
        replacement.validate(DbDialect::Dsql).unwrap();
    }

    #[test]
    fn unrelated_large_replacement_is_rejected_before_mutations() {
        let old = (0..1600)
            .map(|i| registration(i, "old").try_into_model().unwrap())
            .collect();
        let prepared = PreparedRegistrations {
            registrations: (1600..3200).map(|i| registration(i, "new")).collect(),
            auths: Vec::new(),
        };
        let replacement = reconcile(&prepared, old, Vec::new()).unwrap();
        assert_eq!(replacement.rows(), 3204);
        assert!(matches!(
            replacement.validate(DbDialect::Dsql),
            Err(PersistError::Budget(_))
        ));
        replacement.validate(DbDialect::Postgres).unwrap();
    }

    #[test]
    fn route_references_follow_reused_auth_ids() {
        let now = chrono::Utc::now().fixed_offset();
        let old_auth = event_remote_auth::ActiveModel {
            id: Set("old-auth".into()),
            app_id: Set("app".into()),
            event_id: Set("event".into()),
            event_version: Set("1.0.0".into()),
            variant: Set("stable".into()),
            node_id: Set("server".into()),
            kind: Set("rest".into()),
            config_json: Set(serde_json::json!({"type":"bearer"})),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let mut new_auth = old_auth.clone();
        new_auth.id = Set("new-auth".into());
        let mut row = registration(0, "new");
        row.auth_id = Set(Some("new-auth".into()));
        let prepared = PreparedRegistrations {
            registrations: vec![row],
            auths: vec![new_auth],
        };
        let replacement = reconcile(
            &prepared,
            Vec::new(),
            vec![old_auth.try_into_model().unwrap()],
        )
        .unwrap();
        assert_eq!(
            replacement.registrations[0].auth_id.clone().take().unwrap(),
            Some("old-auth".into())
        );
        assert!(replacement.remove_auths.is_empty());
    }

    #[test]
    fn oversized_json_is_refused_even_when_total_transaction_bytes_fit() {
        let mut row = registration(0, "new");
        row.extras_json = Set(Some(serde_json::json!({"value": "x".repeat(1024 * 1024)})));
        let prepared = PreparedRegistrations {
            registrations: vec![row],
            auths: Vec::new(),
        };
        let replacement = reconcile(&prepared, Vec::new(), Vec::new()).unwrap();
        assert!(matches!(
            replacement.validate(DbDialect::Dsql),
            Err(PersistError::Budget(_))
        ));
    }

    #[test]
    fn combined_route_index_key_is_bounded_in_bytes() {
        let mut row = registration(0, "new");
        row.path = Set("é".repeat(450));
        let prepared = PreparedRegistrations {
            registrations: vec![row],
            auths: Vec::new(),
        };
        let replacement = reconcile(&prepared, Vec::new(), Vec::new()).unwrap();
        assert!(matches!(
            replacement.validate(DbDialect::Dsql),
            Err(PersistError::Budget(_))
        ));
        replacement.validate(DbDialect::Postgres).unwrap();
    }
}
