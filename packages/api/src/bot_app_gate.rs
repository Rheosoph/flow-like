//! An app's owner can refuse bots. Every way a bot reaches an app ends in
//! [`crate::bot_permission::bot_grant_in`], which asks here on each call; attaching and importing
//! ask too, so the bot's owner hears it before the bot runs.

use crate::{
    entity::{app, meta},
    error::ApiError,
};
use axum::http::StatusCode;
use flow_like_types::bot_restrictions::{APP_REFUSES_BOTS, app_refuses_bots_message};
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect};
use std::collections::BTreeMap;

const NAME_LANGUAGE: &str = "en";

pub(crate) fn refusal(app_name: &str) -> ApiError {
    ApiError::coded(
        StatusCode::FORBIDDEN,
        APP_REFUSES_BOTS,
        app_refuses_bots_message(app_name),
    )
}

/// The apps among `app_ids` that refuse bots, each with its name (English first, else any
/// language, else its id).
pub(crate) async fn refusing_among<C: ConnectionTrait>(
    db: &C,
    app_ids: impl IntoIterator<Item = impl Into<String>>,
) -> Result<BTreeMap<String, String>, DbErr> {
    let app_ids: Vec<String> = app_ids.into_iter().map(Into::into).collect();
    if app_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let refusing: Vec<String> = app::Entity::find()
        .select_only()
        .column(app::Column::Id)
        .filter(app::Column::Id.is_in(app_ids))
        .filter(app::Column::RefuseBots.eq(true))
        .into_tuple()
        .all(db)
        .await?;
    if refusing.is_empty() {
        return Ok(BTreeMap::new());
    }
    let names: Vec<(Option<String>, String, String)> = meta::Entity::find()
        .select_only()
        .column(meta::Column::AppId)
        .column(meta::Column::Lang)
        .column(meta::Column::Name)
        .filter(meta::Column::AppId.is_in(refusing.iter().cloned()))
        .filter(meta::Column::TemplateId.is_null())
        .filter(meta::Column::WidgetId.is_null())
        .into_tuple()
        .all(db)
        .await?;
    Ok(refusing
        .into_iter()
        .map(|app_id| {
            let named = |english: bool| {
                names.iter().find(|(owner, lang, name)| {
                    owner.as_deref() == Some(app_id.as_str())
                        && (!english || lang == NAME_LANGUAGE)
                        && !name.trim().is_empty()
                })
            };
            let name = named(true)
                .or_else(|| named(false))
                .map_or_else(|| app_id.clone(), |(_, _, name)| name.trim().to_string());
            (app_id, name)
        })
        .collect())
}

/// Refuses with [`APP_REFUSES_BOTS`] when `app_id` refuses bots.
pub(crate) async fn ensure_admits_bots<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
) -> Result<(), ApiError> {
    match refusing_among(db, [app_id]).await?.into_values().next() {
        Some(name) => Err(refusal(&name)),
        None => Ok(()),
    }
}
