use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaymentTaxMode {
    #[default]
    Unconfigured,
    SellerSupplier,
    PlatformSupplier,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaymentFeeBasis {
    #[default]
    Gross,
    NetOfTax,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PaymentLegalText {
    pub kind: String,
    pub version: String,
    pub locale: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PaymentsConfig {
    pub onboarding_enabled: bool,
    pub marketplace_enabled: bool,
    pub node_payments_enabled: bool,
    pub servicing_enabled: bool,
    pub livemode: bool,
    pub live_approved: bool,
    pub platform_account_id: Option<String>,
    pub countries: Vec<String>,
    pub seller_allowlist: Vec<String>,
    pub currencies: Vec<String>,
    pub node_payment_methods: Vec<String>,
    pub marketplace_payment_methods: Vec<String>,
    pub node_method_configuration: Option<String>,
    pub marketplace_method_configuration: Option<String>,
    pub node_fee_bps: u16,
    pub marketplace_fee_bps: u16,
    pub marketplace_fee_basis: PaymentFeeBasis,
    pub marketplace_min_amount: i64,
    pub max_payment_amount: i64,
    pub new_seller_daily_cap: i64,
    pub node_tax_mode: PaymentTaxMode,
    pub marketplace_tax_mode: PaymentTaxMode,
    pub owner_terms_version: Option<String>,
    pub seller_terms_version: Option<String>,
    pub purchase_terms_version: Option<String>,
    pub withdrawal_days: u16,
    pub purchase_waiver_version: Option<String>,
    pub product_tax_code: Option<String>,
    pub frontend_url: Option<String>,
    pub legacy_checkout_until: Option<i64>,
    pub legal_texts: Vec<PaymentLegalText>,
}

impl Default for PaymentsConfig {
    fn default() -> Self {
        Self {
            onboarding_enabled: false,
            marketplace_enabled: false,
            node_payments_enabled: false,
            servicing_enabled: false,
            livemode: false,
            live_approved: false,
            platform_account_id: None,
            countries: vec!["DE".into()],
            seller_allowlist: vec![],
            currencies: vec!["eur".into()],
            node_payment_methods: vec!["card".into(), "link".into()],
            marketplace_payment_methods: vec!["card".into(), "link".into()],
            node_method_configuration: None,
            marketplace_method_configuration: None,
            node_fee_bps: 100,
            marketplace_fee_bps: 1000,
            marketplace_fee_basis: PaymentFeeBasis::Gross,
            marketplace_min_amount: 300,
            max_payment_amount: 100_000,
            new_seller_daily_cap: 200_000,
            node_tax_mode: PaymentTaxMode::Unconfigured,
            marketplace_tax_mode: PaymentTaxMode::Unconfigured,
            owner_terms_version: None,
            seller_terms_version: None,
            purchase_terms_version: None,
            withdrawal_days: 14,
            purchase_waiver_version: None,
            product_tax_code: None,
            frontend_url: None,
            legacy_checkout_until: None,
            legal_texts: vec![],
        }
    }
}

impl PaymentsConfig {
    pub fn creation_enabled(&self) -> bool {
        self.onboarding_enabled || self.marketplace_enabled || self.node_payments_enabled
    }

    pub fn allows_seller_id(&self, user_id: &str) -> bool {
        self.seller_allowlist.is_empty()
            || self.seller_allowlist.iter().any(|seller| seller == user_id)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.creation_enabled() && !self.servicing_enabled {
            return Ok(());
        }
        if self.creation_enabled() && !self.servicing_enabled {
            return Err("Payment creation requires payment servicing".into());
        }
        if self.creation_enabled()
            && self.platform_account_id.as_deref().is_none_or(|id| {
                !id.starts_with("acct_")
                    || id.len() <= 5
                    || id.len() > 128
                    || !id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
        {
            return Err(
                "Payment creation requires the platform account ID used by webhook ingress".into(),
            );
        }
        if self.creation_enabled() && self.livemode && !self.live_approved {
            return Err(
                "Live payments require approved account, tax and commercial settings".into(),
            );
        }
        if self.node_fee_bps == 0
            || self.node_fee_bps >= 10_000
            || self.marketplace_fee_bps == 0
            || self.marketplace_fee_bps >= 10_000
        {
            return Err("Payment fees must be between 1 and 9999 basis points".into());
        }
        if self.marketplace_min_amount < 50
            || self.max_payment_amount < self.marketplace_min_amount
            || self.new_seller_daily_cap < self.max_payment_amount
        {
            return Err("Payment amount limits are inconsistent".into());
        }
        if self.currencies.is_empty() || self.currencies.iter().any(|currency| currency != "eur") {
            return Err("The initial payment release supports EUR".into());
        }
        if self.countries.is_empty()
            || self.countries.iter().any(|country| {
                country.len() != 2
                    || !country.bytes().all(|c| c.is_ascii_uppercase())
                    || matches!(country.as_str(), "BR" | "IN")
            })
        {
            return Err("Choose supported two-letter payment countries".into());
        }
        if self.onboarding_enabled
            && self
                .owner_terms_version
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err("Onboarding requires a versioned owner agreement".into());
        }
        if self.node_payments_enabled && self.node_tax_mode != PaymentTaxMode::SellerSupplier {
            return Err(
                "Direct node payments require the approved seller-supplier tax model".into(),
            );
        }
        if self.marketplace_enabled {
            if !(14..=365).contains(&self.withdrawal_days) {
                return Err("Choose a withdrawal period between 14 and 365 days".into());
            }
            if let Some(version) = &self.purchase_waiver_version
                && !self.legal_texts.iter().any(|text| {
                    text.kind == "PURCHASE_WAIVER"
                        && text.version == *version
                        && !text.text.trim().is_empty()
                })
            {
                return Err("A purchase waiver requires its approved versioned text".into());
            }
            if self.marketplace_tax_mode == PaymentTaxMode::Unconfigured {
                return Err("Marketplace tax treatment must be configured".into());
            }
            if self
                .seller_terms_version
                .as_deref()
                .is_none_or(str::is_empty)
                || self
                    .purchase_terms_version
                    .as_deref()
                    .is_none_or(str::is_empty)
            {
                return Err(
                    "Marketplace payments require versioned seller and purchase terms".into(),
                );
            }
            if self.marketplace_tax_mode == PaymentTaxMode::PlatformSupplier
                && !self
                    .product_tax_code
                    .as_deref()
                    .is_some_and(valid_product_tax_code)
            {
                return Err("Platform tax collection requires a Stripe product tax code (txcd_ followed by eight digits)".into());
            }
        }
        if self.creation_enabled() {
            let mut keys = std::collections::HashSet::new();
            for text in &self.legal_texts {
                if text.kind.trim().is_empty()
                    || text.version.trim().is_empty()
                    || text.locale.trim().is_empty()
                    || text.text.trim().is_empty()
                    || !keys.insert((&text.kind, &text.version, &text.locale))
                {
                    return Err("Payment legal texts must be nonempty and unique by kind, version and locale".into());
                }
            }
            for (required, kind, version) in [
                (
                    self.creation_enabled(),
                    "PAYMENTS_OWNER_TERMS",
                    self.owner_terms_version.as_deref(),
                ),
                (
                    self.marketplace_enabled,
                    "SELLER_TERMS",
                    self.seller_terms_version.as_deref(),
                ),
                (
                    self.marketplace_enabled,
                    "PURCHASE_TERMS",
                    self.purchase_terms_version.as_deref(),
                ),
            ] {
                if !required {
                    continue;
                }
                let version = version
                    .filter(|version| !version.trim().is_empty())
                    .ok_or_else(|| format!("Payment creation requires a version for {kind}"))?;
                if self.livemode && version.to_ascii_lowercase().contains("draft") {
                    return Err(format!(
                        "Draft {kind} must be reviewed and versioned before live payments"
                    ));
                }
                if !self
                    .legal_texts
                    .iter()
                    .any(|text| text.kind == kind && text.version == version && text.locale == "en")
                {
                    return Err(format!(
                        "Payment creation requires matching English text for {kind} version {version}"
                    ));
                }
            }
        }
        for (direct, methods) in [
            (true, &self.node_payment_methods),
            (false, &self.marketplace_payment_methods),
        ] {
            if methods.is_empty()
                || methods
                    .iter()
                    .any(|method| !supported_method(method, direct))
            {
                return Err("Unsupported payment method for the selected charge model".into());
            }
        }
        if self.creation_enabled() {
            let url = self
                .frontend_url
                .as_deref()
                .ok_or("Payments require a frontend URL")?;
            let url = url::Url::parse(url).map_err(|_| "Invalid payment frontend URL")?;
            if !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
                || url.host_str().is_none()
                || (url.scheme() != "https"
                    && !(url.scheme() == "http"
                        && !self.livemode
                        && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))))
            {
                return Err(
                    "Payments require an HTTPS frontend origin (localhost allowed in test mode)"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

pub fn valid_product_tax_code(code: &str) -> bool {
    code.strip_prefix("txcd_")
        .is_some_and(|digits| digits.len() == 8 && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

pub fn supported_method(method: &str, direct: bool) -> bool {
    matches!(
        method,
        "card"
            | "link"
            | "revolut_pay"
            | "alipay"
            | "wechat_pay"
            | "mobilepay"
            | "mb_way"
            | "satispay"
            | "ideal"
            | "bancontact"
            | "eps"
            | "p24"
            | "blik"
    ) || (!direct && method == "paypal")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_do_not_create_or_service_payments() {
        let config = PaymentsConfig::default();
        assert!(!config.creation_enabled());
        assert!(!config.servicing_enabled);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn empty_seller_allowlist_allows_open_enrollment() {
        let config = PaymentsConfig::default();
        assert!(config.allows_seller_id("first_owner"));
        assert!(config.allows_seller_id("another_owner"));
    }

    #[test]
    fn nonempty_seller_allowlist_restricts_enrollment() {
        let config = PaymentsConfig {
            seller_allowlist: vec!["invited_owner".into()],
            ..Default::default()
        };
        assert!(config.allows_seller_id("invited_owner"));
        assert!(!config.allows_seller_id("another_owner"));
        assert!(!config.allows_seller_id("invited_owner_suffix"));
    }

    #[test]
    fn wallet_policy_distinguishes_charge_models() {
        for method in ["card", "link", "revolut_pay", "alipay", "wechat_pay"] {
            assert!(supported_method(method, true));
            assert!(supported_method(method, false));
        }
        assert!(supported_method("paypal", false));
        assert!(!supported_method("paypal", true));
        assert!(!supported_method("sepa_debit", true));
        assert!(!supported_method("made_up", false));
    }

    #[test]
    fn a_creation_kill_switch_does_not_block_servicing() {
        let config = PaymentsConfig {
            servicing_enabled: true,
            livemode: true,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
        let mut enabled = config;
        enabled.onboarding_enabled = true;
        assert!(enabled.validate().is_err());
    }

    #[test]
    fn creation_requires_the_identity_needed_for_webhook_acceptance() {
        let mut config = PaymentsConfig {
            onboarding_enabled: true,
            servicing_enabled: true,
            owner_terms_version: Some("v1".into()),
            legal_texts: vec![PaymentLegalText {
                kind: "PAYMENTS_OWNER_TERMS".into(),
                version: "v1".into(),
                locale: "en".into(),
                text: "Owner agreement".into(),
            }],
            frontend_url: Some("https://example.com".into()),
            ..Default::default()
        };
        assert!(
            config
                .validate()
                .unwrap_err()
                .contains("platform account ID")
        );
        config.platform_account_id = Some("acct_platform".into());
        assert!(config.validate().is_ok());
    }

    fn onboarding() -> PaymentsConfig {
        PaymentsConfig {
            onboarding_enabled: true,
            servicing_enabled: true,
            platform_account_id: Some("acct_platform".into()),
            frontend_url: Some("https://app.example.com".into()),
            owner_terms_version: Some("v1".into()),
            legal_texts: vec![PaymentLegalText {
                kind: "PAYMENTS_OWNER_TERMS".into(),
                version: "v1".into(),
                locale: "en".into(),
                text: "Owner agreement".into(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn versions_without_matching_legal_text_cannot_enable_creation() {
        let mut config = onboarding();
        config.legal_texts.clear();
        assert!(
            config
                .validate()
                .unwrap_err()
                .contains("matching English text")
        );
        config = onboarding();
        config.legal_texts[0].version = "older".into();
        assert!(config.validate().is_err());
        config = onboarding();
        config.legal_texts.push(config.legal_texts[0].clone());
        assert!(config.validate().unwrap_err().contains("unique"));
    }

    #[test]
    fn drafts_work_in_test_mode_but_cannot_be_approved_for_live_payments() {
        let mut config = onboarding();
        config.owner_terms_version = Some("2026-09-20-draft-1".into());
        config.legal_texts[0].version = "2026-09-20-draft-1".into();
        assert!(config.validate().is_ok());
        config.livemode = true;
        config.live_approved = true;
        assert!(config.validate().unwrap_err().contains("Draft"));
        config.onboarding_enabled = false;
        assert!(
            config.validate().is_ok(),
            "Historical payments remain serviceable"
        );
    }

    #[test]
    fn frontend_is_an_origin_and_tax_codes_are_explicit_stripe_identifiers() {
        let mut config = onboarding();
        config.frontend_url = Some("https://app.example.com/payments".into());
        assert!(config.validate().is_err());
        assert!(valid_product_tax_code("txcd_10000000"));
        for code in [
            "txcd_test",
            "txcd_1000000",
            "txcd_100000000",
            "txcd_1000000x",
            "txcd_10000000/else",
        ] {
            assert!(!valid_product_tax_code(code));
        }
    }
}
