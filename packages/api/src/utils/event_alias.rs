//! Event alias resolver.
//!
//! Inbound REST/MCP traffic (and human-facing tooling) addresses events
//! either by raw `event_id` (cuid/uuid) or by a human-readable alias slug.
//! This module is intentionally stateless: it is a thin wrapper around two
//! indexed PK lookups, mirroring the style of other helpers in this crate.
//!
//! Slug rules — keep in sync with the validator below and the prisma
//! `EventAlias` model:
//!   - length 3..=64
//!   - characters: `[a-z0-9-]`
//!   - reserved slugs (router collisions) and any slug starting with `__`
//!     are always rejected.
//!   - slugs with generated id lengths are always rejected so an alias
//!     can never shadow another event id in `/r/{slug_or_id}`.
//!   - brand-sensitive / phishing-prone slugs (`flow-like*`, …) may only
//!     be claimed by platform admins; see
//!     [`is_admin_reserved_slug`].

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::{
    entity::{event, event_alias},
    error::ApiError,
};

/// Slugs that must never resolve to an alias because they collide with
/// first-class router paths or conventional namespaces. Always rejected,
/// regardless of caller.
const RESERVED_SLUGS: &[&str] = &[
    "api",
    "r",
    "m",
    "c",
    "u",
    "f",
    "frontend",
    "health",
    "metrics",
    "swagger",
    "api-doc",
    "openapi",
    "docs",
    "doc",
    "documentation",
    "admin",
    "app",
    "auth",
    "oauth",
    "callback",
    "callbacks",
    "redirect",
    "user",
    "users",
    "me",
    "profile",
    "apps",
    "team",
    "teams",
    "org",
    "organization",
    "organizations",
    "bit",
    "store",
    "ai",
    "chat",
    "courses",
    "embeddings",
    "execution",
    "interaction",
    "usage",
    "registry",
    "audit",
    "sink",
    "webhook",
    "version",
    "info",
    "tmp",
    "og",
    "solution",
    "aliases",
    "www",
    "cdn",
    "static",
    "assets",
    "asset",
    "files",
    "file",
    "public",
    "private",
    "internal",
    "system",
    "root",
    "upload",
    "uploads",
    "download",
    "downloads",
    "status",
    "support",
    "help",
    "contact",
    "settings",
    "dashboard",
    "console",
    "login",
    "signin",
    "sign-in",
    "signup",
    "sign-up",
    "register",
    "logout",
    "signout",
    "password",
    "reset",
    "verify",
    "verification",
    "confirm",
    "confirmation",
    "account",
    "accounts",
    "security",
    "checkout",
    "payment",
    "payments",
    "pay",
    "billing",
    "invoice",
    "invoices",
    "refund",
    "refunds",
    "subscription",
    "subscribe",
    "trial",
    "enterprise",
    "sales",
];

/// Generated id lengths used by FlowLike ids. These are hard-reserved
/// for direct `/r/{event_id}` and `/m/{event_id}` addressing.
const RESERVED_GENERATED_ID_LENGTHS: &[usize] = &[24, 25];

/// Brand-sensitive slugs. These can be claimed but only by a platform
/// admin. Most generic phishing-prone words are hard-reserved above.
const ADMIN_RESERVED_EXACT: &[&str] = &["wallet", "bank", "transfer", "kyc", "identity", "upgrade"];

const ADMIN_RESERVED_PREFIXES: &[&str] = &["flow-like", "flowlike", "flow_like"];

const REST_ALIAS_PREFIX: &str = "rest_";
const MCP_ALIAS_PREFIX: &str = "mcp_";
const CHAT_ALIAS_PREFIX: &str = "simple_chat_";
const FORM_ALIAS_PREFIX: &str = "generic_form_";
const PAGE_ALIAS_PREFIX: &str = "page_";

/// A custom page owns its UI alias independently of its workflow's trigger type.
pub fn alias_event_type<'a>(event_type: &'a str, page_id: Option<&str>) -> &'a str {
    if matches!(event_type, "rest" | "mcp") {
        event_type
    } else if page_id.is_some() {
        "page"
    } else if event_type == "quick_action" {
        "generic_form"
    } else {
        event_type
    }
}

pub fn storage_slug_for_event_type(event_type: &str, slug: &str) -> String {
    match event_type {
        "rest" => format!("{REST_ALIAS_PREFIX}{slug}"),
        "mcp" => format!("{MCP_ALIAS_PREFIX}{slug}"),
        "simple_chat" => format!("{CHAT_ALIAS_PREFIX}{slug}"),
        "generic_form" | "quick_action" => format!("{FORM_ALIAS_PREFIX}{slug}"),
        "page" => format!("{PAGE_ALIAS_PREFIX}{slug}"),
        _ => slug.to_string(),
    }
}

pub fn public_slug_from_storage(slug: &str) -> String {
    slug.strip_prefix(REST_ALIAS_PREFIX)
        .or_else(|| slug.strip_prefix(MCP_ALIAS_PREFIX))
        .or_else(|| slug.strip_prefix(CHAT_ALIAS_PREFIX))
        .or_else(|| slug.strip_prefix(FORM_ALIAS_PREFIX))
        .or_else(|| slug.strip_prefix(PAGE_ALIAS_PREFIX))
        .unwrap_or(slug)
        .to_string()
}

/// Returns true if `slug` is syntactically valid but reserved for
/// platform admins. Call only after [`validate_slug`] passes.
pub fn is_admin_reserved_slug(slug: &str) -> bool {
    if ADMIN_RESERVED_EXACT.contains(&slug) {
        return true;
    }
    ADMIN_RESERVED_PREFIXES.iter().any(|p| slug.starts_with(p))
}

pub fn is_reserved_generated_id_slug(slug: &str) -> bool {
    RESERVED_GENERATED_ID_LENGTHS.contains(&slug.len())
}

fn is_hard_reserved_slug(slug: &str) -> bool {
    slug.starts_with("__") || RESERVED_SLUGS.contains(&slug)
}

/// Validate a slug — syntax + hard reserved list. Brand-sensitive slugs
/// pass this check; callers must additionally gate them via
/// [`is_admin_reserved_slug`] when the caller is not a platform admin.
pub fn validate_slug(slug: &str) -> Result<(), ApiError> {
    let len = slug.len();
    if !(3..=64).contains(&len) {
        return Err(ApiError::bad_request(format!(
            "alias slug must be 3..=64 chars (got {len})"
        )));
    }
    if is_reserved_generated_id_slug(slug) {
        return Err(ApiError::bad_request(format!(
            "alias slug length {len} is reserved for generated event ids"
        )));
    }
    if !slug
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(ApiError::bad_request(
            "alias slug may only contain [a-z0-9-]".to_string(),
        ));
    }
    if slug.starts_with('-') || slug.ends_with('-') {
        return Err(ApiError::bad_request(
            "alias slug may not start or end with '-'".to_string(),
        ));
    }
    if is_hard_reserved_slug(slug) {
        return Err(ApiError::bad_request(format!(
            "alias slug '{slug}' is reserved"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ResolvedAlias {
    pub event_id: String,
    pub app_id: String,
    /// The event row, when resolution went through the event table. Alias
    /// hits never load it.
    pub event: Option<event::Model>,
}

fn resolved_from_alias_model(
    model: event_alias::Model,
    slug_or_id: &str,
    app_hint: Option<&str>,
) -> Result<ResolvedAlias, ApiError> {
    if let Some(app_id) = app_hint
        && app_id != model.app_id
    {
        return Err(ApiError::not_found(format!(
            "alias '{slug_or_id}' not found"
        )));
    }

    Ok(ResolvedAlias {
        event_id: model.event_id,
        app_id: model.app_id,
        event: None,
    })
}

/// Resolve `slug_or_id` to `(app_id, event_id)`.
///
/// Generated-id-length strings are resolved only as event ids. Other
/// strings try scoped and unscoped alias table lookups first (cheap PK
/// lookups), then fall back to a direct event-id lookup.
///
/// When `app_hint` is supplied, the resolved row must belong to that app
/// (cross-app lookups return 404 to avoid leaking which slugs exist).
///
/// This generic resolver is kept for compatibility with public lookup
/// tooling. Inbound `/r` and `/m` dispatch should use
/// [`resolve_for_event_type`] so `rest` and `mcp` aliases can share the
/// same public slug.
pub async fn resolve(
    db: &DatabaseConnection,
    slug_or_id: &str,
    app_hint: Option<&str>,
) -> Result<ResolvedAlias, ApiError> {
    if is_reserved_generated_id_slug(slug_or_id) {
        return resolve_event_id(db, slug_or_id, app_hint).await;
    }
    if is_hard_reserved_slug(slug_or_id) {
        return resolve_event_id(db, slug_or_id, app_hint).await;
    }

    let candidate_storage_slugs = [
        slug_or_id.to_string(),
        storage_slug_for_event_type("rest", slug_or_id),
        storage_slug_for_event_type("mcp", slug_or_id),
        storage_slug_for_event_type("simple_chat", slug_or_id),
        storage_slug_for_event_type("generic_form", slug_or_id),
        storage_slug_for_event_type("page", slug_or_id),
    ];
    let mut matches = event_alias::Entity::find()
        .filter(event_alias::Column::Slug.is_in(candidate_storage_slugs))
        .all(db)
        .await
        .map_err(|e| {
            ApiError::internal_error(flow_like_types::anyhow!("alias resolve db error: {e}"))
        })?
        .into_iter()
        .map(|model| resolved_from_alias_model(model, slug_or_id, app_hint))
        .collect::<Result<Vec<_>, _>>()?;

    if matches.len() > 1 {
        return Err(ApiError::conflict(format!(
            "alias '{slug_or_id}' exists for multiple interfaces; use its interface route"
        )));
    }

    if let Some(resolved) = matches.pop() {
        return Ok(resolved);
    }

    resolve_event_id(db, slug_or_id, app_hint).await
}

pub async fn resolve_for_event_type(
    db: &DatabaseConnection,
    slug_or_id: &str,
    app_hint: Option<&str>,
    event_type: &str,
) -> Result<ResolvedAlias, ApiError> {
    if is_reserved_generated_id_slug(slug_or_id) {
        return resolve_event_id(db, slug_or_id, app_hint).await;
    }
    if is_hard_reserved_slug(slug_or_id) {
        return resolve_event_id(db, slug_or_id, app_hint).await;
    }

    let storage_slug = storage_slug_for_event_type(event_type, slug_or_id);
    if let Some(model) = event_alias::Entity::find_by_id(storage_slug)
        .one(db)
        .await
        .map_err(|e| {
            ApiError::internal_error(flow_like_types::anyhow!("alias resolve db error: {e}"))
        })?
    {
        return resolved_from_alias_model(model, slug_or_id, app_hint);
    }

    // Older frontend aliases used the unscoped key. Service events retain
    // their REST/MCP alias even when they also own a hosted custom page.
    if matches!(event_type, "simple_chat" | "generic_form" | "page") {
        let mut candidates = vec![slug_or_id.to_string()];
        for previous_interface in ["simple_chat", "generic_form", "page"] {
            if previous_interface != event_type {
                candidates.push(storage_slug_for_event_type(previous_interface, slug_or_id));
            }
        }
        if event_type == "page" {
            candidates.push(storage_slug_for_event_type("rest", slug_or_id));
            candidates.push(storage_slug_for_event_type("mcp", slug_or_id));
        }
        let aliases = event_alias::Entity::find()
            .filter(event_alias::Column::Slug.is_in(candidates))
            .all(db)
            .await
            .map_err(ApiError::from)?;
        let mut matches = Vec::new();
        for alias in aliases {
            if app_hint.is_some_and(|app_id| app_id != alias.app_id) {
                continue;
            }
            let Some(row) = event::Entity::find_by_id(&alias.event_id)
                .one(db)
                .await
                .map_err(ApiError::from)?
            else {
                continue;
            };
            let interface = if row.page_id.is_some() {
                "page"
            } else {
                alias_event_type(&row.event_type, None)
            };
            if interface == event_type && row.app_id == alias.app_id {
                matches.push(ResolvedAlias {
                    event_id: row.id.clone(),
                    app_id: row.app_id.clone(),
                    event: Some(row),
                });
            }
        }
        if matches.len() > 1 {
            return Err(ApiError::conflict(
                "This alias names multiple hosted pages; use the event ID",
            ));
        }
        if let Some(resolved) = matches.pop() {
            return Ok(resolved);
        }
    }

    resolve_event_id(db, slug_or_id, app_hint).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_aliases_have_independent_namespaces() {
        let types = ["rest", "mcp", "simple_chat", "generic_form", "page"];
        let keys: std::collections::HashSet<_> = types
            .iter()
            .map(|kind| storage_slug_for_event_type(kind, "support-demo"))
            .collect();
        assert_eq!(keys.len(), types.len());
        for key in keys {
            assert_eq!(public_slug_from_storage(&key), "support-demo");
        }
        assert_eq!(
            storage_slug_for_event_type("quick_action", "support-demo"),
            "generic_form_support-demo"
        );
    }

    #[test]
    fn pages_keep_service_aliases_and_use_ui_namespace_otherwise() {
        assert_eq!(alias_event_type("rest", Some("page")), "rest");
        assert_eq!(alias_event_type("mcp", Some("page")), "mcp");
        assert_eq!(alias_event_type("simple_chat", Some("page")), "page");
        assert_eq!(alias_event_type("quick_action", None), "generic_form");
        assert!(validate_slug("frontend").is_err());
    }
}

async fn resolve_event_id(
    db: &DatabaseConnection,
    event_id: &str,
    app_hint: Option<&str>,
) -> Result<ResolvedAlias, ApiError> {
    let mut q = event::Entity::find_by_id(event_id);
    if let Some(app_id) = app_hint {
        q = q.filter(event::Column::AppId.eq(app_id));
    }
    let model = q
        .one(db)
        .await
        .map_err(|e| {
            ApiError::internal_error(flow_like_types::anyhow!("alias resolve db error: {e}"))
        })?
        .ok_or_else(|| ApiError::not_found(format!("alias or event '{event_id}' not found")))?;
    Ok(ResolvedAlias {
        event_id: model.id.clone(),
        app_id: model.app_id.clone(),
        event: Some(model),
    })
}
