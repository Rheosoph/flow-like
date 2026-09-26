use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    RequestMethod, RetryDisposition, StripeError, StripeGateway, StripeRequest, StripeResponse,
    StripeScope,
};

#[derive(Default)]
pub struct FakeGateway {
    state: Mutex<FakeState>,
}

#[derive(Default)]
struct FakeState {
    sequence: u64,
    objects: HashMap<(String, String), Value>,
    idempotency: HashMap<(String, String), (String, Result<StripeResponse, StripeError>)>,
    scripted: VecDeque<Result<StripeResponse, StripeError>>,
    calls: Vec<StripeRequest>,
    balances: HashMap<(String, String), i64>,
    lose_next_response: bool,
}

impl FakeGateway {
    /// Applies and caches the mutation, then drops its response before the caller sees it.
    pub fn lose_next_response(&self) {
        self.state.lock().unwrap().lose_next_response = true;
    }

    pub fn insert(&self, scope: &StripeScope, object: Value) {
        let id = object["id"].as_str().expect("fake object id").to_owned();
        self.state
            .lock()
            .unwrap()
            .objects
            .insert((scope_key(scope), id), object);
    }

    pub fn script(&self, result: Result<StripeResponse, StripeError>) {
        self.state.lock().unwrap().scripted.push_back(result);
    }

    pub fn calls(&self) -> Vec<StripeRequest> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn set_balance(&self, scope: &StripeScope, currency: &str, amount: i64) {
        self.state
            .lock()
            .unwrap()
            .balances
            .insert((scope_key(scope), currency.into()), amount);
    }

    pub fn complete_session(&self, scope: &StripeScope, session_id: &str, paid: bool) {
        let mut state = self.state.lock().unwrap();
        let session = state
            .objects
            .get_mut(&(scope_key(scope), session_id.into()))
            .expect("known session");
        assert_eq!(session["status"], "open");
        session["status"] = json!("complete");
        session["payment_status"] = json!(if paid { "paid" } else { "unpaid" });
    }

    pub fn update_refund(&self, scope: &StripeScope, refund_id: &str, status: &str) {
        assert!(matches!(
            status,
            "pending" | "succeeded" | "failed" | "canceled"
        ));
        let mut state = self.state.lock().unwrap();
        let refund = state
            .objects
            .get_mut(&(scope_key(scope), refund_id.into()))
            .expect("known refund");
        refund["status"] = json!(status);
    }
}

fn scope_key(scope: &StripeScope) -> String {
    format!(
        "{}:{}:{}",
        scope.platform_account_id,
        scope.livemode,
        scope.connected_account_id.as_deref().unwrap_or("platform")
    )
}

fn api_error(code: &str) -> StripeError {
    StripeError::Api {
        status: 400,
        code: Some(code.into()),
        request_id: Some("req_fake".into()),
        retry: RetryDisposition::Never,
        retry_after_seconds: None,
    }
}

fn integer(parameters: &Value, key: &str) -> Result<u64, StripeError> {
    parameters[key]
        .as_u64()
        .ok_or_else(|| StripeError::invalid("fake expected integer amount"))
}

impl FakeState {
    fn id(&mut self, prefix: &str) -> String {
        self.sequence += 1;
        format!("{prefix}_fake_{}", self.sequence)
    }

    fn apply(&mut self, request: &StripeRequest) -> Result<StripeResponse, StripeError> {
        let scope = scope_key(&request.scope);
        let parts: Vec<_> = request.path.split('/').skip(2).collect();
        let parameters = &request.parameters;
        if request.method == RequestMethod::Get {
            if request.path == "/v1/balance" {
                let available: Vec<Value> = self
                    .balances
                    .iter()
                    .filter(|((key, _), _)| *key == scope)
                    .map(|((_, currency), amount)| json!({"amount": amount, "currency": currency}))
                    .collect();
                return Ok(StripeResponse {
                    body: json!({"livemode":request.scope.livemode,"available":available,"pending":[]}),
                    request_id: Some("req_fake".into()),
                });
            }
            let id = parts.last().copied().unwrap_or_default();
            let body = self
                .objects
                .get(&(scope, id.into()))
                .cloned()
                .ok_or_else(|| api_error("resource_missing"))?;
            return Ok(StripeResponse {
                body,
                request_id: Some("req_fake".into()),
            });
        }
        let object = match parts.as_slice() {
            ["checkout", "sessions"] => {
                let amount = parameters
                    .pointer("/line_items/0/price_data/unit_amount")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| StripeError::invalid("fake requires inline price"))?;
                let quantity = parameters
                    .pointer("/line_items/0/quantity")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| StripeError::invalid("fake requires quantity"))?;
                let total = amount
                    .checked_mul(quantity)
                    .ok_or_else(|| StripeError::invalid("amount overflow"))?;
                let id = self.id("cs");
                json!({"id":id,"object":"checkout.session","livemode":request.scope.livemode,
                    "mode":"payment","status":"open","payment_status":"unpaid","amount_total":total,
                    "currency":parameters.pointer("/line_items/0/price_data/currency"),"expires_at":parameters["expires_at"],
                    "url":format!("https://checkout.stripe.com/test/{id}"),"metadata":parameters["metadata"]})
            }
            ["checkout", "sessions", id, "expire"] => {
                let session = self
                    .objects
                    .get_mut(&(scope.clone(), (*id).into()))
                    .ok_or_else(|| api_error("resource_missing"))?;
                if session["status"] != "open" {
                    return Err(api_error("checkout_session_not_expirable"));
                }
                session["status"] = json!("expired");
                session.clone()
            }
            ["refunds"] => {
                let charge_id = parameters["charge"]
                    .as_str()
                    .ok_or_else(|| StripeError::invalid("fake requires charge"))?;
                let charge = self
                    .objects
                    .get(&(scope.clone(), charge_id.into()))
                    .ok_or_else(|| api_error("resource_missing"))?;
                let captured = integer(charge, "amount")?;
                let currency = charge["currency"].clone();
                let refunded: u64 = self
                    .objects
                    .iter()
                    .filter(|((key, _), object)| {
                        *key == scope
                            && object["object"] == "refund"
                            && object["charge"] == charge_id
                            && matches!(object["status"].as_str(), Some("succeeded" | "pending"))
                    })
                    .map(|(_, object)| object["amount"].as_u64().unwrap())
                    .sum();
                let amount = parameters["amount"]
                    .as_u64()
                    .unwrap_or(captured.saturating_sub(refunded));
                if amount == 0
                    || refunded
                        .checked_add(amount)
                        .is_none_or(|total| total > captured)
                {
                    return Err(api_error("charge_already_refunded"));
                }
                if parameters["reverse_transfer"] == true
                    || parameters["refund_application_fee"] == true
                {
                    return Err(StripeError::invalid(
                        "fake requires explicit adjustment operations",
                    ));
                }
                json!({"id":self.id("re"),"object":"refund","charge":charge_id,"amount":amount,"currency":currency,"status":"succeeded"})
            }
            ["transfers"] => {
                let amount = integer(parameters, "amount")?;
                let currency = parameters["currency"]
                    .as_str()
                    .ok_or_else(|| StripeError::invalid("missing transfer currency"))?;
                let balance = self
                    .balances
                    .entry((scope.clone(), currency.into()))
                    .or_default();
                let amount_signed =
                    i64::try_from(amount).map_err(|_| StripeError::invalid("amount overflow"))?;
                if amount == 0 || *balance < amount_signed {
                    return Err(api_error("balance_insufficient"));
                }
                *balance -= amount_signed;
                json!({"id":self.id("tr"),"object":"transfer","livemode":request.scope.livemode,"amount":amount,"amount_reversed":0,"currency":currency,"destination":parameters["destination"],"transfer_group":parameters["transfer_group"],"source_transaction":parameters["source_transaction"]})
            }
            ["transfers", id, "reversals"] => {
                let transfer = self
                    .objects
                    .get(&(scope.clone(), (*id).into()))
                    .ok_or_else(|| api_error("resource_missing"))?;
                let remaining = integer(transfer, "amount")?
                    .checked_sub(integer(transfer, "amount_reversed")?)
                    .ok_or_else(|| StripeError::invalid("invalid reversal totals"))?;
                let amount = parameters["amount"].as_u64().unwrap_or(remaining);
                if amount == 0 || amount > remaining {
                    return Err(api_error("amount_too_large"));
                }
                let currency = transfer["currency"].clone();
                let destination = transfer["destination"].as_str().unwrap();
                let destination_scope = scope_key(&request.scope.connected(destination));
                let balance = self
                    .balances
                    .entry((destination_scope, currency.as_str().unwrap().into()))
                    .or_default();
                let amount_signed =
                    i64::try_from(amount).map_err(|_| StripeError::invalid("amount overflow"))?;
                // Fixtures explicitly identify destination transfers; source_transaction alone is insufficient.
                if transfer["fake_destination_charge"] != true && *balance < amount_signed {
                    return Err(api_error("balance_insufficient"));
                }
                *balance -= amount_signed;
                self.objects
                    .get_mut(&(scope.clone(), (*id).into()))
                    .unwrap()["amount_reversed"] =
                    json!(integer(transfer, "amount")? - remaining + amount);
                json!({"id":self.id("trr"),"object":"transfer_reversal","transfer":id,"amount":amount,"currency":currency})
            }
            ["application_fees", id, "refunds"] => {
                let fee = self
                    .objects
                    .get_mut(&(scope.clone(), (*id).into()))
                    .ok_or_else(|| api_error("resource_missing"))?;
                let collected = integer(fee, "amount")?;
                let refunded = integer(fee, "amount_refunded")?;
                let remaining = collected
                    .checked_sub(refunded)
                    .ok_or_else(|| StripeError::invalid("invalid fee totals"))?;
                let amount = parameters["amount"].as_u64().unwrap_or(remaining);
                if amount == 0 || amount > remaining {
                    return Err(api_error("amount_too_large"));
                }
                fee["amount_refunded"] = json!(refunded + amount);
                let currency = fee["currency"].clone();
                json!({"id":self.id("fr"),"object":"fee_refund","fee":id,"amount":amount,"currency":currency})
            }
            _ => {
                return Err(StripeError::invalid(
                    "unsupported fake Stripe operation; script its response explicitly",
                ));
            }
        };
        let id = object["id"].as_str().unwrap().to_owned();
        self.objects.insert((scope, id), object.clone());
        Ok(StripeResponse {
            body: object,
            request_id: Some("req_fake".into()),
        })
    }
}

#[async_trait]
impl StripeGateway for FakeGateway {
    async fn execute(&self, request: &StripeRequest) -> Result<StripeResponse, StripeError> {
        let digest = request.digest()?;
        let mut state = self.state.lock().unwrap();
        state.calls.push(request.clone());
        let identity = request
            .idempotency_key
            .as_ref()
            .map(|key| (scope_key(&request.scope), key.clone()));
        if let Some(identity) = &identity
            && let Some((stored, response)) = state.idempotency.get(identity)
        {
            return if *stored == digest {
                response.clone()
            } else {
                Err(api_error("idempotency_key_mismatch"))
            };
        }
        let response = if let Some(scripted) = state.scripted.pop_front() {
            scripted
        } else {
            state.apply(request)
        };
        // Network errors have no provider response to cache. Scripts can model lost responses separately.
        if let Some(identity) = identity
            && !matches!(response, Err(StripeError::Transport { .. }))
        {
            state
                .idempotency
                .insert(identity, (digest, response.clone()));
        }
        if std::mem::take(&mut state.lose_next_response) {
            return Err(StripeError::Transport {
                retry: RetryDisposition::SameOperation,
            });
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn idempotency_reuses_created_object_and_rejects_changed_amount() {
        let fake = FakeGateway::default();
        let scope = StripeScope::platform("acct_platform", false);
        fake.insert(
            &scope,
            json!({"id":"ch_one","amount":11900,"currency":"eur"}),
        );
        let mut request = StripeRequest::post(
            scope,
            "/v1/refunds",
            json!({"charge":"ch_one","amount":1000}),
            "one",
        );
        let first = fake.execute(&request).await.unwrap();
        assert_eq!(first.body, fake.execute(&request).await.unwrap().body);
        request.parameters["amount"] = json!(1001);
        assert!(fake.execute(&request).await.is_err());
    }

    #[tokio::test]
    async fn response_loss_replays_the_applied_mutation() {
        let fake = FakeGateway::default();
        let scope = StripeScope::platform("acct_platform", false);
        fake.insert(
            &scope,
            json!({"id":"ch_one","amount":11900,"currency":"eur"}),
        );
        let request = StripeRequest::post(
            scope,
            "/v1/refunds",
            json!({"charge":"ch_one","amount":1000}),
            "one",
        );
        fake.lose_next_response();
        assert!(matches!(
            fake.execute(&request).await,
            Err(StripeError::Transport { .. })
        ));
        let first = fake.execute(&request).await.unwrap();
        assert_eq!(first.body["id"], "re_fake_1");
        assert_eq!(first.body, fake.execute(&request).await.unwrap().body);
    }

    #[tokio::test]
    async fn tax_reversal_leaves_only_the_remaining_original_transfer() {
        let fake = FakeGateway::default();
        let scope = StripeScope::platform("acct_platform", false);
        fake.insert(&scope, json!({"id":"tr_one","amount":11900,"amount_reversed":0,"currency":"eur","destination":"acct_seller","fake_destination_charge":true}));
        fake.execute(&StripeRequest::post(
            scope.clone(),
            "/v1/transfers/tr_one/reversals",
            json!({"amount":1900}),
            "tax",
        ))
        .await
        .unwrap();
        assert!(
            fake.execute(&StripeRequest::post(
                scope.clone(),
                "/v1/transfers/tr_one/reversals",
                json!({"amount":11900}),
                "too_much"
            ))
            .await
            .is_err()
        );
        fake.execute(&StripeRequest::post(
            scope.clone(),
            "/v1/transfers/tr_one/reversals",
            json!({"amount":10000}),
            "rest",
        ))
        .await
        .unwrap();
        let transfer = fake
            .execute(&StripeRequest::get(
                scope,
                "/v1/transfers/tr_one",
                json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(transfer.body["amount_reversed"], 11900);
    }

    #[tokio::test]
    async fn standalone_reversal_needs_seller_balance() {
        let fake = FakeGateway::default();
        let scope = StripeScope::platform("acct_platform", false);
        fake.insert(&scope, json!({"id":"tr_restored","amount":10000,"amount_reversed":0,"currency":"eur","destination":"acct_seller"}));
        assert!(
            fake.execute(&StripeRequest::post(
                scope,
                "/v1/transfers/tr_restored/reversals",
                json!({"amount":10000}),
                "refund"
            ))
            .await
            .is_err()
        );
    }
}
