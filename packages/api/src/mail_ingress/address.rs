use super::{
    digest,
    limits::{self, Counter, Window},
    now, statement,
};
use crate::{
    ensure_fresh_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ConnectionTrait, FromQueryResult};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Previous aliases that keep routing to their event after a rename.
const RETAINED_ALIASES: u32 = 3;
const DAILY_ALIAS_CHANGES: u32 = 10;
/// Addresses of a deleted event stay reserved this long before anyone can claim them.
const QUARANTINE_MS: i64 = 30 * 86_400_000;
const HOUSEKEEPING_BATCH: u32 = 200;

/// Role mailboxes that certificate authorities, abuse desks and humans expect to reach the
/// domain owner. They are never issued as event aliases.
const RESERVED: &[&str] = &[
    "abuse",
    "admin",
    "administrator",
    "billing",
    "bounce",
    "bounces",
    "dmarc",
    "ftp",
    "host-master",
    "hostmaster",
    "info",
    "is",
    "it",
    "mail",
    "mailer-daemon",
    "mailerdaemon",
    "marketing",
    "mis",
    "no-reply",
    "noc",
    "noreply",
    "postmaster",
    "root",
    "sales",
    "security",
    "ssl-admin",
    "ssladmin",
    "ssladministrator",
    "sslwebmaster",
    "support",
    "sysadmin",
    "webmaster",
    "www",
];

#[derive(Clone, FromQueryResult)]
pub(super) struct Address {
    pub address: String,
    #[sea_orm(from_alias = "appId")]
    pub app_id: String,
    #[sea_orm(from_alias = "eventId")]
    pub event_id: String,
    pub kind: String,
}

#[derive(Serialize, ToSchema)]
pub struct AddressView {
    /// Whether this server receives email at all.
    configured: bool,
    domain: Option<String>,
    /// Stable address issued for the event.
    address: Option<String>,
    /// Local part of the current alias on the same domain.
    alias: Option<String>,
    /// Whether mail sent now would start the event.
    active: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AliasUpdate {
    /// New alias local part; `null` removes the alias.
    alias: Option<String>,
}

pub(super) fn domain(state: &AppState) -> Option<String> {
    state.mail_automation.domain().map(str::to_owned)
}

fn primary(app: &str, event: &str, domain: &str) -> String {
    let key = format!("{app}\0{event}");
    format!(
        "m-{}@{domain}",
        &blake3::hash(key.as_bytes()).to_hex()[..32]
    )
}

pub(super) fn is_reserved(local: &str) -> bool {
    RESERVED.iter().any(|name| name.eq_ignore_ascii_case(local))
}

/// A role mailbox on this server's inbound domain.
pub(super) fn reserved_recipient(domain: &str, raw: &str) -> bool {
    raw.trim()
        .rsplit_once('@')
        .is_some_and(|(local, host)| host.eq_ignore_ascii_case(domain) && is_reserved(local))
}

pub(super) fn normalize_alias(raw: &str) -> Result<String, ApiError> {
    let alias = raw.trim().to_ascii_lowercase();
    if !(3..=64).contains(&alias.len())
        || alias.starts_with('-')
        || alias.ends_with('-')
        || !alias
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err(ApiError::bad_request(
            "Use 3–64 letters, numbers or hyphens; start and end with a letter or number",
        ));
    }
    if alias.starts_with("m-") || is_reserved(&alias) {
        return Err(ApiError::bad_request("This email alias is reserved"));
    }
    Ok(alias)
}

pub async fn sync_address(state: &AppState, app: &str, event: &str) -> Result<(), ApiError> {
    let Some(domain) = domain(state) else {
        return Ok(());
    };
    // A quarantined primary address returns to its event when the event exists again.
    state.db.execute_raw(statement(
        r#"INSERT INTO "InboundMailAddress" ("address","appId","eventId","kind","active","createdAt","updatedAt") VALUES ($1,$2,$3,'primary',TRUE,$4,$4) ON CONFLICT ("address") DO UPDATE SET "kind"='primary',"active"=TRUE,"updatedAt"=$4 WHERE "InboundMailAddress"."kind"='quarantined' AND "InboundMailAddress"."appId"=$2 AND "InboundMailAddress"."eventId"=$3"#,
        vec![primary(app,event,&domain).into(), app.into(), event.into(), now().into()],
    )).await?;
    Ok(())
}

async fn view(state: &AppState, app: &str, event_id: &str) -> Result<AddressView, ApiError> {
    let event = crate::routes::app::events::db::get_event_from_db(&state.db, event_id, app).await?;
    if event.event_type != "inbound_email" {
        return Err(ApiError::bad_request("This event does not receive email"));
    }
    let domain = domain(state);
    sync_address(state, app, event_id).await?;
    let addresses = Address::find_by_statement(statement(
        r#"SELECT * FROM "InboundMailAddress" WHERE "appId"=$1 AND "eventId"=$2 AND "active"=TRUE ORDER BY "createdAt""#,
        vec![app.into(),event_id.into()])).all(&state.db).await?;
    let alias = addresses
        .iter()
        .find(|a| {
            a.kind == "alias"
                && domain
                    .as_ref()
                    .is_some_and(|d| a.address.ends_with(&format!("@{d}")))
        })
        .and_then(|a| a.address.split_once('@').map(|(local, _)| local.to_owned()));
    let active = super::live_target(state, app, event_id).await?.is_some();
    Ok(AddressView {
        configured: domain.is_some(),
        address: domain.as_ref().map(|d| primary(app, event_id, d)),
        domain,
        alias,
        active,
    })
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/email-address",
    tag = "events",
    description = "Show the email address of an inbound email event, its alias and whether mail sent to it now starts the event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Inbound email event ID")
    ),
    responses(
        (status = 200, description = "The event's email address", body = AddressView),
        (status = 400, description = "The event does not receive email"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Missing permission to read events"),
        (status = 404, description = "Event not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn get_address(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app, event)): Path<(String, String)>,
) -> Result<Json<AddressView>, ApiError> {
    ensure_fresh_permission!(user, &app, &state, RolePermissions::ReadEvents);
    Ok(Json(view(&state, &app, &event).await?))
}

/// Alias edits per event and day, so names cannot be cycled to squat them.
struct AliasChange {
    window: Window,
    subject: String,
}

impl AliasChange {
    fn counters(&self) -> [Counter<'_>; 1] {
        [Counter {
            scope: "alias",
            subject: &self.subject,
            limit: DAILY_ALIAS_CHANGES,
        }]
    }

    async fn reserve(state: &AppState, app: &str, event: &str) -> Result<Self, ApiError> {
        let change = Self {
            window: Window::at(now()),
            subject: digest(&[app, event]),
        };
        if limits::consume(state, &change.window, &change.counters(), 1)
            .await?
            .is_some()
        {
            return Err(ApiError::too_many_requests(format!(
                "The email alias of an event can change at most {DAILY_ALIAS_CHANGES} times per day"
            )));
        }
        Ok(change)
    }

    async fn refund(&self, state: &AppState) {
        limits::refund(state, &self.window, &self.counters(), 1).await;
    }
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/events/{event_id}/email-address",
    tag = "events",
    description = "Set or remove a readable alias for an inbound email event. The previous aliases keep delivering to this event, up to three of them; an alias can change ten times per day.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Inbound email event ID")
    ),
    request_body = AliasUpdate,
    responses(
        (status = 200, description = "The event's updated email address", body = AddressView),
        (status = 400, description = "Invalid or reserved alias, or the event does not receive email"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Missing permission to edit events"),
        (status = 404, description = "Event not found"),
        (status = 409, description = "Another event already uses this alias"),
        (status = 429, description = "The alias changed too often today")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn update_alias(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app, event)): Path<(String, String)>,
    Json(input): Json<AliasUpdate>,
) -> Result<Json<AddressView>, ApiError> {
    ensure_fresh_permission!(user, &app, &state, RolePermissions::WriteEvents);
    let current = view(&state, &app, &event).await?;
    let domain = current
        .domain
        .ok_or_else(|| ApiError::bad_request("Inbound email is not configured on this server"))?;
    let primary = primary(&app, &event, &domain);
    let local = input.alias.as_deref().map(normalize_alias).transpose()?;
    let changes = local != current.alias;
    let alias = local.map(|a| format!("{a}@{domain}"));
    let change = if changes {
        Some(AliasChange::reserve(&state, &app, &event).await?)
    } else {
        None
    };
    let app_tx = app.clone();
    let event_tx = event.clone();
    let updated = state.transaction(move |tx| {
        let app = app_tx.clone(); let event = event_tx.clone(); let primary = primary.clone(); let alias = alias.clone();
        Box::pin(async move {
            // Serialize edits to one event, including concurrent alias removal.
            tx.execute_raw(statement(r#"UPDATE "InboundMailAddress" SET "updatedAt"=$1 WHERE "address"=$2"#, vec![now().into(),primary.into()])).await?;
            tx.execute_raw(statement(r#"UPDATE "InboundMailAddress" SET "active"=FALSE,"updatedAt"=$1 WHERE "appId"=$2 AND "eventId"=$3 AND "kind"='alias' AND "active"=TRUE"#,vec![now().into(),app.clone().into(),event.clone().into()])).await?;
            if let Some(alias) = alias {
                let result = tx.execute_raw(statement(r#"INSERT INTO "InboundMailAddress" ("address","appId","eventId","kind","active","createdAt","updatedAt") VALUES ($1,$2,$3,'alias',TRUE,$4,$4) ON CONFLICT ("address") DO UPDATE SET "kind"='alias',"active"=TRUE,"updatedAt"=$4 WHERE "InboundMailAddress"."appId"=$2 AND "InboundMailAddress"."eventId"=$3 AND "InboundMailAddress"."kind" IN ('alias','quarantined')"#,vec![alias.into(),app.clone().into(),event.clone().into(),now().into()])).await?;
                if result.rows_affected() != 1 { return Err(ApiError::conflict("This email alias is already reserved")); }
            }
            // Older aliases are released so one event cannot hoard names.
            tx.execute_raw(statement(&format!(r#"DELETE FROM "InboundMailAddress" WHERE "address" IN (SELECT "address" FROM "InboundMailAddress" WHERE "appId"=$1 AND "eventId"=$2 AND "kind"='alias' AND "active"=FALSE ORDER BY "updatedAt" DESC, "address" OFFSET {RETAINED_ALIASES})"#),vec![app.into(),event.into()])).await?;
            Ok::<_,ApiError>(())
        })
    }).await;
    if let Err(error) = updated {
        if let Some(change) = &change {
            change.refund(&state).await;
        }
        return Err(error);
    }
    crate::audit_branch!(state, user, app, "event.email-alias.update", "Event", event);
    Ok(Json(view(&state, &app, &event).await?))
}

/// Marks addresses whose event no longer exists; they stay reserved during the quarantine.
pub(super) async fn quarantine_orphans(state: &AppState, timestamp: i64) -> Result<u64, ApiError> {
    Ok(state
        .db
        .execute_raw(statement(
            &format!(
                r#"UPDATE "InboundMailAddress" SET "kind"='quarantined',"active"=FALSE,"updatedAt"=$1 WHERE "address" IN (SELECT a."address" FROM "InboundMailAddress" a WHERE a."kind" IN ('primary','alias') AND NOT EXISTS (SELECT 1 FROM "Event" e WHERE e."id"=a."eventId" AND e."appId"=a."appId") LIMIT {HOUSEKEEPING_BATCH})"#
            ),
            vec![timestamp.into()],
        ))
        .await?
        .rows_affected())
}

pub(super) async fn release_quarantined(state: &AppState, timestamp: i64) -> Result<u64, ApiError> {
    Ok(state
        .db
        .execute_raw(statement(
            &format!(
                r#"DELETE FROM "InboundMailAddress" WHERE "address" IN (SELECT "address" FROM "InboundMailAddress" WHERE "kind"='quarantined' AND "updatedAt" <= $1 LIMIT {HOUSEKEEPING_BATCH})"#
            ),
            vec![(timestamp - QUARANTINE_MS).into()],
        ))
        .await?
        .rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_are_unambiguous_and_reserved_names_are_rejected() {
        assert_eq!(normalize_alias(" Invoice-123 ").unwrap(), "invoice-123");
        for alias in [
            "a",
            "m-secret",
            "ADMIN",
            "foo@bar.com",
            "-foo",
            "foo-",
            "föö",
            "a+b",
            "foo.bar",
        ] {
            assert!(normalize_alias(alias).is_err(), "{alias}");
        }
    }

    #[test]
    fn certificate_and_role_mailboxes_are_reserved() {
        for alias in [
            "administrator",
            "Webmaster",
            "hostmaster",
            "mailer-daemon",
            "dmarc",
            "info",
            "support",
            "ssladmin",
            "sslwebmaster",
            "www",
            "ftp",
            "mail",
            "marketing",
            "sales",
            "billing",
            "postmaster",
            "abuse",
            "no-reply",
        ] {
            assert!(normalize_alias(alias).is_err(), "{alias}");
        }
        assert!(normalize_alias("orders").is_ok());
        assert!(reserved_recipient(
            "mail.example.com",
            "PostMaster@Mail.Example.com"
        ));
        assert!(reserved_recipient(
            "mail.example.com",
            " it@mail.example.com"
        ));
        assert!(!reserved_recipient(
            "mail.example.com",
            "postmaster@example.com"
        ));
        assert!(!reserved_recipient(
            "mail.example.com",
            "orders@mail.example.com"
        ));
    }

    #[test]
    fn primary_addresses_are_stable_and_scoped() {
        assert_eq!(
            primary("a", "b", "mail.example.com"),
            primary("a", "b", "mail.example.com")
        );
        assert_ne!(
            primary("ab", "c", "mail.example.com"),
            primary("a", "bc", "mail.example.com")
        );
    }
}

/// Resolve From using the server's registry. A retired alias falls back to the primary address.
pub(crate) async fn sending_address(
    state: &AppState,
    session: &flow_like_catalog_core::MailSession,
    preferred: Option<&str>,
) -> Result<String, ApiError> {
    let domain = domain(state).ok_or_else(|| {
        ApiError::service_unavailable("Inbound email is not configured on this server")
    })?;
    sync_address(state, &session.app_id, &session.event_id).await?;
    let addresses = Address::find_by_statement(statement(
        r#"SELECT * FROM "InboundMailAddress" WHERE "appId"=$1 AND "eventId"=$2 AND "active"=TRUE"#,
        vec![
            session.app_id.clone().into(),
            session.event_id.clone().into(),
        ],
    ))
    .all(&state.db)
    .await?;
    if let Some(preferred) = preferred {
        if let Some(address) = addresses
            .iter()
            .find(|a| a.address == preferred && a.address.ends_with(&format!("@{domain}")))
        {
            return Ok(address.address.clone());
        }
    }
    let expected = primary(&session.app_id, &session.event_id, &domain);
    addresses
        .into_iter()
        .find(|a| a.kind == "primary" && a.address == expected)
        .map(|a| a.address)
        .ok_or_else(|| ApiError::service_unavailable("This event has no sending address"))
}
