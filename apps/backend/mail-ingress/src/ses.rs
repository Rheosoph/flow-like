use crate::{IngestRequest, Result, S3Reference, invalid};
use serde::Deserialize;

#[derive(Clone)]
pub struct ReceiptPolicy {
    pub bucket: String,
    pub prefix: String,
    pub topic_arn: String,
}

#[derive(Debug, Deserialize)]
pub struct SnsEvent {
    #[serde(rename = "Records")]
    pub records: Vec<SnsRecord>,
}

#[derive(Debug, Deserialize)]
pub struct SnsRecord {
    #[serde(rename = "EventSource")]
    pub source: String,
    #[serde(rename = "Sns")]
    pub sns: SnsMessage,
}

#[derive(Debug, Deserialize)]
pub struct SnsMessage {
    #[serde(rename = "TopicArn")]
    pub topic_arn: String,
    #[serde(rename = "Message")]
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SesNotification {
    notification_type: String,
    pub mail: Mail,
    pub receipt: Receipt,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mail {
    pub message_id: String,
    pub source: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub recipients: Vec<String>,
    pub action: S3Action,
    pub spam_verdict: Option<serde_json::Value>,
    pub virus_verdict: Option<serde_json::Value>,
    pub spf_verdict: Option<serde_json::Value>,
    pub dkim_verdict: Option<serde_json::Value>,
    pub dmarc_verdict: Option<serde_json::Value>,
    pub dmarc_policy: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Action {
    #[serde(rename = "type")]
    pub kind: String,
    pub bucket_name: String,
    pub object_key: String,
    pub topic_arn: String,
}

impl ReceiptPolicy {
    pub fn parse(&self, record: &SnsRecord) -> Result<SesNotification> {
        if record.source != "aws:sns" || record.sns.topic_arn != self.topic_arn {
            return Err(invalid("Unexpected SNS event source or topic"));
        }
        let notification: SesNotification = serde_json::from_str(&record.sns.message)?;
        let action = &notification.receipt.action;
        let id = &notification.mail.message_id;
        // SES assigns the identifier. Never use the sender-controlled MIME
        // Message-ID or To/Cc headers for deduplication or recipient routing.
        if notification.notification_type != "Received"
            || action.kind != "S3"
            || action.bucket_name != self.bucket
            || action.topic_arn != self.topic_arn
            || id.is_empty()
            || id.len() > 256
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || action.object_key != format!("{}{id}", self.prefix)
            || notification.receipt.recipients.is_empty()
            || notification.receipt.recipients.len() > 100
        {
            return Err(invalid("Unexpected SES receipt metadata"));
        }
        Ok(notification)
    }
}

impl SesNotification {
    pub fn ingest_request(&self) -> IngestRequest {
        IngestRequest {
            source: "ses",
            delivery_id: format!("ses:{}", self.mail.message_id),
            envelope_from: self.mail.source.clone(),
            recipients: self.receipt.recipients.clone(),
            raw_mime_base64: None,
            s3: Some(S3Reference {
                bucket: self.receipt.action.bucket_name.clone(),
                key: self.receipt.action.object_key.clone(),
                version_id: None,
            }),
            authentication: Some(serde_json::json!({
                "provider": "ses",
                "spam": self.receipt.spam_verdict,
                "virus": self.receipt.virus_verdict,
                "spf": self.receipt.spf_verdict,
                "dkim": self.receipt.dkim_verdict,
                "dmarc": self.receipt.dmarc_verdict,
                "dmarc_policy": self.receipt.dmarc_policy,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (ReceiptPolicy, SnsRecord) {
        let policy = ReceiptPolicy {
            bucket: "mail-bucket".into(),
            prefix: "inbound/".into(),
            topic_arn: "arn:aws:sns:eu-west-1:123456789012:mail".into(),
        };
        let record = SnsRecord {
            source: "aws:sns".into(),
            sns: SnsMessage { topic_arn: policy.topic_arn.clone(), message: serde_json::json!({
                "notificationType": "Received",
                "mail": { "messageId": "ses-id", "source": "sender@example.test", "destination": ["ignored@example.test"] },
                "receipt": { "recipients": ["bcc@example.test"], "spamVerdict": { "status": "PASS" }, "action": {
                    "type": "S3", "bucketName": policy.bucket, "objectKey": "inbound/ses-id", "topicArn": policy.topic_arn
                }}
            }).to_string() },
        };
        (policy, record)
    }

    #[test]
    fn preserves_authoritative_envelope_and_verdicts() {
        let (policy, record) = fixture();
        let notification = policy.parse(&record).unwrap();
        let request = notification.ingest_request();
        assert_eq!(request.recipients, ["bcc@example.test"]);
        assert_eq!(request.delivery_id, "ses:ses-id");
        assert!(request.raw_mime_base64.is_none());
        assert_eq!(request.s3.unwrap().key, "inbound/ses-id");
        assert_eq!(request.authentication.unwrap()["spam"]["status"], "PASS");
    }

    #[test]
    fn rejects_untrusted_topics_and_object_paths() {
        let (policy, mut record) = fixture();
        record.sns.topic_arn.push_str("-other");
        assert!(policy.parse(&record).is_err());
        let (_, mut record) = fixture();
        let mut payload: serde_json::Value = serde_json::from_str(&record.sns.message).unwrap();
        for (field, value) in [
            ("bucketName", "other-bucket"),
            ("objectKey", "inbound/../secret"),
            ("type", "SNS"),
        ] {
            let original = payload["receipt"]["action"][field].clone();
            payload["receipt"]["action"][field] = value.into();
            record.sns.message = payload.to_string();
            assert!(policy.parse(&record).is_err(), "accepted {field}");
            payload["receipt"]["action"][field] = original;
        }
    }
}
