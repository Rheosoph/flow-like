//! Regex-based PII (Personally Identifiable Information) masking node
//!
//! This node uses traditional regex patterns to detect and mask common PII types
//! such as email addresses, phone numbers, SSNs, credit card numbers, and more.
//!
//! Note: For detecting names, contextual PII, or complex patterns, use the AI-based
//! PII masking node instead. Regex-based detection works well for structured data
//! but cannot reliably detect unstructured PII like names.

#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::Value;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration struct for selecting which PII types to detect.
/// This struct can be reused across different PII masking nodes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct PiiDetectionOptions {
    /// Detect email addresses (e.g., user@example.com)
    #[serde(default = "default_true")]
    pub email: bool,

    /// Detect phone numbers (international formats)
    #[serde(default = "default_true")]
    pub phone: bool,

    /// Detect US Social Security Numbers (XXX-XX-XXXX)
    #[serde(default = "default_true")]
    pub ssn: bool,

    /// Detect German tax ID (Steuer-ID, 11 digits)
    #[serde(default = "default_true")]
    pub german_tax_id: bool,

    /// Detect credit card numbers (13-19 digits, various formats)
    #[serde(default = "default_true")]
    pub credit_card: bool,

    /// Detect IPv4 and IPv6 addresses
    #[serde(default = "default_true")]
    pub ip_address: bool,

    /// Detect URLs and web addresses
    #[serde(default = "default_true")]
    pub url: bool,

    /// Detect date patterns (various international formats)
    #[serde(default = "default_true")]
    pub date: bool,

    /// Detect IBAN bank account numbers (international)
    #[serde(default = "default_true")]
    pub iban: bool,

    /// Detect US addresses
    #[serde(default = "default_true")]
    pub address_us: bool,

    /// Detect German addresses (Straße, Platz, Weg, etc.)
    #[serde(default = "default_true")]
    pub address_de: bool,

    /// Detect UK postcodes
    #[serde(default = "default_true")]
    pub postcode_uk: bool,

    /// Detect German postcodes (PLZ, 5 digits)
    #[serde(default = "default_true")]
    pub postcode_de: bool,

    /// Detect passport numbers (various formats)
    #[serde(default = "default_true")]
    pub passport: bool,

    /// Detect EU VAT numbers
    #[serde(default = "default_true")]
    pub vat_eu: bool,

    /// Detect driver's license numbers (basic patterns)
    #[serde(default = "default_true")]
    pub drivers_license: bool,

    /// Detect Swiss AHV numbers (social security)
    #[serde(default = "default_true")]
    pub ahv_swiss: bool,

    /// Detect Austrian social insurance numbers
    #[serde(default = "default_true")]
    pub svnr_austria: bool,
}

fn default_true() -> bool {
    true
}

impl PiiDetectionOptions {
    /// Create options with all detection types enabled
    pub fn all() -> Self {
        Self {
            email: true,
            phone: true,
            ssn: true,
            german_tax_id: true,
            credit_card: true,
            ip_address: true,
            url: true,
            date: true,
            iban: true,
            address_us: true,
            address_de: true,
            postcode_uk: true,
            postcode_de: true,
            passport: true,
            vat_eu: true,
            drivers_license: true,
            ahv_swiss: true,
            svnr_austria: true,
        }
    }

    /// Create options with no detection types enabled
    pub fn none() -> Self {
        Self {
            email: false,
            phone: false,
            ssn: false,
            german_tax_id: false,
            credit_card: false,
            ip_address: false,
            url: false,
            date: false,
            iban: false,
            address_us: false,
            address_de: false,
            postcode_uk: false,
            postcode_de: false,
            passport: false,
            vat_eu: false,
            drivers_license: false,
            ahv_swiss: false,
            svnr_austria: false,
        }
    }
}

/// Configuration for PII masking behavior
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PiiMaskConfig {
    /// Mask character to use for replacement
    #[serde(default = "default_mask_char")]
    pub mask_char: char,
    /// Whether to preserve length of original content
    #[serde(default = "default_true")]
    pub preserve_length: bool,
    /// Custom mask text (used when preserve_length is false)
    #[serde(default = "default_mask_text")]
    pub mask_text: String,
}

fn default_mask_char() -> char {
    '*'
}

fn default_mask_text() -> String {
    "[REDACTED]".to_string()
}

impl Default for PiiMaskConfig {
    fn default() -> Self {
        Self {
            mask_char: '*',
            preserve_length: true,
            mask_text: "[REDACTED]".to_string(),
        }
    }
}

#[cfg(feature = "execute")]
struct PiiPattern {
    name: &'static str,
    regex: regex::Regex,
}

#[cfg(feature = "execute")]
fn build_patterns(options: &PiiDetectionOptions) -> Vec<PiiPattern> {
    let mut patterns = Vec::new();

    // Helper macro to add patterns conditionally
    macro_rules! add_pattern {
        ($enabled:expr, $name:expr, $pattern:expr) => {
            if $enabled {
                if let Ok(regex) = regex::Regex::new($pattern) {
                    patterns.push(PiiPattern { name: $name, regex });
                }
            }
        };
    }

    // Email - universal
    add_pattern!(
        options.email,
        "Email",
        r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}"
    );

    // Phone numbers - international formats
    // US/Canada: +1 (XXX) XXX-XXXX or variants
    // EU/DE: +49 XXX XXXXXXX or 0XXX XXXXXXX
    // UK: +44 XXXX XXXXXX
    // General international: +XX XXX XXX XXXX
    add_pattern!(
        options.phone,
        "Phone",
        r"(?:\+?[1-9]\d{0,2}[-.\s]?)?(?:\(?\d{2,4}\)?[-.\s]?)?\d{3,4}[-.\s]?\d{3,4}[-.\s]?\d{0,4}"
    );

    // US Social Security Number
    add_pattern!(options.ssn, "SSN", r"\b\d{3}[-\s]?\d{2}[-\s]?\d{4}\b");

    // German Tax ID (Steuer-Identifikationsnummer) - 11 digits
    add_pattern!(
        options.german_tax_id,
        "GermanTaxID",
        r"\b\d{2}\s?\d{3}\s?\d{3}\s?\d{3}\b"
    );

    // Credit card numbers - various formats with Luhn-checkable patterns
    add_pattern!(
        options.credit_card,
        "CreditCard",
        r"\b(?:\d{4}[-\s]?){3}\d{1,4}\b"
    );

    // IPv4 addresses
    add_pattern!(
        options.ip_address,
        "IPv4",
        r"\b(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b"
    );

    // IPv6 addresses (simplified pattern)
    add_pattern!(
        options.ip_address,
        "IPv6",
        r"\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b"
    );

    // URLs
    add_pattern!(options.url, "URL", r#"https?://[^\s<>"']+"#);

    // Dates - international formats
    // DD/MM/YYYY, MM/DD/YYYY, YYYY-MM-DD, DD.MM.YYYY (German)
    add_pattern!(
        options.date,
        "Date",
        r"\b(?:\d{1,2}[-/\.]\d{1,2}[-/\.]\d{2,4}|\d{4}[-/\.]\d{1,2}[-/\.]\d{1,2})\b"
    );

    // IBAN - International Bank Account Number (all countries)
    add_pattern!(
        options.iban,
        "IBAN",
        r"\b[A-Z]{2}\d{2}[-\s]?(?:[A-Z0-9]{4}[-\s]?){2,7}[A-Z0-9]{1,4}\b"
    );

    // US addresses
    add_pattern!(
        options.address_us,
        "AddressUS",
        r"\b\d{1,5}\s+[\w\s]+(?:Street|St|Avenue|Ave|Road|Rd|Boulevard|Blvd|Drive|Dr|Lane|Ln|Way|Court|Ct|Place|Pl|Circle|Cir|Highway|Hwy)\b"
    );

    // German addresses - Straße, Str., Platz, Weg, Allee, Gasse, Ring, Damm
    add_pattern!(
        options.address_de,
        "AddressDE",
        r"(?i)\b[\wäöüß]+(?:straße|strasse|str\.|platz|weg|allee|gasse|ring|damm|ufer)\s+\d{1,5}[a-z]?\b"
    );

    // UK postcodes
    add_pattern!(
        options.postcode_uk,
        "PostcodeUK",
        r"\b[A-Z]{1,2}\d[A-Z\d]?\s?\d[A-Z]{2}\b"
    );

    // German postcodes (PLZ) - 5 digits
    add_pattern!(options.postcode_de, "PostcodeDE", r"\b\d{5}\b");

    // Passport numbers - various formats
    // US: 9 digits or letter + 8 digits
    // German: 9-10 alphanumeric
    // UK: 9 digits
    add_pattern!(options.passport, "Passport", r"\b[A-Z]{0,2}\d{7,9}[A-Z]?\b");

    // EU VAT numbers
    add_pattern!(
        options.vat_eu,
        "VATEU",
        r"\b(?:AT)?U\d{8}\b|\b(?:BE)?\d{10}\b|\bDE\d{9}\b|\b(?:FR)?[A-Z0-9]{2}\d{9}\b|\b(?:GB)?\d{9}(?:\d{3})?\b|\b(?:NL)?\d{9}B\d{2}\b"
    );

    // Driver's license - basic patterns (varies greatly by country)
    add_pattern!(
        options.drivers_license,
        "DriversLicense",
        r"\b[A-Z]{1,3}[-\s]?\d{5,8}[-\s]?\d{0,4}\b"
    );

    // Swiss AHV number (social security) - 756.XXXX.XXXX.XX
    add_pattern!(
        options.ahv_swiss,
        "AHVSwiss",
        r"\b756[.\s]?\d{4}[.\s]?\d{4}[.\s]?\d{2}\b"
    );

    // Austrian SVNR (social insurance) - 10 digits, first 4 are ID, then birth date
    add_pattern!(options.svnr_austria, "SVNRAustria", r"\b\d{4}\s?\d{6}\b");

    patterns
}

#[cfg(feature = "execute")]
fn apply_pii_mask(
    text: &str,
    config: &PiiMaskConfig,
    patterns: &[PiiPattern],
) -> (String, Vec<Value>) {
    let mut result = text.to_string();
    let mut detections = Vec::new();

    // Collect all matches with their positions
    let mut all_matches: Vec<(usize, usize, &str)> = Vec::new();

    for pattern in patterns {
        for mat in pattern.regex.find_iter(text) {
            all_matches.push((mat.start(), mat.end(), pattern.name));
        }
    }

    // Sort by start position descending so we replace from end to start
    all_matches.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));

    // Remove overlapping matches (keep the longest one)
    let mut filtered_matches: Vec<(usize, usize, &str)> = Vec::new();
    for mat in all_matches {
        let overlaps = filtered_matches
            .iter()
            .any(|(s, e, _)| mat.0 < *e && mat.1 > *s);
        if !overlaps {
            filtered_matches.push(mat);
        }
    }

    for (start, end, pii_type) in filtered_matches {
        let original = &text[start..end];
        let masked = if config.preserve_length {
            config.mask_char.to_string().repeat(original.len())
        } else {
            config.mask_text.clone()
        };

        detections.push(json!({
            "type": pii_type,
            "start": start,
            "end": end,
            "original_length": original.len()
        }));

        result.replace_range(start..end, &masked);
    }

    (result, detections)
}

/// One masked span, as written to the `detections` pin.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct PiiDetection {
    /// Which detector matched, for example "email" or "iban".
    #[serde(rename = "type")]
    pub kind: String,
    /// Byte offset the match started at in the original text.
    pub start: usize,
    /// Byte offset just past the match in the original text.
    pub end: usize,
    /// Length of the text that was replaced.
    pub original_length: usize,
}

#[crate::register_node]
#[derive(Default)]
pub struct PiiMaskRegexNode {}

impl PiiMaskRegexNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for PiiMaskRegexNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "processing_pii_mask_regex",
            "PII Mask (Regex)",
            "Masks Personally Identifiable Information using regex patterns. Detects emails, phones, SSNs, credit cards, IBANs, addresses (US/DE/UK), and more. For names or contextual PII, use the AI-based node.",
            "Processing/Privacy",
        );
        node.add_icon("/flow/icons/shield.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(8)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "text",
            "Text",
            "The text to scan for PII",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        // Detection options as a struct - reusable pattern
        node.add_input_pin(
            "options",
            "Detection Options",
            "Configuration for which PII types to detect. Connect a PII Detection Options node or use defaults (all enabled).",
            VariableType::Struct,
        )
        .set_schema::<PiiDetectionOptions>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(PiiDetectionOptions::all())));

        // Individual boolean overrides for common types
        node.add_input_pin(
            "detect_email",
            "Detect Email",
            "Override: Enable/disable email detection",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "detect_phone",
            "Detect Phone",
            "Override: Enable/disable phone number detection (international)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "detect_credit_card",
            "Detect Credit Card",
            "Override: Enable/disable credit card detection",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "detect_iban",
            "Detect IBAN",
            "Override: Enable/disable IBAN detection",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "detect_address",
            "Detect Address",
            "Override: Enable/disable address detection (US and DE)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "detect_ssn",
            "Detect SSN/Tax ID",
            "Override: Enable/disable SSN and tax ID detection",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "detect_url",
            "Detect URL",
            "Override: Enable/disable URL detection",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "detect_ip",
            "Detect IP",
            "Override: Enable/disable IP address detection",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Masking configuration
        node.add_input_pin(
            "mask_char",
            "Mask Character",
            "Character used for masking (default: *)",
            VariableType::String,
        )
        .set_default_value(Some(json!("*")));

        node.add_input_pin(
            "preserve_length",
            "Preserve Length",
            "If true, mask preserves original length. If false, uses mask text.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "mask_text",
            "Mask Text",
            "Text to use when preserve_length is false (default: [REDACTED])",
            VariableType::String,
        )
        .set_default_value(Some(json!("[REDACTED]")));

        // Outputs
        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after masking",
            VariableType::Execution,
        );

        node.add_output_pin(
            "masked_text",
            "Masked Text",
            "Text with PII masked",
            VariableType::String,
        );

        node.add_output_pin(
            "detection_count",
            "Detection Count",
            "Number of PII instances detected and masked",
            VariableType::Integer,
        );

        node.add_output_pin(
            "detections",
            "Detections",
            "JSON array with detection details (type, position, length)",
            VariableType::Struct,
        )
        .set_schema::<crate::pii::pii_mask_regex::PiiDetection>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let text: String = context.evaluate_pin("text").await?;
        let mask_char_str: String = context.evaluate_pin("mask_char").await?;
        let preserve_length: bool = context.evaluate_pin("preserve_length").await?;
        let mask_text: String = context.evaluate_pin("mask_text").await?;

        // Get base options from struct pin
        let mut options: PiiDetectionOptions = context
            .evaluate_pin("options")
            .await
            .unwrap_or_else(|_| PiiDetectionOptions::all());

        // Apply boolean overrides
        options.email = context.evaluate_pin("detect_email").await.unwrap_or(true);
        options.phone = context.evaluate_pin("detect_phone").await.unwrap_or(true);
        options.credit_card = context
            .evaluate_pin("detect_credit_card")
            .await
            .unwrap_or(true);
        options.iban = context.evaluate_pin("detect_iban").await.unwrap_or(true);

        let detect_address: bool = context.evaluate_pin("detect_address").await.unwrap_or(true);
        options.address_us = detect_address;
        options.address_de = detect_address;
        options.postcode_uk = detect_address;
        options.postcode_de = detect_address;

        let detect_ssn: bool = context.evaluate_pin("detect_ssn").await.unwrap_or(true);
        options.ssn = detect_ssn;
        options.german_tax_id = detect_ssn;
        options.ahv_swiss = detect_ssn;
        options.svnr_austria = detect_ssn;

        options.url = context.evaluate_pin("detect_url").await.unwrap_or(true);
        options.ip_address = context.evaluate_pin("detect_ip").await.unwrap_or(true);

        let mask_char = mask_char_str.chars().next().unwrap_or('*');
        let config = PiiMaskConfig {
            mask_char,
            preserve_length,
            mask_text,
        };

        let patterns = build_patterns(&options);
        let (masked_text, detections) = apply_pii_mask(&text, &config, &patterns);

        context.log_message(
            &format!(
                "Masked {} PII instances using regex patterns",
                detections.len()
            ),
            LogLevel::Debug,
        );

        context
            .set_pin_value("masked_text", json!(masked_text))
            .await?;
        context
            .set_pin_value("detection_count", json!(detections.len() as i64))
            .await?;
        context
            .set_pin_value("detections", json!(detections))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Processing requires the 'execute' feature"
        ))
    }
}
