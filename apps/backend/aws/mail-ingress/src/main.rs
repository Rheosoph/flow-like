use flow_like_mail_ingress::{
    ApiClient, IngestApi, Result, invalid,
    ses::{ReceiptPolicy, SnsEvent},
};
use lambda_runtime::{LambdaEvent, run, service_fn};
use serde_json::Value;
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

mod config;

const DEADLINE_MARGIN: Duration = Duration::from_secs(5);

struct Handler {
    api: ApiClient,
    policy: ReceiptPolicy,
    dispatch_rule_arn: String,
}

fn is_dispatch_event(event: &Value, rule_arn: &str) -> bool {
    event["source"] == "aws.events"
        && event["detail-type"] == "Scheduled Event"
        && event["resources"].as_array().is_some_and(|resources| {
            resources.len() == 1 && resources[0].as_str() == Some(rule_arn)
        })
}

impl Handler {
    async fn handle(&self, payload: Value, deadline: SystemTime) -> Result<()> {
        if is_dispatch_event(&payload, &self.dispatch_rule_arn) {
            return self.api.dispatch().await;
        }
        let event: SnsEvent = serde_json::from_value(payload)?;
        if event.records.is_empty() || event.records.len() > 10 {
            return Err(invalid("Unexpected SNS record count"));
        }
        // SNS invokes this function through an IAM-scoped Lambda permission.
        // Validate the event metadata as well; a payload alone is not proof of
        // origin, and public HTTP invocation is deliberately not configured.
        for record in &event.records {
            let notification = self.policy.parse(record)?;
            self.api.ingest(&notification.ingest_request()).await?;
        }
        // Only persistence failures fail the invocation so Lambda retries;
        // ingestion deduplication makes replay safe. The scheduled invocation
        // dispatches whatever this attempt leaves pending.
        self.dispatch_before(deadline).await;
        Ok(())
    }

    async fn dispatch_before(&self, deadline: SystemTime) {
        let budget = deadline
            .duration_since(SystemTime::now())
            .unwrap_or_default()
            .saturating_sub(DEADLINE_MARGIN);
        let error = match tokio::time::timeout(budget, self.api.dispatch()).await {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            Err(_) => invalid("Dispatch reached the invocation deadline"),
        };
        tracing::warn!(%error, "Mail dispatch after ingestion failed; the schedule retries it");
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    lambda_runtime::tracing::init_default_subscriber();
    let (config, token) = config::Config::load().await?;
    let handler = Arc::new(Handler {
        api: ApiClient::new(&config.api_base_url, token)?,
        policy: ReceiptPolicy {
            bucket: config.bucket,
            prefix: config.prefix,
            topic_arn: config.topic_arn,
        },
        dispatch_rule_arn: config.dispatch_rule_arn,
    });
    run(service_fn(move |event: LambdaEvent<Value>| {
        let handler = handler.clone();
        async move {
            handler
                .handle(event.payload, event.context.deadline())
                .await
        }
    }))
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    const TOPIC: &str = "arn:aws:sns:eu-west-1:123456789012:mail";
    const RULE: &str = "arn:aws:events:eu-west-1:123456789012:rule/mail-dispatch";

    #[test]
    fn schedule_requires_the_configured_rule() {
        let mut event = schedule();
        assert!(is_dispatch_event(&event, RULE));
        event["resources"] = serde_json::json!(["other-rule"]);
        assert!(!is_dispatch_event(&event, RULE));
        assert!(!is_dispatch_event(
            &serde_json::json!({ "dispatch": true }),
            RULE
        ));
    }

    fn schedule() -> Value {
        serde_json::json!({
            "source": "aws.events",
            "detail-type": "Scheduled Event",
            "resources": [RULE]
        })
    }

    async fn stub_api(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (Handler, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (path, status) in responses {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                assert_eq!(line, format!("POST /api/v1/sink/mail/{path} HTTP/1.1\r\n"));
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).await.unwrap();
                reader
                    .get_mut()
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let handler = Handler {
            api: ApiClient::new(&base, "scoped-token".into()).unwrap(),
            policy: ReceiptPolicy {
                bucket: "mail-bucket".into(),
                prefix: "raw/".into(),
                topic_arn: TOPIC.into(),
            },
            dispatch_rule_arn: RULE.into(),
        };
        (handler, server)
    }

    fn receipt() -> Value {
        let message = serde_json::json!({
            "notificationType": "Received",
            "mail": { "messageId": "ses-id", "source": "sender@example.test" },
            "receipt": {
                "recipients": ["event@example.test"],
                "action": {
                    "type": "S3",
                    "bucketName": "mail-bucket",
                    "objectKey": "raw/ses-id",
                    "topicArn": TOPIC
                }
            }
        });
        serde_json::json!({ "Records": [{
            "EventSource": "aws:sns",
            "Sns": { "TopicArn": TOPIC, "Message": message.to_string() }
        }] })
    }

    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(60)
    }

    #[tokio::test]
    async fn dispatch_failure_after_ingestion_does_not_fail_the_invocation() {
        let (handler, server) = stub_api(vec![
            ("ingest", "202 Accepted"),
            ("dispatch", "503 Service Unavailable"),
        ])
        .await;
        handler.handle(receipt(), later()).await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dispatch_after_ingestion_stays_within_the_invocation_deadline() {
        let (handler, server) = stub_api(vec![("ingest", "202 Accepted")]).await;
        handler
            .handle(receipt(), SystemTime::UNIX_EPOCH)
            .await
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn ingestion_failure_fails_the_invocation() {
        let (handler, server) = stub_api(vec![("ingest", "500 Internal Server Error")]).await;
        assert!(handler.handle(receipt(), later()).await.is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn scheduled_dispatch_failure_is_reported() {
        let (handler, server) = stub_api(vec![("dispatch", "503 Service Unavailable")]).await;
        assert!(handler.handle(schedule(), later()).await.is_err());
        server.await.unwrap();
    }
}
