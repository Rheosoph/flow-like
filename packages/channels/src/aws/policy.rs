//! Naming and IAM session policies shared by the API (minting grants, forwarding) and the
//! waiter (validating what it was handed).

use flow_like_types::{Result, bail};
use serde_json::{Value, json};

pub const EXECUTOR_CLIENT_ID_PREFIX: &str = "run-";

/// AWS IoT limits: topics are at most 256 bytes with at most 7 forward slashes, client ids at
/// most 128 bytes.
const MAX_TOPIC_BYTES: usize = 256;
const MAX_TOPIC_SLASHES: usize = 7;
const MAX_CHANNEL_ID_BYTES: usize = 128 - EXECUTOR_CLIENT_ID_PREFIX.len();

pub fn topic_for(prefix: &str, channel_id: &str) -> String {
    format!("{}/{channel_id}/inbox", prefix.trim_matches('/'))
}

pub fn executor_client_id(channel_id: &str) -> String {
    format!("{EXECUTOR_CLIENT_ID_PREFIX}{channel_id}")
}

/// A channel id becomes a topic segment and part of a client id, so it must be a single
/// non-empty segment without wildcards, separators or whitespace.
pub fn validate_channel_id(channel_id: &str) -> Result<()> {
    if channel_id.is_empty() {
        bail!("channel id must not be empty");
    }
    if channel_id.len() > MAX_CHANNEL_ID_BYTES {
        bail!(
            "channel id '{channel_id}' is longer than {MAX_CHANNEL_ID_BYTES} bytes and would not fit an AWS IoT client id"
        );
    }
    if let Some(offending) = channel_id
        .chars()
        .find(|c| matches!(c, '/' | '+' | '#') || c.is_whitespace() || c.is_control())
    {
        bail!("channel id '{channel_id}' contains the forbidden character {offending:?}");
    }
    Ok(())
}

pub fn validate_topic(topic: &str) -> Result<()> {
    if topic.is_empty() {
        bail!("AWS IoT topic must not be empty");
    }
    if topic.len() > MAX_TOPIC_BYTES {
        bail!(
            "AWS IoT topic '{topic}' is {} bytes; the limit is {MAX_TOPIC_BYTES}",
            topic.len()
        );
    }
    if topic.starts_with('$') {
        bail!("AWS IoT topic '{topic}' uses the reserved '$' prefix");
    }
    if topic.contains(['+', '#']) {
        bail!("AWS IoT topic '{topic}' must not contain wildcards");
    }
    if topic.split('/').any(str::is_empty) {
        bail!("AWS IoT topic '{topic}' has an empty segment");
    }
    let slashes = topic.matches('/').count();
    if slashes > MAX_TOPIC_SLASHES {
        bail!("AWS IoT topic '{topic}' has {slashes} slashes; the limit is {MAX_TOPIC_SLASHES}");
    }
    Ok(())
}

/// Session policy for the client grant: it may deliver direct messages to exactly this waiter
/// on exactly this topic.
pub fn client_session_policy(
    region: &str,
    account_id: &str,
    target_client_id: &str,
    topic: &str,
) -> String {
    policy_document(vec![statement(
        "iot:SendDirectMessage",
        iot_arn(region, account_id, &format!("client/{target_client_id}")),
        Some(json!({ "StringEquals": { "iot:Topic": topic } })),
    )])
}

/// Session policy for the executor grant: connect as `client_id`, subscribe to and receive on
/// `inbox_topic`.
pub fn executor_session_policy(
    region: &str,
    account_id: &str,
    client_id: &str,
    inbox_topic: &str,
) -> String {
    policy_document(vec![
        statement(
            "iot:Connect",
            iot_arn(region, account_id, &format!("client/{client_id}")),
            None,
        ),
        statement(
            "iot:Subscribe",
            iot_arn(region, account_id, &format!("topicfilter/{inbox_topic}")),
            None,
        ),
        statement(
            "iot:Receive",
            iot_arn(region, account_id, &format!("topic/{inbox_topic}")),
            None,
        ),
    ])
}

fn partition_for(region: &str) -> &'static str {
    if region.starts_with("cn-") {
        "aws-cn"
    } else if region.starts_with("us-gov-") {
        "aws-us-gov"
    } else {
        "aws"
    }
}

fn iot_arn(region: &str, account_id: &str, resource: &str) -> String {
    format!(
        "arn:{}:iot:{region}:{account_id}:{resource}",
        partition_for(region)
    )
}

fn statement(action: &str, resource: String, condition: Option<Value>) -> Value {
    let mut statement = json!({
        "Effect": "Allow",
        "Action": action,
        "Resource": resource,
    });
    if let Some(condition) = condition {
        statement["Condition"] = condition;
    }
    statement
}

fn policy_document(statements: Vec<Value>) -> String {
    json!({ "Version": "2012-10-17", "Statement": statements }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_and_client_id_shapes() {
        assert_eq!(topic_for("runs", "abc"), "runs/abc/inbox");
        assert_eq!(topic_for("/flow/runs/", "abc"), "flow/runs/abc/inbox");
        assert_eq!(executor_client_id("abc"), "run-abc");
    }

    #[test]
    fn channel_id_validation() {
        assert!(validate_channel_id("clx1abc").is_ok());
        for bad in ["", "a/b", "a+b", "a#b", "a b", "a\nb"] {
            assert!(
                validate_channel_id(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
        assert!(validate_channel_id(&"x".repeat(MAX_CHANNEL_ID_BYTES)).is_ok());
        assert!(validate_channel_id(&"x".repeat(MAX_CHANNEL_ID_BYTES + 1)).is_err());
    }

    #[test]
    fn topic_validation() {
        assert!(validate_topic("runs/abc/inbox").is_ok());
        assert!(validate_topic("a/b/c/d/e/f/g/h").is_ok());
        assert!(validate_topic("a/b/c/d/e/f/g/h/i").is_err());
        assert!(validate_topic(&"x".repeat(MAX_TOPIC_BYTES)).is_ok());
        assert!(validate_topic(&"x".repeat(MAX_TOPIC_BYTES + 1)).is_err());
        for bad in [
            "",
            "runs/+/inbox",
            "runs/#",
            "$aws/things",
            "runs//inbox",
            "/runs",
        ] {
            assert!(validate_topic(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn client_policy_allows_only_direct_messages_on_one_topic() {
        let policy =
            client_session_policy("eu-central-1", "123456789012", "run-abc", "runs/abc/inbox");
        let document: Value = serde_json::from_str(&policy).unwrap();
        assert_eq!(document["Version"], "2012-10-17");
        let statements = document["Statement"].as_array().unwrap();
        assert_eq!(statements.len(), 1);
        assert_eq!(
            statements[0],
            json!({
                "Effect": "Allow",
                "Action": "iot:SendDirectMessage",
                "Resource": "arn:aws:iot:eu-central-1:123456789012:client/run-abc",
                "Condition": { "StringEquals": { "iot:Topic": "runs/abc/inbox" } }
            })
        );
    }

    #[test]
    fn executor_policy_allows_connect_subscribe_receive_only() {
        let policy =
            executor_session_policy("us-gov-west-1", "123456789012", "run-abc", "runs/abc/inbox");
        let document: Value = serde_json::from_str(&policy).unwrap();
        let statements = document["Statement"].as_array().unwrap();
        assert_eq!(
            statements,
            &vec![
                json!({
                    "Effect": "Allow",
                    "Action": "iot:Connect",
                    "Resource": "arn:aws-us-gov:iot:us-gov-west-1:123456789012:client/run-abc"
                }),
                json!({
                    "Effect": "Allow",
                    "Action": "iot:Subscribe",
                    "Resource": "arn:aws-us-gov:iot:us-gov-west-1:123456789012:topicfilter/runs/abc/inbox"
                }),
                json!({
                    "Effect": "Allow",
                    "Action": "iot:Receive",
                    "Resource": "arn:aws-us-gov:iot:us-gov-west-1:123456789012:topic/runs/abc/inbox"
                }),
            ]
        );
        assert!(!policy.contains("iot:Publish"));
        assert!(!policy.contains("iot:SendDirectMessage"));
        assert!(!policy.contains("Condition"));
    }

    #[test]
    fn china_partition() {
        assert!(
            client_session_policy("cn-north-1", "1", "c", "t")
                .contains("arn:aws-cn:iot:cn-north-1:1:client/c")
        );
    }
}
