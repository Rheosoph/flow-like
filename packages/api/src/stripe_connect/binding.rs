use serde::{Deserialize, Serialize};

use super::{
    StripeScope,
    types::{Charge, CheckoutSession, PaymentIntent},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExpectedPayment {
    pub scope: StripeScope,
    pub session_id: String,
    pub payment_intent_id: Option<String>,
    pub charge_id: Option<String>,
    pub amount: u64,
    pub currency: String,
    pub application_fee_amount: u64,
    pub destination_account_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifiedPayment {
    Paid,
    Processing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Stripe payment binding mismatch: {0}")]
pub struct BindingError(pub &'static str);

impl ExpectedPayment {
    pub fn verify(
        &self,
        observed_scope: &StripeScope,
        session: &CheckoutSession,
        intent: &PaymentIntent,
        charge: Option<&Charge>,
    ) -> Result<VerifiedPayment, BindingError> {
        ensure(
            self.amount > 0 && self.application_fee_amount < self.amount,
            "expected amount/fee",
        )?;
        ensure(self.scope == *observed_scope, "account scope")?;
        ensure(
            self.scope.livemode == session.livemode && self.scope.livemode == intent.livemode,
            "livemode",
        )?;
        ensure(session.id == self.session_id, "session id")?;
        ensure(session.mode == "payment", "session mode")?;
        ensure(
            session.amount_total == Some(self.amount) && intent.amount == self.amount,
            "amount",
        )?;
        ensure(
            session.currency.as_deref() == Some(self.currency.as_str())
                && intent.currency == self.currency,
            "currency",
        )?;
        ensure(
            intent.application_fee_amount.unwrap_or(0) == self.application_fee_amount,
            "application fee",
        )?;
        ensure(
            session
                .payment_intent
                .as_ref()
                .is_some_and(|id| id.id() == intent.id),
            "session payment intent",
        )?;
        ensure(
            self.payment_intent_id
                .as_ref()
                .is_none_or(|id| *id == intent.id),
            "payment intent id",
        )?;
        let destination = intent
            .transfer_data
            .as_ref()
            .map(|data| data.destination.id());
        ensure(
            destination == self.destination_account_id.as_deref(),
            "destination",
        )?;
        ensure(intent.on_behalf_of.is_none(), "settlement merchant")?;
        let platform_owned =
            self.scope.connected_account_id.is_none() && self.destination_account_id.is_none();
        ensure(
            self.scope.connected_account_id.is_none() || self.destination_account_id.is_none(),
            "charge model",
        )?;
        if platform_owned {
            ensure(
                self.application_fee_amount == 0
                    && intent.application_fee_amount.is_none()
                    && intent.transfer_data.is_none(),
                "platform charge routing",
            )?;
        }
        if let Some(charge) = charge {
            ensure(charge.livemode == self.scope.livemode, "charge livemode")?;
            ensure(
                intent
                    .latest_charge
                    .as_ref()
                    .is_some_and(|id| id.id() == charge.id),
                "latest charge",
            )?;
            ensure(
                charge
                    .payment_intent
                    .as_ref()
                    .is_some_and(|id| id.id() == intent.id),
                "charge payment intent",
            )?;
            ensure(
                self.charge_id.as_ref().is_none_or(|id| *id == charge.id),
                "charge id",
            )?;
            ensure(
                charge.amount == self.amount && charge.currency == self.currency,
                "charge amount/currency",
            )?;
            ensure(
                charge.application_fee_amount.unwrap_or(0) == self.application_fee_amount,
                "charge application fee",
            )?;
            if platform_owned {
                ensure(
                    charge.application_fee_amount.is_none()
                        && charge.application_fee.is_none()
                        && charge.transfer.is_none(),
                    "platform charge routing",
                )?;
            }
        }
        ensure(
            session.status.as_deref() == Some("complete"),
            "session completion",
        )?;
        match session.payment_status.as_str() {
            "unpaid" => Ok(VerifiedPayment::Processing),
            "paid" => {
                let charge = charge.ok_or(BindingError("missing paid charge"))?;
                ensure(
                    intent.status == "succeeded" && intent.amount_received == self.amount,
                    "payment intent settlement",
                )?;
                ensure(
                    charge.paid && charge.captured && charge.amount_captured == self.amount,
                    "charge capture",
                )?;
                Ok(VerifiedPayment::Paid)
            }
            _ => Err(BindingError("payment status")),
        }
    }
}

fn ensure(valid: bool, field: &'static str) -> Result<(), BindingError> {
    if valid {
        Ok(())
    } else {
        Err(BindingError(field))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (ExpectedPayment, CheckoutSession, PaymentIntent, Charge) {
        let expected = ExpectedPayment {
            scope: StripeScope::platform("acct_platform", false),
            session_id: "cs_test".into(),
            payment_intent_id: Some("pi_test".into()),
            charge_id: Some("ch_test".into()),
            amount: 11900,
            currency: "eur".into(),
            application_fee_amount: 1190,
            destination_account_id: Some("acct_seller".into()),
        };
        let session = serde_json::from_value(json!({"id":"cs_test","livemode":false,"mode":"payment","status":"complete","payment_status":"paid","amount_total":11900,"currency":"eur","payment_intent":"pi_test","ui_mode":"future_mode"})).unwrap();
        let intent = serde_json::from_value(json!({"id":"pi_test","livemode":false,"amount":11900,"amount_received":11900,"currency":"eur","status":"succeeded","application_fee_amount":1190,"transfer_data":{"destination":"acct_seller"},"latest_charge":"ch_test"})).unwrap();
        let charge = serde_json::from_value(json!({"id":"ch_test","livemode":false,"amount":11900,"amount_captured":11900,"currency":"eur","paid":true,"captured":true,"payment_intent":"pi_test","application_fee_amount":1190,"transfer":null,"application_fee":null,"balance_transaction":null})).unwrap();
        (expected, session, intent, charge)
    }

    #[test]
    fn paid_capture_can_await_transfer_objects() {
        let (expected, session, intent, charge) = fixture();
        assert_eq!(
            expected.verify(&expected.scope, &session, &intent, Some(&charge)),
            Ok(VerifiedPayment::Paid)
        );
    }

    #[test]
    fn platform_capture_has_no_connected_recipient_or_fee() {
        let (mut expected, session, mut intent, mut charge) = fixture();
        expected.destination_account_id = None;
        expected.application_fee_amount = 0;
        intent.transfer_data = None;
        intent.application_fee_amount = None;
        charge.application_fee_amount = None;
        assert_eq!(
            expected.verify(&expected.scope, &session, &intent, Some(&charge)),
            Ok(VerifiedPayment::Paid)
        );

        charge.transfer = Some(serde_json::from_value(json!("tr_unexpected")).unwrap());
        assert_eq!(
            expected.verify(&expected.scope, &session, &intent, Some(&charge)),
            Err(BindingError("platform charge routing"))
        );
        charge.transfer = None;
        charge.application_fee = Some(serde_json::from_value(json!("fee_unexpected")).unwrap());
        assert_eq!(
            expected.verify(&expected.scope, &session, &intent, Some(&charge)),
            Err(BindingError("platform charge routing"))
        );
        charge.application_fee = None;
        intent.application_fee_amount = Some(0);
        assert_eq!(
            expected.verify(&expected.scope, &session, &intent, Some(&charge)),
            Err(BindingError("platform charge routing"))
        );
        intent.application_fee_amount = None;
        intent.transfer_data =
            Some(serde_json::from_value(json!({"destination":"acct_unexpected"})).unwrap());
        assert_eq!(
            expected.verify(&expected.scope, &session, &intent, Some(&charge)),
            Err(BindingError("destination"))
        );
    }

    #[test]
    fn zero_fee_destination_capture_does_not_require_fee_objects() {
        let (mut expected, session, mut intent, mut charge) = fixture();
        expected.application_fee_amount = 0;
        intent.application_fee_amount = None;
        charge.application_fee_amount = None;
        assert_eq!(
            expected.verify(&expected.scope, &session, &intent, Some(&charge)),
            Ok(VerifiedPayment::Paid)
        );
    }

    #[test]
    fn rejects_money_scope_and_object_substitution() {
        let (expected, mut session, mut intent, charge) = fixture();
        session.amount_total = Some(1);
        assert_eq!(
            expected.verify(&expected.scope, &session, &intent, Some(&charge)),
            Err(BindingError("amount"))
        );
        session.amount_total = Some(11900);
        intent.application_fee_amount = Some(1000);
        assert_eq!(
            expected.verify(&expected.scope, &session, &intent, Some(&charge)),
            Err(BindingError("application fee"))
        );
        let wrong_scope = expected.scope.connected("acct_other");
        assert_eq!(
            expected.verify(&wrong_scope, &session, &intent, Some(&charge)),
            Err(BindingError("account scope"))
        );
    }

    #[test]
    fn unpaid_does_not_fulfill_and_no_payment_required_is_rejected() {
        let (expected, mut session, intent, charge) = fixture();
        session.payment_status = "unpaid".into();
        assert_eq!(
            expected.verify(&expected.scope, &session, &intent, None),
            Ok(VerifiedPayment::Processing)
        );
        session.payment_status = "no_payment_required".into();
        assert!(
            expected
                .verify(&expected.scope, &session, &intent, Some(&charge))
                .is_err()
        );
    }

    #[test]
    fn required_money_fields_reject_missing_negative_or_fractional_values() {
        let (_, _, intent, _) = fixture();
        let mut json = serde_json::to_value(intent).unwrap();
        json["amount"] = json!(-1);
        assert!(serde_json::from_value::<PaymentIntent>(json.clone()).is_err());
        json["amount"] = json!(1.5);
        assert!(serde_json::from_value::<PaymentIntent>(json.clone()).is_err());
        json.as_object_mut().unwrap().remove("amount");
        assert!(serde_json::from_value::<PaymentIntent>(json).is_err());
    }
}
