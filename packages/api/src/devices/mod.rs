//! Device registration authenticates an agent to the API. The registered public
//! identity binding remains independently verifiable by management controllers.

pub(crate) mod archives;
pub(crate) mod certificates;
pub(crate) mod fleet;
pub(crate) mod inventory;
pub(crate) mod jwt;
pub(crate) mod management;
pub(crate) mod readiness;
pub(crate) mod recovery;
pub(crate) mod repository;

use crate::{
    backend_jwt::{self, TokenType},
    error::ApiError,
    middleware::jwt::{AppUser, fresh_pat_permissions, viewer_authorization},
    permission::pat_permission::PatPermission,
    state::AppState,
};
use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like::hub::StandaloneConfig;
use flow_like_device_protocol::*;
use jwt::{Confirmation, EnrollmentClaims, SessionClaims};
use repository::{Device, Enrollment, Repository};
use std::result::Result;

/// Device registration needs registry storage and public policy only. Keeping
/// this boundary separate avoids initializing cloud workload services for it.
pub(crate) struct DeviceContext<'a> {
    pub(crate) db: &'a sea_orm::DatabaseConnection,
    pub(crate) dialect: crate::db::DbDialect,
    pub(crate) config: &'a StandaloneConfig,
    pub(crate) domain: &'a str,
    pub(crate) secure: bool,
}

pub(crate) fn context(state: &AppState) -> DeviceContext<'_> {
    DeviceContext {
        db: &state.db,
        dialect: state.db_dialect,
        config: &state.platform_config.standalone,
        domain: &state.platform_config.domain,
        secure: state.platform_config.secure,
    }
}

pub(crate) fn repository<'a>(state: &DeviceContext<'a>) -> Repository<'a> {
    Repository {
        db: state.db,
        dialect: state.dialect,
    }
}

pub(crate) fn enabled(state: &DeviceContext<'_>) -> Result<(), ApiError> {
    let config = state.config;
    if !config.enabled {
        return Err(ApiError::service_unavailable(
            "Standalone device enrollment is not enabled",
        ));
    }
    if !(1..=1000).contains(&config.max_devices_per_user)
        || !(1..=100).contains(&config.max_pending_enrollments_per_user)
        || !(60..=86_400).contains(&config.enrollment_ttl_seconds)
    {
        return Err(ApiError::service_unavailable(
            "Standalone device policy is invalid",
        ));
    }
    if !backend_jwt::is_configured() {
        return Err(ApiError::service_unavailable(
            "Backend device signing is not configured",
        ));
    }
    Ok(())
}

fn bad_protocol(_: ProtocolError) -> ApiError {
    ApiError::bad_request("Invalid device protocol input")
}
fn bad_proof(_: impl std::fmt::Display) -> ApiError {
    ApiError::unauthorized("Device proof is invalid or expired")
}

pub(crate) fn api_base_url(state: &DeviceContext<'_>) -> Result<String, ApiError> {
    if let Some(base) = &state.config.api_base_url {
        return canonical_api_base_url(base)
            .map_err(|_| ApiError::service_unavailable("Standalone API URL is invalid"));
    }
    let domain = state.domain.trim_end_matches('/');
    let origin = if domain.contains("://") {
        domain.to_owned()
    } else {
        format!("{}://{domain}", if state.secure { "https" } else { "http" })
    };
    let url = reqwest::Url::parse(&origin)
        .map_err(|_| ApiError::service_unavailable("Standalone API origin is invalid"))?;
    if url.path() != "/" && !url.path().is_empty() {
        return Err(ApiError::service_unavailable(
            "Set standalone.api_base_url when the API has a path prefix",
        ));
    }
    canonical_api_base_url(&format!("{origin}/api/v1"))
        .map_err(|_| ApiError::service_unavailable("Standalone API origin is invalid"))
}

fn endpoint(state: &DeviceContext<'_>, path: &str) -> Result<String, ApiError> {
    endpoint_url(&api_base_url(state)?, path).map_err(bad_protocol)
}

pub(crate) async fn human_owner(state: &AppState, user: &AppUser) -> Result<String, ApiError> {
    let owner = user.sub()?;
    if let AppUser::PAT(pat) = user {
        require_device_pat_permission(fresh_pat_permissions(pat, state).await?)?;
    }
    repository(&context(state)).active_account(&owner).await?;
    Ok(owner)
}

fn require_device_pat_permission(bits: i64) -> Result<(), ApiError> {
    // Until PATs have a dedicated Devices scope, restricted project or account
    // scopes must not gain device enrollment, inventory or revocation access.
    if !PatPermission::from_bits(bits)
        .is_some_and(|permissions| permissions.contains(PatPermission::All))
    {
        return Err(ApiError::forbidden(
            "Device registry access requires an unrestricted personal access token",
        ));
    }
    Ok(())
}

pub(crate) async fn create_enrollment(
    state: &DeviceContext<'_>,
    owner: &str,
    request: CreateEnrollmentRequest,
) -> Result<CreateEnrollmentResponse, ApiError> {
    enabled(state)?;
    let canonical = api_base_url(state)?;
    if canonical_api_base_url(&request.api_base_url).map_err(bad_protocol)? != canonical {
        return Err(ApiError::bad_request(
            "Enrollment API URL does not match this server",
        ));
    }
    let now = chrono::Utc::now().timestamp();
    let manifest = OnboardingManifest {
        version: PROTOCOL_VERSION,
        enrollment_id: uuid::Uuid::new_v4().to_string(),
        device_id: uuid::Uuid::new_v4().to_string(),
        owner_id: owner.to_owned(),
        name: request.name,
        api_base_url: canonical,
        bootstrap_key: request.bootstrap_key,
        controller_key: request.controller_key,
        owner_invitation_key: request.owner_invitation_key,
        issued_at: now,
        expires_at: now + state.config.enrollment_ttl_seconds as i64,
    };
    validate_manifest(&manifest).map_err(bad_protocol)?;
    let claims = EnrollmentClaims {
        sub: owner.to_owned(),
        enrollment_id: manifest.enrollment_id.clone(),
        device_id: manifest.device_id.clone(),
        cnf: Confirmation {
            jkt: manifest.bootstrap_key.thumbprint().map_err(bad_protocol)?,
        },
        scope: "device:enroll".into(),
        typ: TokenType::DeviceEnrollment,
        iss: backend_jwt::issuer().into(),
        aud: TokenType::DeviceEnrollment.audience().into(),
        iat: now,
        nbf: now,
        exp: manifest.expires_at,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    // Signing cannot fail after the record is committed and strand its reservation.
    let token = backend_jwt::sign_typed(&claims, jwt::ENROLLMENT_JOSE_TYPE)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    repository(state)
        .create_enrollment(
            &manifest,
            &claims.jti,
            state.config.max_devices_per_user,
            state.config.max_pending_enrollments_per_user,
        )
        .await?;
    Ok(CreateEnrollmentResponse {
        enrollment_token: token,
        manifest,
    })
}

async fn pending_enrollment(
    state: &DeviceContext<'_>,
    id: &str,
    token: &str,
) -> Result<(EnrollmentClaims, Enrollment), ApiError> {
    enabled(state)?;
    if token.len() > MAX_COMPACT_JWS_BYTES {
        return Err(ApiError::UNAUTHORIZED);
    }
    let claims = jwt::verify_enrollment(token).map_err(bad_proof)?;
    if claims.enrollment_id != id {
        return Err(ApiError::UNAUTHORIZED);
    }
    let stored = repository(state).enrollment(id).await?;
    let manifest = &stored.manifest;
    if stored.status != "pending"
        || stored.jwt_id != claims.jti
        || manifest.owner_id != claims.sub
        || manifest.device_id != claims.device_id
        || manifest.expires_at != claims.exp
        || manifest.issued_at != claims.iat
        || manifest.bootstrap_key.thumbprint().map_err(bad_proof)? != claims.cnf.jkt
        || manifest.api_base_url != api_base_url(state)?
    {
        return Err(ApiError::unauthorized(
            "Enrollment is no longer pending or does not match this server",
        ));
    }
    Ok((claims, stored))
}

pub(crate) async fn challenge(
    state: &DeviceContext<'_>,
    id: &str,
    request: ChallengeRequest,
) -> Result<EnrollmentChallenge, ApiError> {
    let (claims, _) = pending_enrollment(state, id, &request.enrollment_token).await?;
    let now = chrono::Utc::now().timestamp();
    let nonce = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let response = EnrollmentChallenge {
        challenge_id: uuid::Uuid::new_v4().to_string(),
        nonce,
        expires_at: (now + 60).min(claims.exp),
    };
    repository(state)
        .challenge(
            id,
            &claims.jti,
            &response.challenge_id,
            &compact_digest(&response.nonce),
            response.expires_at,
            now,
        )
        .await?;
    Ok(response)
}

pub(crate) async fn redeem(
    state: &DeviceContext<'_>,
    id: &str,
    request: RedeemEnrollmentRequest,
) -> Result<DeviceReceipt, ApiError> {
    let (claims, stored) = pending_enrollment(state, id, &request.enrollment_token).await?;
    let now = chrono::Utc::now().timestamp();
    let manifest = verify_manifest(&request.manifest_jws, &stored.manifest.controller_key, now)
        .map_err(bad_proof)?;
    if manifest != stored.manifest {
        return Err(ApiError::unauthorized(
            "Signed onboarding manifest does not match its authorization",
        ));
    }
    let binding =
        verify_binding(&request.binding_jws, &manifest.bootstrap_key, now).map_err(bad_proof)?;
    for authorization_key in [
        &manifest.bootstrap_key,
        &manifest.controller_key,
        &manifest.owner_invitation_key,
    ] {
        if binding.identity.auth_key == *authorization_key
            || binding.identity.telemetry_key == *authorization_key
            || binding.identity.management_key == authorization_key.to_bytes().map_err(bad_proof)?
        {
            return Err(ApiError::unauthorized(
                "Permanent device keys must be separate from onboarding and controller keys",
            ));
        }
    }
    let manifest_digest = compact_digest(&request.manifest_jws);
    if binding.enrollment_id != id
        || binding.device_id != manifest.device_id
        || binding.manifest_digest != manifest_digest
        || binding.expires_at > manifest.expires_at
    {
        return Err(ApiError::unauthorized(
            "Device identity binding does not match enrollment",
        ));
    }
    let binding_digest = compact_digest(&request.binding_jws);
    let endpoint = endpoint(state, &format!("/devices/enrollments/{id}/redeem"))?;
    let proof = verify_enrollment_proof(
        &request.proof_jws,
        &binding.identity.auth_key,
        &EnrollmentProofContext {
            device_id: &manifest.device_id,
            endpoint: &endpoint,
            challenge_id: &binding.challenge_id,
            challenge_nonce: &binding.challenge_nonce,
            binding_digest: &binding_digest,
            manifest_digest: &manifest_digest,
            now,
        },
    )
    .map_err(bad_proof)?;
    let receipt = DeviceReceipt {
        enrollment_id: id.to_owned(),
        device_id: manifest.device_id,
        owner_id: manifest.owner_id,
        name: manifest.name,
        identity: binding.identity,
        manifest_jws: request.manifest_jws,
        binding_jws: request.binding_jws,
        registered_at: now,
        auth_epoch: 1,
    };
    repository(state)
        .redeem(
            &receipt,
            &claims.jti,
            &binding.challenge_id,
            &compact_digest(&binding.challenge_nonce),
            proof.exp.min(binding.expires_at).min(claims.exp),
            state.config.max_devices_per_user,
        )
        .await
}

pub(crate) async fn registered_assertion(
    state: &DeviceContext<'_>,
    id: &str,
    assertion: &str,
    path: &str,
) -> Result<Device, ApiError> {
    enabled(state)?;
    let registered = repository(state).device(id).await?;
    let proof = verify_client_assertion(
        assertion,
        &registered.status.identity.auth_key,
        id,
        &endpoint(state, path)?,
        chrono::Utc::now().timestamp(),
    )
    .map_err(bad_proof)?;
    repository(state)
        .authorize_proof(
            id,
            registered.status.auth_epoch,
            &proof.jti,
            proof.exp,
            false,
        )
        .await
}

pub(crate) async fn token(
    state: &DeviceContext<'_>,
    request: DeviceTokenRequest,
) -> Result<DeviceTokenResponse, ApiError> {
    let registered = registered_assertion(
        state,
        &request.device_id,
        &request.client_assertion,
        "/devices/token",
    )
    .await?;
    let now = chrono::Utc::now().timestamp();
    let claims = SessionClaims {
        sub: registered.status.device_id.clone(),
        device_id: registered.status.device_id,
        owner_id: registered.status.owner_id,
        auth_epoch: registered.status.auth_epoch,
        cnf: Confirmation {
            jkt: registered
                .status
                .identity
                .auth_key
                .thumbprint()
                .map_err(bad_proof)?,
        },
        scope: "device:read device:presence".into(),
        typ: TokenType::DeviceSession,
        iss: backend_jwt::issuer().into(),
        aud: TokenType::DeviceSession.audience().into(),
        iat: now,
        nbf: now,
        exp: now + jwt::SESSION_SECONDS,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    Ok(DeviceTokenResponse {
        access_token: backend_jwt::sign_typed(&claims, jwt::SESSION_JOSE_TYPE)
            .map_err(|error| ApiError::internal(error.to_string()))?,
        token_type: "DPoP".into(),
        expires_in: jwt::SESSION_SECONDS as u64,
        expires_at: claims.exp,
    })
}

pub(crate) async fn recover_receipt(
    state: &DeviceContext<'_>,
    id: &str,
    request: ReceiptRequest,
) -> Result<DeviceReceipt, ApiError> {
    // The consumed enrollment never becomes pending again. Only its permanent
    // registered key can recover the receipt, even after the package JWT expires.
    Ok(registered_assertion(
        state,
        id,
        &request.client_assertion,
        &format!("/devices/{id}/receipt"),
    )
    .await?
    .receipt)
}

/// A device identity grants access only to the explicitly selected device routes.
/// It is deliberately not representable as an AppUser or a human subject.
pub(crate) struct DevicePrincipal {
    pub status: DeviceStatus,
}

fn proof_credentials(headers: &HeaderMap) -> Result<(&str, &str), ApiError> {
    if headers.get_all("dpop").iter().count() != 1
        || headers.get_all("authorization").iter().count() > 1
        || headers
            .get_all(crate::middleware::jwt::FORWARDED_AUTHORIZATION_HEADER)
            .iter()
            .count()
            > 1
    {
        return Err(ApiError::unauthorized(
            "Exactly one device request proof and authorization value are required",
        ));
    }
    let token = viewer_authorization(headers)
        .and_then(|value| value.strip_prefix("DPoP "))
        .ok_or(ApiError::UNAUTHORIZED)?;
    if token.len() > MAX_COMPACT_JWS_BYTES {
        return Err(ApiError::UNAUTHORIZED);
    }
    let dpop = headers
        .get("dpop")
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::UNAUTHORIZED)?;
    if dpop.len() > MAX_COMPACT_JWS_BYTES {
        return Err(ApiError::UNAUTHORIZED);
    }
    Ok((token, dpop))
}

pub(crate) async fn device_principal(
    state: &DeviceContext<'_>,
    id: &str,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    heartbeat: bool,
) -> Result<DevicePrincipal, ApiError> {
    enabled(state)?;
    let (token, dpop) = proof_credentials(headers)?;
    let claims = jwt::verify_session(token).map_err(bad_proof)?;
    if claims.device_id != id {
        return Err(ApiError::FORBIDDEN);
    }
    let registered = repository(state).device(id).await?;
    if claims.owner_id != registered.status.owner_id
        || claims.auth_epoch != registered.status.auth_epoch
    {
        return Err(ApiError::UNAUTHORIZED);
    }
    let now = chrono::Utc::now().timestamp();
    let proof = verify_dpop(
        dpop,
        &registered.status.identity.auth_key,
        &DpopContext {
            method,
            url: &endpoint(state, path)?,
            access_token: Some(token),
            nonce: None,
            key_thumbprint: &claims.cnf.jkt,
            now,
        },
    )
    .map_err(bad_proof)?;
    let current = repository(state)
        .authorize_proof(
            id,
            claims.auth_epoch,
            &proof.jti,
            (proof.iat + MAX_ASSERTION_TTL_SECONDS).min(claims.exp),
            heartbeat,
        )
        .await?;
    Ok(DevicePrincipal {
        status: current.status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn device_registry_requires_current_unrestricted_pat_permissions() {
        for permissions in [
            PatPermission::empty(),
            PatPermission::Projects,
            PatPermission::Teams,
            PatPermission::Billing,
            PatPermission::all() & !PatPermission::All,
        ] {
            assert_eq!(
                require_device_pat_permission(permissions.bits())
                    .unwrap_err()
                    .status(),
                axum::http::StatusCode::FORBIDDEN
            );
        }
        assert!(require_device_pat_permission(PatPermission::All.bits()).is_ok());
        assert!(
            require_device_pat_permission((PatPermission::All | PatPermission::Projects).bits())
                .is_ok()
        );
        assert!(require_device_pat_permission(-1).is_err());
        assert!(require_device_pat_permission(PatPermission::All.bits() | (1 << 62)).is_err());
    }

    #[test]
    fn dpop_admission_requires_one_proof_and_one_selected_authorization() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("DPoP access-token"),
        );
        assert!(proof_credentials(&headers).is_err());
        headers.insert("dpop", HeaderValue::from_static("signed-proof"));
        assert_eq!(
            proof_credentials(&headers).unwrap(),
            ("access-token", "signed-proof")
        );
        headers.append("dpop", HeaderValue::from_static("signed-proof"));
        assert!(proof_credentials(&headers).is_err());
        headers.remove("dpop");
        headers.insert("dpop", HeaderValue::from_static("signed-proof"));
        headers.append(
            "authorization",
            HeaderValue::from_static("DPoP access-token"),
        );
        assert!(proof_credentials(&headers).is_err());

        // The AWS origin signature and its single forwarded viewer credential
        // are separate header names with defined precedence.
        headers.insert(
            "authorization",
            HeaderValue::from_static("AWS4-HMAC-SHA256 origin-signature"),
        );
        headers.insert(
            crate::middleware::jwt::FORWARDED_AUTHORIZATION_HEADER,
            HeaderValue::from_static("DPoP forwarded-token"),
        );
        assert_eq!(
            proof_credentials(&headers).unwrap(),
            ("forwarded-token", "signed-proof")
        );
        headers.append(
            crate::middleware::jwt::FORWARDED_AUTHORIZATION_HEADER,
            HeaderValue::from_static("DPoP forwarded-token"),
        );
        assert!(proof_credentials(&headers).is_err());
    }
}

#[cfg(test)]
mod service_tests;
