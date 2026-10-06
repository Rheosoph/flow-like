use crate::compilation::jwt;
use crate::entity::sea_orm_active_enums::{
    WasmCompilationStatus, WasmPackageStatus, WasmPackageVisibility,
};
use crate::entity::{wasm_package, wasm_package_version};
use crate::routes::registry::server::{
    permissions_with_node_capabilities, with_current_wasmtime_version,
};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use flow_like_types::dispatch::{CompilationResult, CompilationStatus};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::{NotSet, Set},
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait,
    UpdateMany,
};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize)]
pub struct CallbackResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn handle_compilation_callback(
    State(db): State<Arc<DatabaseConnection>>,
    headers: axum::http::HeaderMap,
    Json(result): Json<CompilationResult>,
) -> Result<Json<CallbackResponse>, (StatusCode, Json<CallbackResponse>)> {
    let token = crate::middleware::jwt::viewer_authorization(&headers)
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(CallbackResponse {
                    ok: false,
                    error: Some("Missing or invalid Authorization header".to_string()),
                }),
            )
        })?;

    let claims = jwt::verify(token).map_err(|e| {
        (
            StatusCode::UNAUTHORIZED,
            Json(CallbackResponse {
                ok: false,
                error: Some(format!("JWT verification failed: {e}")),
            }),
        )
    })?;

    if claims.job_id != result.job_id
        || claims.package_id != result.package_id
        || claims.version != result.version
        || claims
            .artifact_generation
            .as_deref()
            .is_some_and(|generation| generation != claims.job_id)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(CallbackResponse {
                ok: false,
                error: Some("JWT claims do not match result payload".to_string()),
            }),
        ));
    }

    let version_record = wasm_package_version::Entity::find()
        .filter(wasm_package_version::Column::PackageId.eq(&result.package_id))
        .filter(wasm_package_version::Column::Version.eq(&result.version))
        .find_also_related(wasm_package::Entity)
        .one(db.as_ref())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(CallbackResponse {
                    ok: false,
                    error: Some(format!("DB error: {e}")),
                }),
            )
        })?;

    let (version_record, package) = version_record.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(CallbackResponse {
                ok: false,
                error: Some("Package version not found".to_string()),
            }),
        )
    })?;

    let nodes = result.nodes;

    tracing::info!(
        job_id = %result.job_id,
        has_nodes = nodes.is_some(),
        platforms_count = result.compiled_platforms.len(),
        "Compilation callback received"
    );

    let (compilation_status, platforms, error) = match result.status {
        CompilationStatus::Compiled => (
            WasmCompilationStatus::Compiled,
            Some(result.compiled_platforms),
            None,
        ),
        CompilationStatus::Failed => (
            WasmCompilationStatus::LocalOnly,
            Some(Vec::new()),
            Some(result.error.unwrap_or_else(|| "Compilation failed".into())),
        ),
    };

    let compiled_ok = compilation_status == WasmCompilationStatus::Compiled;
    let supported_wasmtime_versions = version_record.supported_wasmtime_versions.clone();

    let mut update = wasm_package_version::ActiveModel {
        id: Set(version_record.id.clone()),
        compilation_status: Set(compilation_status),
        compiled_platforms: Set(platforms.map(Into::into)),
        compilation_error: Set(replacement_error(
            error,
            version_record.compilation_error.as_deref(),
            chrono::Utc::now().timestamp(),
        )),
        compiled_artifact_generation: Set(if compiled_ok {
            claims.artifact_generation.clone()
        } else {
            None
        }),
        ..Default::default()
    };

    if compiled_ok && let Some(ref nodes) = nodes {
        update.nodes = Set(nodes.clone());
    }

    if compiled_ok {
        update.supported_wasmtime_versions = Set(Some(
            supported_versions_for_publication(
                supported_wasmtime_versions.map(Into::into),
                claims.artifact_generation.as_deref(),
            )
            .into(),
        ));
    }

    // Auto-approve private packages on successful compilation
    let auto_approve = needs_private_approval(compiled_ok, &version_record.status)
        && package
            .as_ref()
            .is_some_and(|p| p.visibility == WasmPackageVisibility::Private);

    if auto_approve {
        let now = chrono::Utc::now().fixed_offset();
        update.status = Set(WasmPackageStatus::Active);
        update.approved_at = Set(Some(now));
    }

    // Publish the generation and its package metadata together. A failed
    // metadata write rolls back the generation so a callback retry can repair it.
    let transaction = db.begin().await.map_err(|e| {
        callback_write_error(format!("Failed to begin compilation publication: {e}"))
    })?;
    let published = publication_update(
        update,
        &version_record.id,
        claims.artifact_generation.as_deref(),
    )
    .exec(&transaction)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(CallbackResponse {
                ok: false,
                error: Some(format!("Failed to update version: {e}")),
            }),
        )
    })?;
    if published.rows_affected == 0 {
        transaction.commit().await.map_err(|e| {
            callback_write_error(format!(
                "Failed to finish duplicate compilation callback: {e}"
            ))
        })?;
        tracing::info!(job_id = %result.job_id, "Ignoring callback for an already published generation");
        return Ok(Json(CallbackResponse {
            ok: true,
            error: None,
        }));
    }

    // The store lists capabilities from the compiled nodes, so they follow the
    // node definitions onto the package row.
    let derived_permissions = package
        .as_ref()
        .zip(nodes.as_ref())
        .filter(|_| compiled_ok)
        .and_then(|(pkg, nodes)| permissions_with_node_capabilities(&pkg.permissions, nodes));

    // Promote version data to parent package for private auto-approved packages
    if auto_approve && let Some(pkg) = &package {
        let now = chrono::Utc::now().fixed_offset();
        let mut pkg_update = wasm_package::ActiveModel {
            id: Set(pkg.id.clone()),
            version: Set(result.version.clone()),
            wasm_path: Set(version_record.wasm_path.clone()),
            wasm_hash: Set(version_record.wasm_hash.clone()),
            wasm_size: Set(version_record.wasm_size),
            widgets: Set(version_record.widgets.clone()),
            widget_bundle_hash: Set(version_record.widget_bundle_hash.clone()),
            widget_bundle_size: Set(version_record.widget_bundle_size),
            updated_at: Set(now),
            ..Default::default()
        };
        if let Some(ref nodes) = nodes {
            pkg_update.nodes = Set(nodes.clone());
        }
        if let Some(ref permissions) = derived_permissions {
            pkg_update.permissions = Set(permissions.clone());
        }
        pkg_update.update(&transaction).await.map_err(|e| {
            callback_write_error(format!("Failed to promote version to parent package: {e}"))
        })?;
    } else if compiled_ok
        && let (Some(pkg), Some(nodes)) = (&package, &nodes)
        && pkg.version == result.version
    {
        // Recheck the current version in the update so a concurrent approval
        // cannot receive node definitions from an older version.
        let mut update = wasm_package::Entity::update_many()
            .col_expr(wasm_package::Column::Nodes, Expr::value(nodes.clone()));
        if let Some(ref permissions) = derived_permissions {
            update = update.col_expr(
                wasm_package::Column::Permissions,
                Expr::value(permissions.clone()),
            );
        }
        update
            .filter(wasm_package::Column::Id.eq(&result.package_id))
            .filter(wasm_package::Column::Version.eq(&result.version))
            .exec(&transaction)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(CallbackResponse {
                        ok: false,
                        error: Some(format!("Failed to update current package nodes: {e}")),
                    }),
                )
            })?;
    }
    transaction.commit().await.map_err(|e| {
        callback_write_error(format!("Failed to commit compilation publication: {e}"))
    })?;

    tracing::info!(
        job_id = %result.job_id,
        package_id = %result.package_id,
        version = %result.version,
        auto_approved = auto_approve,
        "Compilation callback processed"
    );

    Ok(Json(CallbackResponse {
        ok: true,
        error: None,
    }))
}

fn callback_write_error(message: String) -> (StatusCode, Json<CallbackResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(CallbackResponse {
            ok: false,
            error: Some(message),
        }),
    )
}

fn supported_versions_for_publication(
    existing: Option<Vec<String>>,
    generation: Option<&str>,
) -> Vec<String> {
    // A generation contains only this job's targets. Older engine artifacts
    // remain under their previous paths and are not part of this publication.
    with_current_wasmtime_version(if generation.is_some() { None } else { existing })
}

fn replacement_error(error: Option<String>, previous: Option<&str>, now: i64) -> Option<String> {
    let marker = crate::execution::wasm_resolve::UPGRADE_MARKER;
    error.map(|error| {
        if previous.is_some_and(|previous| previous.starts_with(marker)) {
            format!("{marker}{now}:{error}")
        } else {
            error
        }
    })
}

fn needs_private_approval(compiled_ok: bool, status: &WasmPackageStatus) -> bool {
    // Rebuilding an approved historical pin must not promote it over the
    // package's current version. The guarded metadata update handles rebuilds.
    compiled_ok && *status == WasmPackageStatus::PendingReview
}

fn publication_update(
    mut update: wasm_package_version::ActiveModel,
    version_id: &str,
    generation: Option<&str>,
) -> UpdateMany<wasm_package_version::Entity> {
    let failed = matches!(
        update.compilation_status,
        Set(WasmCompilationStatus::LocalOnly)
    );
    if failed {
        // A failed replacement must leave the last successful publication
        // addressable. Read these fields in the mutation, since another
        // callback may publish after this handler reads the version row.
        update.compiled_artifact_generation = NotSet;
        update.compilation_status = NotSet;
        update.compiled_platforms = NotSet;
        update.nodes = NotSet;
    }
    update.id = NotSet;
    let mut query = wasm_package_version::Entity::update_many()
        .set(update)
        .filter(wasm_package_version::Column::Id.eq(version_id));
    if failed {
        use wasm_package_version::Column;
        // Legacy publications have no generation id. Their nonempty artifact
        // list must remain usable if the portable replacement fails too.
        let has_publication = Condition::any()
            .add(Column::CompiledArtifactGeneration.is_not_null())
            .add(Column::CompiledPlatforms.ne(serde_json::json!([])));
        query = query
            .col_expr(
                Column::CompilationStatus,
                Column::CompilationStatus.save_as(Expr::expr(
                    Expr::case(
                        has_publication.clone(),
                        Expr::value(WasmCompilationStatus::Compiled),
                    )
                    .finally(Expr::value(WasmCompilationStatus::LocalOnly)),
                )),
            )
            .col_expr(
                Column::CompiledPlatforms,
                Column::CompiledPlatforms.save_as(Expr::expr(
                    Expr::case(has_publication, Expr::col(Column::CompiledPlatforms))
                        .finally(Expr::value(serde_json::json!([]))),
                )),
            );
    }
    if let Some(generation) = generation {
        // A retry may have read Pending before another callback published.
        // Check in the mutation so it cannot replace those nodes or fail that
        // generation after publication, including while a newer job is Pending.
        // Different jobs retain the existing callback arrival ordering.
        query = query.filter(
            Condition::any()
                .add(wasm_package_version::Column::CompiledArtifactGeneration.is_null())
                .add(wasm_package_version::Column::CompiledArtifactGeneration.ne(generation)),
        );
    }
    query
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use sea_orm::{DatabaseBackend, QueryTrait};

    #[test]
    fn generation_publication_replaces_engine_support_while_legacy_preserves_it() {
        let current = flow_like_wasm_schema::runtime::WASMTIME_MAJOR_VERSION.to_string();
        let previous = (current.parse::<u32>().unwrap() - 1).to_string();
        let existing = Some(vec![previous.clone(), current.clone()]);

        assert_eq!(
            supported_versions_for_publication(existing.clone(), Some("new-job")),
            vec![current.clone()],
        );
        assert_eq!(
            supported_versions_for_publication(existing, None),
            vec![previous, current.clone()],
        );
        assert_eq!(
            supported_versions_for_publication(None, Some("first-job")),
            vec![current],
        );
    }

    #[test]
    fn successful_and_failed_retries_guard_publication_in_the_database_write() {
        for status in [
            WasmCompilationStatus::Compiled,
            WasmCompilationStatus::LocalOnly,
        ] {
            let update = wasm_package_version::ActiveModel {
                compilation_status: Set(status),
                nodes: Set(serde_json::json!([{"name": "retry-node"}])),
                ..Default::default()
            };
            let sql = publication_update(update, "version-row", Some("published-job"))
                .build(DatabaseBackend::Postgres)
                .to_string();
            let (_, predicate) = sql.split_once(" WHERE ").unwrap();
            assert!(predicate.contains("\"id\" = 'version-row'"));
            assert!(predicate.contains("\"compiledArtifactGeneration\" IS NULL"));
            assert!(predicate.contains(" OR "));
            assert!(predicate.contains("\"compiledArtifactGeneration\" <> 'published-job'"));
        }
    }

    #[test]
    fn legacy_callback_keeps_its_original_publication_behavior() {
        let update = wasm_package_version::ActiveModel {
            compilation_status: Set(WasmCompilationStatus::Compiled),
            ..Default::default()
        };
        let sql = publication_update(update, "version-row", None)
            .build(DatabaseBackend::Postgres)
            .to_string();
        let (_, predicate) = sql.split_once(" WHERE ").unwrap();
        assert!(predicate.contains("\"id\" = 'version-row'"));
        assert!(!predicate.contains("compiledArtifactGeneration"));
    }

    #[test]
    fn failed_recompile_preserves_published_generation_and_nodes_atomically() {
        let update = wasm_package_version::ActiveModel {
            compilation_status: Set(WasmCompilationStatus::LocalOnly),
            compiled_artifact_generation: Set(None),
            compiled_platforms: Set(Some(Default::default())),
            compilation_error: Set(Some("replacement failed".into())),
            nodes: Set(serde_json::json!([{"name": "failed-job-node"}])),
            ..Default::default()
        };
        let sql = publication_update(update, "version-row", Some("new-job"))
            .build(DatabaseBackend::Postgres)
            .to_string();
        let (assignments, _) = sql.split_once(" WHERE ").unwrap();
        assert!(assignments.contains("\"compiledArtifactGeneration\" IS NOT NULL"));
        assert!(assignments.contains("\"compiledPlatforms\" <> '[]'"));
        assert!(assignments.contains("THEN 'COMPILED' ELSE 'LOCAL_ONLY' END"));
        assert!(assignments.contains("THEN \"compiledPlatforms\""));
        assert!(!assignments.contains("\"compiledArtifactGeneration\" ="));
        assert!(!assignments.contains("\"nodes\" ="));
        assert!(assignments.contains("replacement failed"));
    }

    #[test]
    fn failed_automatic_upgrade_retains_retry_backoff() {
        let previous = Some("artifact-upgrade:1000:claim");
        assert_eq!(replacement_error(None, previous, 2000), None);
        assert_eq!(
            replacement_error(Some("failed".into()), previous, 2000),
            Some("artifact-upgrade:2000:failed".into())
        );
        assert_eq!(
            replacement_error(Some("failed".into()), None, 2000),
            Some("failed".into())
        );
    }

    #[test]
    fn recompiling_an_approved_private_pin_does_not_promote_it_again() {
        assert!(needs_private_approval(
            true,
            &WasmPackageStatus::PendingReview
        ));
        for status in [
            WasmPackageStatus::Active,
            WasmPackageStatus::Deprecated,
            WasmPackageStatus::Disabled,
            WasmPackageStatus::Rejected,
        ] {
            assert!(!needs_private_approval(true, &status));
        }
        assert!(!needs_private_approval(
            false,
            &WasmPackageStatus::PendingReview
        ));
    }

    #[flow_like_types::tokio::test]
    async fn failed_upgrade_keeps_legacy_and_generation_publications_executable() {
        use sea_orm::{ConnectionTrait, Database, QuerySelect};
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        db.execute_unprepared(
            r#"CREATE TABLE public."WasmPackageVersion" (
            id TEXT PRIMARY KEY, "compilationStatus" TEXT, "compiledPlatforms" TEXT,
            "compiledArtifactGeneration" TEXT, "compilationError" TEXT
        )"#,
        )
        .await
        .unwrap();
        for (id, generation, platforms, expected) in [
            (
                "legacy",
                None,
                vec!["linux-x86_64-wt48".to_string()],
                WasmCompilationStatus::Compiled,
            ),
            (
                "generation",
                Some("old-job"),
                vec!["linux-x86_64-wt48".to_string()],
                WasmCompilationStatus::Compiled,
            ),
            (
                "first-publication",
                None,
                Vec::new(),
                WasmCompilationStatus::LocalOnly,
            ),
        ] {
            wasm_package_version::Entity::insert(wasm_package_version::ActiveModel {
                id: Set(id.into()),
                compilation_status: Set(WasmCompilationStatus::Pending),
                compiled_platforms: Set(Some(platforms.clone().into())),
                compiled_artifact_generation: Set(generation.map(str::to_owned)),
                ..Default::default()
            })
            .exec(&db)
            .await
            .unwrap();
            publication_update(
                wasm_package_version::ActiveModel {
                    compilation_status: Set(WasmCompilationStatus::LocalOnly),
                    compiled_platforms: Set(Some(Default::default())),
                    compiled_artifact_generation: Set(None),
                    compilation_error: Set(Some("replacement failed".into())),
                    ..Default::default()
                },
                id,
                Some("new-job"),
            )
            .exec(&db)
            .await
            .unwrap();
            let (status, published, published_generation) =
                wasm_package_version::Entity::find_by_id(id)
                    .select_only()
                    .column(wasm_package_version::Column::CompilationStatus)
                    .column(wasm_package_version::Column::CompiledPlatforms)
                    .column(wasm_package_version::Column::CompiledArtifactGeneration)
                    .into_tuple::<(
                        WasmCompilationStatus,
                        crate::entity::json_types::StringList,
                        Option<String>,
                    )>()
                    .one(&db)
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(status, expected);
            assert_eq!(published, platforms);
            assert_eq!(published_generation.as_deref(), generation);
        }
    }
}
