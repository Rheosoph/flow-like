//! Tells a person that a device stopped confirming the schedules and bots they released to it.
//! The hub skips a claimed schedule for as long as its approval works, so while such a device
//! is off nothing runs it and nothing says so.

use super::{schedules, *};
use crate::{
    entity::{notification, sea_orm_active_enums::NotificationType},
    push_notifications::{DispatchNotificationInput, dispatch_notification_idempotent},
};
use sea_orm::EntityTrait;

/// A running service confirms its claims every 30 minutes. After four missed confirmations
/// the device counts as silent.
pub(crate) const SILENT_AFTER_SECONDS: i64 = 2 * 60 * 60;
const BATCH: i64 = 32;

/// One row per device and person: the claims of working approvals on a registered device that
/// were last confirmed before `$1`. `$2` is now, `$3` the batch size.
const SILENT: &str = r#"SELECT c."deviceId" AS "deviceId", c."releasedBy" AS "releasedBy", MAX(c."seenAt") AS "seenAt", COUNT(*) AS "held", MIN(d."name") AS "name", MIN(d."displayName") AS "displayName" FROM "DeviceScheduleClaim" c JOIN "PlacementResourceGrant" g ON g."id" = c."grantId" AND g."status" = 'active' AND g."expiresAt" > $2 JOIN "ManagedDevice" d ON d."id" = c."deviceId" AND d."status" = 'active' WHERE c."claimedAt" IS NOT NULL AND c."releasedBy" IS NOT NULL AND c."seenAt" < $1 GROUP BY c."deviceId", c."releasedBy" ORDER BY "seenAt" DESC LIMIT $3"#;

/// One notice per silence: a later confirmation followed by a new silence is a new notice.
fn notice_id(device_id: &str, person: &str, seen_at: i64) -> String {
    let encoded =
        serde_json::to_vec(&(device_id, person, seen_at)).expect("serializable claim identity");
    format!("device-schedule-silent:{}", blake3::hash(&encoded).to_hex())
}

fn notice_text(device: &str, held: i64, silent_seconds: i64) -> (String, String) {
    let (what, them) = if held == 1 {
        ("a schedule or bot".to_owned(), "it")
    } else {
        (format!("{held} schedules or bots"), "them")
    };
    let hours = (silent_seconds / 3_600).max(1);
    (
        format!("{device} stopped confirming {what}"),
        format!(
            "{device} last confirmed {them} {hours} h ago. The hub does not run what a device holds, so nothing may be running {them} now. Bring the device back online, or give {them} back to the hub in the project's events."
        ),
    )
}

fn device_link(device_id: &str) -> String {
    format!(
        "/settings/devices?device={}",
        urlencoding::encode(device_id)
    )
}

/// Tells each person who released schedules or bots to a device that has not confirmed them
/// for [`SILENT_AFTER_SECONDS`]. Returns how many notices went out.
pub(crate) async fn notify_unconfirmed(state: &AppState) -> Result<u64, ApiError> {
    if !schedules::table_exists(&state.db).await? {
        return Ok(0);
    }
    let now = now();
    let rows = state
        .db
        .query_all_raw(sql(
            SILENT,
            [
                (now - SILENT_AFTER_SECONDS).into(),
                now.into(),
                BATCH.into(),
            ],
        ))
        .await?;
    let mut told = 0;
    for row in &rows {
        let device_id: String = row.try_get("", "deviceId")?;
        let person: String = row.try_get("", "releasedBy")?;
        let seen_at: i64 = row.try_get("", "seenAt")?;
        let held: i64 = row.try_get("", "held")?;
        let id = notice_id(&device_id, &person, seen_at);
        if notification::Entity::find_by_id(id.as_str())
            .one(&state.db)
            .await?
            .is_some()
        {
            continue;
        }
        let name = match row
            .try_get::<Option<String>>("", "displayName")?
            .filter(|name| !name.trim().is_empty())
        {
            Some(name) => name,
            None => row.try_get("", "name")?,
        };
        let (title, description) = notice_text(&name, held, now - seen_at);
        let sent = dispatch_notification_idempotent(
            state,
            &id,
            DispatchNotificationInput {
                notification_priority: Default::default(),
                bot_context: None,
                user_id: person,
                app_id: None,
                title,
                description: Some(description),
                icon: Some("calendar-clock".into()),
                image: None,
                link: Some(device_link(&device_id)),
                notification_type: NotificationType::System,
                source_run_id: None,
                source_node_id: None,
            },
        )
        .await;
        match sent {
            Ok(_) => told += 1,
            Err(error) => {
                tracing::warn!(%error, device_id, "Silent device notice was not stored")
            }
        }
    }
    Ok(told)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_is_one_per_device_person_and_silence() {
        let first = notice_id("dev-1", "user-a", 1_790_000_000);
        assert!(first.starts_with("device-schedule-silent:"));
        assert_eq!(first, notice_id("dev-1", "user-a", 1_790_000_000));
        for other in [
            notice_id("dev-2", "user-a", 1_790_000_000),
            notice_id("dev-1", "user-b", 1_790_000_000),
            notice_id("dev-1", "user-a", 1_790_001_800),
        ] {
            assert_ne!(first, other);
        }
    }

    #[test]
    fn the_notice_names_the_device_how_many_it_holds_and_for_how_long() {
        let (title, description) = notice_text("Lab GPU", 1, SILENT_AFTER_SECONDS);
        assert_eq!(title, "Lab GPU stopped confirming a schedule or bot");
        assert!(description.starts_with("Lab GPU last confirmed it 2 h ago."));
        let (title, description) = notice_text("Lab GPU", 3, 26 * 3_600 + 5);
        assert_eq!(title, "Lab GPU stopped confirming 3 schedules or bots");
        assert!(description.starts_with("Lab GPU last confirmed them 26 h ago."));
        assert_eq!(
            device_link("device a/b"),
            "/settings/devices?device=device%20a%2Fb"
        );
    }
}
