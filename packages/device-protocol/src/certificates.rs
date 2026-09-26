use crate::{
    Ed25519PublicKey, ProtocolError, Result, SigningKey,
    proof::{sign_pinned, verify_pinned},
    validate_management_id,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_DEVICE_CERTIFICATES: usize = 32;
pub const MAX_CERTIFICATE_PEM_BYTES: usize = 12 * 1024;
const INVENTORY_TYPE: &str = "flow-like-device-certificate-inventory+jwt";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateBinding {
    pub placement_id: String,
    pub project_id: String,
    pub service: String,
}

/// Detailed identity metadata travels only through authenticated encrypted management.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateMetadata {
    pub certificate_id: String,
    pub label: String,
    pub revision: u64,
    pub subject: String,
    pub issuer: String,
    pub dns_names: Vec<String>,
    pub ip_addresses: Vec<String>,
    pub sha256_fingerprint: String,
    pub not_before: i64,
    pub not_after: i64,
    #[serde(default)]
    pub binding_count: usize,
    pub bindings: Vec<CertificateBinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificateRequestPurpose {
    Service,
    Issuer,
}

/// The private key remains on the requesting device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateSigningRequest {
    pub request_id: String,
    pub certificate_id: String,
    pub label: String,
    pub expected_revision: u64,
    pub dns_names: Vec<String>,
    pub ip_addresses: Vec<String>,
    pub csr_pem: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub purpose: CertificateRequestPurpose,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leaf_lifetime_days: Option<u16>,
}

/// Owner-approved local renewal policy. Issuer keys never reach project workloads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateIssuerMetadata {
    pub certificate_id: String,
    pub revision: u64,
    pub dns_names: Vec<String>,
    pub ip_addresses: Vec<String>,
    pub leaf_lifetime_days: u16,
    pub not_after: i64,
    pub last_renewed_at: Option<i64>,
    pub next_renewal_at: i64,
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcmeEnvironment {
    LetsEncryptStaging,
    LetsEncryptProduction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcmeCertificateMetadata {
    pub certificate_id: String,
    pub label: String,
    pub revision: u64,
    pub dns_names: Vec<String>,
    pub environment: AcmeEnvironment,
    pub http_bind: String,
    pub next_attempt_at: i64,
    pub last_renewed_at: Option<i64>,
    pub last_error: Option<String>,
}

/// The control plane receives only the fields needed to schedule expiry reminders.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateInventoryEntry {
    pub certificate_id: String,
    pub revision: u64,
    pub fingerprint_sha256: String,
    pub not_after: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateInventory {
    pub version: u32,
    pub device_id: String,
    pub revision: u64,
    pub issued_at: i64,
    pub certificates: Vec<CertificateInventoryEntry>,
}

pub fn validate_certificate_id(id: &str) -> Result<()> {
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_digit() || (b'a'..=b'f').contains(&c)
            }
        })
    {
        return Err(ProtocolError::Invalid(
            "certificate ID must be a canonical UUID",
        ));
    }
    Ok(())
}

pub fn validate_certificate_inventory(value: &CertificateInventory) -> Result<()> {
    validate_management_id(&value.device_id)?;
    if value.version != 1
        || value.revision == 0
        || value.revision > MAX_SAFE_INTEGER
        || value.issued_at <= 0
        || value.issued_at as u64 > MAX_SAFE_INTEGER
        || value.certificates.len() > MAX_DEVICE_CERTIFICATES
    {
        return Err(ProtocolError::Invalid("certificate inventory"));
    }
    let mut seen = HashSet::new();
    for item in &value.certificates {
        validate_certificate_id(&item.certificate_id)?;
        if !seen.insert(&item.certificate_id)
            || item.revision == 0
            || item.revision > MAX_SAFE_INTEGER
            || item.not_after <= 0
            || item.not_after as u64 > MAX_SAFE_INTEGER
            || item.fingerprint_sha256.len() != 64
            || !item
                .fingerprint_sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(ProtocolError::Invalid("certificate inventory entry"));
        }
    }
    Ok(())
}

pub fn sign_certificate_inventory(
    value: &CertificateInventory,
    key: &SigningKey,
) -> Result<String> {
    validate_certificate_inventory(value)?;
    sign_pinned(value, key, INVENTORY_TYPE)
}

pub fn verify_certificate_inventory(
    compact: &str,
    key: &Ed25519PublicKey,
    device_id: &str,
    now: i64,
) -> Result<CertificateInventory> {
    let value: CertificateInventory = verify_pinned(compact, key, INVENTORY_TYPE)?;
    validate_certificate_inventory(&value)?;
    if value.device_id != device_id {
        return Err(ProtocolError::BindingMismatch);
    }
    if value.issued_at > now.saturating_add(60) || value.issued_at < now.saturating_sub(300) {
        return Err(ProtocolError::InvalidTime);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_inventory_is_bound_bounded_and_contains_no_private_metadata() {
        let key = SigningKey::generate();
        let mut value = CertificateInventory {
            version: 1,
            device_id: "device".into(),
            revision: MAX_SAFE_INTEGER,
            issued_at: 100,
            certificates: (0..MAX_DEVICE_CERTIFICATES)
                .map(|i| CertificateInventoryEntry {
                    certificate_id: format!("00000000-0000-0000-0000-{i:012x}"),
                    revision: MAX_SAFE_INTEGER,
                    fingerprint_sha256: "a".repeat(64),
                    not_after: MAX_SAFE_INTEGER as i64,
                })
                .collect(),
        };
        let compact = sign_certificate_inventory(&value, &key).unwrap();
        assert_eq!(
            verify_certificate_inventory(&compact, &key.public_key(), "device", 101).unwrap(),
            value
        );
        assert!(verify_certificate_inventory(&compact, &key.public_key(), "other", 101).is_err());
        assert!(
            verify_certificate_inventory(
                &compact,
                &SigningKey::generate().public_key(),
                "device",
                101
            )
            .is_err()
        );
        assert!(verify_certificate_inventory(&compact, &key.public_key(), "device", 401).is_err());
        value.certificates.push(value.certificates[0].clone());
        assert!(sign_certificate_inventory(&value, &key).is_err());
        value.certificates.clear();
        assert!(sign_certificate_inventory(&value, &key).is_ok());
    }
}
