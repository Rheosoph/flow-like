pub mod get_comments;
pub mod remove_comment;
pub mod upsert_comment;

use axum::{Router, routing::get};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};

use crate::{
    entity::{sea_orm_active_enums::WasmPackageVisibility, wasm_package, wasm_package_user},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::wasm_package_permission::WasmPackagePermission,
    state::AppState,
};

/// Whether `user` may read and write the reviews of `package_id`. Reviews follow the package
/// itself, as its readme and metadata do: a private package opens only to the people it was
/// shared with.
pub(super) async fn ensure_reviewable<C: ConnectionTrait>(
    db: &C,
    user: &AppUser,
    package_id: &str,
) -> Result<(), ApiError> {
    let visibility = wasm_package::Entity::find_by_id(package_id)
        .select_only()
        .column(wasm_package::Column::Visibility)
        .into_tuple::<WasmPackageVisibility>()
        .one(db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if visibility != WasmPackageVisibility::Private {
        return Ok(());
    }
    let sub = user.sub()?;
    let shared = wasm_package_user::Entity::find()
        .select_only()
        .column(wasm_package_user::Column::Permission)
        .filter(wasm_package_user::Column::PackageId.eq(package_id))
        .filter(wasm_package_user::Column::UserId.eq(&sub))
        .into_tuple::<i64>()
        .one(db)
        .await?
        .is_some_and(|bits| !WasmPackagePermission::from_bits_truncate(bits).is_empty());
    if shared {
        Ok(())
    } else {
        Err(ApiError::FORBIDDEN)
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(get_comments::get_comments).put(upsert_comment::upsert_comment),
        )
        .route(
            "/{comment_id}",
            axum::routing::delete(remove_comment::remove_comment),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::OpenIDUser;
    use axum::http::StatusCode;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};

    async fn packages() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        for statement in [
            r#"CREATE TABLE public."WasmPackage" (id TEXT PRIMARY KEY, visibility TEXT NOT NULL)"#,
            r#"CREATE TABLE public."WasmPackageUser" (id TEXT PRIMARY KEY, "packageId" TEXT NOT NULL, "userId" TEXT NOT NULL, permission BIGINT NOT NULL)"#,
            r#"INSERT INTO public."WasmPackage" VALUES ('com.acme.private', 'PRIVATE'), ('com.acme.public', 'PUBLIC'), ('com.acme.gated', 'PUBLIC_REQUEST_ACCESS')"#,
            r#"INSERT INTO public."WasmPackageUser" VALUES ('grant-1', 'com.acme.private', 'collaborator', 4), ('grant-2', 'com.acme.private', 'revoked', 0)"#,
        ] {
            db.execute_unprepared(statement).await.unwrap();
        }
        db
    }

    fn person(sub: &str) -> AppUser {
        AppUser::OpenID(OpenIDUser {
            sub: sub.to_string(),
            access_token: String::new(),
        })
    }

    async fn status(db: &DatabaseConnection, user: &AppUser, package_id: &str) -> StatusCode {
        match ensure_reviewable(db, user, package_id).await {
            Ok(()) => StatusCode::OK,
            Err(error) => error.status(),
        }
    }

    #[tokio::test]
    async fn reviews_of_a_private_package_open_only_to_the_people_it_is_shared_with() {
        let db = packages().await;
        let private = "com.acme.private";
        assert_eq!(
            status(&db, &person("collaborator"), private).await,
            StatusCode::OK
        );
        for outsider in ["stranger", "revoked"] {
            assert_eq!(
                status(&db, &person(outsider), private).await,
                StatusCode::FORBIDDEN,
                "{outsider}"
            );
        }
        assert_eq!(
            status(&db, &AppUser::Unauthorized, private).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn reviews_of_other_packages_stay_open_and_a_missing_package_is_not_found() {
        let db = packages().await;
        for package_id in ["com.acme.public", "com.acme.gated"] {
            for reader in [person("stranger"), AppUser::Unauthorized] {
                assert_eq!(status(&db, &reader, package_id).await, StatusCode::OK);
            }
        }
        assert_eq!(
            status(&db, &person("stranger"), "com.acme.missing").await,
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn both_review_routes_ask_before_they_touch_a_comment() {
        for (name, source) in [
            ("get_comments", include_str!("get_comments.rs")),
            ("upsert_comment", include_str!("upsert_comment.rs")),
        ] {
            let gate = source
                .find("super::ensure_reviewable(&state.db, &user, &package_id).await?")
                .unwrap_or_else(|| panic!("{name} asks ensure_reviewable"));
            let first_comment_query = source
                .find("comment::Entity::find()")
                .unwrap_or_else(|| panic!("{name} reads comments"));
            assert!(gate < first_comment_query, "{name}");
        }
    }
}
