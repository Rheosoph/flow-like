//! Out-of-band delivery of telemetry alert transitions.
//!
//! The inbox row written by `telemetry::alerts` stays the source of truth; this
//! module only carries an already-committed transition to a human. Every send is
//! best-effort: failures are logged and swallowed so a mail or push outage can
//! never fail an evaluation pass, abort the remaining rules or lose an alert.
//!
//! Email goes to the single platform alerting mailbox, push to the users holding
//! the global Admin permission. Both carry aggregates only, never identity.

use futures::future::{join, join_all};
use sea_orm::sea_query::{Expr, ExprTrait, SimpleExpr};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect,
};

use crate::alerting::send_alert_email;
use crate::entity::sea_orm_active_enums::{NotificationType, UserStatus};
use crate::entity::{telemetry_alert_event, telemetry_alert_rule, user};
use crate::permission::global_permission::GlobalPermission;
use crate::push_notifications::{DispatchNotificationInput, dispatch_notification};
use crate::state::AppState;
use crate::telemetry::alerts::{ALERT_STATUS_TRIGGERED, format_metric_value};

/// Upper bound on the admins a single alert transition pushes to.
pub const MAX_PUSH_RECIPIENTS: usize = 50;
/// Where a push recipient lands when they open the notification.
const ALERT_INBOX_LINK: &str = "/admin/telemetry/alerts";

/// Admin candidates as read back from the user table.
#[derive(Clone, Debug, FromQueryResult)]
struct AdminRecipientRow {
    id: String,
    permission: i64,
}

/// Deliver an alert transition out of band, honouring the rule's channels.
///
/// Called after the inbox row is committed and never returns an error: the
/// caller's evaluation pass must survive any delivery failure untouched.
pub async fn notify_alert_transition(
    state: &AppState,
    rule: &telemetry_alert_rule::Model,
    event: &telemetry_alert_event::Model,
) {
    if !rule.notify_email && !rule.notify_push {
        return;
    }

    let triggered = event.status == ALERT_STATUS_TRIGGERED;
    let title = alert_title(&rule.name, &rule.metric, triggered);
    let body = alert_body(
        &event.message,
        &rule.metric,
        rule.source.as_deref(),
        event.value,
        event.threshold,
        rule.window_minutes,
    );

    join(
        notify_by_email(state, rule, event, &title, &body),
        notify_admins_by_push(state, rule, event, &title),
    )
    .await;
}

/// The platform alerting mailbox. `send_alert_email` bails when alerting or the
/// mail client is unconfigured, which is logged once per event, not per channel.
async fn notify_by_email(
    state: &AppState,
    rule: &telemetry_alert_rule::Model,
    event: &telemetry_alert_event::Model,
    title: &str,
    body: &str,
) {
    if !rule.notify_email {
        return;
    }

    if let Err(error) = send_alert_email(state, title, body).await {
        tracing::warn!(
            rule_id = %rule.id,
            event_id = %event.id,
            error = %error,
            "Telemetry alert email delivery failed"
        );
    }
}

/// Every global admin, concurrently and capped.
async fn notify_admins_by_push(
    state: &AppState,
    rule: &telemetry_alert_rule::Model,
    event: &telemetry_alert_event::Model,
    title: &str,
) {
    if !rule.notify_push {
        return;
    }

    let recipients = match admin_recipients(state).await {
        Ok(recipients) => recipients,
        Err(error) => {
            tracing::warn!(
                rule_id = %rule.id,
                event_id = %event.id,
                error = %error,
                "Failed to resolve telemetry alert push recipients"
            );
            return;
        }
    };

    if recipients.is_empty() {
        tracing::debug!(
            rule_id = %rule.id,
            event_id = %event.id,
            "No global admin holds a push recipient slot for this telemetry alert"
        );
        return;
    }

    let description = alert_push_description(&event.message, rule.source.as_deref());
    join_all(recipients.into_iter().map(|user_id| {
        let input = DispatchNotificationInput {
            user_id: user_id.clone(),
            app_id: None,
            title: title.to_string(),
            description: Some(description.clone()),
            icon: None,
            image: None,
            link: Some(ALERT_INBOX_LINK.to_string()),
            notification_type: NotificationType::System,
            source_run_id: None,
            source_node_id: None,
        };
        async move {
            if let Err(error) = dispatch_notification(state, input).await {
                tracing::warn!(
                    rule_id = %rule.id,
                    event_id = %event.id,
                    user_id = %user_id,
                    error = %error,
                    "Telemetry alert push delivery failed"
                );
            }
        }
    }))
    .await;
}

/// Active users holding the global Admin permission, capped at
/// [`MAX_PUSH_RECIPIENTS`]. The bit is tested in SQL so the fan-out never reads
/// the whole user table, and again in Rust so the cap applies to real admins.
async fn admin_recipients(state: &AppState) -> Result<Vec<String>, sea_orm::DbErr> {
    let rows = user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .column(user::Column::Permission)
        .filter(admin_permission_filter())
        .filter(user::Column::Status.eq(UserStatus::Active))
        .order_by_asc(user::Column::CreatedAt)
        .limit(MAX_PUSH_RECIPIENTS as u64 + 1)
        .into_model::<AdminRecipientRow>()
        .all(&state.db)
        .await?;

    Ok(cap_recipients(rows))
}

/// `permission & <Admin bit> <> 0`, portable across the supported backends.
fn admin_permission_filter() -> SimpleExpr {
    Expr::col(user::Column::Permission)
        .bit_and(GlobalPermission::Admin.bits())
        .ne(0)
}

/// Whether a stored permission bitfield carries the global Admin bit. Unknown
/// bits are truncated instead of rejected so a future permission can never
/// silently drop an admin from the fan-out.
fn holds_admin(permission: i64) -> bool {
    GlobalPermission::from_bits_truncate(permission).contains(GlobalPermission::Admin)
}

/// Keeps only real admins and bounds the fan-out, warning when the cap bites.
fn cap_recipients(candidates: Vec<AdminRecipientRow>) -> Vec<String> {
    let mut recipients: Vec<String> = candidates
        .into_iter()
        .filter(|row| holds_admin(row.permission))
        .map(|row| row.id)
        .collect();

    if recipients.len() > MAX_PUSH_RECIPIENTS {
        tracing::warn!(
            recipients = recipients.len(),
            cap = MAX_PUSH_RECIPIENTS,
            "Telemetry alert push fan-out exceeds the recipient cap, notifying the oldest admins only"
        );
        recipients.truncate(MAX_PUSH_RECIPIENTS);
    }

    recipients
}

/// Email subject and push title: what happened, to which rule, on which metric.
fn alert_title(rule_name: &str, metric: &str, triggered: bool) -> String {
    let transition = if triggered { "triggered" } else { "resolved" };
    format!("Telemetry alert {transition}: {rule_name} ({metric})")
}

/// Email body: the engine's message plus the numbers behind it.
fn alert_body(
    message: &str,
    metric: &str,
    source: Option<&str>,
    value: f64,
    threshold: Option<f64>,
    window_minutes: i32,
) -> String {
    let threshold = match threshold {
        Some(threshold) => format_metric_value(threshold),
        None => "none (anomaly mode)".to_string(),
    };
    format!(
        "{message}\n\nMetric: {metric}\nSource: {}\nValue: {}\nThreshold: {threshold}\nWindow: {window_minutes} min",
        source.unwrap_or("all sources"),
        format_metric_value(value)
    )
}

/// Push description: one line, since the message already carries value,
/// threshold and window.
fn alert_push_description(message: &str, source: Option<&str>) -> String {
    match source {
        Some(source) => format!("{message} (source: {source})"),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, permission: i64) -> AdminRecipientRow {
        AdminRecipientRow {
            id: id.to_string(),
            permission,
        }
    }

    #[test]
    fn only_the_admin_bit_makes_a_recipient() {
        assert!(holds_admin(GlobalPermission::Admin.bits()));
        assert!(holds_admin(
            (GlobalPermission::Admin | GlobalPermission::ReadLogs).bits()
        ));
        assert!(!holds_admin(0));
        assert!(!holds_admin(GlobalPermission::ReadLogs.bits()));
        assert!(!holds_admin(GlobalPermission::WriteApps.bits()));
    }

    #[test]
    fn unknown_permission_bits_never_drop_an_admin() {
        let future_bit = 1_i64 << 40;
        assert!(holds_admin(GlobalPermission::Admin.bits() | future_bit));
        assert!(!holds_admin(future_bit));
    }

    #[test]
    fn non_admin_rows_are_filtered_out_of_the_fan_out() {
        let recipients = cap_recipients(vec![
            row("admin", GlobalPermission::Admin.bits()),
            row("reader", GlobalPermission::ReadLogs.bits()),
            row("nobody", 0),
        ]);

        assert_eq!(recipients, vec!["admin".to_string()]);
    }

    #[test]
    fn the_fan_out_is_capped() {
        let candidates: Vec<AdminRecipientRow> = (0..MAX_PUSH_RECIPIENTS + 10)
            .map(|index| row(&format!("admin-{index}"), GlobalPermission::Admin.bits()))
            .collect();

        let recipients = cap_recipients(candidates);
        assert_eq!(recipients.len(), MAX_PUSH_RECIPIENTS);
        assert_eq!(recipients.first().unwrap(), "admin-0");
    }

    #[test]
    fn titles_name_the_rule_the_metric_and_the_transition() {
        assert_eq!(
            alert_title("Error rate", "error_rate", true),
            "Telemetry alert triggered: Error rate (error_rate)"
        );
        assert_eq!(
            alert_title("Error rate", "error_rate", false),
            "Telemetry alert resolved: Error rate (error_rate)"
        );
    }

    #[test]
    fn bodies_carry_the_message_value_threshold_and_window() {
        let body = alert_body(
            "error_rate is 0.123 over the last 15 min, above the threshold of 0.050",
            "error_rate",
            Some("desktop"),
            0.1234,
            Some(0.05),
            15,
        );

        assert!(body.starts_with("error_rate is 0.123"), "{body}");
        assert!(body.contains("Metric: error_rate"), "{body}");
        assert!(body.contains("Source: desktop"), "{body}");
        assert!(body.contains("Value: 0.123"), "{body}");
        assert!(body.contains("Threshold: 0.050"), "{body}");
        assert!(body.contains("Window: 15 min"), "{body}");
    }

    #[test]
    fn bodies_of_anomaly_rules_state_the_absent_threshold() {
        let body = alert_body("latency_p95 spiked", "latency_p95", None, 900.0, None, 60);

        assert!(body.contains("Source: all sources"), "{body}");
        assert!(body.contains("Value: 900"), "{body}");
        assert!(body.contains("Threshold: none (anomaly mode)"), "{body}");
    }

    #[test]
    fn push_descriptions_stay_one_line_and_name_the_source() {
        assert_eq!(
            alert_push_description("error_rate recovered to 0.010", Some("web")),
            "error_rate recovered to 0.010 (source: web)"
        );
        assert_eq!(
            alert_push_description("error_rate recovered to 0.010", None),
            "error_rate recovered to 0.010"
        );
    }
}
