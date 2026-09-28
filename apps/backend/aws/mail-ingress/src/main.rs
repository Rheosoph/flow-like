use flow_like_mail_ingress::{
    ApiClient, IngestApi, Result, invalid,
    ses::{ReceiptPolicy, SnsEvent},
};
use lambda_runtime::{LambdaEvent, run, service_fn};
use serde_json::Value;
use std::{env, sync::Arc};

struct Handler {
    api: ApiClient,
    policy: ReceiptPolicy,
    dispatch_rule_arn: String,
}

fn required(name: &str) -> Result<String> {
    let value = env::var(name).map_err(|_| invalid(format!("{name} is required")))?;
    if value.trim().is_empty() {
        return Err(invalid(format!("{name} is empty")));
    }
    Ok(value)
}

fn is_dispatch_event(event: &Value, rule_arn: &str) -> bool {
    event["source"] == "aws.events"
        && event["detail-type"] == "Scheduled Event"
        && event["resources"].as_array().is_some_and(|resources| {
            resources.len() == 1 && resources[0].as_str() == Some(rule_arn)
        })
}

impl Handler {
    async fn handle(&self, payload: Value) -> Result<()> {
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
        // Fail the invocation on either persistence or dispatch failure so
        // Lambda retries. Ingestion deduplication makes replay safe. The
        // scheduled invocation also recovers pending deliveries independently.
        self.api.dispatch().await
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    lambda_runtime::tracing::init_default_subscriber();
    let base = required("API_BASE_URL")?;
    if !base.starts_with("https://")
        && env::var("ALLOW_INSECURE_API_BASE_URL").as_deref() != Ok("1")
    {
        return Err(invalid("API_BASE_URL must use HTTPS"));
    }
    let handler = Arc::new(Handler {
        api: ApiClient::new(&base, required("SINK_TRIGGER_JWT")?)?,
        policy: ReceiptPolicy {
            bucket: required("INBOUND_MAIL_BUCKET")?,
            prefix: required("INBOUND_MAIL_PREFIX")?,
            topic_arn: required("MAIL_SNS_TOPIC_ARN")?,
        },
        dispatch_rule_arn: required("MAIL_DISPATCH_RULE_ARN")?,
    });
    run(service_fn(move |event: LambdaEvent<Value>| {
        let handler = handler.clone();
        async move { handler.handle(event.payload).await }
    }))
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_requires_the_configured_rule() {
        let rule = "arn:aws:events:eu-west-1:123456789012:rule/mail-dispatch";
        let mut event = serde_json::json!({ "source": "aws.events", "detail-type": "Scheduled Event", "resources": [rule] });
        assert!(is_dispatch_event(&event, rule));
        event["resources"] = serde_json::json!(["other-rule"]);
        assert!(!is_dispatch_event(&event, rule));
        assert!(!is_dispatch_event(
            &serde_json::json!({ "dispatch": true }),
            rule
        ));
    }
}
