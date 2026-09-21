use flow_like::flow::{
    execution::{ExecutionEnvironment, ExecutionMode, context::ExecutionContext},
    node::{Node, NodeLogic},
    regression::discover::is_test_event_node,
    variable::VariableType,
};
use flow_like_types::{Value, anyhow, async_trait, json::json, reqwest, tokio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[crate::register_node]
#[derive(Default)]
pub struct RequestPaymentNode;

#[async_trait]
impl NodeLogic for RequestPaymentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "request_payment",
            "Request Payment",
            "Ask the signed-in user to pay the app owner and wait for the verified result",
            "Payments",
        );
        node.set_flowscript_name("payments", "request");
        node.add_icon("/flow/icons/credit-card.svg");
        node.long_running = Some(true);
        node.docs = Some("https://docs.flow-like.com/nodes/payments/".into());
        node.add_input_pin("exec_in", "Input", "Start payment", VariableType::Execution);
        for (name, label, description, default) in [
            (
                "currency",
                "Currency",
                "Supported three-letter currency",
                "eur",
            ),
            (
                "product_name",
                "Product",
                "Plain-text product name",
                "Payment",
            ),
            ("description", "Description", "Plain-text description", ""),
            (
                "product_tax_code",
                "Product Tax Code",
                "Required Stripe tax code for the product or service, selected from Stripe's tax code list",
                "",
            ),
            (
                "shipping_countries",
                "Shipping Countries",
                "For shipped goods, comma-separated delivery country codes such as DE,FR. Leave empty when no delivery address is needed.",
                "",
            ),
            (
                "reference",
                "Reference",
                "Optional application reference",
                "",
            ),
            (
                "simulation",
                "Board Test Result",
                "Local Board Test only: paid, canceled, expired or failed. Empty requests a real payment.",
                "",
            ),
        ] {
            node.add_input_pin(name, label, description, VariableType::String)
                .set_default_value(Some(json!(default)));
        }
        node.add_input_pin(
            "amount_minor",
            "Amount",
            "Total including applicable tax in integer minor units, for example 119 for EUR 1.19",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_input_pin(
            "ttl_seconds",
            "Timeout",
            "Requested timeout in seconds, limited by the remaining run quota",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(300)));
        for status in ["paid", "canceled", "expired", "failed"] {
            node.add_output_pin(
                status,
                status,
                "Authoritative payment result",
                VariableType::Execution,
            );
        }
        node.add_output_pin(
            "payment_id",
            "Payment ID",
            "Server payment request identifier, or a sim_ identifier in Board Test",
            VariableType::String,
        );
        node.add_output_pin(
            "reason",
            "Reason",
            "Machine-readable outcome reason",
            VariableType::String,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        for status in ["paid", "canceled", "expired", "failed"] {
            context.deactivate_exec_pin(status).await?;
        }
        context.set_pin_value("payment_id", json!("")).await?;
        context.set_pin_value("reason", json!("")).await?;
        let simulation = context.evaluate_pin::<String>("simulation").await?;
        if !simulation.is_empty() {
            let board_test = if let Some(run) = context.run.upgrade() {
                let run = run.lock().await;
                !run.shadow
                    && run
                        .board
                        .nodes
                        .get(&run.payload.id)
                        .is_some_and(is_test_event_node)
            } else {
                false
            };
            if !simulation_allowed(context.execution_environment(), board_test) {
                return finish(
                    context,
                    "failed",
                    "PAYMENT_SIMULATION_REQUIRES_LOCAL_BOARD_TEST",
                )
                .await;
            }
            if !matches!(
                simulation.as_str(),
                "paid" | "canceled" | "expired" | "failed"
            ) {
                return finish(context, "failed", "INVALID_PAYMENT_SIMULATION").await;
            }
            let simulated_id = format!("sim_{}", flow_like_types::create_id());
            let amount = context.evaluate_pin::<i64>("amount_minor").await?;
            let currency = context.evaluate_pin::<String>("currency").await?;
            let product = context.evaluate_pin::<String>("product_name").await?;
            context
                .set_pin_value("payment_id", json!(simulated_id))
                .await?;
            context.stream_response("payment_simulation",json!({"id":simulated_id,"status":simulation,"amountMinor":amount,"currency":currency,"productName":product})).await?;
            return finish(context, &simulation, "SIMULATED").await;
        }
        if context.execution_environment() != ExecutionEnvironment::Server
            || !matches!(
                context.execution_mode(),
                ExecutionMode::Sync | ExecutionMode::Event
            )
            || context
                .execution_cache
                .as_ref()
                .is_some_and(|cache| cache.shadow)
        {
            return finish(context, "failed", "PAYMENT_REQUIRES_ATTENDED_REMOTE_RUN").await;
        }
        let Some(auth) = context.executor_payment_auth.clone() else {
            return finish(context, "failed", "PAYMENT_REQUIRES_ATTENDED_REMOTE_RUN").await;
        };
        let base = reqwest::Url::parse(auth.callback_url())
            .map_err(|_| anyhow!("Invalid payment API origin"))?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(anyhow!("Invalid payment API origin"));
        }
        let endpoint = format!("{}/execution/payments", base.as_str().trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()?;
        let amount = context.evaluate_pin::<i64>("amount_minor").await?;
        let currency = context.evaluate_pin::<String>("currency").await?;
        let product = context.evaluate_pin::<String>("product_name").await?;
        let product_tax_code = context.evaluate_pin::<String>("product_tax_code").await?;
        let shipping_countries = context
            .evaluate_pin::<String>("shipping_countries")
            .await?
            .split(',')
            .map(str::trim)
            .filter(|country| !country.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let description = context.evaluate_pin::<String>("description").await?;
        let reference = context.evaluate_pin::<String>("reference").await?;
        let ttl = context.evaluate_pin::<i64>("ttl_seconds").await?;
        let nonce = flow_like_types::create_id();
        let body = json!({"nodeId":context.id.as_ref(),"nonce":nonce,"amountMinor":amount,"currency":currency,"productName":product,"productTaxCode":product_tax_code,"shippingCountries":shipping_countries,"description":description,"reference":reference,"ttlSeconds":ttl});
        let mut created = None;
        for attempt in 0..3 {
            match client
                .post(&endpoint)
                .bearer_auth(auth.token())
                .json(&body)
                .send()
                .await
            {
                Ok(response) => {
                    created = Some(response);
                    break;
                }
                Err(_) if attempt < 2 => {
                    tokio::time::sleep(Duration::from_millis(250 * (attempt + 1))).await
                }
                Err(_) => return finish(context, "failed", "PAYMENT_API_UNAVAILABLE").await,
            }
        }
        let response = created.ok_or_else(|| anyhow!("Payment request did not complete"))?;
        if !response.status().is_success() {
            let body: Value = response.json().await.unwrap_or_default();
            let reason = body["code"]
                .as_str()
                .filter(|code| {
                    code.len() <= 100 && code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
                })
                .unwrap_or("PAYMENT_REQUEST_REJECTED");
            return finish(context, "failed", reason).await;
        }
        let mut status: Value = response
            .json()
            .await
            .map_err(|_| anyhow!("Invalid payment response"))?;
        let id = status["id"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing payment identifier"))?
            .to_owned();
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(anyhow!("Invalid payment identifier"));
        }
        context.set_pin_value("payment_id", json!(id)).await?;
        context
            .stream_response(
                "payment_request",
                json!({"id":id,"appId":status["appId"],"runId":status["runId"]}),
            )
            .await?;
        let url = format!("{endpoint}/{id}");
        let cancellation = context.cancellation_token();
        let jitter = nonce.bytes().fold(0u64, |sum, b| sum + u64::from(b)) % 700;
        loop {
            if let Some((pin, reason)) = terminal(&status) {
                return finish(context, pin, &reason).await;
            }
            let expired = status["expiresAt"]
                .as_i64()
                .is_some_and(|expiry| now_ms() >= expiry);
            if expired
                || cancellation
                    .as_ref()
                    .is_some_and(|token| token.is_cancelled())
            {
                if let Ok(response) = client
                    .post(format!("{url}/cancel"))
                    .bearer_auth(auth.token())
                    .send()
                    .await
                {
                    if response.status().is_success()
                        && let Ok(value) = response.json::<Value>().await
                        && !cancellation
                            .as_ref()
                            .is_some_and(|token| token.is_cancelled())
                        && let Some((pin, reason)) = terminal(&value)
                    {
                        return finish(context, pin, &reason).await;
                    }
                }
                return finish(
                    context,
                    if expired { "expired" } else { "canceled" },
                    if expired {
                        "DEADLINE_EXPIRED"
                    } else {
                        "RUN_CANCELED"
                    },
                )
                .await;
            }
            if let Some(token) = &cancellation {
                tokio::select! { _=token.cancelled()=>{}, _=tokio::time::sleep(Duration::from_millis(1500+jitter))=>{} }
            } else {
                tokio::time::sleep(Duration::from_millis(1500 + jitter)).await;
            }
            if let Ok(response) = client.get(&url).bearer_auth(auth.token()).send().await {
                if response.status().is_success() {
                    if let Ok(value) = response.json().await {
                        status = value;
                    }
                }
            }
        }
    }
}
fn simulation_allowed(environment: ExecutionEnvironment, board_test: bool) -> bool {
    environment == ExecutionEnvironment::Local && board_test
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn terminal(value: &Value) -> Option<(&'static str, String)> {
    let pin = match value["status"].as_str()? {
        "PAID" => "paid",
        "CANCELED" => "canceled",
        "EXPIRED" => "expired",
        "FAILED" => "failed",
        _ => return None,
    };
    Some((pin, value["reason"].as_str().unwrap_or("").to_owned()))
}
async fn finish(
    context: &mut ExecutionContext,
    pin: &str,
    reason: &str,
) -> flow_like_types::Result<()> {
    context.set_pin_value("reason", json!(reason)).await?;
    context.activate_exec_pin(pin).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simulation_requires_a_local_board_test_even_when_remote_alias_is_test() {
        assert!(simulation_allowed(ExecutionEnvironment::Local, true));
        assert!(!simulation_allowed(ExecutionEnvironment::Local, false));
        assert!(!simulation_allowed(ExecutionEnvironment::Server, true));
        assert!(!simulation_allowed(ExecutionEnvironment::Server, false));
    }
    #[test]
    fn processing_and_cancel_pending_do_not_resume_the_flow() {
        for value in ["CREATED", "OPENING", "OPEN", "PROCESSING", "CANCEL_PENDING"] {
            assert!(terminal(&json!({"status":value})).is_none());
        }
    }
    #[test]
    fn only_server_paid_status_selects_paid() {
        assert_eq!(
            terminal(&json!({"status":"PAID","reason":"verified"})),
            Some(("paid", "verified".into()))
        );
        assert!(terminal(&json!({"status":"complete","payment_status":"paid"})).is_none());
    }
}
