use crate::{
    entity::{home_default, template_profile},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::{Value, create_id};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, EntityTrait, QueryFilter,
    sea_query::{Expr, OnConflict},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize)]
pub struct HomeDefaultsQuery {
    default_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HomeDefaults {
    main: Option<home_default::Model>,
    profile: Option<home_default::Model>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SaveHomeDefault {
    #[serde(deserialize_with = "deserialize_layout")]
    layout: Option<Value>,
    expected_revision: Option<String>,
}

fn deserialize_layout<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Option::<Value>::deserialize(deserializer)
}

fn validate_id(id: &str) -> Result<(), ApiError> {
    if id.is_empty()
        || id.len() > 200
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(ApiError::bad_request("Invalid home default ID"));
    }
    Ok(())
}

fn check_revision_result(affected: u64) -> Result<(), ApiError> {
    if affected != 1 {
        return Err(ApiError::conflict(
            "The home default changed. Reload it before publishing.",
        ));
    }
    Ok(())
}

pub async fn get_home_defaults(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<HomeDefaultsQuery>,
) -> Result<Json<HomeDefaults>, ApiError> {
    if !state.platform_config.features.unauthorized_read {
        user.sub()?;
    }
    let mut ids = vec!["main".to_string()];
    if let Some(id) = &query.default_id {
        validate_id(id)?;
        if id != "main" {
            ids.push(id.clone());
        }
    }
    let defaults = home_default::Entity::find()
        .filter(home_default::Column::Id.is_in(ids))
        .all(&state.db)
        .await?;
    let main = defaults.iter().find(|record| record.id == "main").cloned();
    let profile = defaults
        .into_iter()
        .find(|record| record.id != "main" && Some(&record.id) == query.default_id.as_ref());
    Ok(Json(HomeDefaults { main, profile }))
}

pub async fn save_home_default(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(body): Json<SaveHomeDefault>,
) -> Result<Json<Option<home_default::Model>>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteLandingPage)
        .await?;
    validate_id(&id)?;
    if let Some(revision) = &body.expected_revision {
        validate_id(revision)?;
    }
    if let Some(layout) = &body.layout {
        flow_like::profile::validate_home_layout(layout).map_err(ApiError::bad_request)?;
    }

    state
        .transaction(|txn| {
            let id = id.clone();
            let body = body.clone();
            Box::pin(async move {
                crate::db::coordination::coordinate(txn, "profile-template", &[&id]).await?;
                if body.layout.is_some()
                    && id != "main"
                    && template_profile::Entity::find_by_id(&id)
                        .one(txn)
                        .await?
                        .is_none()
                {
                    return Err(ApiError::not_found("Profile template not found"));
                }

                let Some(layout) = body.layout else {
                    if let Some(revision) = body.expected_revision {
                        let result = home_default::Entity::delete_many()
                            .filter(home_default::Column::Id.eq(&id))
                            .filter(home_default::Column::Revision.eq(revision))
                            .exec(txn)
                            .await?;
                        check_revision_result(result.rows_affected)?;
                    } else if home_default::Entity::find_by_id(&id)
                        .one(txn)
                        .await?
                        .is_some()
                    {
                        return Err(ApiError::conflict(
                            "Reload the home default before removing it.",
                        ));
                    }
                    return Ok(Json(None));
                };

                let revision = create_id();
                if let Some(expected) = body.expected_revision {
                    // The comparison is part of the write, so another publish cannot slip
                    // between a freshness check and an update.
                    let result = home_default::Entity::update_many()
                        .col_expr(home_default::Column::Layout, Expr::value(layout.clone()))
                        .col_expr(
                            home_default::Column::Revision,
                            Expr::value(revision.clone()),
                        )
                        .filter(home_default::Column::Id.eq(&id))
                        .filter(home_default::Column::Revision.eq(expected))
                        .exec(txn)
                        .await?;
                    check_revision_result(result.rows_affected)?;
                } else {
                    let insert = home_default::Entity::insert(home_default::ActiveModel {
                        id: Set(id.clone()),
                        layout: Set(layout.clone()),
                        revision: Set(revision.clone()),
                    })
                    .on_conflict(
                        OnConflict::column(home_default::Column::Id)
                            .do_nothing()
                            .to_owned(),
                    )
                    .exec(txn)
                    .await;
                    match insert {
                        Ok(_) => {}
                        Err(sea_orm::DbErr::RecordNotInserted) => {
                            return Err(ApiError::conflict(
                                "A home default already exists. Reload it before publishing.",
                            ));
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Ok(Json(Some(home_default::Model {
                    id,
                    layout,
                    revision,
                })))
            })
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_or_missing_revision_is_a_conflict() {
        assert_eq!(
            check_revision_result(0).unwrap_err().status(),
            axum::http::StatusCode::CONFLICT
        );
        assert!(check_revision_result(1).is_ok());
    }

    #[test]
    fn invalid_default_identifiers_are_rejected() {
        for id in ["", "../main", "template/one", "a?b"] {
            assert!(validate_id(id).is_err());
        }
        assert!(validate_id("main").is_ok());
        assert!(validate_id("profile-template_1").is_ok());
    }

    #[test]
    fn removing_a_default_requires_an_explicit_null_layout() {
        assert!(serde_json::from_str::<SaveHomeDefault>(r#"{}"#).is_err());
        let reset: SaveHomeDefault =
            serde_json::from_str(r#"{"layout":null,"expected_revision":"revision"}"#).unwrap();
        assert!(reset.layout.is_none());
        assert_eq!(reset.expected_revision.as_deref(), Some("revision"));
    }
}
