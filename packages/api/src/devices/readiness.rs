use crate::{
    backend_jwt::{self, TokenType},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use flow_like::hub::StandaloneConfig;
use flow_like_device_protocol::{Ed25519PublicKey, validate_release_url};
use sea_orm::ConnectionTrait;
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct SetupReadiness {
    version: u32,
    ready: bool,
    checks: Vec<SetupCheck>,
}

#[derive(Serialize)]
struct SetupCheck {
    id: &'static str,
    ready: bool,
    message: &'static str,
}

fn check(
    id: &'static str,
    ready: bool,
    success: &'static str,
    failure: &'static str,
) -> SetupCheck {
    SetupCheck {
        id,
        ready,
        message: if ready { success } else { failure },
    }
}

fn release_configured(config: &StandaloneConfig) -> bool {
    let Some(release) = &config.release_trust else {
        return false;
    };
    validate_release_url(&release.manifest_url).is_ok()
        && (1..=8).contains(&release.public_keys.len())
        && release.minimum_sequence <= 9_007_199_254_740_991
        && release.public_keys.iter().all(|key| {
            serde_json::from_value::<Ed25519PublicKey>(serde_json::json!({
                "kty": "OKP", "crv": "Ed25519", "x": key,
            }))
            .is_ok_and(|public| public.validate().is_ok())
        })
        && release
            .public_keys
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == release.public_keys.len()
}

fn signaling_configured(urls: &[String]) -> bool {
    !urls.is_empty()
        && urls.len() <= 4
        && urls.iter().all(|value| {
            reqwest::Url::parse(value).is_ok_and(|url| {
                url.scheme() == "wss"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
            })
        })
}

fn signing_works() -> bool {
    let now = chrono::Utc::now().timestamp();
    let claims = serde_json::json!({"typ": TokenType::DeviceSession, "iss": backend_jwt::issuer(),
        "aud": TokenType::DeviceSession.audience(), "sub": "device-setup-probe", "iat": now, "nbf": now, "exp": now + 30});
    backend_jwt::sign_typed(&claims, "flow-like-device-setup-probe+jwt")
        .and_then(|token| {
            backend_jwt::verify_typed::<serde_json::Value>(
                &token,
                TokenType::DeviceSession,
                "flow-like-device-setup-probe+jwt",
            )
        })
        .is_ok()
}

pub(crate) async fn get(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<SetupReadiness>, ApiError> {
    super::human_owner(&state, &user).await?;
    let config = &state.platform_config.standalone;
    let policy = config.enabled
        && (1..=1000).contains(&config.max_devices_per_user)
        && (1..=100).contains(&config.max_pending_enrollments_per_user)
        && (60..=86_400).contains(&config.enrollment_ttl_seconds);
    let mut checks = vec![
        check(
            "policy",
            policy,
            "Device enrollment is enabled.",
            "The hub must enable a valid standalone enrollment policy.",
        ),
        check(
            "signing",
            signing_works(),
            "Device authentication signing is configured.",
            "The hub's backend signing keys are missing, invalid, or do not match.",
        ),
        check(
            "api",
            super::api_base_url(&super::context(&state)).is_ok(),
            "The public device API address is configured.",
            "The hub must configure a valid public device API address.",
        ),
        check(
            "signaling",
            signaling_configured(
                state
                    .platform_config
                    .signaling
                    .as_deref()
                    .unwrap_or_default(),
            ),
            "Secure signaling endpoints are configured. Connectivity is checked when connecting to a device.",
            "The hub must configure one to four secure signaling endpoints.",
        ),
        check(
            "release",
            release_configured(config),
            "Release trust is configured. This app will verify the signed release before creating a package.",
            "The hub must configure a release manifest URL and trusted release public keys.",
        ),
    ];
    // Select the columns used by the device lifecycle. A table existing alone
    // does not prove that a newer migration was applied. Each read is separate
    // so one missing table cannot abort the other checks' transaction.
    let queries = [
        r#"SELECT "authEpoch",receipt FROM "ManagedDevice" LIMIT 0"#,
        r#"SELECT "jwtId",manifest FROM "DeviceEnrollment" LIMIT 0"#,
        r#"SELECT "policyJws",version FROM "DeviceManagementPolicy" LIMIT 0"#,
        r#"SELECT ciphertext,revision FROM "DeviceControllerVault" LIMIT 0"#,
        r#"SELECT "scopeKey",revision,payload FROM "DeviceInventoryObservation" LIMIT 0"#,
        r#"SELECT revision,"readerJws",deleted FROM "DeviceFleetReader" LIMIT 0"#,
        r#"SELECT sequence,digest FROM "DeviceFleetHead" LIMIT 0"#,
        r#"SELECT "streamId","manifestJws",bundle FROM "DeviceFleetSnapshot" LIMIT 0"#,
        r#"SELECT "authzVersion","delegatingUserId" FROM "PlacementResourceGrant" LIMIT 0"#,
        r#"SELECT "limitMicros","payerId" FROM "PlacementBillingGrant" LIMIT 0"#,
        r#"SELECT purpose,"leaseExpiresAt" FROM "WorkloadInstance" LIMIT 0"#,
        r#"SELECT digest,result FROM "InstanceOfflineReceipt" LIMIT 0"#,
    ];
    let schema_ready = futures::future::join_all(
        queries
            .into_iter()
            .map(|query| state.db.execute_unprepared(query)),
    )
    .await
    .into_iter()
    .all(|result| result.is_ok());
    checks.push(check("database", schema_ready, "The device database schema is ready.", "The device database schema is incomplete or unavailable. Apply the device, resource, validation, offline-replay, inventory and fleet migrations."));
    Ok(Json(SetupReadiness {
        version: 1,
        ready: checks.iter().all(|check| check.ready),
        checks,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::SigningKey;

    #[test]
    fn release_trust_requires_real_unique_keys_and_canonical_https() {
        let mut config = StandaloneConfig::default();
        assert!(!release_configured(&config));
        let key = serde_json::to_value(SigningKey::generate().public_key()).unwrap()["x"]
            .as_str()
            .unwrap()
            .to_owned();
        config.release_trust = Some(
            serde_json::from_value(serde_json::json!({
                "manifest_url": "https://releases.example/release.jws",
                "public_keys": [key.clone()],
                "minimum_sequence": 1,
            }))
            .unwrap(),
        );
        assert!(release_configured(&config));
        config.release_trust.as_mut().unwrap().public_keys.push(key);
        assert!(!release_configured(&config));
        config.release_trust.as_mut().unwrap().public_keys.pop();
        config.release_trust.as_mut().unwrap().manifest_url =
            "http://releases.example/release.jws".into();
        assert!(!release_configured(&config));
    }

    #[test]
    fn signaling_requires_secure_bounded_endpoints_without_credentials() {
        assert!(!signaling_configured(&[]));
        assert!(signaling_configured(&["wss://signals.example".into()]));
        for value in [
            "ws://signals.example",
            "wss://user:password@signals.example",
            "wss://signals.example?token=secret",
            "wss://signals.example#fragment",
        ] {
            assert!(!signaling_configured(&[value.into()]));
        }
        assert!(!signaling_configured(&vec![
            "wss://signals.example".into();
            5
        ]));
    }
}
