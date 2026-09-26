use async_trait::async_trait;
use serde::de::DeserializeOwned;

use super::{RequestMethod, StripeError, StripeGateway, StripeRequest, types::*};

/// Typed reads and mutations accept the exact request already recorded by the caller.
#[async_trait]
pub trait StripeResources: StripeGateway {
    async fn create_account(&self, request: &StripeRequest) -> Result<Account, StripeError> {
        resource(self, request, RequestMethod::Post, "/v1/accounts", false).await
    }

    async fn retrieve_account(&self, request: &StripeRequest) -> Result<Account, StripeError> {
        resource(self, request, RequestMethod::Get, "/v1/accounts/", true).await
    }

    async fn create_account_link(
        &self,
        request: &StripeRequest,
    ) -> Result<AccountLink, StripeError> {
        resource(
            self,
            request,
            RequestMethod::Post,
            "/v1/account_links",
            false,
        )
        .await
    }

    async fn create_checkout_session(
        &self,
        request: &StripeRequest,
    ) -> Result<CheckoutSession, StripeError> {
        resource(
            self,
            request,
            RequestMethod::Post,
            "/v1/checkout/sessions",
            false,
        )
        .await
    }

    async fn retrieve_checkout_session(
        &self,
        request: &StripeRequest,
    ) -> Result<CheckoutSession, StripeError> {
        resource(
            self,
            request,
            RequestMethod::Get,
            "/v1/checkout/sessions/",
            true,
        )
        .await
    }

    async fn expire_checkout_session(
        &self,
        request: &StripeRequest,
    ) -> Result<CheckoutSession, StripeError> {
        if !request.path.ends_with("/expire") {
            return Err(StripeError::invalid("expected Checkout expiration request"));
        }
        resource(
            self,
            request,
            RequestMethod::Post,
            "/v1/checkout/sessions/",
            true,
        )
        .await
    }

    async fn retrieve_payment_intent(
        &self,
        request: &StripeRequest,
    ) -> Result<PaymentIntent, StripeError> {
        resource(
            self,
            request,
            RequestMethod::Get,
            "/v1/payment_intents/",
            true,
        )
        .await
    }

    async fn retrieve_charge(&self, request: &StripeRequest) -> Result<Charge, StripeError> {
        resource(self, request, RequestMethod::Get, "/v1/charges/", true).await
    }

    async fn create_refund(&self, request: &StripeRequest) -> Result<Refund, StripeError> {
        resource(self, request, RequestMethod::Post, "/v1/refunds", false).await
    }

    async fn retrieve_refund(&self, request: &StripeRequest) -> Result<Refund, StripeError> {
        resource(self, request, RequestMethod::Get, "/v1/refunds/", true).await
    }

    async fn list_refunds(
        &self,
        request: &StripeRequest,
    ) -> Result<StripeList<Refund>, StripeError> {
        resource(self, request, RequestMethod::Get, "/v1/refunds", false).await
    }

    async fn create_transfer(&self, request: &StripeRequest) -> Result<Transfer, StripeError> {
        resource(self, request, RequestMethod::Post, "/v1/transfers", false).await
    }

    async fn retrieve_transfer(&self, request: &StripeRequest) -> Result<Transfer, StripeError> {
        resource(self, request, RequestMethod::Get, "/v1/transfers/", true).await
    }

    async fn list_transfers(
        &self,
        request: &StripeRequest,
    ) -> Result<StripeList<Transfer>, StripeError> {
        resource(self, request, RequestMethod::Get, "/v1/transfers", false).await
    }

    async fn create_transfer_reversal(
        &self,
        request: &StripeRequest,
    ) -> Result<TransferReversal, StripeError> {
        if !request.path.ends_with("/reversals") {
            return Err(StripeError::invalid("expected transfer reversal request"));
        }
        resource(self, request, RequestMethod::Post, "/v1/transfers/", true).await
    }

    async fn list_transfer_reversals(
        &self,
        request: &StripeRequest,
    ) -> Result<StripeList<TransferReversal>, StripeError> {
        if !request.path.ends_with("/reversals") {
            return Err(StripeError::invalid(
                "expected transfer reversal list request",
            ));
        }
        resource(self, request, RequestMethod::Get, "/v1/transfers/", true).await
    }

    async fn create_application_fee_refund(
        &self,
        request: &StripeRequest,
    ) -> Result<FeeRefund, StripeError> {
        if !request.path.ends_with("/refunds") {
            return Err(StripeError::invalid(
                "expected application fee refund request",
            ));
        }
        resource(
            self,
            request,
            RequestMethod::Post,
            "/v1/application_fees/",
            true,
        )
        .await
    }

    async fn retrieve_application_fee(
        &self,
        request: &StripeRequest,
    ) -> Result<ApplicationFee, StripeError> {
        resource(
            self,
            request,
            RequestMethod::Get,
            "/v1/application_fees/",
            true,
        )
        .await
    }

    async fn retrieve_dispute(&self, request: &StripeRequest) -> Result<Dispute, StripeError> {
        resource(self, request, RequestMethod::Get, "/v1/disputes/", true).await
    }

    async fn retrieve_balance(&self, request: &StripeRequest) -> Result<Balance, StripeError> {
        resource(self, request, RequestMethod::Get, "/v1/balance", false).await
    }

    async fn list_payouts(
        &self,
        request: &StripeRequest,
    ) -> Result<StripeList<Payout>, StripeError> {
        resource(self, request, RequestMethod::Get, "/v1/payouts", false).await
    }

    async fn retrieve_balance_transaction(
        &self,
        request: &StripeRequest,
    ) -> Result<BalanceTransaction, StripeError> {
        resource(
            self,
            request,
            RequestMethod::Get,
            "/v1/balance_transactions/",
            true,
        )
        .await
    }
}

impl<G: StripeGateway + ?Sized> StripeResources for G {}

async fn resource<G: StripeGateway + ?Sized, T: DeserializeOwned>(
    gateway: &G,
    request: &StripeRequest,
    method: RequestMethod,
    path: &str,
    prefix: bool,
) -> Result<T, StripeError> {
    if request.method != method
        || !(if prefix {
            request.path.starts_with(path) && request.path.len() > path.len()
        } else {
            request.path == path
        })
    {
        return Err(StripeError::invalid(
            "request does not match the expected Stripe resource",
        ));
    }
    gateway.execute(request).await?.decode()
}
