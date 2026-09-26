use super::error;
use crate::{entity::payment_order, error::ApiError, mail::EmailMessage, state::AppState};
use sea_orm::EntityTrait;
use serde_json::Value;

pub async fn send_order_confirmation(
    state: &AppState,
    id: &str,
    kind: &str,
) -> Result<(), ApiError> {
    let order = payment_order::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let email = confirmation_recipient(&order.snapshot, kind)?;
    let mail = state.mail_client.as_ref().ok_or_else(|| {
        error(
            "PAYMENT_EMAIL_UNAVAILABLE",
            "Payment confirmation email is not configured",
        )
    })?;
    let (subject, intro) = match kind {
        "purchased" => (
            "Your purchase confirmation",
            "Your payment was confirmed. Keep this message as a record of your purchase and the terms you accepted.",
        ),
        "withdrawn" => (
            "Your withdrawal confirmation",
            "Your withdrawal request was received. Payment history shows the refund status; this message does not confirm that funds have reached your payment account.",
        ),
        _ => {
            return Err(error(
                "PAYMENT_EMAIL_INVALID",
                "Unknown payment confirmation kind",
            ));
        }
    };
    let title = order
        .snapshot
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("App access");
    let terms = order
        .snapshot
        .get("terms_text")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            error(
                "PAYMENT_TERMS_REQUIRED",
                "The order has no retained purchase terms",
            )
        })?;
    let terms_version = order
        .snapshot
        .get("terms_version")
        .and_then(Value::as_str)
        .unwrap_or("");
    let waiver = order
        .snapshot
        .get("withdrawal_waiver")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let event = confirmation_event(&order, kind)?;
    let text = format!(
        "{intro}\n\n{event}\nOrder: {}\nProduct: {title}\nAmount: {}.{:02} {}\nConfirmation email: {email}\nPurchase terms version: {terms_version}\nImmediate delivery withdrawal waiver accepted: {waiver}\n\nAccepted purchase terms\n\n{terms}\n",
        order.id,
        order.amount / 100,
        order.amount % 100,
        order.currency.to_uppercase()
    );
    mail.send(EmailMessage {
        to: email.to_owned(),
        subject: format!("{subject} ({})", order.id),
        body_html: None,
        body_text: Some(text),
    })
    .await
    .map_err(|_| {
        error(
            "PAYMENT_EMAIL_RETRY",
            "Payment confirmation delivery will be retried",
        )
    })
}

fn confirmation_event(order: &payment_order::Model, kind: &str) -> Result<String, ApiError> {
    let timestamp = if kind == "withdrawn" {
        order.withdrawn_at.ok_or_else(|| {
            error(
                "PAYMENT_CONFIRMATION_INVALID",
                "No withdrawal was recorded for this order",
            )
        })?
    } else {
        let Some(confirmed_at) = order.snapshot.get("confirmed_at").and_then(Value::as_i64) else {
            // Older orders did not retain the first confirmation timestamp.
            return Ok("Purchase confirmed.".into());
        };
        confirmed_at
    };
    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp)
        .ok_or_else(|| ApiError::internal("Invalid payment confirmation timestamp"))?
        .to_rfc3339();
    Ok(if kind == "withdrawn" {
        let evidence = order.snapshot.get("withdrawal_confirmation");
        let declaration = evidence
            .and_then(|value| value.get("declaration"))
            .and_then(Value::as_str)
            .unwrap_or("Withdrawal from the purchase identified below.");
        let consumer = evidence
            .and_then(|value| value.get("consumer_name"))
            .and_then(Value::as_str);
        let name = consumer
            .map(|name| format!("Consumer name: {name}\n"))
            .unwrap_or_default();
        format!("{name}Declaration received: {declaration}\nReceived at: {timestamp}")
    } else {
        format!("Purchase confirmed at: {timestamp}")
    })
}

fn confirmation_recipient<'a>(snapshot: &'a Value, kind: &str) -> Result<&'a str, ApiError> {
    let chosen = (kind == "withdrawn")
        .then(|| snapshot.pointer("/withdrawal_confirmation/confirmation_email"))
        .flatten();
    let email = chosen
        .or_else(|| snapshot.get("buyer_email"))
        .or_else(|| snapshot.get("buyerEmail"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            error(
                "PAYMENT_EMAIL_UNAVAILABLE",
                "The order has no confirmation email recipient",
            )
        })?;
    validate_confirmation_email(email)?;
    Ok(email.trim())
}

pub(super) fn validate_confirmation_email(address: &str) -> Result<(), ApiError> {
    if !address.is_ascii() || address.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(ApiError::bad_request(
            "Enter a valid confirmation email address",
        ));
    }
    let address = address.trim();
    let mut parts = address.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    let safe_local = local
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&byte));
    let safe_domain = domain.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    });
    if parts.next().is_some()
        || address.len() > 254
        || local.is_empty()
        || local.len() > 64
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !safe_local
        || !domain.contains('.')
        || !safe_domain
    {
        return Err(ApiError::bad_request(
            "Enter a valid confirmation email address",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn withdrawal_receipt_uses_chosen_channel_without_changing_purchase_email() {
        let snapshot = json!({"buyer_email":"purchase@example.com","withdrawal_confirmation":{"confirmation_email":"receipt@example.com"}});
        assert_eq!(
            confirmation_recipient(&snapshot, "withdrawn").unwrap(),
            "receipt@example.com"
        );
        assert_eq!(
            confirmation_recipient(&snapshot, "purchased").unwrap(),
            "purchase@example.com"
        );
        assert_eq!(
            confirmation_recipient(&json!({"buyerEmail":"legacy@example.com"}), "withdrawn")
                .unwrap(),
            "legacy@example.com"
        );
    }

    #[test]
    fn confirmation_addresses_reject_header_injection_and_ambiguous_recipients() {
        for invalid in [
            "buyer@example.com\r\nBcc: other@example.com",
            "buyer@example.com,other@example.com",
            "buyer@.example.com",
            "buyer@example..com",
            "buyer@example.com\n",
            "buyer@@example.com",
        ] {
            assert!(
                validate_confirmation_email(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
        assert!(validate_confirmation_email("buyer+receipt@example.com").is_ok());
        assert!(validate_confirmation_email(&format!("{}@example.com", "a".repeat(65))).is_err());
    }

    #[test]
    fn withdrawal_receipt_contains_recorded_declaration_name_and_received_time() {
        let order: payment_order::Model = serde_json::from_value(json!({"id":"order","kind":"APP","user_id":"buyer","item_id":"app","payee_user_id":"seller","platform_account_id":"acct_platform","livemode":false,"status":"WITHDRAWN","charge_type":"PLATFORM","amount":1000,"currency":"eur","application_fee_amount":0,"fee_bps":0,"snapshot":{"withdrawal_confirmation":{"consumer_name":"Jörg Buyer","confirmation_email":"receipt@example.com","declaration":"I withdraw from order order."}},"cancel_requested":false,"withdrawn_at":0,"expires_at":0,"next_check_at":0,"revision":1,"created_at":0,"updated_at":0})).unwrap();
        let receipt = confirmation_event(&order, "withdrawn").unwrap();
        assert!(receipt.contains("Consumer name: Jörg Buyer"));
        assert!(receipt.contains("Declaration received: I withdraw from order order."));
        assert!(receipt.contains("Received at: 1970-01-01T00:00:00+00:00"));
    }
}
