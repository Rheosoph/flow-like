use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

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

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct PaymentLegalText {
    pub kind: String,
    pub version: String,
    pub locale: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

// Website pages and payment consent use the same immutable source bytes.
static BUNDLED_LEGAL_TEXTS: LazyLock<Vec<PaymentLegalText>> = LazyLock::new(|| {
    // Append new versions and retain existing bundles for historical consent.
    let sources = [include_str!(
        "../../../../../apps/website/src/content/legal/payments/2026-09-20-draft-1.json"
    )];
    sources
        .into_iter()
        .flat_map(|source| {
            serde_json::from_str::<Vec<PaymentLegalText>>(source)
                .expect("Bundled payment legal texts must be valid JSON")
        })
        .collect()
});

impl PaymentLegalText {
    /// Resolve the exact content used for display, consent hashes and evidence.
    pub fn content(&self) -> Result<&str, String> {
        if self.kind.trim().is_empty()
            || self.version.trim().is_empty()
            || self.locale.trim().is_empty()
        {
            return Err("Payment legal texts require kind, version and locale".into());
        }
        if let Some(url) = &self.url {
            let url = url::Url::parse(url).map_err(|_| "Invalid payment legal text URL")?;
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("Payment legal text URLs must use HTTPS without credentials".into());
            }
        }
        let content = if !self.text.trim().is_empty() {
            self.text.as_str()
        } else {
            let url = self
                .url
                .as_deref()
                .ok_or("Payment legal texts require inline content or a bundled URL")?;
            if self.hash.is_none() {
                return Err(
                    "Payment legal text URL references require a pinned BLAKE3 hash".into(),
                );
            }
            let slug = match self.kind.as_str() {
                "PAYMENTS_OWNER_TERMS" => "owner-terms",
                "SELLER_TERMS" => "seller-terms",
                "PURCHASE_TERMS" => "purchase-terms",
                _ => "",
            };
            let expected_url = format!(
                "https://flow-like.com/legal/payments/{slug}/{}/{}/",
                self.version, self.locale
            );
            let bundled = BUNDLED_LEGAL_TEXTS
                .iter()
                .find(|text| {
                    !slug.is_empty()
                        && url == expected_url
                        && text.kind == self.kind
                        && text.version == self.version
                        && text.locale == self.locale
                })
                .ok_or_else(|| {
                    format!(
                        "Payment legal text {}/{}/{} is not bundled; custom agreements require inline content",
                        self.kind, self.version, self.locale
                    )
                })?;
            bundled.text.as_str()
        };
        if content.trim().is_empty() {
            return Err("Payment legal text content must not be empty".into());
        }
        if let Some(hash) = &self.hash {
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(
                    "Payment legal text hashes must be lowercase BLAKE3 hex digests".into(),
                );
            }
            if blake3::hash(content.as_bytes()).to_hex().as_str() != hash {
                return Err(format!(
                    "Payment legal text hash mismatch for {}/{}/{}",
                    self.kind, self.version, self.locale
                ));
            }
        }
        Ok(content)
    }
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
                        && text.content().is_ok()
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
                    || !keys.insert((&text.kind, &text.version, &text.locale))
                {
                    return Err("Payment legal texts must be nonempty and unique by kind, version and locale".into());
                }
                text.content()?;
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
                ..Default::default()
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
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn bundled_reference() -> PaymentLegalText {
        serde_json::from_value(serde_json::json!({
            "kind": "PAYMENTS_OWNER_TERMS",
            "version": "2026-09-20-draft-1",
            "locale": "en",
            "url": "https://flow-like.com/legal/payments/owner-terms/2026-09-20-draft-1/en/",
            "hash": "16991d96d4ef055c598d88282ba8945d50d1792477627f68d9a4ec1968480968"
        }))
        .unwrap()
    }

    #[test]
    fn configured_website_references_resolve_the_original_content() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../../../../flow-like.config.json")).unwrap();
        let payments: PaymentsConfig = serde_json::from_value(config["payments"].clone()).unwrap();
        assert!(!payments.legal_texts.is_empty());
        for reference in &payments.legal_texts {
            assert!(reference.text.is_empty());
            let content = reference.content().unwrap();
            let original = BUNDLED_LEGAL_TEXTS
                .iter()
                .find(|text| {
                    text.kind == reference.kind
                        && text.version == reference.version
                        && text.locale == reference.locale
                })
                .unwrap();
            assert_eq!(content, original.text);
            assert_eq!(
                reference.hash.as_deref(),
                Some(blake3::hash(content.as_bytes()).to_hex().as_str())
            );
            let serialized = serde_json::to_value(reference).unwrap();
            assert!(serialized.get("text").is_none());
        }
    }

    #[test]
    fn inline_agreements_remain_supported_and_verify_optional_hashes() {
        let mut text: PaymentLegalText = serde_json::from_value(serde_json::json!({
            "kind": "PAYMENTS_OWNER_TERMS", "version": "v1", "locale": "en",
            "text": "Owner agreement\n"
        }))
        .unwrap();
        assert_eq!(text.content().unwrap(), "Owner agreement\n");
        text.url = Some("https://self-hosted.example/legal/v1/en/".into());
        text.hash = Some(blake3::hash(text.text.as_bytes()).to_hex().to_string());
        assert_eq!(text.content().unwrap(), "Owner agreement\n");
        text.text.pop();
        assert!(text.content().unwrap_err().contains("hash mismatch"));
    }

    #[test]
    fn website_references_require_the_pinned_content_hash() {
        let mut text = bundled_reference();
        assert!(text.content().is_ok());
        text.hash = None;
        assert!(text.content().unwrap_err().contains("pinned BLAKE3"));
        text.hash = Some("0".repeat(64));
        assert!(text.content().unwrap_err().contains("hash mismatch"));
        for hash in ["short".to_owned(), "F".repeat(64)] {
            text.hash = Some(hash);
            assert!(text.content().unwrap_err().contains("lowercase BLAKE3"));
        }
    }

    #[test]
    fn website_references_cannot_substitute_versions_locales_kinds_or_hosts() {
        for change in ["version", "locale", "kind", "url"] {
            let mut text = bundled_reference();
            match change {
                "version" => text.version = "unavailable-version".into(),
                "locale" => text.locale = "fr".into(),
                "kind" => text.kind = "SELLER_TERMS".into(),
                "url" => text.url = Some("https://example.com/other-terms".into()),
                _ => unreachable!(),
            }
            assert!(text.content().unwrap_err().contains("not bundled"));
        }
    }

    #[test]
    fn legal_urls_require_https_without_credentials_even_with_inline_content() {
        let mut text = onboarding().legal_texts.remove(0);
        for url in [
            "http://example.com/terms",
            "https://user@example.com/terms",
            "https://user:password@example.com/terms",
            "file:///terms.txt",
            "invalid-url",
        ] {
            text.url = Some(url.into());
            assert!(text.content().is_err(), "{url}");
        }
        text.url = Some("https://example.com/terms/v1/en/".into());
        assert!(text.content().is_ok());
    }

    #[test]
    fn bundled_agreements_preserve_creation_and_live_draft_checks() {
        let mut config = onboarding();
        let reference = bundled_reference();
        config.owner_terms_version = Some(reference.version.clone());
        config.legal_texts = vec![reference];
        assert!(config.validate().is_ok());
        config.legal_texts.push(config.legal_texts[0].clone());
        assert!(config.validate().unwrap_err().contains("unique"));
        config.legal_texts.pop();
        config.legal_texts[0].hash = Some("0".repeat(64));
        assert!(config.validate().unwrap_err().contains("hash mismatch"));
        config.legal_texts[0] = bundled_reference();
        config.livemode = true;
        config.live_approved = true;
        assert!(config.validate().unwrap_err().contains("Draft"));
        config.onboarding_enabled = false;
        assert!(config.validate().is_ok());
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
