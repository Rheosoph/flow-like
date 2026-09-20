//! Stripe transport and verification for persisted payment operations.

pub mod binding;
pub mod gateway;
pub mod policy;
pub mod request;
pub mod resources;
pub mod types;
pub mod webhook;

#[cfg(test)]
pub mod fake;

pub use binding::{BindingError, ExpectedPayment, VerifiedPayment};
pub use gateway::{
    HttpStripeGateway, RetryDisposition, StripeError, StripeGateway, StripeResponse,
};
pub use policy::{ChargeModel, PaymentMethodPolicy};
pub use request::{RequestMethod, StripeRequest, StripeScope};
pub use resources::StripeResources;
pub use webhook::{EventEnvelope, WebhookError, verify_webhook};

/// Candidate stable version. Live activation still requires the account-model gate.
pub const STRIPE_API_VERSION: &str = "2026-08-26.dahlia";
