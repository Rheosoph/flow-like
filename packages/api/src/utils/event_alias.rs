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

pub fn storage_slug_for_event_type(event_type: &str, slug: &str) -> String {
    match event_type {
        "rest" => format!("{REST_ALIAS_PREFIX}{slug}"),
        "mcp" => format!("{MCP_ALIAS_PREFIX}{slug}"),
        _ => slug.to_string(),
    }
}

pub fn public_slug_from_storage(slug: &str) -> String {
    slug.strip_prefix(REST_ALIAS_PREFIX)
        .or_else(|| slug.strip_prefix(MCP_ALIAS_PREFIX))
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
    ];
    let mut matches = Vec::new();

    for storage_slug in candidate_storage_slugs {
        if let Some(model) = event_alias::Entity::find_by_id(storage_slug)
            .one(db)
            .await
            .map_err(|e| {
                ApiError::internal_error(flow_like_types::anyhow!("alias resolve db error: {e}"))
            })?
        {
            matches.push(resolved_from_alias_model(model, slug_or_id, app_hint)?);
        }
    }

    if matches.len() > 1 {
        return Err(ApiError::conflict(format!(
            "alias '{slug_or_id}' exists for multiple interfaces; use the REST or MCP route"
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

    resolve_event_id(db, slug_or_id, app_hint).await
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
        event_id: model.id,
        app_id: model.app_id,
    })
}
