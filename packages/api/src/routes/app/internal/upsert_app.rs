use std::{sync::Arc, time::SystemTime};

use crate::{
    audit_branch,
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
use flow_like::{app::App, bit::Metadata};
use flow_like_types::{anyhow, create_id};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::{NotSet, Set},
    ColumnTrait, DbErr, EntityTrait, IntoActiveModel, JoinType, PaginatorTrait, QueryFilter,
    QuerySelect, RelationTrait, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
    let tier = user.tier(&state).await?;

    let app = app::Entity::find()
        .filter(app::Column::Id.eq(&app_id))
        .one(&state.db)
        .await?;

    let language = query.language.clone().unwrap_or_else(|| "en".to_string());

    let now = chrono::Utc::now().naive_utc();

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
        let app: app::Model = app.save(&state.db).await?.try_into()?;
        audit_branch!(
            state,
            user,
            app_id,
            "app.update",
            "App",
            app_id,
            "Application updated"
        );
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

    if tier.max_non_visible_projects == 0 {
        tracing::warn!("Configuration doesn't allow for the creation of non-visible projects",);
        return Err(ApiError::payment_required(
            "Your current plan does not include online projects.",
        ));
    }

    if tier.max_non_visible_projects > 0 {
        let count = membership::Entity::find()
            .join(JoinType::InnerJoin, membership::Relation::App.def())
            .join(JoinType::InnerJoin, membership::Relation::Role.def())
            .filter(
                app::Column::Visibility
                    .eq(Visibility::Prototype)
                    .or(app::Column::Visibility.eq(Visibility::Private)),
            )
            // Owner Permission is 1, so we filter out roles that have Owner permission
            .filter(role::Column::Permissions.eq(1))
            .filter(membership::Column::UserId.eq(&sub))
            .count(&state.db)
            .await?;

        if count >= tier.max_non_visible_projects as u64 {
            tracing::warn!(
                "User {} has reached the limit of {} non-visible projects [{}]",
                sub,
                tier.max_non_visible_projects,
                count
            );
            return Err(ApiError::payment_required(format!(
                "You have used {} of {} online projects included in your plan.",
                count, tier.max_non_visible_projects
            )));
        }
    }

    let new_id = create_id();
    let drive_app = {
        let credentials = state
            .scoped_credentials(
                &sub,
                &new_id,
                crate::credentials::CredentialsAccess::EditApp,
            )
            .await?;
        let flow_like_state = Arc::new(credentials.to_state(state.clone()).await?);
        let new_app = App::new(
            Some(new_id.clone()),
            metadata.clone(),
            app_body.bits.clone().unwrap_or_default(),
            flow_like_state,
        )
        .await?;
        new_app.save().await?;
        new_app
    };

    let _app = state
        .db
        .transaction::<_, app::Model, DbErr>(|txn| {
            Box::pin(async move {
                let app = app::ActiveModel {
                    id: Set(new_id),
                    status: Set(Status::Active),
                    created_at: Set(now),
                    updated_at: Set(now),
                    visibility: Set(Visibility::Private),
                    bits: Set(Some(app_body.bits.unwrap_or_default())),
                    ..Default::default()
                };

                let app = app.insert(txn).await?;
                let app_id = app.id.clone();

                let meta = meta::ActiveModel {
                    id: Set(create_id()),
                    app_id: Set(Some(app_id.clone())),
                    name: Set(metadata.name),
                    description: Set(Some(metadata.description)),
                    long_description: Set(metadata.long_description),
                    tags: Set(Some(metadata.tags)),
                    lang: Set(language),
                    created_at: Set(now),
                    updated_at: Set(now),
                    ..Default::default()
                };
                meta.insert(txn).await?;

                let owner_role = role::ActiveModel {
                    id: Set(create_id()),
                    name: Set("Owner".to_string()),
                    description: Set(Some("Owner role".to_string())),
                    permissions: Set(RolePermissions::Owner.bits()),
                    created_at: Set(now),
                    updated_at: Set(now),
                    app_id: Set(Some(app_id.clone())),
                    attributes: NotSet,
                };

                let owner_role = owner_role.insert(txn).await?;

                let admin_role = role::ActiveModel {
                    id: Set(create_id()),
                    name: Set("Admin".to_string()),
                    description: Set(Some("Admin role".to_string())),
                    permissions: Set(RolePermissions::Admin.bits()),
                    created_at: Set(now),
                    updated_at: Set(now),
                    app_id: Set(Some(app_id.clone())),
                    attributes: NotSet,
                };

                admin_role.insert(txn).await?;

                let mut user_permission = RolePermissions::ReadTemplates;
                user_permission.insert(RolePermissions::ExecuteEvents);
                user_permission.insert(RolePermissions::ListEvents);

                let user_role = role::ActiveModel {
                    id: Set(create_id()),
                    name: Set("User".to_string()),
                    description: Set(Some("User role".to_string())),
                    permissions: Set(user_permission.bits()),
                    created_at: Set(now),
                    updated_at: Set(now),
                    app_id: Set(Some(app_id.clone())),
                    attributes: NotSet,
                };

                let user_role = user_role.insert(txn).await?;

                let mut app = app.into_active_model();
                app.owner_role_id = Set(Some(owner_role.id.clone()));
                app.default_role_id = Set(Some(user_role.id.clone()));

                let app = app.update(txn).await?;

                let membership = membership::ActiveModel {
                    id: Set(create_id()),
                    user_id: Set(sub),
                    app_id: Set(app.id.clone()),
                    role_id: Set(owner_role.id.clone()),
                    joined_via: NotSet,
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                membership.insert(txn).await?;

                Ok(app)
            })
        })
        .await
        .map_err(|e| match e {
            sea_orm::TransactionError::Connection(db_err) => ApiError::from(db_err),
            sea_orm::TransactionError::Transaction(db_err) => ApiError::from(db_err),
        })?;

    audit_branch!(
        state,
        user,
        drive_app.id,
        "app.create",
        "App",
        drive_app.id,
        "Application created"
    );
    Ok(Json(drive_app))
}
