use std::collections::HashMap;
use std::time::SystemTime;

use crate::{
    app,
    sea_orm_active_enums::{
        AppType as DbAppType, Category as DbCategory, ExecutionMode as DbExecutionMode,
        Status as DbStatus, Visibility as DbVisibility,
    },
};
use flow_like::app::{App, AppCategory, AppExecutionMode, AppStatus, AppType, AppVisibility};

impl From<DbAppType> for AppType {
    fn from(value: DbAppType) -> Self {
        match value {
            DbAppType::Agent => Self::Agent,
            DbAppType::CustomInterface => Self::CustomInterface,
            DbAppType::DataFocus => Self::DataFocus,
            DbAppType::DataPipeline => Self::DataPipeline,
            DbAppType::Analytics => Self::Analytics,
            DbAppType::Form => Self::Form,
        }
    }
}

impl From<AppType> for DbAppType {
    fn from(value: AppType) -> Self {
        match value {
            AppType::Agent => Self::Agent,
            AppType::CustomInterface => Self::CustomInterface,
            AppType::DataFocus => Self::DataFocus,
            AppType::DataPipeline => Self::DataPipeline,
            AppType::Analytics => Self::Analytics,
            AppType::Form => Self::Form,
        }
    }
}

impl From<DbCategory> for AppCategory {
    fn from(value: DbCategory) -> Self {
        match value {
            DbCategory::Other => Self::Other,
            DbCategory::Productivity => Self::Productivity,
            DbCategory::Social => Self::Social,
            DbCategory::Entertainment => Self::Entertainment,
            DbCategory::Education => Self::Education,
            DbCategory::Health => Self::Health,
            DbCategory::Finance => Self::Finance,
            DbCategory::Lifestyle => Self::Lifestyle,
            DbCategory::Travel => Self::Travel,
            DbCategory::News => Self::News,
            DbCategory::Sports => Self::Sports,
            DbCategory::Shopping => Self::Shopping,
            DbCategory::FoodAndDrink => Self::FoodAndDrink,
            DbCategory::Music => Self::Music,
            DbCategory::Photography => Self::Photography,
            DbCategory::Utilities => Self::Utilities,
            DbCategory::Weather => Self::Weather,
            DbCategory::Games => Self::Games,
            DbCategory::Business => Self::Business,
            DbCategory::Communication => Self::Communication,
            DbCategory::Anime => Self::Anime,
        }
    }
}

impl From<AppCategory> for DbCategory {
    fn from(value: AppCategory) -> Self {
        match value {
            AppCategory::Other => Self::Other,
            AppCategory::Productivity => Self::Productivity,
            AppCategory::Social => Self::Social,
            AppCategory::Entertainment => Self::Entertainment,
            AppCategory::Education => Self::Education,
            AppCategory::Health => Self::Health,
            AppCategory::Finance => Self::Finance,
            AppCategory::Lifestyle => Self::Lifestyle,
            AppCategory::Travel => Self::Travel,
            AppCategory::News => Self::News,
            AppCategory::Sports => Self::Sports,
            AppCategory::Shopping => Self::Shopping,
            AppCategory::FoodAndDrink => Self::FoodAndDrink,
            AppCategory::Music => Self::Music,
            AppCategory::Photography => Self::Photography,
            AppCategory::Utilities => Self::Utilities,
            AppCategory::Weather => Self::Weather,
            AppCategory::Games => Self::Games,
            AppCategory::Business => Self::Business,
            AppCategory::Communication => Self::Communication,
            AppCategory::Anime => Self::Anime,
        }
    }
}

impl From<app::Model> for App {
    fn from(model: app::Model) -> Self {
        Self {
            id: model.id,
            price: Some(model.price as u32),
            packages: HashMap::new(),
            execution_mode: match model.execution_mode {
                DbExecutionMode::Any => AppExecutionMode::Any,
                DbExecutionMode::Local => AppExecutionMode::Local,
                DbExecutionMode::Remote => AppExecutionMode::Remote,
            },
            status: match model.status {
                DbStatus::Active => AppStatus::Active,
                DbStatus::Inactive => AppStatus::Inactive,
                DbStatus::Archived => AppStatus::Archived,
            },
            visibility: match model.visibility {
                DbVisibility::Public => AppVisibility::Public,
                DbVisibility::PublicRequestAccess => AppVisibility::PublicRequestAccess,
                DbVisibility::Private => AppVisibility::Private,
                DbVisibility::Prototype => AppVisibility::Prototype,
                DbVisibility::Offline => AppVisibility::Offline,
            },
            authors: vec![],
            bits: model.bits.unwrap_or_default().into_inner(),
            boards: vec![],
            events: vec![],
            templates: vec![],
            changelog: model.changelog,
            avg_rating: model.avg_rating,
            download_count: model.download_count as u64,
            interactions_count: model.interactions_count as u64,
            rating_count: model.rating_count as u64,
            rating_sum: model.rating_sum as u64,
            relevance_score: model.relevance_score,
            primary_category: model.primary_category.map(Into::into),
            secondary_category: model.secondary_category.map(Into::into),
            app_type: model.app_type.map(Into::into),
            updated_at: SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(model.updated_at.timestamp() as u64),
            created_at: SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(model.created_at.timestamp() as u64),
            version: model.version,
            frontend: None,
            app_state: None,
            widget_ids: vec![],
            page_ids: vec![],
            allow_forking: model.allow_forking,
            forked_from: model.forked_from,
            forked_at: model.forked_at.map(|date_time| {
                SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_secs(date_time.timestamp() as u64)
            }),
        }
    }
}

impl From<App> for app::Model {
    fn from(app: App) -> Self {
        Self {
            id: app.id,
            execution_mode: match app.execution_mode {
                AppExecutionMode::Any => DbExecutionMode::Any,
                AppExecutionMode::Local => DbExecutionMode::Local,
                AppExecutionMode::Remote => DbExecutionMode::Remote,
            },
            status: match app.status {
                AppStatus::Active => DbStatus::Active,
                AppStatus::Inactive => DbStatus::Inactive,
                AppStatus::Archived => DbStatus::Archived,
            },
            visibility: match app.visibility {
                AppVisibility::Public => DbVisibility::Public,
                AppVisibility::PublicRequestAccess => DbVisibility::PublicRequestAccess,
                AppVisibility::Private => DbVisibility::Private,
                AppVisibility::Prototype => DbVisibility::Prototype,
                AppVisibility::Offline => DbVisibility::Offline,
            },
            bits: Some(app.bits.into()),
            changelog: app.changelog,
            default_role_id: None,
            owner_role_id: None,
            price: app.price.unwrap_or(0) as i64,
            avg_rating: app.avg_rating,
            download_count: app.download_count as i64,
            interactions_count: app.interactions_count as i64,
            relevance_score: app.relevance_score,
            total_size: 0,
            rating_count: app.rating_count as i64,
            rating_sum: app.rating_sum as i64,
            version: app.version,
            updated_at: chrono::Utc::now().fixed_offset(),
            created_at: chrono::Utc::now().fixed_offset(),
            primary_category: app.primary_category.map(Into::into),
            secondary_category: app.secondary_category.map(Into::into),
            app_type: app.app_type.map(Into::into),
            allow_forking: app.allow_forking,
            // Server-authoritative and absent from the core `App` struct — callers must not use
            // this conversion to overwrite the owner's fork policy.
            fork_policy: None,
            forked_from: app.forked_from,
            forked_at: app.forked_at.and_then(|time| {
                time.duration_since(SystemTime::UNIX_EPOCH)
                    .ok()
                    .and_then(|duration| {
                        chrono::DateTime::<chrono::Utc>::from_timestamp(
                            duration.as_secs() as i64,
                            0,
                        )
                        .map(|date_time| date_time.fixed_offset())
                    })
            }),
        }
    }
}
