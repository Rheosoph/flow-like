use super::{error, gateway_for_version, stripe_error};
use crate::{
    error::ApiError,
    state::AppState,
    stripe_connect::{STRIPE_API_VERSION, StripeGateway, StripeRequest, StripeScope},
};
use serde_json::json;

/// Check the actual liable Stripe account before opening a new taxable checkout.
/// An active registration is a launch prerequisite, not proof of coverage in every market.
pub async fn require_ready(
    state: &AppState,
    scope: &StripeScope,
    product_tax_code: &str,
) -> Result<(), ApiError> {
    let gateway = gateway_for_version(state, STRIPE_API_VERSION).await?;
    require_ready_with_gateway(gateway.as_ref(), scope, product_tax_code).await
}

async fn require_ready_with_gateway(
    gateway: &dyn StripeGateway,
    scope: &StripeScope,
    product_tax_code: &str,
) -> Result<(), ApiError> {
    if !flow_like::hub::valid_product_tax_code(product_tax_code) {
        return Err(error(
            "PAYMENT_TAX_CODE_REQUIRED",
            "Choose the Stripe tax code for the product being sold",
        ));
    }
    let settings = gateway
        .execute(&StripeRequest::get(
            scope.clone(),
            "/v1/tax/settings",
            json!({}),
        ))
        .await
        .map_err(stripe_error)?
        .body;
    if settings["object"] != "tax.settings"
        || settings["status"] != "active"
        || settings["livemode"].as_bool() != Some(scope.livemode)
    {
        return Err(error(
            "PAYMENT_TAX_SETUP_REQUIRED",
            "Complete Stripe Tax settings in the account receiving this payment",
        ));
    }
    let registrations = gateway
        .execute(&StripeRequest::get(
            scope.clone(),
            "/v1/tax/registrations",
            json!({"status":"active","limit":1}),
        ))
        .await
        .map_err(stripe_error)?
        .body;
    let registered = registrations["object"] == "list"
        && registrations["data"].as_array().is_some_and(|rows| {
            rows.iter().any(|row| {
                row["object"] == "tax.registration"
                    && row["status"] == "active"
                    && row["livemode"].as_bool() == Some(scope.livemode)
            })
        });
    if !registered {
        return Err(error(
            "PAYMENT_TAX_REGISTRATION_REQUIRED",
            "Add the receiving business's active tax registrations to Stripe Tax before accepting payments",
        ));
    }
    let tax_code = gateway
        .execute(&StripeRequest::get(
            scope.clone(),
            format!("/v1/tax_codes/{product_tax_code}"),
            json!({}),
        ))
        .await
        .map_err(stripe_error)?
        .body;
    if tax_code["object"] != "tax_code" || tax_code["id"] != product_tax_code {
        return Err(error(
            "PAYMENT_TAX_CODE_REQUIRED",
            "The product tax code could not be verified with Stripe",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stripe_connect::{StripeError, StripeResponse};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::{collections::VecDeque, sync::Mutex};

    struct Gateway {
        requests: Mutex<Vec<StripeRequest>>,
        replies: Mutex<VecDeque<Value>>,
    }

    #[async_trait]
    impl StripeGateway for Gateway {
        async fn execute(&self, request: &StripeRequest) -> Result<StripeResponse, StripeError> {
            self.requests.lock().unwrap().push(request.clone());
            Ok(StripeResponse {
                body: self
                    .replies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected request"),
                request_id: None,
            })
        }
    }

    fn gateway() -> Gateway {
        Gateway {
            requests: Mutex::new(Vec::new()),
            replies: Mutex::new(VecDeque::from([
                json!({"object":"tax.settings","status":"active","livemode":false}),
                json!({"object":"list","data":[{"object":"tax.registration","status":"active","livemode":false}]}),
                json!({"object":"tax_code","id":"txcd_10000000"}),
            ])),
        }
    }

    #[tokio::test]
    async fn readiness_uses_the_immutable_liable_account_scope() {
        for scope in [
            StripeScope::platform("acct_platform", false),
            StripeScope::platform("acct_platform", false).connected("acct_seller"),
        ] {
            let gateway = gateway();
            require_ready_with_gateway(&gateway, &scope, "txcd_10000000")
                .await
                .unwrap();
            let requests = gateway.requests.lock().unwrap();
            assert_eq!(requests.len(), 3);
            assert!(requests.iter().all(|request| request.scope == scope));
        }
    }

    #[tokio::test]
    async fn incomplete_or_wrong_mode_tax_settings_block_checkout() {
        for settings in [
            json!({"object":"tax.settings","status":"pending","livemode":false}),
            json!({"object":"tax.settings","status":"active","livemode":true}),
            json!({}),
        ] {
            let gateway = gateway();
            gateway.replies.lock().unwrap()[0] = settings;
            let error = require_ready_with_gateway(
                &gateway,
                &StripeScope::platform("acct_platform", false),
                "txcd_10000000",
            )
            .await
            .unwrap_err();
            assert_eq!(error.public_code(), "PAYMENT_TAX_SETUP_REQUIRED");
            assert_eq!(gateway.requests.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn missing_scheduled_and_wrong_mode_registrations_block_checkout() {
        for registrations in [
            json!({"object":"list","data":[]}),
            json!({"object":"list","data":[{"object":"tax.registration","status":"scheduled","livemode":false}]}),
            json!({"object":"list","data":[{"object":"tax.registration","status":"active","livemode":true}]}),
        ] {
            let gateway = gateway();
            gateway.replies.lock().unwrap()[1] = registrations;
            let error = require_ready_with_gateway(
                &gateway,
                &StripeScope::platform("acct_platform", false),
                "txcd_10000000",
            )
            .await
            .unwrap_err();
            assert_eq!(error.public_code(), "PAYMENT_TAX_REGISTRATION_REQUIRED");
            assert_eq!(gateway.requests.lock().unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn tax_codes_are_validated_before_use() {
        let gateway = gateway();
        let scope = StripeScope::platform("acct_platform", false);
        let error = require_ready_with_gateway(&gateway, &scope, "txcd_guess")
            .await
            .unwrap_err();
        assert_eq!(error.public_code(), "PAYMENT_TAX_CODE_REQUIRED");
        assert!(gateway.requests.lock().unwrap().is_empty());
        gateway.replies.lock().unwrap()[2] = json!({"object":"tax_code","id":"txcd_99999999"});
        assert_eq!(
            require_ready_with_gateway(&gateway, &scope, "txcd_10000000")
                .await
                .unwrap_err()
                .public_code(),
            "PAYMENT_TAX_CODE_REQUIRED"
        );
    }
}
