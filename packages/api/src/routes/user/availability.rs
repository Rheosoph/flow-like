use crate::{entity::user, error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State};
use flow_like_types::user_availability::{
    AvailabilityOverride, AvailabilitySource, AvailabilityState, STORAGE_KEY, SetAvailability,
    UserAvailability,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect, sea_query::Expr,
};
use serde_json::{Value, json};

#[path = "availability/calendars.rs"]
pub mod calendars;

pub(crate) async fn load<C: ConnectionTrait>(
    db: &C,
    sub: &str,
) -> Result<AvailabilityState, ApiError> {
    let information = user::Entity::find_by_id(sub)
        .select_only()
        .column(user::Column::AdditionalInformation)
        .into_tuple::<Option<Value>>()
        .one(db)
        .await?
        .flatten();
    Ok(AvailabilityState::read(information.as_ref()))
}

/// Compare the complete JSON value so changing status never overwrites another account setting.
/// The caller may pass an inbox transaction to commit approved status with its response message.
pub(crate) async fn update<C, F>(
    db: &C,
    sub: &str,
    change: F,
) -> Result<AvailabilityState, ApiError>
where
    C: ConnectionTrait,
    F: Fn(&mut AvailabilityState) -> Result<(), ApiError>,
{
    for _ in 0..4 {
        let previous = user::Entity::find_by_id(sub)
            .select_only()
            .column(user::Column::AdditionalInformation)
            .into_tuple::<Option<Value>>()
            .one(db)
            .await?
            .ok_or_else(|| ApiError::not_found("Account not found"))?;
        let mut state = AvailabilityState::read(previous.as_ref());
        change(&mut state)?;
        let mut information = previous.clone().unwrap_or_else(|| json!({}));
        let object = information
            .as_object_mut()
            .ok_or_else(|| ApiError::bad_request("Account settings are not an object"))?;
        object.insert(
            STORAGE_KEY.into(),
            serde_json::to_value(&state)
                .map_err(|_| ApiError::bad_request("Invalid availability"))?,
        );
        let mut query = user::Entity::update_many()
            .col_expr(
                user::Column::AdditionalInformation,
                Expr::value(Some(information)),
            )
            .col_expr(
                user::Column::UpdatedAt,
                Expr::value(chrono::Utc::now().fixed_offset()),
            )
            .filter(user::Column::Id.eq(sub));
        query = match previous {
            Some(previous) => query.filter(user::Column::AdditionalInformation.eq(previous)),
            None => query.filter(user::Column::AdditionalInformation.is_null()),
        };
        if query.exec(db).await?.rows_affected == 1 {
            return Ok(state);
        }
    }
    Err(ApiError::conflict("Your availability changed. Try again"))
}

pub async fn get(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<UserAvailability>, ApiError> {
    let sub = user.sub()?;
    let stored = load(&state.db, &sub).await?;
    let now = chrono::Utc::now().timestamp_millis();
    if !stored.calendar_links.is_empty()
        && stored.calendar_valid_until <= now
        && stored.calendar_checked_at.saturating_add(60_000) <= now
    {
        return calendars::refresh_for(&state, &sub).await.map(Json);
    }
    Ok(Json(stored.view(now)))
}

pub async fn set(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<SetAvailability>,
) -> Result<Json<UserAvailability>, ApiError> {
    let sub = user.sub()?;
    let duration = body.validate().map_err(ApiError::bad_request)?;
    super::ensure_user_exists(&state, &sub).await?;
    let now = chrono::Utc::now().timestamp_millis();
    let updated = update(&state.db, &sub, |stored| {
        if let Some(status) = body.status {
            stored.status_override = Some(AvailabilityOverride {
                status,
                source: AvailabilitySource::Manual,
                until: now + i64::from(duration) * 60_000,
            });
        }
        if let Some(preferences) = &body.preferences {
            stored.preferences = preferences.clone();
        }
        Ok(())
    })
    .await?;
    state.channels.publish_realtime_invalidation(&sub).await;
    Ok(Json(updated.view(now)))
}

pub async fn clear(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<UserAvailability>, ApiError> {
    let sub = user.sub()?;
    super::ensure_user_exists(&state, &sub).await?;
    let updated = update(&state.db, &sub, |stored| {
        stored.status_override = None;
        Ok(())
    })
    .await?;
    state.channels.publish_realtime_invalidation(&sub).await;
    Ok(Json(updated.view(chrono::Utc::now().timestamp_millis())))
}

pub(crate) async fn apply_approved<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    approved: AvailabilityOverride,
) -> Result<(), ApiError> {
    update(db, sub, |stored| {
        stored.status_override = Some(approved.clone());
        Ok(())
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::user_availability::AvailabilityStatus;
    use sea_orm::Database;

    #[tokio::test]
    async fn updates_preserve_other_account_information_and_clear_only_the_override() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        db.execute_unprepared(r#"CREATE TABLE public."User" ("id" TEXT PRIMARY KEY, "additionalInformation" TEXT, "updatedAt" TEXT)"#).await.unwrap();
        db.execute_unprepared(r#"INSERT INTO public."User" ("id", "additionalInformation") VALUES ('person', '{"team":"ops"}')"#).await.unwrap();
        apply_approved(
            &db,
            "person",
            AvailabilityOverride {
                status: AvailabilityStatus::Focus,
                source: AvailabilitySource::Bot,
                until: 100,
            },
        )
        .await
        .unwrap();
        let information = user::Entity::find_by_id("person")
            .select_only()
            .column(user::Column::AdditionalInformation)
            .into_tuple::<Option<Value>>()
            .one(&db)
            .await
            .unwrap()
            .flatten()
            .unwrap();
        assert_eq!(information["team"], "ops");
        assert_eq!(
            load(&db, "person").await.unwrap().current(0).status,
            AvailabilityStatus::Focus
        );
        update(&db, "person", |state| {
            state.preferences.allow_calls = false;
            state.status_override = None;
            Ok(())
        })
        .await
        .unwrap();
        let stored = load(&db, "person").await.unwrap();
        assert_eq!(stored.current(0).status, AvailabilityStatus::Available);
        assert!(!stored.preferences.allow_calls);
    }
}
