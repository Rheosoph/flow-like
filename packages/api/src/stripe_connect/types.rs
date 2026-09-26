use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Expandable<T> {
    Id(String),
    Object(Box<T>),
}

pub trait StripeObject {
    fn id(&self) -> &str;
}

impl<T: StripeObject> Expandable<T> {
    pub fn id(&self) -> &str {
        match self {
            Self::Id(id) => id,
            Self::Object(object) => object.id(),
        }
    }
    pub fn expanded(&self) -> Option<&T> {
        match self {
            Self::Object(object) => Some(object),
            Self::Id(_) => None,
        }
    }
}

macro_rules! stripe_object {
    ($($ty:ty),+ $(,)?) => { $(impl StripeObject for $ty { fn id(&self) -> &str { &self.id } })+ };
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Requirements {
    #[serde(default)]
    pub currently_due: Vec<String>,
    #[serde(default)]
    pub eventually_due: Vec<String>,
    #[serde(default)]
    pub past_due: Vec<String>,
    #[serde(default)]
    pub pending_verification: Vec<String>,
    #[serde(default)]
    pub disabled_reason: Option<String>,
    #[serde(default)]
    pub current_deadline: Option<u64>,
    #[serde(default)]
    pub errors: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub default_currency: Option<String>,
    #[serde(default)]
    pub charges_enabled: bool,
    #[serde(default)]
    pub payouts_enabled: bool,
    #[serde(default)]
    pub details_submitted: bool,
    #[serde(default)]
    pub capabilities: BTreeMap<String, String>,
    #[serde(default)]
    pub requirements: Requirements,
    #[serde(default)]
    pub future_requirements: Option<Requirements>,
    #[serde(default)]
    pub controller: Value,
    #[serde(default)]
    pub business_profile: Value,
    #[serde(default)]
    pub settings: Value,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AccountLink {
    pub url: String,
    pub created: u64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TotalDetails {
    pub amount_tax: u64,
    #[serde(default)]
    pub amount_discount: u64,
    #[serde(default)]
    pub amount_shipping: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutomaticTaxLiability {
    #[serde(rename = "type")]
    pub liability_type: String,
    #[serde(default)]
    pub account: Option<Expandable<Account>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutomaticTax {
    pub enabled: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub liability: Option<AutomaticTaxLiability>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CheckoutSession {
    pub id: String,
    pub livemode: bool,
    pub mode: String,
    pub payment_status: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub amount_total: Option<u64>,
    #[serde(default)]
    pub amount_subtotal: Option<u64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub payment_intent: Option<Expandable<PaymentIntent>>,
    #[serde(default)]
    pub invoice: Option<Expandable<Invoice>>,
    #[serde(default)]
    pub total_details: Option<TotalDetails>,
    #[serde(default)]
    pub automatic_tax: Option<AutomaticTax>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub client_reference_id: Option<String>,
    #[serde(default)]
    pub ui_mode: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invoice {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferData {
    pub destination: Expandable<Account>,
    #[serde(default)]
    pub amount: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaymentIntent {
    pub id: String,
    pub livemode: bool,
    pub amount: u64,
    pub currency: String,
    pub status: String,
    #[serde(default)]
    pub amount_received: u64,
    #[serde(default)]
    pub application_fee_amount: Option<u64>,
    #[serde(default)]
    pub transfer_data: Option<TransferData>,
    #[serde(default)]
    pub transfer_group: Option<String>,
    #[serde(default)]
    pub on_behalf_of: Option<Expandable<Account>>,
    #[serde(default)]
    pub latest_charge: Option<Expandable<Charge>>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Charge {
    pub id: String,
    #[serde(default)]
    pub created: u64,
    pub livemode: bool,
    pub amount: u64,
    pub currency: String,
    pub paid: bool,
    pub captured: bool,
    #[serde(default)]
    pub amount_captured: u64,
    #[serde(default)]
    pub amount_refunded: u64,
    #[serde(default)]
    pub application_fee_amount: Option<u64>,
    #[serde(default)]
    pub payment_intent: Option<Expandable<PaymentIntent>>,
    #[serde(default)]
    pub transfer: Option<Expandable<Transfer>>,
    #[serde(default)]
    pub transfer_data: Option<TransferData>,
    #[serde(default)]
    pub application_fee: Option<Expandable<ApplicationFee>>,
    #[serde(default)]
    pub balance_transaction: Option<Expandable<BalanceTransaction>>,
    #[serde(default)]
    pub receipt_url: Option<String>,
    #[serde(default)]
    pub payment_method_details: Value,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Refund {
    pub id: String,
    pub amount: u64,
    pub currency: String,
    #[serde(default)]
    pub status: Option<String>,
    pub charge: Expandable<Charge>,
    #[serde(default)]
    pub payment_intent: Option<Expandable<PaymentIntent>>,
    #[serde(default)]
    pub pending_reason: Option<String>,
    #[serde(default)]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub transfer_reversal: Option<Expandable<TransferReversal>>,
    #[serde(default)]
    pub balance_transaction: Option<Expandable<BalanceTransaction>>,
    #[serde(default)]
    pub failure_balance_transaction: Option<Expandable<BalanceTransaction>>,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transfer {
    pub id: String,
    pub livemode: bool,
    pub amount: u64,
    pub amount_reversed: u64,
    pub currency: String,
    pub destination: Expandable<Account>,
    #[serde(default)]
    pub source_transaction: Option<Expandable<Charge>>,
    #[serde(default)]
    pub transfer_group: Option<String>,
    #[serde(default)]
    pub balance_transaction: Option<Expandable<BalanceTransaction>>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferReversal {
    pub id: String,
    pub amount: u64,
    pub currency: String,
    pub transfer: Expandable<Transfer>,
    #[serde(default)]
    pub source_refund: Option<Expandable<Refund>>,
    #[serde(default)]
    pub balance_transaction: Option<Expandable<BalanceTransaction>>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApplicationFee {
    pub id: String,
    #[serde(default)]
    pub created: u64,
    pub amount: u64,
    pub amount_refunded: u64,
    pub currency: String,
    pub livemode: bool,
    pub account: Expandable<Account>,
    pub charge: Expandable<Charge>,
    #[serde(default)]
    pub originating_transaction: Option<Expandable<Charge>>,
    #[serde(default)]
    pub balance_transaction: Option<Expandable<BalanceTransaction>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeeRefund {
    pub id: String,
    #[serde(default)]
    pub created: u64,
    pub amount: u64,
    pub currency: String,
    pub fee: Expandable<ApplicationFee>,
    #[serde(default)]
    pub balance_transaction: Option<Expandable<BalanceTransaction>>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dispute {
    pub id: String,
    pub amount: u64,
    pub currency: String,
    pub livemode: bool,
    pub status: String,
    pub charge: Expandable<Charge>,
    #[serde(default)]
    pub payment_intent: Option<Expandable<PaymentIntent>>,
    #[serde(default)]
    pub balance_transactions: Vec<BalanceTransaction>,
    #[serde(default)]
    pub evidence_details: Value,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceTransaction {
    pub id: String,
    pub amount: i64,
    pub currency: String,
    pub fee: i64,
    pub net: i64,
    #[serde(rename = "type")]
    pub transaction_type: String,
    pub status: String,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub available_on: u64,
    #[serde(default)]
    pub exchange_rate: Option<f64>,
    #[serde(default)]
    pub fee_details: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceAmount {
    pub amount: i64,
    pub currency: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Balance {
    pub livemode: bool,
    pub available: Vec<BalanceAmount>,
    pub pending: Vec<BalanceAmount>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Payout {
    pub id: String,
    pub livemode: bool,
    pub amount: u64,
    pub currency: String,
    pub status: String,
    pub arrival_date: u64,
    #[serde(default)]
    pub failure_code: Option<String>,
    #[serde(default)]
    pub balance_transaction: Option<Expandable<BalanceTransaction>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StripeList<T> {
    pub data: Vec<T>,
    pub has_more: bool,
}

stripe_object!(
    Account,
    CheckoutSession,
    Invoice,
    PaymentIntent,
    Charge,
    Refund,
    Transfer,
    TransferReversal,
    ApplicationFee,
    FeeRefund,
    Dispute,
    BalanceTransaction,
    Payout
);
