use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Signer as _, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use url::{Host, Url};

use crate::{
    ClientAssertion, DpopProof, EnrollmentBinding, EnrollmentProof, MAX_ASSERTION_TTL_SECONDS,
    MAX_ENROLLMENT_TTL_SECONDS, OnboardingManifest, PROOF_CLOCK_SKEW_SECONDS, PROTOCOL_VERSION,
    ProtocolError, Result,
};

pub const ONBOARDING_JWS_TYPE: &str = "flow-like-device-onboarding+jwt";
pub const BINDING_JWS_TYPE: &str = "flow-like-device-enrollment-binding+jwt";
pub const CLIENT_ASSERTION_JWS_TYPE: &str = "flow-like-device-client-assertion+jwt";
pub const ENROLLMENT_PROOF_JWS_TYPE: &str = "flow-like-device-enrollment-proof+jwt";
pub const DPOP_JWS_TYPE: &str = "dpop+jwt";
pub const MAX_COMPACT_JWS_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum KeyType {
    #[serde(rename = "OKP")]
    Okp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Curve {
    Ed25519,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ed25519PublicKey {
    kty: KeyType,
    crv: Curve,
    x: String,
}

impl Ed25519PublicKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self> {
        validate_verifying_key(&bytes)?;
        Ok(Self {
            kty: KeyType::Okp,
            crv: Curve::Ed25519,
            x: URL_SAFE_NO_PAD.encode(bytes),
        })
    }

    pub fn to_bytes(&self) -> Result<[u8; 32]> {
        let bytes = decode_exact::<32>(&self.x)?;
        validate_verifying_key(&bytes)?;
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<()> {
        self.to_bytes().map(|_| ())
    }

    /// RFC 7638 and RFC 8037 require precisely these public members, in this order.
    pub fn thumbprint(&self) -> Result<String> {
        self.validate()?;
        let canonical = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#, self.x);
        Ok(digest(canonical.as_bytes()))
    }
}

/// Private key bytes must be persisted by the caller in protected local storage.
/// This type deliberately implements neither Debug nor Serialize.
pub struct SigningKey(ed25519_dalek::SigningKey);

impl SigningKey {
    pub fn generate() -> Self {
        Self(ed25519_dalek::SigningKey::generate(&mut OsRng))
    }

    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self(ed25519_dalek::SigningKey::from_bytes(bytes))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn public_key(&self) -> Ed25519PublicKey {
        Ed25519PublicKey {
            kty: KeyType::Okp,
            crv: Curve::Ed25519,
            x: URL_SAFE_NO_PAD.encode(self.0.verifying_key().as_bytes()),
        }
    }
}

#[derive(Serialize, Deserialize)]
enum SignatureAlgorithm {
    EdDSA,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedHeader {
    alg: SignatureAlgorithm,
    typ: String,
    kid: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DpopHeader {
    alg: SignatureAlgorithm,
    typ: String,
    jwk: Ed25519PublicKey,
}

pub struct EnrollmentProofContext<'a> {
    pub device_id: &'a str,
    pub endpoint: &'a str,
    pub challenge_id: &'a str,
    pub challenge_nonce: &'a str,
    pub binding_digest: &'a str,
    pub manifest_digest: &'a str,
    pub now: i64,
}

pub struct DpopContext<'a> {
    pub method: &'a str,
    /// Constructed from the configured public API origin and actual route, not Host headers.
    pub url: &'a str,
    pub access_token: Option<&'a str>,
    pub nonce: Option<&'a str>,
    /// The verified access token's cnf.jkt, or the registration thumbprint at issuance.
    pub key_thumbprint: &'a str,
    pub now: i64,
}

pub fn sign_manifest(manifest: &OnboardingManifest, key: &SigningKey) -> Result<String> {
    validate_manifest(manifest)?;
    if key.public_key() != manifest.controller_key {
        return Err(ProtocolError::KeyMismatch);
    }
    sign_pinned(manifest, key, ONBOARDING_JWS_TYPE)
}

pub fn verify_manifest(
    compact: &str,
    pinned: &Ed25519PublicKey,
    now: i64,
) -> Result<OnboardingManifest> {
    let manifest: OnboardingManifest = verify_pinned(compact, pinned, ONBOARDING_JWS_TYPE)?;
    validate_manifest(&manifest)?;
    if &manifest.controller_key != pinned {
        return Err(ProtocolError::KeyMismatch);
    }
    check_time(
        manifest.issued_at,
        manifest.issued_at,
        manifest.expires_at,
        MAX_ENROLLMENT_TTL_SECONDS,
        now,
    )?;
    Ok(manifest)
}

pub fn sign_binding(binding: &EnrollmentBinding, key: &SigningKey) -> Result<String> {
    validate_binding(binding)?;
    sign_pinned(binding, key, BINDING_JWS_TYPE)
}

pub fn verify_binding(
    compact: &str,
    pinned: &Ed25519PublicKey,
    now: i64,
) -> Result<EnrollmentBinding> {
    let binding: EnrollmentBinding = verify_pinned(compact, pinned, BINDING_JWS_TYPE)?;
    validate_binding(&binding)?;
    check_time(
        binding.issued_at,
        binding.issued_at,
        binding.expires_at,
        MAX_ASSERTION_TTL_SECONDS,
        now,
    )?;
    Ok(binding)
}

pub fn sign_client_assertion(claims: &ClientAssertion, key: &SigningKey) -> Result<String> {
    validate_assertion(claims)?;
    sign_pinned(claims, key, CLIENT_ASSERTION_JWS_TYPE)
}

/// Successful verification must be followed by an atomic replay-ID claim before issuance.
pub fn verify_client_assertion(
    compact: &str,
    registered_key: &Ed25519PublicKey,
    expected_device_id: &str,
    exact_endpoint: &str,
    now: i64,
) -> Result<ClientAssertion> {
    let claims: ClientAssertion =
        verify_pinned(compact, registered_key, CLIENT_ASSERTION_JWS_TYPE)?;
    validate_assertion(&claims)?;
    check_assertion_request(&claims, expected_device_id, exact_endpoint, now)?;
    Ok(claims)
}

pub fn sign_enrollment_proof(claims: &EnrollmentProof, key: &SigningKey) -> Result<String> {
    validate_enrollment_proof(claims)?;
    sign_pinned(claims, key, ENROLLMENT_PROOF_JWS_TYPE)
}

pub fn verify_enrollment_proof(
    compact: &str,
    auth_key: &Ed25519PublicKey,
    context: &EnrollmentProofContext<'_>,
) -> Result<EnrollmentProof> {
    let claims: EnrollmentProof = verify_pinned(compact, auth_key, ENROLLMENT_PROOF_JWS_TYPE)?;
    validate_enrollment_proof(&claims)?;
    check_assertion_request(
        &assertion_from_proof(&claims),
        context.device_id,
        context.endpoint,
        context.now,
    )?;
    if claims.challenge_id != context.challenge_id
        || claims.challenge_nonce != context.challenge_nonce
        || claims.binding_digest != context.binding_digest
        || claims.manifest_digest != context.manifest_digest
    {
        return Err(ProtocolError::BindingMismatch);
    }
    Ok(claims)
}

pub fn sign_dpop(claims: &DpopProof, key: &SigningKey) -> Result<String> {
    validate_dpop_shape(claims)?;
    let header = DpopHeader {
        alg: SignatureAlgorithm::EdDSA,
        typ: DPOP_JWS_TYPE.to_string(),
        jwk: key.public_key(),
    };
    sign_compact(&header, claims, key)
}

/// The API must atomically consume (key thumbprint, jti) across its replicas.
/// DPoP covers the method, URI and token hash; it does not sign request bodies.
pub fn verify_dpop(
    compact: &str,
    registered_key: &Ed25519PublicKey,
    context: &DpopContext<'_>,
) -> Result<DpopProof> {
    let segments = split_compact(compact)?;
    let header: DpopHeader = decode_json(segments[0], 2048)?;
    if header.typ != DPOP_JWS_TYPE {
        return Err(ProtocolError::Invalid("DPoP JOSE type"));
    }
    if header.jwk != *registered_key || header.jwk.thumbprint()? != context.key_thumbprint {
        return Err(ProtocolError::KeyMismatch);
    }
    verify_signature(&segments, registered_key)?;
    let claims: DpopProof = decode_json(segments[1], MAX_COMPACT_JWS_BYTES)?;
    validate_dpop_shape(&claims)?;
    if claims.htm != context.method || claims.htu != canonical_dpop_htu(context.url)? {
        return Err(ProtocolError::BindingMismatch);
    }
    if claims.ath != context.access_token.map(access_token_hash)
        || claims.nonce.as_deref() != context.nonce
    {
        return Err(ProtocolError::BindingMismatch);
    }
    let oldest = context
        .now
        .checked_sub(MAX_ASSERTION_TTL_SECONDS)
        .ok_or(ProtocolError::InvalidTime)?;
    let newest = context
        .now
        .checked_add(PROOF_CLOCK_SKEW_SECONDS)
        .ok_or(ProtocolError::InvalidTime)?;
    if claims.iat <= oldest || claims.iat > newest {
        return Err(ProtocolError::InvalidTime);
    }
    Ok(claims)
}

pub fn compact_digest(compact: &str) -> String {
    digest(compact.as_bytes())
}

pub fn access_token_hash(token: &str) -> String {
    digest(token.as_bytes())
}

pub fn canonical_api_base_url(value: &str) -> Result<String> {
    let url = secure_url(value)?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ProtocolError::Invalid("API base URL query or fragment"));
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Paths are relative to the configured base, including a possible /api/v1 prefix.
pub fn endpoint_url(base: &str, path: &str) -> Result<String> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains(['?', '#', '\\'])
        || path.chars().any(char::is_control)
    {
        return Err(ProtocolError::Invalid("endpoint path"));
    }
    let base = canonical_api_base_url(base)?;
    let raw = format!("{base}{path}");
    let parsed = secure_url(&raw)?;
    if parsed.as_str() != raw {
        return Err(ProtocolError::Invalid("noncanonical endpoint path"));
    }
    Ok(raw)
}

pub fn canonical_dpop_htu(value: &str) -> Result<String> {
    let mut url = secure_url(value)?;
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.into())
}

pub fn validate_manifest(manifest: &OnboardingManifest) -> Result<()> {
    if manifest.version != PROTOCOL_VERSION {
        return Err(ProtocolError::Invalid("manifest version"));
    }
    for id in [
        &manifest.enrollment_id,
        &manifest.device_id,
        &manifest.owner_id,
    ] {
        bounded_text(id, 256)?;
    }
    bounded_text(&manifest.name, 256)?;
    if canonical_api_base_url(&manifest.api_base_url)? != manifest.api_base_url {
        return Err(ProtocolError::Invalid("noncanonical API base URL"));
    }
    manifest.bootstrap_key.validate()?;
    manifest.controller_key.validate()?;
    manifest.owner_invitation_key.validate()?;
    if manifest.bootstrap_key == manifest.controller_key
        || manifest.bootstrap_key == manifest.owner_invitation_key
        || manifest.controller_key == manifest.owner_invitation_key
    {
        return Err(ProtocolError::Invalid(
            "onboarding keys must have separate purposes",
        ));
    }
    check_time_shape(
        manifest.issued_at,
        manifest.issued_at,
        manifest.expires_at,
        MAX_ENROLLMENT_TTL_SECONDS,
    )
}

fn validate_binding(binding: &EnrollmentBinding) -> Result<()> {
    if binding.version != PROTOCOL_VERSION {
        return Err(ProtocolError::Invalid("binding version"));
    }
    for id in [
        &binding.enrollment_id,
        &binding.device_id,
        &binding.challenge_id,
    ] {
        bounded_text(id, 256)?;
    }
    nonce(&binding.challenge_nonce)?;
    decode_exact::<32>(&binding.manifest_digest)?;
    binding.identity.validate()?;
    check_time_shape(
        binding.issued_at,
        binding.issued_at,
        binding.expires_at,
        MAX_ASSERTION_TTL_SECONDS,
    )
}

pub(crate) fn validate_assertion(claims: &ClientAssertion) -> Result<()> {
    bounded_text(&claims.iss, 256)?;
    if claims.iss != claims.sub {
        return Err(ProtocolError::BindingMismatch);
    }
    validate_jti(&claims.jti)?;
    if canonical_dpop_htu(&claims.aud)? != claims.aud {
        return Err(ProtocolError::Invalid("noncanonical assertion audience"));
    }
    check_time_shape(
        claims.iat,
        claims.nbf,
        claims.exp,
        MAX_ASSERTION_TTL_SECONDS,
    )
}

fn validate_enrollment_proof(claims: &EnrollmentProof) -> Result<()> {
    validate_assertion(&assertion_from_proof(claims))?;
    bounded_text(&claims.challenge_id, 256)?;
    nonce(&claims.challenge_nonce)?;
    decode_exact::<32>(&claims.binding_digest)?;
    decode_exact::<32>(&claims.manifest_digest)?;
    Ok(())
}

fn assertion_from_proof(claims: &EnrollmentProof) -> ClientAssertion {
    ClientAssertion {
        iss: claims.iss.clone(),
        sub: claims.sub.clone(),
        aud: claims.aud.clone(),
        iat: claims.iat,
        nbf: claims.nbf,
        exp: claims.exp,
        jti: claims.jti.clone(),
    }
}

pub(crate) fn check_assertion_request(
    claims: &ClientAssertion,
    client: &str,
    endpoint: &str,
    now: i64,
) -> Result<()> {
    if claims.iss != client
        || claims.sub != client
        || claims.aud != endpoint
        || canonical_dpop_htu(endpoint)? != endpoint
    {
        return Err(ProtocolError::BindingMismatch);
    }
    check_time(
        claims.iat,
        claims.nbf,
        claims.exp,
        MAX_ASSERTION_TTL_SECONDS,
        now,
    )
}

fn validate_dpop_shape(claims: &DpopProof) -> Result<()> {
    validate_jti(&claims.jti)?;
    if claims.htm.is_empty()
        || claims.htm.len() > 32
        || !claims.htm.bytes().all(|b| b.is_ascii_uppercase())
    {
        return Err(ProtocolError::Invalid("DPoP method"));
    }
    if canonical_dpop_htu(&claims.htu)? != claims.htu {
        return Err(ProtocolError::Invalid(
            "DPoP URI must omit query and fragment",
        ));
    }
    if claims.iat < 0 {
        return Err(ProtocolError::InvalidTime);
    }
    if let Some(hash) = &claims.ath {
        decode_exact::<32>(hash)?;
    }
    if let Some(value) = &claims.nonce {
        nonce(value)?;
    }
    Ok(())
}

pub(crate) fn sign_pinned(payload: &impl Serialize, key: &SigningKey, typ: &str) -> Result<String> {
    let header = PinnedHeader {
        alg: SignatureAlgorithm::EdDSA,
        typ: typ.to_string(),
        kid: key.public_key().thumbprint()?,
    };
    sign_compact(&header, payload, key)
}

fn sign_compact(
    header: &impl Serialize,
    payload: &impl Serialize,
    key: &SigningKey,
) -> Result<String> {
    let header =
        serde_json::to_vec(header).map_err(|_| ProtocolError::Invalid("JOSE header encoding"))?;
    let payload =
        serde_json::to_vec(payload).map_err(|_| ProtocolError::Invalid("JWS payload encoding"))?;
    let signed = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(payload)
    );
    if signed.len() + 87 > MAX_COMPACT_JWS_BYTES {
        return Err(ProtocolError::Invalid("JWS size"));
    }
    let signature = key.0.sign(signed.as_bytes());
    Ok(format!(
        "{signed}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

pub(crate) fn verify_pinned<T: DeserializeOwned>(
    compact: &str,
    pinned: &Ed25519PublicKey,
    typ: &str,
) -> Result<T> {
    let segments = split_compact(compact)?;
    let header: PinnedHeader = decode_json(segments[0], 1024)?;
    if header.typ != typ {
        return Err(ProtocolError::Invalid("JOSE type"));
    }
    if header.kid != pinned.thumbprint()? {
        return Err(ProtocolError::KeyMismatch);
    }
    verify_signature(&segments, pinned)?;
    decode_json(segments[1], MAX_COMPACT_JWS_BYTES)
}

fn verify_signature(segments: &[&str; 3], pinned: &Ed25519PublicKey) -> Result<()> {
    let bytes = pinned.to_bytes()?;
    let key = validate_verifying_key(&bytes)?;
    let signature = Signature::from_bytes(&decode_exact::<64>(segments[2])?);
    let signed = format!("{}.{}", segments[0], segments[1]);
    key.verify_strict(signed.as_bytes(), &signature)
        .map_err(|_| ProtocolError::InvalidSignature)
}

fn split_compact(compact: &str) -> Result<[&str; 3]> {
    if compact.len() > MAX_COMPACT_JWS_BYTES {
        return Err(ProtocolError::Invalid("JWS size"));
    }
    let mut segments = compact.split('.');
    let parts = [
        segments.next().unwrap_or_default(),
        segments.next().unwrap_or_default(),
        segments.next().unwrap_or_default(),
    ];
    if segments.next().is_some() || parts.iter().any(|part| part.is_empty()) {
        return Err(ProtocolError::Invalid("JWS compact encoding"));
    }
    Ok(parts)
}

fn decode_json<T: DeserializeOwned>(value: &str, max: usize) -> Result<T> {
    if value.len() > max {
        return Err(ProtocolError::Invalid("JWS component size"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ProtocolError::Invalid("base64url encoding"))?;
    if URL_SAFE_NO_PAD.encode(&bytes) != value {
        return Err(ProtocolError::Invalid("noncanonical base64url"));
    }
    serde_json::from_slice(&bytes).map_err(|_| ProtocolError::Invalid("JWS JSON schema"))
}

pub(crate) fn decode_exact<const N: usize>(value: &str) -> Result<[u8; N]> {
    if value.len() > (N * 4).div_ceil(3) {
        return Err(ProtocolError::Invalid("encoded key or digest size"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ProtocolError::Invalid("base64url encoding"))?;
    if URL_SAFE_NO_PAD.encode(&bytes) != value {
        return Err(ProtocolError::Invalid("noncanonical base64url"));
    }
    bytes
        .try_into()
        .map_err(|_| ProtocolError::Invalid("decoded key or digest size"))
}

fn validate_verifying_key(bytes: &[u8; 32]) -> Result<VerifyingKey> {
    let key = VerifyingKey::from_bytes(bytes)
        .map_err(|_| ProtocolError::Invalid("Ed25519 public key"))?;
    if key.is_weak() {
        return Err(ProtocolError::Invalid("weak Ed25519 public key"));
    }
    Ok(key)
}

fn digest(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
}

pub(crate) fn bounded_text(value: &str, max: usize) -> Result<()> {
    if value.is_empty()
        || value.len() > max
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(ProtocolError::Invalid("text field"));
    }
    Ok(())
}

fn nonce(value: &str) -> Result<()> {
    bounded_text(value, 256)?;
    if value.len() < 16 || !value.is_ascii() {
        return Err(ProtocolError::Invalid("nonce"));
    }
    Ok(())
}

fn validate_jti(value: &str) -> Result<()> {
    if value.len() < 16
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ProtocolError::Invalid("proof identifier"));
    }
    Ok(())
}

fn check_time_shape(iat: i64, nbf: i64, exp: i64, max_ttl: i64) -> Result<()> {
    let lifetime = exp.checked_sub(iat).ok_or(ProtocolError::InvalidTime)?;
    if iat < 0 || nbf < iat || nbf >= exp || !(1..=max_ttl).contains(&lifetime) {
        return Err(ProtocolError::InvalidTime);
    }
    Ok(())
}

fn check_time(iat: i64, nbf: i64, exp: i64, max_ttl: i64, now: i64) -> Result<()> {
    check_time_shape(iat, nbf, exp, max_ttl)?;
    let newest = now
        .checked_add(PROOF_CLOCK_SKEW_SECONDS)
        .ok_or(ProtocolError::InvalidTime)?;
    if iat > newest || nbf > newest || exp <= now {
        return Err(ProtocolError::InvalidTime);
    }
    Ok(())
}

fn secure_url(value: &str) -> Result<Url> {
    bounded_text(value, 2048)?;
    if value.contains('\\') {
        return Err(ProtocolError::Invalid("URL backslash"));
    }
    let url = Url::parse(value).map_err(|_| ProtocolError::Invalid("absolute URL"))?;
    let loopback = match url.host() {
        Some(Host::Domain("localhost")) => true,
        Some(Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST,
        _ => false,
    };
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ProtocolError::Invalid(
            "HTTPS required outside exact loopback hosts",
        ));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceIdentity;

    const NOW: i64 = 1_800_000_000;
    const DEVICE: &str = "device-123";
    const TOKEN_ENDPOINT: &str = "https://api.example.test/api/v1/devices/token";

    fn manifest(controller: &SigningKey, bootstrap: &SigningKey) -> OnboardingManifest {
        OnboardingManifest {
            version: 1,
            enrollment_id: "enrollment-123".into(),
            device_id: DEVICE.into(),
            owner_id: "owner-123".into(),
            name: "Local device".into(),
            api_base_url: "https://api.example.test/api/v1".into(),
            bootstrap_key: bootstrap.public_key(),
            controller_key: controller.public_key(),
            owner_invitation_key: SigningKey::generate().public_key(),
            issued_at: NOW,
            expires_at: NOW + MAX_ENROLLMENT_TTL_SECONDS,
        }
    }

    fn assertion() -> ClientAssertion {
        ClientAssertion {
            iss: DEVICE.into(),
            sub: DEVICE.into(),
            aud: TOKEN_ENDPOINT.into(),
            iat: NOW,
            nbf: NOW,
            exp: NOW + 60,
            jti: "0123456789abcdef-unique-proof".into(),
        }
    }

    #[test]
    fn thumbprint_matches_rfc8037_and_malformed_keys_fail_without_panicking() {
        let key: Ed25519PublicKey = serde_json::from_value(serde_json::json!({
            "kty": "OKP", "crv": "Ed25519", "x": "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo"
        }))
        .unwrap();
        assert_eq!(
            key.thumbprint().unwrap(),
            "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k"
        );
        assert!(
            serde_json::from_value::<Ed25519PublicKey>(serde_json::json!({
                "kty": "OKP", "crv": "Ed25519", "x": key.x, "d": "private-key"
            }))
            .is_err()
        );
        let mut short = key.clone();
        short.x = URL_SAFE_NO_PAD.encode([1; 3]);
        assert!(short.to_bytes().is_err());
        let mut large = key.clone();
        large.x = "a".repeat(10_000);
        assert!(large.thumbprint().is_err());
        let mut identity = [0; 32];
        identity[0] = 1;
        assert!(Ed25519PublicKey::from_bytes(identity).is_err());
        assert!(Ed25519PublicKey::from_bytes([0; 32]).is_err());
        assert!(crate::validate_management_key(&[0; 32]).is_err());
        assert!(crate::validate_management_key(&identity).is_err());
        assert!(
            crate::validate_management_key(&x25519_dalek::x25519(
                [1; 32],
                x25519_dalek::X25519_BASEPOINT_BYTES
            ))
            .is_ok()
        );
    }

    #[test]
    fn controller_signature_pins_manifest_and_key_purposes() {
        let controller = SigningKey::generate();
        let bootstrap = SigningKey::generate();
        let manifest = manifest(&controller, &bootstrap);
        let signed = sign_manifest(&manifest, &controller).unwrap();
        assert_eq!(
            verify_manifest(&signed, &controller.public_key(), NOW).unwrap(),
            manifest
        );
        assert!(verify_manifest(&signed, &bootstrap.public_key(), NOW).is_err());
        assert!(verify_manifest(&signed, &controller.public_key(), manifest.expires_at).is_err());
        let mut reused = manifest.clone();
        reused.owner_invitation_key = reused.controller_key.clone();
        assert!(sign_manifest(&reused, &controller).is_err());
        let mut segments = signed.split('.').map(str::to_owned).collect::<Vec<_>>();
        let mut changed = manifest;
        changed.device_id = "substituted-device".into();
        segments[1] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&changed).unwrap());
        assert!(verify_manifest(&segments.join("."), &controller.public_key(), NOW).is_err());
    }

    #[test]
    fn enrollment_proof_binds_both_signed_documents_and_exact_challenge_endpoint() {
        let controller = SigningKey::generate();
        let bootstrap = SigningKey::generate();
        let auth = SigningKey::generate();
        let manifest = manifest(&controller, &bootstrap);
        let manifest_jws = sign_manifest(&manifest, &controller).unwrap();
        let manifest_digest = compact_digest(&manifest_jws);
        let binding = EnrollmentBinding {
            version: 1,
            enrollment_id: manifest.enrollment_id.clone(),
            device_id: DEVICE.into(),
            identity: DeviceIdentity {
                auth_key: auth.public_key(),
                management_key: [42; 32],
                telemetry_key: SigningKey::generate().public_key(),
            },
            manifest_digest: manifest_digest.clone(),
            challenge_id: "challenge-123".into(),
            challenge_nonce: "server-random-nonce-0123456789".into(),
            issued_at: NOW,
            expires_at: NOW + 60,
        };
        let binding_jws = sign_binding(&binding, &bootstrap).unwrap();
        assert_eq!(
            verify_binding(&binding_jws, &bootstrap.public_key(), NOW).unwrap(),
            binding
        );
        let binding_digest = compact_digest(&binding_jws);
        let endpoint = "https://api.example.test/api/v1/devices/enrollments/enrollment-123/redeem";
        let proof = EnrollmentProof {
            iss: DEVICE.into(),
            sub: DEVICE.into(),
            aud: endpoint.into(),
            iat: NOW,
            nbf: NOW,
            exp: NOW + 60,
            jti: "0123456789abcdef-enrollment-proof".into(),
            challenge_id: binding.challenge_id.clone(),
            challenge_nonce: binding.challenge_nonce.clone(),
            binding_digest: binding_digest.clone(),
            manifest_digest: manifest_digest.clone(),
        };
        let signed = sign_enrollment_proof(&proof, &auth).unwrap();
        let mut context = EnrollmentProofContext {
            device_id: DEVICE,
            endpoint,
            challenge_id: &binding.challenge_id,
            challenge_nonce: &binding.challenge_nonce,
            binding_digest: &binding_digest,
            manifest_digest: &manifest_digest,
            now: NOW,
        };
        assert_eq!(
            verify_enrollment_proof(&signed, &auth.public_key(), &context).unwrap(),
            proof
        );
        context.endpoint = TOKEN_ENDPOINT;
        assert!(verify_enrollment_proof(&signed, &auth.public_key(), &context).is_err());
        context.endpoint = endpoint;
        let mut altered_binding = binding.clone();
        altered_binding.identity.management_key = [43; 32];
        let altered_digest = compact_digest(&sign_binding(&altered_binding, &bootstrap).unwrap());
        context.binding_digest = &altered_digest;
        assert!(verify_enrollment_proof(&signed, &auth.public_key(), &context).is_err());
        assert!(
            verify_client_assertion(&signed, &auth.public_key(), DEVICE, endpoint, NOW).is_err()
        );
    }

    #[test]
    fn assertions_reject_wrong_audience_client_clock_and_excess_lifetime() {
        let key = SigningKey::generate();
        let claims = assertion();
        let compact = sign_client_assertion(&claims, &key).unwrap();
        assert_eq!(
            verify_client_assertion(&compact, &key.public_key(), DEVICE, TOKEN_ENDPOINT, NOW)
                .unwrap(),
            claims
        );
        assert!(
            verify_client_assertion(
                &compact,
                &key.public_key(),
                "other-device",
                TOKEN_ENDPOINT,
                NOW
            )
            .is_err()
        );
        assert!(
            verify_client_assertion(
                &compact,
                &key.public_key(),
                DEVICE,
                "https://attacker.test/devices/token",
                NOW
            )
            .is_err()
        );
        assert!(
            verify_client_assertion(
                &compact,
                &key.public_key(),
                DEVICE,
                TOKEN_ENDPOINT,
                NOW + 60
            )
            .is_err()
        );
        assert!(
            verify_client_assertion(&compact, &key.public_key(), DEVICE, TOKEN_ENDPOINT, NOW - 6)
                .is_err()
        );
        let mut excessive = claims.clone();
        excessive.exp += 1;
        assert!(sign_client_assertion(&excessive, &key).is_err());
        let mut bad_nbf = claims;
        bad_nbf.nbf -= 1;
        assert!(sign_client_assertion(&bad_nbf, &key).is_err());
        assert!(check_time_shape(i64::MIN, 0, i64::MAX, 60).is_err());
    }

    #[test]
    fn signed_unknown_headers_and_algorithm_confusion_are_rejected() {
        let key = SigningKey::generate();
        let claims = assertion();
        let wrong_alg = serde_json::json!({ "alg": "HS256", "typ": CLIENT_ASSERTION_JWS_TYPE, "kid": key.public_key().thumbprint().unwrap() });
        let compact = sign_compact(&wrong_alg, &claims, &key).unwrap();
        assert!(
            verify_client_assertion(&compact, &key.public_key(), DEVICE, TOKEN_ENDPOINT, NOW)
                .is_err()
        );
        let injected_key = serde_json::json!({ "alg": "EdDSA", "typ": CLIENT_ASSERTION_JWS_TYPE,
            "kid": key.public_key().thumbprint().unwrap(), "jwk": key.public_key() });
        let compact = sign_compact(&injected_key, &claims, &key).unwrap();
        assert!(
            verify_client_assertion(&compact, &key.public_key(), DEVICE, TOKEN_ENDPOINT, NOW)
                .is_err()
        );
        let mut payload = serde_json::to_string(&claims).unwrap();
        payload.pop();
        payload.push_str(",\"sub\":\"another-device\"}");
        let header = serde_json::to_vec(&PinnedHeader {
            alg: SignatureAlgorithm::EdDSA,
            typ: CLIENT_ASSERTION_JWS_TYPE.into(),
            kid: key.public_key().thumbprint().unwrap(),
        })
        .unwrap();
        let signed = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header),
            URL_SAFE_NO_PAD.encode(payload)
        );
        let duplicate = format!(
            "{signed}.{}",
            URL_SAFE_NO_PAD.encode(key.0.sign(signed.as_bytes()).to_bytes())
        );
        assert!(
            verify_client_assertion(&duplicate, &key.public_key(), DEVICE, TOKEN_ENDPOINT, NOW)
                .is_err()
        );
        assert!(
            verify_client_assertion(
                &"a".repeat(MAX_COMPACT_JWS_BYTES + 1),
                &key.public_key(),
                DEVICE,
                TOKEN_ENDPOINT,
                NOW
            )
            .is_err()
        );
    }

    #[test]
    fn dpop_binds_method_resource_token_nonce_and_registered_key() {
        let key = SigningKey::generate();
        let token = "server.signed.access-token";
        let claims = DpopProof {
            jti: "0123456789abcdef-dpop-proof".into(),
            htm: "POST".into(),
            htu: "https://api.example.test/api/v1/devices/device-123/heartbeat".into(),
            iat: NOW,
            ath: Some(access_token_hash(token)),
            nonce: Some("server-nonce-0123456789".into()),
        };
        let compact = sign_dpop(&claims, &key).unwrap();
        let thumbprint = key.public_key().thumbprint().unwrap();
        let mut context = DpopContext {
            method: "POST",
            url: "https://api.example.test/api/v1/devices/device-123/heartbeat?ignored=query",
            access_token: Some(token),
            nonce: claims.nonce.as_deref(),
            key_thumbprint: &thumbprint,
            now: NOW,
        };
        assert_eq!(
            verify_dpop(&compact, &key.public_key(), &context).unwrap(),
            claims
        );
        context.method = "GET";
        assert!(verify_dpop(&compact, &key.public_key(), &context).is_err());
        context.method = "POST";
        context.access_token = Some("other.access.token");
        assert!(verify_dpop(&compact, &key.public_key(), &context).is_err());
        context.access_token = Some(token);
        context.nonce = None;
        assert!(verify_dpop(&compact, &key.public_key(), &context).is_err());
        context.nonce = claims.nonce.as_deref();
        context.key_thumbprint = "substituted-key";
        assert!(verify_dpop(&compact, &key.public_key(), &context).is_err());
        context.key_thumbprint = &thumbprint;
        context.now = NOW + 60;
        assert!(verify_dpop(&compact, &key.public_key(), &context).is_err());
    }

    #[test]
    fn dpop_does_not_accept_a_private_jwk_or_noncanonical_target() {
        let key = SigningKey::generate();
        let mut claims = DpopProof {
            jti: "0123456789abcdef-dpop-proof".into(),
            htm: "POST".into(),
            htu: TOKEN_ENDPOINT.into(),
            iat: NOW,
            ath: None,
            nonce: None,
        };
        let thumbprint = key.public_key().thumbprint().unwrap();
        let context = DpopContext {
            method: "POST",
            url: TOKEN_ENDPOINT,
            access_token: None,
            nonce: None,
            key_thumbprint: &thumbprint,
            now: NOW,
        };
        assert!(
            verify_dpop(
                &sign_dpop(&claims, &key).unwrap(),
                &key.public_key(),
                &context
            )
            .is_ok()
        );
        let mut jwk = serde_json::to_value(key.public_key()).unwrap();
        jwk["d"] = "private-material".into();
        let header = serde_json::json!({ "alg": "EdDSA", "typ": DPOP_JWS_TYPE, "jwk": jwk });
        let compact = sign_compact(&header, &claims, &key).unwrap();
        assert!(verify_dpop(&compact, &key.public_key(), &context).is_err());
        claims.htu.push_str("?query=is-not-signed");
        assert!(sign_dpop(&claims, &key).is_err());
    }

    #[test]
    fn endpoint_helpers_preserve_api_prefix_and_restrict_insecure_urls() {
        assert_eq!(
            endpoint_url("https://api.example.test/api/v1/", "/devices/token").unwrap(),
            TOKEN_ENDPOINT
        );
        assert_eq!(
            canonical_dpop_htu("https://API.example.test:443/api/v1/devices/token?q=1#fragment")
                .unwrap(),
            TOKEN_ENDPOINT
        );
        for base in [
            "http://localhost:8080/api/v1",
            "http://127.0.0.1:8080/api/v1",
            "http://[::1]:8080/api/v1",
        ] {
            assert!(canonical_api_base_url(base).is_ok());
        }
        for base in [
            "http://api.example.test",
            "http://127.0.0.2",
            "http://localhost.example.test",
            "https://user:password@api.example.test",
            "https://api.example.test?query=1",
            "https://api.example.test#fragment",
        ] {
            assert!(canonical_api_base_url(base).is_err());
        }
        assert!(endpoint_url("https://api.example.test/api/v1", "/../devices/token").is_err());
        assert!(
            endpoint_url(
                "https://api.example.test/api/v1",
                "//attacker.test/devices/token"
            )
            .is_err()
        );
    }
}
