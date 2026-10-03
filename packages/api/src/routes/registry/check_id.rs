//! Package ID availability check endpoint

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckIdRequest {
    pub id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CheckIdResponse {
    pub available: bool,
    pub owned_by_caller: bool,
}

/// POST /registry/check-id
/// Check whether a package ID is available or already owned by the caller.
#[utoipa::path(
    post,
    path = "/registry/check-id",
    tag = "registry",
    request_body = CheckIdRequest,
    responses(
        (status = 200, description = "ID availability result", body = CheckIdResponse),
        (status = 401, description = "Authentication required"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn check_id(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<CheckIdRequest>,
) -> Result<Json<CheckIdResponse>, ApiError> {
    let sub = user.sub()?;

    if request.id.is_empty() {
        return Err(ApiError::bad_request("id is required"));
    }

    use crate::entity::wasm_package;
    use sea_orm::EntityTrait;

    let existing = wasm_package::Entity::find_by_id(&request.id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    if existing.is_none() {
        return Ok(Json(CheckIdResponse {
            available: look_alike_id(&state.db, &request.id).await?.is_none(),
            owned_by_caller: false,
        }));
    }

    let perm = crate::check_wasm_access!(state, &sub, &request.id);
    let owned = perm
        .map(|p| {
            p.has_permission(
                crate::permission::wasm_package_permission::WasmPackagePermission::Maintainer,
            )
        })
        .unwrap_or(false);

    Ok(Json(CheckIdResponse {
        available: owned,
        owned_by_caller: owned,
    }))
}

/// How a package id names its directory where the file system ignores letter
/// case (macOS, Windows) and a trailing dot (Windows).
fn directory_name(id: &str) -> String {
    id.trim_end_matches('.').to_ascii_lowercase()
}

/// An existing package whose files `id` would share a directory with on the
/// devices that install both. A new id is refused while one exists: an
/// install of either would overwrite the other's node binary.
pub(super) async fn look_alike_id(
    db: &sea_orm::DatabaseConnection,
    id: &str,
) -> Result<Option<String>, ApiError> {
    use crate::entity::wasm_package;
    use sea_orm::sea_query::{Expr, ExprTrait, Func};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

    let name = directory_name(id);
    let candidates: Vec<String> = wasm_package::Entity::find()
        .select_only()
        .column(wasm_package::Column::Id)
        .filter(wasm_package::Column::Id.ne(id))
        .filter(
            Expr::expr(Func::lower(Expr::col(wasm_package::Column::Id))).like(format!(
                "{}%",
                name.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            )),
        )
        .into_tuple()
        .all(db)
        .await?;
    Ok(candidates
        .into_iter()
        .find(|existing| directory_name(existing) == name))
}

#[cfg(test)]
mod tests {
    use super::{directory_name, look_alike_id};

    #[test]
    fn ids_that_share_a_directory_fold_to_one_name() {
        assert_eq!(directory_name("Com.Acme.Maps"), "com.acme.maps");
        assert_eq!(directory_name("com.acme.maps.."), "com.acme.maps");
        assert_ne!(
            directory_name("com.acme.map"),
            directory_name("com.acme.maps")
        );
        assert_ne!(
            directory_name("com.acme_maps"),
            directory_name("com.acme.maps")
        );
    }

    #[tokio::test]
    #[ignore = "requires PACKAGE_LICENSE_TEST_DATABASE_URL pointing at a database with the full PostgreSQL schema"]
    async fn a_new_id_is_refused_next_to_one_it_shares_a_directory_with() {
        use sea_orm::ConnectionTrait;
        let url = std::env::var("PACKAGE_LICENSE_TEST_DATABASE_URL")
            .expect("PACKAGE_LICENSE_TEST_DATABASE_URL must point at a disposable database");
        let db = sea_orm::Database::connect(url).await.unwrap();
        db.execute_unprepared(
            r#"
INSERT INTO "WasmPackage" (id,name,description,version,"wasmPath","wasmHash","wasmSize",nodes,permissions,visibility,status,price,"updatedAt") VALUES
 ('la.Acme.Maps','Maps','','1.0.0','p','h',1,'[]','{}','PUBLIC','ACTIVE',0,now()),
 ('la.acme.maps-pro','Maps Pro','','1.0.0','p','h',1,'[]','{}','PUBLIC','ACTIVE',0,now()),
 ('laXacme.tiles','Tiles','','1.0.0','p','h',1,'[]','{}','PUBLIC','ACTIVE',0,now());
"#,
        )
        .await
        .expect("database must carry the full schema and none of this test's rows");

        for (id, taken) in [
            ("la.acme.maps", Some("la.Acme.Maps")),
            ("LA.ACME.MAPS.", Some("la.Acme.Maps")),
            ("la.Acme.Maps", None),
            ("la.acme.map", None),
            ("la.acme.maps-pr", None),
            ("la_acme.tiles", None),
        ] {
            assert_eq!(
                look_alike_id(&db, id).await.unwrap().as_deref(),
                taken,
                "{id}"
            );
        }
    }
}
