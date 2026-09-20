use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::gateway::StripeError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChargeModel {
    Direct,
    Destination,
    Platform,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentMethodPolicy {
    pub version: String,
    pub charge_model: ChargeModel,
    pub allowed_methods: Vec<String>,
    #[serde(default)]
    pub configuration_id: Option<String>,
    #[serde(default)]
    pub allow_delayed: bool,
}

impl PaymentMethodPolicy {
    pub fn cards_and_wallets(version: impl Into<String>, charge_model: ChargeModel) -> Self {
        Self {
            version: version.into(),
            charge_model,
            // Apple Pay and Google Pay are presented through the card method.
            allowed_methods: vec!["card".into(), "link".into()],
            configuration_id: None,
            allow_delayed: false,
        }
    }

    pub fn validate(&self) -> Result<(), StripeError> {
        if self.version.is_empty() || self.version.len() > 128 || self.allowed_methods.is_empty() {
            return Err(StripeError::invalid(
                "payment method policy requires a version and approved methods",
            ));
        }
        if self
            .configuration_id
            .as_ref()
            .is_some_and(|id| !super::request::valid_id(id, "pmc_"))
        {
            return Err(StripeError::invalid("invalid payment method configuration"));
        }
        let mut seen = BTreeSet::new();
        for method in &self.allowed_methods {
            if !seen.insert(method) {
                return Err(StripeError::invalid("duplicate payment method"));
            }
            if method == "paypal" && self.charge_model == ChargeModel::Direct {
                return Err(StripeError::invalid(
                    "PayPal does not support direct Connect charges",
                ));
            }
            let immediate = matches!(
                method.as_str(),
                "card"
                    | "link"
                    | "paypal"
                    | "revolut_pay"
                    | "alipay"
                    | "wechat_pay"
                    | "amazon_pay"
                    | "cashapp"
                    | "grabpay"
                    | "mobilepay"
                    | "mb_way"
                    | "satispay"
                    | "bancontact"
                    | "ideal"
                    | "eps"
                    | "p24"
                    | "blik"
            );
            let delayed = matches!(
                method.as_str(),
                "sepa_debit"
                    | "us_bank_account"
                    | "bacs_debit"
                    | "au_becs_debit"
                    | "acss_debit"
                    | "customer_balance"
                    | "boleto"
                    | "oxxo"
                    | "konbini"
            );
            if !immediate && !(delayed && self.allow_delayed) {
                return Err(StripeError::invalid(
                    "method has not passed the configured confirmation policy",
                ));
            }
        }
        if seen.contains(&"link".to_owned()) && !seen.contains(&"card".to_owned()) {
            return Err(StripeError::invalid(
                "Checkout Link requires the card method",
            ));
        }
        Ok(())
    }

    /// The allowlist intersects Stripe eligibility, including seller Dashboard settings.
    pub fn checkout_parameters(&self) -> Result<Value, StripeError> {
        self.validate()?;
        let mut parameters = json!({"allowed_payment_method_types": self.allowed_methods});
        if let Some(id) = &self.configuration_id {
            parameters["payment_method_configuration"] = json!(id);
        }
        Ok(parameters)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_policy_keeps_card_wallets_and_link_enabled() {
        let policy = PaymentMethodPolicy::cards_and_wallets("v1", ChargeModel::Direct);
        assert_eq!(
            policy.checkout_parameters().unwrap()["allowed_payment_method_types"],
            json!(["card", "link"])
        );
    }

    #[test]
    fn seller_settings_cannot_add_delayed_methods_or_paypal_direct() {
        let mut policy = PaymentMethodPolicy::cards_and_wallets("v1", ChargeModel::Direct);
        policy.allowed_methods.push("sepa_debit".into());
        assert!(policy.validate().is_err());
        policy.allowed_methods.pop();
        policy.allowed_methods.push("paypal".into());
        assert!(policy.validate().is_err());
        policy.charge_model = ChargeModel::Destination;
        assert!(policy.validate().is_ok());
        policy.charge_model = ChargeModel::Platform;
        assert!(policy.validate().is_ok());
    }
}
