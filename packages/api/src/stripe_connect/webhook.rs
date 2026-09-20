use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;

const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_SIGNATURE_HEADER_BYTES: usize = 8192;

#[derive(Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub account: Option<String>,
    pub livemode: bool,
    #[serde(default)]
    pub api_version: Option<String>,
    pub created: u64,
    pub data: EventData,
    #[serde(default)]
    pub request: Option<Value>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct EventData {
    pub object: Value,
    #[serde(default)]
    pub previous_attributes: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WebhookError {
    #[error("Stripe webhook signing secrets are not configured")]
    MissingSecret,
    #[error("malformed Stripe signature header")]
    MalformedSignature,
    #[error("Stripe signature timestamp is outside the permitted window")]
    TimestampOutsideTolerance,
    #[error("Stripe signature does not match")]
    SignatureMismatch,
    #[error("Stripe webhook body is too large")]
    BodyTooLarge,
    #[error("invalid Stripe event envelope")]
    InvalidEnvelope,
}

pub fn verify_webhook(
    body: &[u8],
    signature: &str,
    secrets: &[&str],
    now_secs: u64,
    tolerance_secs: u64,
) -> Result<EventEnvelope, WebhookError> {
    if secrets.is_empty() || secrets.iter().any(|secret| secret.is_empty()) {
        return Err(WebhookError::MissingSecret);
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(WebhookError::BodyTooLarge);
    }
    if signature.len() > MAX_SIGNATURE_HEADER_BYTES {
        return Err(WebhookError::MalformedSignature);
    }
    let mut timestamp = None;
    let mut candidates = Vec::new();
    for field in signature.split(',') {
        let (name, value) = field
            .trim()
            .split_once('=')
            .ok_or(WebhookError::MalformedSignature)?;
        match name {
            "t" => {
                if timestamp.is_some()
                    || value.is_empty()
                    || !value.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(WebhookError::MalformedSignature);
                }
                timestamp = Some((
                    value,
                    value
                        .parse::<u64>()
                        .map_err(|_| WebhookError::MalformedSignature)?,
                ));
            }
            "v1" => {
                // Invalid alternatives cannot make a valid rotation signature disappear.
                if value.len() == 64
                    && let Ok(bytes) = hex::decode(value)
                {
                    candidates.push(bytes);
                }
            }
            _ => {}
        }
    }
    let (timestamp_text, timestamp) = timestamp.ok_or(WebhookError::MalformedSignature)?;
    if candidates.is_empty() {
        return Err(WebhookError::MalformedSignature);
    }
    if now_secs.abs_diff(timestamp) > tolerance_secs {
        return Err(WebhookError::TimestampOutsideTolerance);
    }
    let mut matched = false;
    for secret in secrets {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
            .map_err(|_| WebhookError::MissingSecret)?;
        mac.update(timestamp_text.as_bytes());
        mac.update(b".");
        mac.update(body);
        for candidate in &candidates {
            matched |= mac.clone().verify_slice(candidate).is_ok();
        }
    }
    if !matched {
        return Err(WebhookError::SignatureMismatch);
    }
    let event: EventEnvelope =
        serde_json::from_slice(body).map_err(|_| WebhookError::InvalidEnvelope)?;
    if !super::request::valid_id(&event.id, "evt_")
        || event.event_type.is_empty()
        || !event.data.object.is_object()
        || event
            .account
            .as_ref()
            .is_some_and(|account| !super::request::valid_id(account, "acct_"))
    {
        return Err(WebhookError::InvalidEnvelope);
    }
    Ok(event)
}

#[cfg(test)]
pub fn sign_webhook(body: &[u8], secret: &str, timestamp: u64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!(
        "t={timestamp},v1={}",
        hex::encode(mac.finalize().into_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &[u8] = br#"{"id":"evt_test","type":"future.event","livemode":false,"created":1000,"data":{"object":{"id":"cs_test"}}}"#;

    #[test]
    fn verifies_raw_body_rotated_secrets_and_multiple_signatures() {
        let signature = format!(
            "{},v1={},v0=legacy",
            sign_webhook(BODY, "new", 1000),
            "00".repeat(32)
        );
        let event = verify_webhook(BODY, &signature, &["old", "new"], 1001, 300).unwrap();
        assert_eq!(event.event_type, "future.event");
        assert!(event.account.is_none());
        assert!(matches!(
            verify_webhook(b"{}", &signature, &["new"], 1000, 300),
            Err(WebhookError::SignatureMismatch)
        ));
    }

    #[test]
    fn rejects_past_future_duplicate_and_overflow_timestamps() {
        let signature = sign_webhook(BODY, "secret", 1000);
        for now in [699, 1301] {
            assert!(matches!(
                verify_webhook(BODY, &signature, &["secret"], now, 300),
                Err(WebhookError::TimestampOutsideTolerance)
            ));
        }
        assert!(
            verify_webhook(BODY, &format!("{signature},t=1000"), &["secret"], 1000, 300).is_err()
        );
        assert!(
            verify_webhook(
                BODY,
                &format!("t=18446744073709551616,v1={}", "00".repeat(32)),
                &["secret"],
                1000,
                300
            )
            .is_err()
        );
    }

    #[test]
    fn only_parses_envelope_after_authentication_and_keeps_unknown_fields() {
        let invalid = br#"{"id":"evt_test"}"#;
        assert!(matches!(
            verify_webhook(
                invalid,
                &sign_webhook(invalid, "secret", 1000),
                &["secret"],
                1000,
                0
            ),
            Err(WebhookError::InvalidEnvelope)
        ));
        assert!(matches!(
            verify_webhook(BODY, "t=1000,v1=bad", &["secret"], 1000, 300),
            Err(WebhookError::MalformedSignature)
        ));
        assert!(matches!(
            verify_webhook(BODY, &sign_webhook(BODY, "secret", 1000), &[], 1000, 300),
            Err(WebhookError::MissingSecret)
        ));
    }
}
