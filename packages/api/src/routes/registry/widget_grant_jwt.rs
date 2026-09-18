//! Web carrier for widget policy grants.
//!
//! A widget grant JWT is URL path data, not an identity credential. It binds
//! one package version, bundle hash, widget and preview mode to the digest of
//! the policy the viewer approved. The sandbox route re-derives the policy on
//! every request and only widens the document when the digest still matches;
//! access to the package is checked independently and never granted by it.
//!
//! Widget code can read its own URL, so a grant never names the viewer. Grants
//! minted before `sub` was dropped still verify; the field is ignored.
//!
//! Version 1 grants carry the declared policy digest. Version 2 grants also
//! approve runtime sources: `policy_digest` is the effective digest, and
//! `declared_digest` plus `runtime_digest` bind the declared part and the
//! runtime component the frame and document URLs carry as `{token}~{runtime}`.
//! The runtime digest covers `app_id`, so the token names the app it was
//! derived for.

use crate::backend_jwt::{self, BackendJwtError, TokenType, issuer, make_time_claims};
use flow_like_wasm_schema::widget_frame::{
    is_frame_widget_id, is_valid_bundle_hash, is_valid_package_id,
};
use flow_like_wasm_schema::widget_policy::is_valid_widget_app_id;
use serde::{Deserialize, Serialize};

pub type WidgetGrantJwtError = BackendJwtError;

/// Wire version of declared-only widget grant claims.
pub const WIDGET_GRANT_CAPABILITY_VERSION: u8 = 1;
/// Wire version of widget grant claims that also approve runtime sources.
pub const WIDGET_GRANT_RUNTIME_CAPABILITY_VERSION: u8 = 2;
pub const MIN_WIDGET_GRANT_TTL_SECONDS: i64 = 60;
pub const MAX_WIDGET_GRANT_TTL_SECONDS: i64 = 60 * 60;
/// Leeway for widget documents, whose request can trail the wrapper's by a
/// frame load.
pub const WIDGET_GRANT_DOCUMENT_LEEWAY_SECONDS: i64 = 60;
const WIDGET_GRANT_CLOCK_SKEW_SECONDS: i64 = 30;
const POLICY_DIGEST_PREFIX: &str = "sha256:";

/// How strictly a grant's lifetime is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetGrantCheck {
    /// Wrapper documents: expired the second `exp` passes.
    Strict,
    /// Widget documents: [`WIDGET_GRANT_DOCUMENT_LEEWAY_SECONDS`] of leeway.
    Document,
}

impl WidgetGrantCheck {
    pub fn leeway_seconds(self) -> i64 {
        match self {
            WidgetGrantCheck::Strict => 0,
            WidgetGrantCheck::Document => WIDGET_GRANT_DOCUMENT_LEEWAY_SECONDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetGrantClaims {
    pub capability_version: u8,
    pub package_id: String,
    pub version: String,
    pub bundle_hash: String,
    pub widget_id: String,
    pub preview: bool,
    pub policy_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,

    #[serde(rename = "typ")]
    pub token_type: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

impl WidgetGrantClaims {
    /// Whether the grant was minted for exactly this widget of this bundle.
    pub fn is_bound_to(
        &self,
        package_id: &str,
        version: &str,
        bundle_hash: &str,
        widget_id: &str,
    ) -> bool {
        self.package_id == package_id
            && self.version == version
            && self.bundle_hash == bundle_hash
            && self.widget_id == widget_id
    }

    pub fn approves_runtime(&self) -> bool {
        self.capability_version == WIDGET_GRANT_RUNTIME_CAPABILITY_VERSION
    }
}

/// The runtime part of a version 2 grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetGrantRuntime {
    pub declared_digest: String,
    pub runtime_digest: String,
    pub app_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WidgetGrantParams {
    pub package_id: String,
    pub version: String,
    pub bundle_hash: String,
    pub widget_id: String,
    pub preview: bool,
    /// Effective digest: the declared digest, or declared plus runtime.
    pub policy_digest: String,
    /// `Some` signs a version 2 grant.
    pub runtime: Option<WidgetGrantRuntime>,
    pub ttl_seconds: Option<i64>,
}

/// A signed grant and its `jti`, which server logs use to correlate the grant
/// with the viewer that minted it.
#[derive(Debug, Clone)]
pub struct SignedWidgetGrant {
    pub token: String,
    pub jti: String,
}

fn is_policy_digest(value: &str) -> bool {
    value.strip_prefix(POLICY_DIGEST_PREFIX).is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

fn validate_runtime_claims(claims: &WidgetGrantClaims) -> Result<(), String> {
    match claims.capability_version {
        WIDGET_GRANT_CAPABILITY_VERSION => {
            if claims.declared_digest.is_some()
                || claims.runtime_digest.is_some()
                || claims.app_id.is_some()
            {
                return Err(
                    "Version 1 widget grant carries runtime digests or an app id".to_string(),
                );
            }
        }
        WIDGET_GRANT_RUNTIME_CAPABILITY_VERSION => {
            for (name, digest) in [
                ("declared", &claims.declared_digest),
                ("runtime", &claims.runtime_digest),
            ] {
                match digest.as_deref() {
                    Some(digest) if is_policy_digest(digest) => {}
                    Some(digest) => {
                        return Err(format!(
                            "Widget grant carries an invalid {name} digest {digest:?}"
                        ));
                    }
                    None => {
                        return Err(format!(
                            "Version 2 widget grant is missing its {name} digest"
                        ));
                    }
                }
            }
            if let Some(app_id) = claims
                .app_id
                .as_deref()
                .filter(|app_id| !is_valid_widget_app_id(app_id))
            {
                return Err(format!("Widget grant carries an invalid app id {app_id:?}"));
            }
        }
        version => {
            return Err(format!(
                "Unsupported widget grant capability version {version}"
            ));
        }
    }
    Ok(())
}

fn validate_claims(claims: &WidgetGrantClaims, leeway_seconds: i64) -> Result<(), String> {
    validate_runtime_claims(claims)?;
    if !is_valid_package_id(&claims.package_id) {
        return Err(format!(
            "Widget grant carries an invalid package id {:?}",
            claims.package_id
        ));
    }
    if claims.version.trim().is_empty() {
        return Err("Widget grant is missing version".to_string());
    }
    if !is_valid_bundle_hash(&claims.bundle_hash) {
        return Err(format!(
            "Widget grant carries an invalid bundle hash {:?}",
            claims.bundle_hash
        ));
    }
    if !is_frame_widget_id(&claims.widget_id) {
        return Err(format!(
            "Widget grant carries an invalid widget id {:?}",
            claims.widget_id
        ));
    }
    if !is_policy_digest(&claims.policy_digest) {
        return Err(format!(
            "Widget grant carries an invalid policy digest {:?}",
            claims.policy_digest
        ));
    }
    if claims.jti.trim().is_empty() {
        return Err("Widget grant is missing jti".to_string());
    }

    let lifetime = claims
        .exp
        .checked_sub(claims.iat)
        .ok_or_else(|| "Invalid widget grant lifetime".to_string())?;
    if !(1..=MAX_WIDGET_GRANT_TTL_SECONDS).contains(&lifetime) {
        return Err(format!(
            "Widget grant lifetime {lifetime}s is outside 1..={MAX_WIDGET_GRANT_TTL_SECONDS}s"
        ));
    }
    if claims.nbf > claims.iat {
        return Err("Widget grant cannot become valid after it was issued".to_string());
    }

    let now = chrono::Utc::now().timestamp();
    if claims.iat > now + WIDGET_GRANT_CLOCK_SKEW_SECONDS || claims.nbf > now + leeway_seconds {
        return Err("Widget grant is not valid yet".to_string());
    }
    if claims.exp.saturating_add(leeway_seconds) <= now {
        return Err("Widget grant has expired".to_string());
    }

    Ok(())
}

/// Sign a grant for one widget document. The TTL is clamped to
/// [`MIN_WIDGET_GRANT_TTL_SECONDS`]..=[`MAX_WIDGET_GRANT_TTL_SECONDS`].
pub fn sign_widget_grant(
    params: WidgetGrantParams,
) -> Result<SignedWidgetGrant, WidgetGrantJwtError> {
    let ttl = params
        .ttl_seconds
        .unwrap_or_else(|| TokenType::WidgetGrant.default_ttl_seconds())
        .clamp(MIN_WIDGET_GRANT_TTL_SECONDS, MAX_WIDGET_GRANT_TTL_SECONDS);
    let time = make_time_claims(TokenType::WidgetGrant, Some(ttl));
    let (capability_version, declared_digest, runtime_digest, app_id) = match params.runtime {
        Some(runtime) => (
            WIDGET_GRANT_RUNTIME_CAPABILITY_VERSION,
            Some(runtime.declared_digest),
            Some(runtime.runtime_digest),
            runtime.app_id,
        ),
        None => (WIDGET_GRANT_CAPABILITY_VERSION, None, None, None),
    };

    let claims = WidgetGrantClaims {
        capability_version,
        package_id: params.package_id,
        version: params.version,
        bundle_hash: params.bundle_hash,
        widget_id: params.widget_id,
        preview: params.preview,
        policy_digest: params.policy_digest,
        declared_digest,
        runtime_digest,
        app_id,
        token_type: TokenType::WidgetGrant,
        iss: issuer().to_string(),
        aud: TokenType::WidgetGrant.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti: flow_like_types::create_id(),
    };

    validate_claims(&claims, 0).map_err(BackendJwtError::EncodingError)?;
    Ok(SignedWidgetGrant {
        token: backend_jwt::sign(&claims)?,
        jti: claims.jti,
    })
}

/// Verify a widget grant. Binding it to a request path, bundle hash and
/// re-derived policy digest is the caller's responsibility.
pub fn verify_widget_grant(
    token: &str,
    check: WidgetGrantCheck,
) -> Result<WidgetGrantClaims, WidgetGrantJwtError> {
    let leeway = check.leeway_seconds();
    let claims: WidgetGrantClaims =
        backend_jwt::verify_with_leeway(token, TokenType::WidgetGrant, leeway.unsigned_abs())?;
    if claims.token_type != TokenType::WidgetGrant {
        return Err(BackendJwtError::TokenTypeMismatch {
            expected: TokenType::WidgetGrant,
            got: claims.token_type,
        });
    }
    validate_claims(&claims, leeway).map_err(BackendJwtError::DecodingError)?;
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::base64::Engine;
    use flow_like_types::base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use flow_like_wasm_schema::widget_frame::is_web_grant_token;
    use flow_like_wasm_schema::widget_policy::WidgetPolicy;
    use serde_json::{Map, Value};

    const HASH: &str = "4f1c0a3b2d5e6f708192a3b4c5d6e7f80112233445566778899aabbccddeeff0";

    fn params(ttl_seconds: Option<i64>) -> WidgetGrantParams {
        WidgetGrantParams {
            package_id: "com.example.maps".into(),
            version: "1.2.0".into(),
            bundle_hash: HASH.into(),
            widget_id: "live-map".into(),
            preview: false,
            policy_digest: WidgetPolicy {
                workers: true,
                ..WidgetPolicy::default()
            }
            .digest(),
            runtime: None,
            ttl_seconds,
        }
    }

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn runtime_params() -> WidgetGrantParams {
        WidgetGrantParams {
            policy_digest: digest('e'),
            runtime: Some(WidgetGrantRuntime {
                declared_digest: digest('d'),
                runtime_digest: digest('a'),
                app_id: Some("app_01".into()),
            }),
            ..params(Some(120))
        }
    }

    fn resign(claims: &WidgetGrantClaims) -> String {
        backend_jwt::sign(claims).unwrap()
    }

    fn signed_token(ttl_seconds: Option<i64>) -> String {
        sign_widget_grant(params(ttl_seconds)).unwrap().token
    }

    fn signed_claims() -> WidgetGrantClaims {
        verify_widget_grant(&signed_token(Some(120)), WidgetGrantCheck::Strict).unwrap()
    }

    fn payload(token: &str) -> Map<String, Value> {
        let segment = token.split('.').nth(1).unwrap();
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(segment).unwrap()).unwrap()
    }

    #[test]
    fn widget_grant_roundtrip_preserves_bindings() {
        backend_jwt::init_for_tests();

        let grant = sign_widget_grant(params(Some(120))).unwrap();
        let token = grant.token;
        assert!(is_web_grant_token(&token), "token must fit a URL segment");
        let claims = verify_widget_grant(&token, WidgetGrantCheck::Strict).unwrap();

        assert_eq!(claims.capability_version, WIDGET_GRANT_CAPABILITY_VERSION);
        assert_eq!(claims.jti, grant.jti);
        assert_eq!(claims.package_id, "com.example.maps");
        assert_eq!(claims.version, "1.2.0");
        assert_eq!(claims.bundle_hash, HASH);
        assert_eq!(claims.widget_id, "live-map");
        assert!(!claims.preview);
        assert_eq!(claims.policy_digest, params(None).policy_digest);
        assert_eq!(claims.token_type, TokenType::WidgetGrant);
        assert_eq!(claims.aud, "flow-like-widget-grant");
        assert_eq!(claims.exp - claims.iat, 120);
        assert!(!claims.jti.is_empty());
        assert!(claims.is_bound_to("com.example.maps", "1.2.0", HASH, "live-map"));
        assert!(!claims.is_bound_to("com.example.maps", "1.2.1", HASH, "live-map"));
        assert!(!claims.is_bound_to("com.example.maps", "1.2.0", HASH, "other-map"));
        assert!(!claims.is_bound_to("com.example.other", "1.2.0", HASH, "live-map"));
        assert!(!claims.is_bound_to("com.example.maps", "1.2.0", &"0".repeat(64), "live-map"));
    }

    #[test]
    fn widget_grant_payload_never_names_the_viewer() {
        backend_jwt::init_for_tests();

        let grant = sign_widget_grant(params(Some(120))).unwrap();
        let payload = payload(&grant.token);
        assert!(!payload.contains_key("sub"), "{payload:?}");
        assert_eq!(payload["jti"], grant.jti.as_str());
        let mut keys: Vec<&str> = payload.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "aud",
                "bundle_hash",
                "capability_version",
                "exp",
                "iat",
                "iss",
                "jti",
                "nbf",
                "package_id",
                "policy_digest",
                "preview",
                "typ",
                "version",
                "widget_id",
            ]
        );
    }

    #[test]
    fn widget_grant_verifier_ignores_sub_on_grants_minted_before_it_was_dropped() {
        backend_jwt::init_for_tests();

        let claims = signed_claims();
        for sub in ["user-1", "", " "] {
            let mut legacy = serde_json::to_value(&claims).unwrap();
            legacy["sub"] = Value::from(sub);
            let token = backend_jwt::sign(&legacy).unwrap();
            assert!(payload(&token).contains_key("sub"));

            let verified = verify_widget_grant(&token, WidgetGrantCheck::Document).unwrap();
            assert_eq!(verified, claims);
            assert!(
                !serde_json::to_value(&verified)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .contains_key("sub")
            );
        }
    }

    #[test]
    fn widget_grant_signer_clamps_lifetime() {
        backend_jwt::init_for_tests();

        let lifetime = |ttl: Option<i64>| {
            let claims = verify_widget_grant(&signed_token(ttl), WidgetGrantCheck::Strict).unwrap();
            claims.exp - claims.iat
        };
        assert_eq!(lifetime(None), MAX_WIDGET_GRANT_TTL_SECONDS);
        assert_eq!(lifetime(Some(1)), MIN_WIDGET_GRANT_TTL_SECONDS);
        assert_eq!(lifetime(Some(i64::MAX)), MAX_WIDGET_GRANT_TTL_SECONDS);
    }

    #[test]
    fn widget_grant_signer_rejects_invalid_bindings() {
        backend_jwt::init_for_tests();

        let cases: [fn(&mut WidgetGrantParams); 5] = [
            |p| p.package_id = "bad id;".into(),
            |p| p.bundle_hash = "ABC".into(),
            |p| p.widget_id = "../x".into(),
            |p| p.policy_digest = "sha256:xyz".into(),
            |p| p.version = " ".into(),
        ];
        for mutate in cases {
            let mut params = params(Some(120));
            mutate(&mut params);
            assert!(sign_widget_grant(params).is_err());
        }
    }

    #[test]
    fn widget_grant_strict_rejects_what_document_leeway_accepts() {
        backend_jwt::init_for_tests();
        let now = chrono::Utc::now().timestamp();

        let mut just_expired = signed_claims();
        just_expired.iat = now - 100;
        just_expired.nbf = now - 130;
        just_expired.exp = now - 10;
        let token = resign(&just_expired);
        assert!(verify_widget_grant(&token, WidgetGrantCheck::Strict).is_err());
        assert!(verify_widget_grant(&token, WidgetGrantCheck::Document).is_ok());

        let mut long_expired = just_expired.clone();
        long_expired.iat = now - 300;
        long_expired.nbf = now - 330;
        long_expired.exp = now - 120;
        let token = resign(&long_expired);
        assert!(verify_widget_grant(&token, WidgetGrantCheck::Strict).is_err());
        assert!(verify_widget_grant(&token, WidgetGrantCheck::Document).is_err());
    }

    #[test]
    fn widget_grant_verifier_rejects_tampering_and_foreign_audiences() {
        backend_jwt::init_for_tests();

        let token = signed_token(Some(120));
        let (head, signature) = token.rsplit_once('.').unwrap();
        let flipped = if signature.starts_with('A') { "B" } else { "A" };
        let tampered = format!("{head}.{flipped}{}", &signature[1..]);
        assert!(verify_widget_grant(&tampered, WidgetGrantCheck::Document).is_err());

        assert!(backend_jwt::verify::<WidgetGrantClaims>(&token, TokenType::PageAction).is_err());
        assert!(crate::execution::verify_execution_jwt(&token).is_err());

        let mut claims = signed_claims();
        claims.aud = TokenType::PageAction.audience().to_string();
        assert!(verify_widget_grant(&resign(&claims), WidgetGrantCheck::Document).is_err());

        let mut claims = signed_claims();
        claims.token_type = TokenType::PageAction;
        assert!(verify_widget_grant(&resign(&claims), WidgetGrantCheck::Document).is_err());
    }

    #[test]
    fn widget_grant_verifier_rejects_invalid_claims() {
        backend_jwt::init_for_tests();

        let cases: [fn(&mut WidgetGrantClaims); 5] = [
            |c| c.capability_version += 1,
            |c| c.exp = c.iat + MAX_WIDGET_GRANT_TTL_SECONDS + 1,
            |c| {
                c.iat += 120;
                c.nbf += 120;
                c.exp += 120;
            },
            |c| c.policy_digest = "sha256:".to_string() + &"A".repeat(64),
            |c| c.jti = String::new(),
        ];
        for mutate in cases {
            let mut claims = signed_claims();
            mutate(&mut claims);
            assert!(verify_widget_grant(&resign(&claims), WidgetGrantCheck::Document).is_err());
        }
    }

    #[test]
    fn widget_grant_jwt_version_two_roundtrips_runtime_bindings() {
        backend_jwt::init_for_tests();

        let token = sign_widget_grant(runtime_params()).unwrap().token;
        assert!(is_web_grant_token(&token));
        let claims = verify_widget_grant(&token, WidgetGrantCheck::Document).unwrap();
        assert_eq!(
            claims.capability_version,
            WIDGET_GRANT_RUNTIME_CAPABILITY_VERSION
        );
        assert!(claims.approves_runtime());
        assert_eq!(claims.policy_digest, digest('e'));
        assert_eq!(claims.declared_digest, Some(digest('d')));
        assert_eq!(claims.runtime_digest, Some(digest('a')));
        assert_eq!(claims.app_id.as_deref(), Some("app_01"));

        let runtime_payload = payload(&token);
        assert_eq!(runtime_payload["capability_version"], 2);
        assert_eq!(runtime_payload["app_id"], "app_01");
        assert!(!runtime_payload.contains_key("sub"));

        let declared_only = signed_claims();
        assert!(!declared_only.approves_runtime());
        let declared_payload = payload(&signed_token(Some(120)));
        for key in ["declared_digest", "runtime_digest", "app_id"] {
            assert!(!declared_payload.contains_key(key), "{key}");
        }

        let mut anonymous = runtime_params();
        anonymous.runtime.as_mut().unwrap().app_id = None;
        let claims = verify_widget_grant(
            &sign_widget_grant(anonymous).unwrap().token,
            WidgetGrantCheck::Strict,
        )
        .unwrap();
        assert_eq!(claims.app_id, None);
    }

    #[test]
    fn widget_grant_jwt_rejects_mixed_versions_and_missing_digests() {
        backend_jwt::init_for_tests();

        let version_two = verify_widget_grant(
            &sign_widget_grant(runtime_params()).unwrap().token,
            WidgetGrantCheck::Strict,
        )
        .unwrap();
        let version_one = signed_claims();

        let cases: [(&WidgetGrantClaims, fn(&mut WidgetGrantClaims)); 8] = [
            (&version_two, |c| c.declared_digest = None),
            (&version_two, |c| c.runtime_digest = None),
            (&version_two, |c| {
                c.runtime_digest = Some("sha256:zz".into())
            }),
            (&version_two, |c| c.declared_digest = Some(String::new())),
            (&version_two, |c| c.app_id = Some("app/../x".into())),
            (&version_one, |c| c.declared_digest = Some(digest('d'))),
            (&version_one, |c| c.runtime_digest = Some(digest('a'))),
            (&version_one, |c| c.app_id = Some("app_01".into())),
        ];
        for (claims, mutate) in cases {
            let mut claims = claims.clone();
            mutate(&mut claims);
            assert!(
                verify_widget_grant(&resign(&claims), WidgetGrantCheck::Document).is_err(),
                "{claims:?}"
            );
        }

        let mut unsupported = version_two.clone();
        unsupported.capability_version = 3;
        assert!(verify_widget_grant(&resign(&unsupported), WidgetGrantCheck::Document).is_err());

        let mut bad = runtime_params();
        bad.runtime.as_mut().unwrap().runtime_digest = "sha256:".into();
        assert!(sign_widget_grant(bad).is_err());
    }
}
