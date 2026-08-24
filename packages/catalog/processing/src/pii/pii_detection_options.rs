//! PII Detection Options builder node
//!
//! This node creates a PiiDetectionOptions configuration struct that can be
//! passed to PII masking nodes for fine-grained control over detection.

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use super::pii_mask_regex::PiiDetectionOptions;

#[crate::register_node]
#[derive(Default)]
pub struct PiiDetectionOptionsNode {}

impl PiiDetectionOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for PiiDetectionOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "processing_pii_detection_options",
            "PII Detection Options",
            "Configure which PII types to detect. Connect to PII Mask nodes for fine-grained control.",
            "Processing/Privacy",
        );
        node.set_flowscript_name("ai.processing", "piiDetectionOptions");
        node.add_icon("/flow/icons/settings.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(10)
                .set_governance(10)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        // Contact Information
        node.add_input_pin(
            "email",
            "Email",
            "Detect email addresses (e.g., user@example.com)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "phone",
            "Phone",
            "Detect phone numbers (international formats)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "url",
            "URL",
            "Detect URLs and web addresses",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "ip_address",
            "IP Address",
            "Detect IPv4 and IPv6 addresses",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Financial
        node.add_input_pin(
            "credit_card",
            "Credit Card",
            "Detect credit card numbers (13-19 digits)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "iban",
            "IBAN",
            "Detect IBAN bank account numbers (international)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "vat_eu",
            "EU VAT",
            "Detect EU VAT numbers",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Government IDs - US
        node.add_input_pin(
            "ssn",
            "US SSN",
            "Detect US Social Security Numbers (XXX-XX-XXXX)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Government IDs - DACH (Germany, Austria, Switzerland)
        node.add_input_pin(
            "german_tax_id",
            "German Tax ID",
            "Detect German Steuer-ID (11 digits)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "ahv_swiss",
            "Swiss AHV",
            "Detect Swiss AHV numbers (756.XXXX.XXXX.XX)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "svnr_austria",
            "Austrian SVNR",
            "Detect Austrian social insurance numbers",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Documents
        node.add_input_pin(
            "passport",
            "Passport",
            "Detect passport numbers (various formats)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "drivers_license",
            "Driver's License",
            "Detect driver's license numbers (basic patterns)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Addresses - US
        node.add_input_pin(
            "address_us",
            "US Address",
            "Detect US street addresses",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Addresses - DACH
        node.add_input_pin(
            "address_de",
            "German Address",
            "Detect German addresses (Straße, Platz, Weg, etc.)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Postcodes
        node.add_input_pin(
            "postcode_uk",
            "UK Postcode",
            "Detect UK postcodes",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "postcode_de",
            "German PLZ",
            "Detect German postcodes (5 digits)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Date
        node.add_input_pin(
            "date",
            "Date",
            "Detect date patterns (DD/MM/YYYY, YYYY-MM-DD, etc.)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Output
        node.add_output_pin(
            "options",
            "Options",
            "PII Detection Options configuration struct",
            VariableType::Struct,
        )
        .set_schema::<PiiDetectionOptions>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let options = PiiDetectionOptions {
            email: context.evaluate_pin("email").await.unwrap_or(true),
            phone: context.evaluate_pin("phone").await.unwrap_or(true),
            ssn: context.evaluate_pin("ssn").await.unwrap_or(true),
            german_tax_id: context.evaluate_pin("german_tax_id").await.unwrap_or(true),
            credit_card: context.evaluate_pin("credit_card").await.unwrap_or(true),
            ip_address: context.evaluate_pin("ip_address").await.unwrap_or(true),
            url: context.evaluate_pin("url").await.unwrap_or(true),
            date: context.evaluate_pin("date").await.unwrap_or(true),
            iban: context.evaluate_pin("iban").await.unwrap_or(true),
            address_us: context.evaluate_pin("address_us").await.unwrap_or(true),
            address_de: context.evaluate_pin("address_de").await.unwrap_or(true),
            postcode_uk: context.evaluate_pin("postcode_uk").await.unwrap_or(true),
            postcode_de: context.evaluate_pin("postcode_de").await.unwrap_or(true),
            passport: context.evaluate_pin("passport").await.unwrap_or(true),
            vat_eu: context.evaluate_pin("vat_eu").await.unwrap_or(true),
            drivers_license: context
                .evaluate_pin("drivers_license")
                .await
                .unwrap_or(true),
            ahv_swiss: context.evaluate_pin("ahv_swiss").await.unwrap_or(true),
            svnr_austria: context.evaluate_pin("svnr_austria").await.unwrap_or(true),
        };

        context.set_pin_value("options", json!(options)).await?;

        Ok(())
    }
}
