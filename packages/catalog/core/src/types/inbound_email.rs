use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::flow_path::FlowPath;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MailSession {
    pub app_id: String,
    pub event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MailMessageRef {
    pub session: MailSession,
    /// Internal inbound delivery ID. The mail provider's ID stays on InboundEmail.
    pub delivery_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct InboundEmailAddress {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InboundEmailHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InboundEmailAttachment {
    #[serde(default)]
    pub filename: Option<String>,
    pub content_type: String,
    pub size: u64,
    pub path: FlowPath,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InboundEmailVerdict {
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct InboundEmailAuthentication {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub spam: Option<InboundEmailVerdict>,
    #[serde(default)]
    pub virus: Option<InboundEmailVerdict>,
    #[serde(default)]
    pub spf: Option<InboundEmailVerdict>,
    #[serde(default)]
    pub dkim: Option<InboundEmailVerdict>,
    #[serde(default)]
    pub dmarc: Option<InboundEmailVerdict>,
    #[serde(default)]
    pub dmarc_policy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InboundEmail {
    pub id: String,
    pub delivery_id: String,
    pub envelope_from: String,
    pub recipient: String,
    #[serde(default)]
    pub session: Option<MailSession>,
    #[serde(default)]
    pub reference: Option<MailMessageRef>,
    #[serde(default)]
    pub sender: Option<InboundEmailAddress>,
    #[serde(default)]
    pub from: Vec<InboundEmailAddress>,
    #[serde(default)]
    pub to: Vec<InboundEmailAddress>,
    #[serde(default)]
    pub cc: Vec<InboundEmailAddress>,
    #[serde(default)]
    pub reply_to: Vec<InboundEmailAddress>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub text_truncated: bool,
    #[serde(default)]
    pub html_truncated: bool,
    #[serde(default)]
    pub text_path: Option<FlowPath>,
    #[serde(default)]
    pub html_path: Option<FlowPath>,
    #[serde(default)]
    pub headers: Vec<InboundEmailHeader>,
    #[serde(default)]
    pub attachments: Vec<InboundEmailAttachment>,
    pub raw_path: FlowPath,
    #[serde(default)]
    pub received_at: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub authentication: Option<InboundEmailAuthentication>,
    /// Bounce, auto-reply, list or other machine-generated mail. Replies to it are refused.
    #[serde(default)]
    pub automated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::{
        Value,
        json::{self, json},
    };

    fn required_fields() -> Value {
        json!({
            "id": "mail-1", "delivery_id": "provider-1",
            "envelope_from": "bounce@example.com", "recipient": "orders@example.com",
            "raw_path": {"path": "tmp/mail/raw.eml", "store_ref": "__request_files__"}
        })
    }

    #[test]
    fn legacy_payloads_default_optional_fields_and_collections() {
        let email: InboundEmail = json::from_value(required_fields()).unwrap();
        assert!(email.sender.is_none());
        assert!(email.session.is_none() && email.reference.is_none());
        assert!(email.from.is_empty() && email.to.is_empty() && email.cc.is_empty());
        assert!(email.reply_to.is_empty() && email.headers.is_empty());
        assert!(email.attachments.is_empty());
        assert!(!email.text_truncated && !email.html_truncated);
        assert!(email.text_path.is_none() && email.html_path.is_none());
        assert!(email.authentication.is_none());
        assert!(!email.automated);
        assert_eq!(email.raw_path.store_ref, "__request_files__");
        let mut automated = required_fields();
        automated["automated"] = json!(true);
        assert!(
            json::from_value::<InboundEmail>(automated)
                .unwrap()
                .automated
        );
    }

    #[test]
    fn schema_and_serde_require_identity_and_typed_storage_paths() {
        let schema = json::to_value(schemars::schema_for!(InboundEmail)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(validator.is_valid(&required_fields()));
        let required = schema["required"].as_array().unwrap();
        for field in [
            "id",
            "delivery_id",
            "envelope_from",
            "recipient",
            "raw_path",
        ] {
            assert!(required.contains(&json!(field)));
            let mut missing = required_fields();
            missing.as_object_mut().unwrap().remove(field);
            assert!(!validator.is_valid(&missing));
            assert!(json::from_value::<InboundEmail>(missing).is_err());
        }
        let mut invalid = required_fields();
        invalid["raw_path"] = json!("tmp/mail/raw.eml");
        assert!(!validator.is_valid(&invalid));
        assert!(json::from_value::<InboundEmail>(invalid).is_err());

        let mut invalid = required_fields();
        invalid["attachments"] = json!([{
            "content_type": "application/pdf", "size": -1,
            "path": {"path": "tmp/mail/attachments/0", "store_ref": "__request_files__"}
        }]);
        assert!(!validator.is_valid(&invalid));
        assert!(json::from_value::<InboundEmail>(invalid).is_err());
    }

    #[test]
    fn authentication_retains_provider_verdict_objects() {
        let mut payload = required_fields();
        payload["authentication"] = json!({
            "provider": "ses", "spam": {"status": "PASS"},
            "dkim": {"status": "PASS"}, "dmarc_policy": "reject"
        });
        let email: InboundEmail = json::from_value(payload.clone()).unwrap();
        let authentication = email.authentication.unwrap();
        assert_eq!(authentication.provider.as_deref(), Some("ses"));
        assert_eq!(authentication.dkim.unwrap().status, "PASS");
        assert_eq!(authentication.dmarc_policy.as_deref(), Some("reject"));
        let schema = json::to_value(schemars::schema_for!(InboundEmail)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(validator.is_valid(&payload));
        payload["authentication"]["dkim"] = json!("PASS");
        assert!(!validator.is_valid(&payload));
        assert!(json::from_value::<InboundEmail>(payload).is_err());
    }

    #[test]
    fn mail_references_reject_extra_fields_and_preserve_internal_identity() {
        let reference = json!({
            "session": {"app_id": "app-1", "event_id": "event-1"},
            "delivery_id": "internal-mail-1"
        });
        let parsed: MailMessageRef = json::from_value(reference.clone()).unwrap();
        assert_eq!(parsed.delivery_id, "internal-mail-1");
        assert_eq!(json::to_value(&parsed).unwrap(), reference);
        let schema = json::to_value(schemars::schema_for!(MailMessageRef)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(validator.is_valid(&reference));
        let mut invalid = reference.clone();
        invalid["from"] = json!("spoof@example.com");
        assert!(!validator.is_valid(&invalid));
        assert!(json::from_value::<MailMessageRef>(invalid).is_err());
        let mut invalid = reference;
        invalid["session"]["headers"] = json!({"From": "spoof@example.com"});
        assert!(!validator.is_valid(&invalid));
        assert!(json::from_value::<MailMessageRef>(invalid).is_err());
    }
}
