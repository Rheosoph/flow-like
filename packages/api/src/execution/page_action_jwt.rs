//! Short-lived capabilities for workflow actions emitted by a Page run.
//!
//! A Page-action JWT is request data, not an identity credential. The caller
//! still authenticates normally and the Event endpoint still checks Page
//! runtime permission plus every claim binding. This token only proves that a
//! trusted Page run resolved one action to one exact board entry node.

use crate::backend_jwt::{self, BackendJwtError, TokenType, issuer, make_time_claims};
use serde::{Deserialize, Serialize};

pub type PageActionJwtError = BackendJwtError;

/// Wire version for Page-action capability claims.
pub const PAGE_ACTION_CAPABILITY_VERSION: u8 = 1;

/// Dynamic actions remain usable for the lifetime of an executor result while
/// their exact Board artifacts and executable WASM package set remain valid.
/// The token is still secondary to normal auth and current runtime permission.
pub const MAX_PAGE_ACTION_TTL_SECONDS: i64 = 24 * 60 * 60;
const MIN_PAGE_ACTION_TTL_SECONDS: i64 = 60;
const PAGE_ACTION_CLOCK_SKEW_SECONDS: i64 = 30;

/// Claims carried by a Page-action capability.
///
/// Source fields identify the Page contract that produced the action. Target
/// fields pin the only workflow entry the capability may invoke. The opaque
/// locator is the canonical Page/widget action locator used by the extractor;
/// keeping it as a string lets that format evolve independently of this JWT
/// envelope while the capability version provides an explicit compatibility
/// boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageActionClaims {
    pub capability_version: u8,
    /// Effective user that initiated the Page run.
    pub sub: String,
    /// API key or other technical principal acting for `sub`, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical_user_id: Option<String>,

    pub source_app_id: String,
    pub source_event_id: String,
    pub source_page_id: String,
    /// Revision/signature of the compiled PrerunManifest used to mint the action.
    pub source_manifest_revision: String,

    pub target_app_id: String,
    pub target_board_id: String,
    /// Immutable version selector. `None` retains the Event's `Latest`
    /// semantics and requires `target_board_etag`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_board_version: Option<(u32, u32, u32)>,
    /// Exact source object identity for a floating Latest board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_board_etag: Option<String>,
    /// Executable WASM package-set revision of the run that emitted this
    /// action. Older capabilities omit it for rolling compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_wasm_authority_revision: Option<String>,
    pub target_node_id: String,

    /// Public action identity paired with this token in the invoke request.
    pub action_id: String,
    /// Run that emitted or dynamically configured the action.
    pub origin_run_id: String,
    /// Canonical component/widget handler locator, retained for binding and audit.
    pub origin_locator: String,

    #[serde(rename = "typ")]
    pub token_type: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

#[derive(Debug, Clone)]
pub struct PageActionJwtParams {
    pub sub: String,
    pub technical_user_id: Option<String>,
    pub source_app_id: String,
    pub source_event_id: String,
    pub source_page_id: String,
    pub source_manifest_revision: String,
    pub target_app_id: String,
    pub target_board_id: String,
    pub target_board_version: Option<(u32, u32, u32)>,
    pub target_board_etag: Option<String>,
    pub target_wasm_authority_revision: Option<String>,
    pub target_node_id: String,
    pub action_id: String,
    pub origin_run_id: String,
    pub origin_locator: String,
    pub ttl_seconds: Option<i64>,
}

fn ensure_nonempty(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("Page-action capability is missing {field}"));
    }
    Ok(())
}

fn validate_claims(claims: &PageActionClaims) -> Result<(), String> {
    if claims.capability_version != PAGE_ACTION_CAPABILITY_VERSION {
        return Err(format!(
            "Unsupported Page-action capability version {}",
            claims.capability_version
        ));
    }

    ensure_nonempty("sub", &claims.sub)?;
    if let Some(technical_user_id) = &claims.technical_user_id {
        ensure_nonempty("technical_user_id", technical_user_id)?;
    }
    ensure_nonempty("source_app_id", &claims.source_app_id)?;
    ensure_nonempty("source_event_id", &claims.source_event_id)?;
    ensure_nonempty("source_page_id", &claims.source_page_id)?;
    ensure_nonempty("source_manifest_revision", &claims.source_manifest_revision)?;
    ensure_nonempty("target_app_id", &claims.target_app_id)?;
    ensure_nonempty("target_board_id", &claims.target_board_id)?;
    match (&claims.target_board_version, &claims.target_board_etag) {
        (Some(_), None) => {}
        (None, Some(etag)) => ensure_nonempty("target_board_etag", etag)?,
        (Some(_), Some(_)) => {
            return Err(
                "Page-action capability cannot bind both a board version and Latest ETag"
                    .to_string(),
            );
        }
        (None, None) => {
            return Err(
                "Page-action capability must bind a board version or Latest ETag".to_string(),
            );
        }
    }
    if let Some(revision) = &claims.target_wasm_authority_revision {
        ensure_nonempty("target_wasm_authority_revision", revision)?;
    }
    ensure_nonempty("target_node_id", &claims.target_node_id)?;
    ensure_nonempty("action_id", &claims.action_id)?;
    ensure_nonempty("origin_run_id", &claims.origin_run_id)?;
    ensure_nonempty("origin_locator", &claims.origin_locator)?;
    ensure_nonempty("jti", &claims.jti)?;

    let lifetime = claims
        .exp
        .checked_sub(claims.iat)
        .ok_or_else(|| "Invalid Page-action capability lifetime".to_string())?;
    if !(1..=MAX_PAGE_ACTION_TTL_SECONDS).contains(&lifetime) {
        return Err("Page-action capability lifetime exceeds its allowed range".to_string());
    }
    if claims.nbf > claims.iat {
        return Err("Page-action capability cannot become valid after it was issued".to_string());
    }

    let now = chrono::Utc::now().timestamp();
    if claims.iat > now + PAGE_ACTION_CLOCK_SKEW_SECONDS
        || claims.nbf > now + PAGE_ACTION_CLOCK_SKEW_SECONDS
    {
        return Err("Page-action capability is not valid yet".to_string());
    }
    if claims.exp <= now {
        return Err("Page-action capability has expired".to_string());
    }

    Ok(())
}

/// Sign a capability for one exact Page action target.
pub fn sign_page_action_capability(
    params: PageActionJwtParams,
) -> Result<String, PageActionJwtError> {
    let ttl = params
        .ttl_seconds
        .unwrap_or_else(|| TokenType::PageAction.default_ttl_seconds())
        .clamp(MIN_PAGE_ACTION_TTL_SECONDS, MAX_PAGE_ACTION_TTL_SECONDS);
    let time = make_time_claims(TokenType::PageAction, Some(ttl));

    let claims = PageActionClaims {
        capability_version: PAGE_ACTION_CAPABILITY_VERSION,
        sub: params.sub,
        technical_user_id: params.technical_user_id,
        source_app_id: params.source_app_id,
        source_event_id: params.source_event_id,
        source_page_id: params.source_page_id,
        source_manifest_revision: params.source_manifest_revision,
        target_app_id: params.target_app_id,
        target_board_id: params.target_board_id,
        target_board_version: params.target_board_version,
        target_board_etag: params.target_board_etag,
        target_wasm_authority_revision: params.target_wasm_authority_revision,
        target_node_id: params.target_node_id,
        action_id: params.action_id,
        origin_run_id: params.origin_run_id,
        origin_locator: params.origin_locator,
        token_type: TokenType::PageAction,
        iss: issuer().to_string(),
        aud: TokenType::PageAction.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti: flow_like_types::create_id(),
    };

    validate_claims(&claims).map_err(BackendJwtError::EncodingError)?;
    backend_jwt::sign(&claims)
}

/// Verify a Page-action capability without turning it into an authenticated
/// API principal. Event routes call this on the optional request-body token.
pub fn verify_page_action_capability(token: &str) -> Result<PageActionClaims, PageActionJwtError> {
    let claims: PageActionClaims = backend_jwt::verify(token, TokenType::PageAction)?;
    if claims.token_type != TokenType::PageAction {
        return Err(BackendJwtError::TokenTypeMismatch {
            expected: TokenType::PageAction,
            got: claims.token_type,
        });
    }
    validate_claims(&claims).map_err(BackendJwtError::DecodingError)?;
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(ttl_seconds: Option<i64>) -> PageActionJwtParams {
        PageActionJwtParams {
            sub: "user-1".into(),
            technical_user_id: Some("technical-1".into()),
            source_app_id: "app-1".into(),
            source_event_id: "event-1".into(),
            source_page_id: "page-1".into(),
            source_manifest_revision: "manifest-revision-1".into(),
            target_app_id: "app-1".into(),
            target_board_id: "board-1".into(),
            target_board_version: Some((4, 2, 1)),
            target_board_etag: None,
            target_wasm_authority_revision: Some("wasm-revision-1".into()),
            target_node_id: "node-1".into(),
            action_id: "pa1_action".into(),
            origin_run_id: "run-1".into(),
            origin_locator: "page-1/component-1/event:click/0".into(),
            ttl_seconds,
        }
    }

    #[test]
    fn roundtrip_preserves_every_authority_binding() {
        backend_jwt::init_for_tests();

        let token = sign_page_action_capability(params(Some(120))).unwrap();
        let claims = verify_page_action_capability(&token).unwrap();

        assert_eq!(claims.capability_version, PAGE_ACTION_CAPABILITY_VERSION);
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.technical_user_id.as_deref(), Some("technical-1"));
        assert_eq!(claims.source_app_id, "app-1");
        assert_eq!(claims.source_event_id, "event-1");
        assert_eq!(claims.source_page_id, "page-1");
        assert_eq!(claims.source_manifest_revision, "manifest-revision-1");
        assert_eq!(claims.target_app_id, "app-1");
        assert_eq!(claims.target_board_id, "board-1");
        assert_eq!(claims.target_board_version, Some((4, 2, 1)));
        assert_eq!(claims.target_board_etag, None);
        assert_eq!(
            claims.target_wasm_authority_revision.as_deref(),
            Some("wasm-revision-1")
        );
        assert_eq!(claims.target_node_id, "node-1");
        assert_eq!(claims.action_id, "pa1_action");
        assert_eq!(claims.origin_run_id, "run-1");
        assert_eq!(claims.origin_locator, "page-1/component-1/event:click/0");
        assert_eq!(claims.token_type, TokenType::PageAction);
        assert_eq!(claims.aud, "flow-like-page-action");
        assert_eq!(claims.exp - claims.iat, 120);
        assert!(!claims.jti.is_empty());
    }

    #[test]
    fn roundtrip_binds_a_latest_board_to_its_etag() {
        backend_jwt::init_for_tests();
        let mut params = params(Some(120));
        params.target_board_version = None;
        params.target_board_etag = Some("etag-latest-1".into());

        let token = sign_page_action_capability(params).unwrap();
        let claims = verify_page_action_capability(&token).unwrap();

        assert_eq!(claims.target_board_version, None);
        assert_eq!(claims.target_board_etag.as_deref(), Some("etag-latest-1"));
    }

    #[test]
    fn signer_rejects_an_unbound_board_selector() {
        backend_jwt::init_for_tests();
        let mut params = params(Some(120));
        params.target_board_version = None;
        params.target_board_etag = None;

        assert!(sign_page_action_capability(params).is_err());
    }

    #[test]
    fn signer_clamps_capability_lifetime() {
        backend_jwt::init_for_tests();

        let default =
            verify_page_action_capability(&sign_page_action_capability(params(None)).unwrap())
                .unwrap();
        assert_eq!(
            default.exp - default.iat,
            TokenType::PageAction.default_ttl_seconds()
        );

        let short =
            verify_page_action_capability(&sign_page_action_capability(params(Some(1))).unwrap())
                .unwrap();
        assert_eq!(short.exp - short.iat, MIN_PAGE_ACTION_TTL_SECONDS);

        let long = verify_page_action_capability(
            &sign_page_action_capability(params(Some(i64::MAX))).unwrap(),
        )
        .unwrap();
        assert_eq!(long.exp - long.iat, MAX_PAGE_ACTION_TTL_SECONDS);
    }

    #[test]
    fn audience_isolation_rejects_page_action_as_executor_identity() {
        backend_jwt::init_for_tests();

        let token = sign_page_action_capability(params(Some(120))).unwrap();
        assert!(
            backend_jwt::verify::<PageActionClaims>(&token, TokenType::Executor).is_err(),
            "the dedicated audience must reject the capability before identity use"
        );
        assert!(crate::execution::verify_execution_jwt(&token).is_err());
        assert!(crate::app_connection_jwt::verify(&token).is_err());
    }

    #[test]
    fn verifier_rejects_unknown_capability_versions() {
        backend_jwt::init_for_tests();

        let token = sign_page_action_capability(params(Some(120))).unwrap();
        let mut claims = verify_page_action_capability(&token).unwrap();
        claims.capability_version += 1;
        let token = backend_jwt::sign(&claims).unwrap();

        assert!(verify_page_action_capability(&token).is_err());
    }

    #[test]
    fn verifier_rejects_tokens_with_excessive_lifetime() {
        backend_jwt::init_for_tests();

        let token = sign_page_action_capability(params(Some(120))).unwrap();
        let mut claims = verify_page_action_capability(&token).unwrap();
        claims.exp = claims.iat + MAX_PAGE_ACTION_TTL_SECONDS + 1;
        let token = backend_jwt::sign(&claims).unwrap();

        assert!(verify_page_action_capability(&token).is_err());
    }

    #[test]
    fn verifier_rejects_tokens_issued_in_the_future() {
        backend_jwt::init_for_tests();

        let token = sign_page_action_capability(params(Some(120))).unwrap();
        let mut claims = verify_page_action_capability(&token).unwrap();
        claims.iat += 120;
        claims.nbf += 120;
        claims.exp += 120;
        let token = backend_jwt::sign(&claims).unwrap();

        assert!(verify_page_action_capability(&token).is_err());
    }
}
