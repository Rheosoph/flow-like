use super::{ApiError, error};

pub fn fee(amount: i64, basis_points: u16) -> Result<i64, ApiError> {
    if amount <= 0 || basis_points == 0 || basis_points >= 10_000 {
        return Err(error("PAYMENT_AMOUNT_INVALID", "Invalid amount or fee"));
    }
    let fee = amount
        .checked_mul(i64::from(basis_points))
        .and_then(|value| value.checked_add(9999))
        .map(|value| value / 10_000)
        .ok_or_else(|| error("PAYMENT_AMOUNT_INVALID", "Payment amount is too large"))?;
    if fee >= amount {
        return Err(error(
            "PAYMENT_AMOUNT_INVALID",
            "The fee must be smaller than the payment",
        ));
    }
    Ok(fee)
}

pub fn validate_amount(
    amount: i64,
    currency: &str,
    minimum: i64,
    maximum: i64,
) -> Result<(), ApiError> {
    if currency != "eur" {
        return Err(error(
            "PAYMENT_CURRENCY_UNSUPPORTED",
            "This payment requires EUR",
        ));
    }
    if amount < minimum || amount > maximum {
        return Err(error(
            "PAYMENT_AMOUNT_INVALID",
            "Payment amount is outside the allowed range",
        ));
    }
    Ok(())
}

pub fn request_deadline(
    now_ms: i64,
    ttl_seconds: i64,
    quota_deadline_ms: i64,
    jwt_exp_seconds: i64,
) -> Result<i64, ApiError> {
    let quota_deadline = quota_deadline_ms
        .checked_sub(120_000 + 30_000)
        .ok_or_else(|| error("PAYMENT_RUN_ENDED", "The execution deadline is unavailable"))?;
    let jwt_deadline = jwt_exp_seconds.checked_mul(1000).ok_or_else(|| {
        error(
            "PAYMENT_RUN_ENDED",
            "The executor credential has an invalid deadline",
        )
    })?;
    let ttl = ttl_seconds
        .clamp(30, 3600)
        .checked_mul(1000)
        .and_then(|ttl| now_ms.checked_add(ttl))
        .ok_or_else(|| error("PAYMENT_AMOUNT_INVALID", "Invalid payment lifetime"))?;
    let deadline = ttl.min(quota_deadline).min(jwt_deadline);
    if deadline <= now_ms {
        return Err(error(
            "PAYMENT_RUN_ENDED",
            "There is not enough execution time to accept a payment",
        ));
    }
    Ok(deadline)
}

#[derive(Debug, PartialEq, Eq)]
pub enum PaidDisposition {
    Accept,
    Duplicate,
    Orphan,
}

pub fn classify_paid(
    accepted_attempt: Option<&str>,
    attempt: &str,
    canceled: bool,
    expired: bool,
) -> PaidDisposition {
    match accepted_attempt {
        Some(accepted) if accepted == attempt => PaidDisposition::Accept,
        Some(_) => PaidDisposition::Duplicate,
        None if canceled || expired => PaidDisposition::Orphan,
        None => PaidDisposition::Accept,
    }
}

pub fn remaining_refund(
    captured: i64,
    refunded: i64,
    reserved: i64,
    amount: i64,
) -> Result<i64, ApiError> {
    if [captured, refunded, reserved]
        .iter()
        .any(|value| *value < 0)
        || amount <= 0
    {
        return Err(error("PAYMENT_REFUND_INVALID", "Invalid refund amount"));
    }
    captured
        .checked_sub(refunded)
        .and_then(|left| left.checked_sub(reserved))
        .filter(|left| amount <= *left)
        .ok_or_else(|| {
            error(
                "PAYMENT_REFUND_INVALID",
                "The refund exceeds the unreserved charge amount",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_rounding_and_limits_are_checked() {
        assert_eq!(fee(11900, 1000).unwrap(), 1190);
        assert_eq!(fee(10000, 1000).unwrap(), 1000);
        assert_eq!(fee(51, 100).unwrap(), 1);
        assert!(fee(1, 100).is_err());
        assert!(fee(i64::MAX, 1000).is_err());
        assert!(fee(100, 10000).is_err());
    }

    #[test]
    fn deadline_uses_claimed_runtime_without_bookkeeping_grace() {
        assert_eq!(
            request_deadline(1_000_000, 900, 1_420_000, 2000).unwrap(),
            1_270_000
        );
        assert!(request_deadline(1_300_000, 900, 1_420_000, 2000).is_err());
        assert_eq!(
            request_deadline(1_000_000, 900, 1_420_000, 1100).unwrap(),
            1_100_000
        );
    }

    #[test]
    fn later_success_cannot_replace_accepted_payment() {
        assert_eq!(
            classify_paid(Some("first"), "second", false, false),
            PaidDisposition::Duplicate
        );
        assert_eq!(
            classify_paid(Some("first"), "first", true, true),
            PaidDisposition::Accept
        );
        assert_eq!(
            classify_paid(None, "first", true, false),
            PaidDisposition::Orphan
        );
        assert_eq!(
            classify_paid(None, "first", false, true),
            PaidDisposition::Orphan
        );
    }

    #[test]
    fn pending_refunds_reserve_the_remaining_charge() {
        assert!(remaining_refund(100, 30, 50, 21).is_err());
        assert_eq!(remaining_refund(100, 30, 50, 20).unwrap(), 20);
        assert!(remaining_refund(100, 100, 0, 1).is_err());
    }
}

/// A package price is free, or within the marketplace range. Private packages
/// may carry a price ahead of publication; packages whose maintainers grant
/// access by request are never sold.
pub fn package_listing_price(
    amount: i64,
    visibility: &crate::entity::sea_orm_active_enums::WasmPackageVisibility,
    minimum: i64,
    maximum: i64,
) -> Result<(), ApiError> {
    use crate::entity::sea_orm_active_enums::WasmPackageVisibility;
    if amount == 0 {
        return Ok(());
    }
    if *visibility == WasmPackageVisibility::PublicRequestAccess {
        return Err(error(
            "PRICE_REQUIRES_OPEN_LISTING",
            "Packages whose maintainers approve access requests can't be sold. Make the package public to sell it.",
        ));
    }
    if amount < minimum {
        return Err(error(
            "PRICE_BELOW_MINIMUM",
            "The price is below the platform minimum",
        ));
    }
    validate_amount(amount, "eur", minimum, maximum)
}

pub fn listing_price(
    amount: i64,
    visibility: &crate::entity::sea_orm_active_enums::Visibility,
    minimum: i64,
    maximum: i64,
) -> Result<(), ApiError> {
    use crate::entity::sea_orm_active_enums::Visibility;
    if amount == 0 {
        return Ok(());
    }
    if !matches!(
        visibility,
        Visibility::Public | Visibility::PublicRequestAccess
    ) {
        return Err(error(
            "PRICE_REQUIRES_PUBLIC",
            "Publish the app before setting a price",
        ));
    }
    if amount < minimum {
        return Err(error(
            "PRICE_BELOW_MINIMUM",
            "The price is below the platform minimum",
        ));
    }
    validate_amount(amount, "eur", minimum, maximum)
}
