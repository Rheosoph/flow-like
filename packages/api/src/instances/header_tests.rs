use super::*;
use crate::middleware::jwt::FORWARDED_AUTHORIZATION_HEADER;
use axum::http::{HeaderMap, HeaderValue};

fn valid_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        HeaderValue::from_static("DPoP resource-token"),
    );
    headers.insert("dpop", HeaderValue::from_static("request-proof"));
    headers
}

#[test]
fn ambiguous_instance_credentials_are_rejected_before_verification() {
    for name in ["authorization", "dpop", FORWARDED_AUTHORIZATION_HEADER] {
        let mut headers = valid_headers();
        if name == FORWARDED_AUTHORIZATION_HEADER {
            headers.insert(name, HeaderValue::from_static("DPoP forwarded-token"));
        }
        headers.append(name, HeaderValue::from_static("second-value"));
        assert!(credentials(&headers).is_err(), "accepted duplicate {name}");
    }
    let mut missing = valid_headers();
    missing.remove("dpop");
    assert!(credentials(&missing).is_err());
    let mut bearer = valid_headers();
    bearer.insert(
        "authorization",
        HeaderValue::from_static("Bearer resource-token"),
    );
    assert!(credentials(&bearer).is_err());
    let mut oversized = valid_headers();
    oversized.insert(
        "dpop",
        HeaderValue::from_str(&"a".repeat(MAX_COMPACT_JWS_BYTES + 1)).unwrap(),
    );
    assert!(credentials(&oversized).is_err());
}

#[test]
fn instance_auth_uses_the_same_viewer_credential_as_the_cloudfront_boundary() {
    let mut headers = valid_headers();
    headers.insert(
        "authorization",
        HeaderValue::from_static("AWS4-HMAC-SHA256 origin-signature"),
    );
    assert!(credentials(&headers).is_err());
    headers.insert(
        FORWARDED_AUTHORIZATION_HEADER,
        HeaderValue::from_static("DPoP viewer-token"),
    );
    assert_eq!(
        credentials(&headers).unwrap(),
        ("viewer-token", "request-proof")
    );
    // A forwarded human token must not select the instance credential from a
    // different header. Authentication and request dispatch see the same identity.
    headers.insert(
        "authorization",
        HeaderValue::from_static("DPoP other-token"),
    );
    headers.insert(
        FORWARDED_AUTHORIZATION_HEADER,
        HeaderValue::from_static("Bearer human-token"),
    );
    assert!(credentials(&headers).is_err());
}

#[test]
fn reserved_instance_jose_types_never_fall_through_to_human_authentication() {
    backend_jwt::init_for_tests();
    for typ in [
        jwt::JOSE_TYPE,
        WORKLOAD_ASSERTION_JWS_TYPE,
        INSTANCE_REGISTRATION_JWS_TYPE,
        "flow-like-instance-future+jwt",
    ] {
        let signed = backend_jwt::sign_typed(
            &serde_json::json!({"sub":"human-attribution","typ":"future_profile"}),
            typ,
        )
        .unwrap();
        assert!(devices::jwt::is_device_credential(&signed));
        assert!(devices::jwt::is_device_credential(&format!(
            "DPoP {signed}"
        )));
        assert!(jwt::verify(&signed).is_err());
        assert!(devices::jwt::verify_session(&signed).is_err());
    }
}
