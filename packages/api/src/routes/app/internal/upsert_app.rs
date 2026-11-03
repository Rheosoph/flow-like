use std::{sync::Arc, time::SystemTime};

use crate::{
    audit_branch,
    credentials::CredentialsAccess,
    deletion::DeletionRoot,
    entity::{
        app, membership, meta, role,
        sea_orm_active_enums::{Status, Visibility},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::LanguageParams,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, FixedOffset};
use flow_like::{app::App, bit::Metadata};
use flow_like_types::{anyhow, create_id};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::{NotSet, Set},
    ColumnTrait, DatabaseTransaction, EntityTrait, IntoActiveModel, QueryFilter,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// An app being created: its rows go in first ([`insert_app_rows`]), its stored manifest after
/// they commit ([`materialize_new_app`]).
#[derive(Clone, Debug)]
pub(crate) struct NewApp {
    pub id: String,
    /// The creator: the only member, with the Owner role.
    pub owner_id: String,
    pub metadata: Metadata,
    pub language: String,
    pub bits: Vec<String>,
    /// Set when the app is this bot's workspace.
    pub bot_id: Option<String>,
}

fn role_row(
    app_id: &str,
    name: &str,
    permissions: RolePermissions,
    now: DateTime<FixedOffset>,
) -> role::ActiveModel {
    role::ActiveModel {
        id: Set(create_id()),
        name: Set(name.to_string()),
        description: Set(Some(format!("{name} role"))),
        permissions: Set(permissions.bits()),
        created_at: Set(now),
        updated_at: Set(now),
        app_id: Set(Some(app_id.to_string())),
        attributes: NotSet,
    }
}

/// The app (Private, Active), its metadata, the Owner, Admin and User roles and the owner's
/// membership. Call inside the transaction that admits the app.
pub(crate) async fn insert_app_rows(
    txn: &DatabaseTransaction,
    new_app: &NewApp,
    now: DateTime<FixedOffset>,
) -> Result<app::Model, ApiError> {
    let app = app::ActiveModel {
        id: Set(new_app.id.clone()),
        status: Set(Status::Active),
        created_at: Set(now),
        updated_at: Set(now),
        visibility: Set(Visibility::Private),
        bits: Set(Some(new_app.bits.clone().into())),
        bot_id: Set(new_app.bot_id.clone()),
        ..Default::default()
    }
    .insert(txn)
    .await?;

    let metadata = new_app.metadata.clone();
    meta::ActiveModel {
        id: Set(create_id()),
        app_id: Set(Some(app.id.clone())),
        name: Set(metadata.name),
        description: Set(Some(metadata.description)),
        long_description: Set(metadata.long_description),
        tags: Set(Some(metadata.tags.into())),
        lang: Set(new_app.language.clone()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(txn)
    .await?;

    let owner_role = role_row(&app.id, "Owner", RolePermissions::Owner, now)
        .insert(txn)
        .await?;
    role_row(&app.id, "Admin", RolePermissions::Admin, now)
        .insert(txn)
        .await?;
    let user_permissions = RolePermissions::ReadTemplates
        | RolePermissions::ExecuteEvents
        | RolePermissions::ListEvents;
    let user_role = role_row(&app.id, "User", user_permissions, now)
        .insert(txn)
        .await?;

    let mut app = app.into_active_model();
    app.owner_role_id = Set(Some(owner_role.id.clone()));
    app.default_role_id = Set(Some(user_role.id));
    let app = app.update(txn).await?;

    membership::ActiveModel {
        id: Set(create_id()),
        user_id: Set(new_app.owner_id.clone()),
        app_id: Set(app.id.clone()),
        role_id: Set(owner_role.id),
        joined_via: NotSet,
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(txn)
    .await?;
    Ok(app)
}

/// Saves the new app's manifest and metadata with credentials scoped to it. When that fails the
/// rows [`insert_app_rows`] committed are deleted again, so no app stays half made.
pub(crate) async fn materialize_new_app(
    state: &AppState,
    new_app: &NewApp,
) -> Result<App, ApiError> {
    let materialized: Result<App, ApiError> = async {
        let credentials = state
            .scoped_credentials(&new_app.owner_id, &new_app.id, CredentialsAccess::EditApp)
            .await?;
        let flow_like_state = Arc::new(credentials.to_state(state.clone()).await?);
        let mut drive_app = App::new(
            Some(new_app.id.clone()),
            new_app.metadata.clone(),
            new_app.bits.clone(),
            flow_like_state,
        )
        .await?;
        drive_app.bot_id = new_app.bot_id.clone();
        drive_app.save().await?;
        Ok(drive_app)
    }
    .await;
    if materialized.is_err()
        && let Err(cleanup) = crate::deletion::delete_now(
            state,
            DeletionRoot::App,
            &new_app.id,
            Some(&new_app.owner_id),
            (),
        )
        .await
    {
        tracing::error!(new_id = %new_app.id, %cleanup, "Project materialization failed and deletion needs retry");
    }
    materialized
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AppUpsertBody {
    #[schema(value_type = Option<Object>)]
    pub app: Option<App>,
    #[schema(value_type = Option<Object>)]
    pub meta: Option<Metadata>,
    pub bits: Option<Vec<String>>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}",
    tag = "apps",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("language" = Option<String>, Query, description = "Language code (default: en)")
    ),
    request_body = AppUpsertBody,
    responses(
        (status = 200, description = "Application created or updated", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 402, description = "Plan limit reached — upgrading the subscription unlocks more projects"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}", skip(state, user, app_body, query))]
pub async fn upsert_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<LanguageParams>,
    Json(app_body): Json<AppUpsertBody>,
) -> Result<Json<App>, ApiError> {
    let sub = user.sub()?;
    let (plan, tier) = crate::quota::payer_plan(&state, &sub).await?;

    let app = app::Entity::find()
        .filter(app::Column::Id.eq(&app_id))
        .one(&state.db)
        .await?;

    let language = query.language.clone().unwrap_or_else(|| "en".to_string());

    let now = chrono::Utc::now().fixed_offset();

    if let (Some(app), Some(app_updates)) = (&app, &app_body.app) {
        let sub = user.app_permission(&app_id, &state).await?;
        if !sub.has_permission(RolePermissions::Owner) {
            tracing::warn!(
                "User {} attempted to update app {} without sufficient permissions",
                sub.sub()?,
                app_id
            );
            return Err(ApiError::FORBIDDEN);
        }

        let new_price = i64::from(app_updates.price.unwrap_or(0));
        if new_price != app.price && new_price > 0 {
            crate::payments::domain::listing_price(
                new_price,
                &app.visibility,
                state.platform_config.payments.marketplace_min_amount,
                state.platform_config.payments.max_payment_amount,
            )?;
            if state.platform_config.payments.marketplace_enabled {
                crate::payments::accounts::require_can_sell(&state, &user.sub()?).await?;
            }
        }

        {
            let mut bucket_app = state
                .scoped_app(
                    &sub.sub()?,
                    &app_id,
                    &state,
                    crate::credentials::CredentialsAccess::EditApp,
                )
                .await?;

            bucket_app.changelog = app_updates.changelog.clone();
            bucket_app.primary_category = app_updates.primary_category.clone();
            bucket_app.secondary_category = app_updates.secondary_category.clone();
            bucket_app.app_type = app_updates.app_type.clone();
            bucket_app.price = app_updates.price;
            bucket_app.updated_at = SystemTime::now();
            bucket_app.status = app_updates.status.clone();
            bucket_app.version = app_updates.version.clone();
            bucket_app.execution_mode = app_updates.execution_mode.clone();
            bucket_app.save().await?;
        }
        let app_updates = app::Model::from(app_updates.clone());
        let mut app: app::ActiveModel = app.clone().into();
        app.status = sea_orm::ActiveValue::Set(app_updates.status);
        app.changelog = sea_orm::ActiveValue::Set(app_updates.changelog);

        app.primary_category = sea_orm::ActiveValue::Set(app_updates.primary_category);
        app.secondary_category = sea_orm::ActiveValue::Set(app_updates.secondary_category);
        app.app_type = sea_orm::ActiveValue::Set(app_updates.app_type);
        app.price = sea_orm::ActiveValue::Set(app_updates.price);
        app.version = sea_orm::ActiveValue::Set(app_updates.version);
        app.execution_mode = sea_orm::ActiveValue::Set(app_updates.execution_mode);
        app.updated_at = sea_orm::ActiveValue::Set(now);
        let actor = user.sub()?;
        let expected_price = app_updates.price;
        let app: app::Model = state
            .transaction(|txn| {
                let app = app.clone();
                let app_id = app_id.clone();
                let actor = actor.clone();
                let minimum = state.platform_config.payments.marketplace_min_amount;
                let maximum = state.platform_config.payments.max_payment_amount;
                Box::pin(async move {
                    crate::db::coordination::coordinate(txn, "payments-app", &[&app_id]).await?;
                    crate::payments::ensure_app_owner(txn, &app_id, &actor).await?;
                    let current = app::Entity::find_by_id(&app_id)
                        .one(txn)
                        .await?
                        .ok_or(ApiError::NOT_FOUND)?;
                    if current.price != expected_price && expected_price > 0 {
                        crate::payments::domain::listing_price(
                            expected_price,
                            &current.visibility,
                            minimum,
                            maximum,
                        )?;
                    }
                    Ok::<app::Model, ApiError>(app.save(txn).await?.try_into()?)
                })
            })
            .await?;
        audit_branch!(state, user, app_id, "app.update", "App", app_id);
        return Ok(Json(App::from(app)));
    }

    // Somehow the user sent an app body without an app, which is not allowed for existing apps.
    if app.is_some() {
        tracing::warn!(
            "User {} attempted to update app {} without providing an app body",
            sub,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    let Some(metadata) = app_body.meta else {
        tracing::warn!(
            "User {} attempted to create app {} without providing metadata",
            sub,
            app_id
        );
        return Err(ApiError::internal_error(anyhow!(
            "Meta is required for new apps"
        )));
    };

    crate::capacity::check_storage_write(&state, "", &sub, 0).await?;
    let new_app = NewApp {
        id: create_id(),
        owner_id: sub,
        metadata,
        language,
        bits: app_body.bits.clone().unwrap_or_default(),
        bot_id: None,
    };
    state
        .transaction(|txn| {
            let new_app = new_app.clone();
            let plan = plan.clone();
            Box::pin(async move {
                crate::capacity::admit_project(
                    txn,
                    &new_app.owner_id,
                    &new_app.id,
                    &Visibility::Private,
                    None,
                    &plan,
                    i64::from(tier.max_non_visible_projects),
                )
                .await?;
                insert_app_rows(txn, &new_app, now).await?;
                Ok::<_, ApiError>(())
            })
        })
        .await?;
    let drive_app = materialize_new_app(&state, &new_app).await?;

    audit_branch!(state, user, drive_app.id, "app.create", "App", drive_app.id);
    Ok(Json(drive_app))
}
