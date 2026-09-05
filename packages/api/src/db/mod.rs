//! Database-engine portability layer, provided by `flow-like-db` so the AWS,
//! Azure and GCP workers share it without depending on this crate.

pub use flow_like_db::*;

#[cfg(feature = "db-chaos")]
pub mod testing;

pub(crate) mod coordination;

#[cfg(test)]
mod consistency_tests;
pub mod lease;

#[cfg(test)]
mod tests {
    use crate::state::State;
    use sea_orm::DbErr;

    fn assert_send<T: Send>(_: &T) {}

    #[allow(dead_code)]
    fn state_transaction_future_is_send(state: &State) {
        let future = state.transaction(|_txn| Box::pin(async { Ok::<(), DbErr>(()) }));
        assert_send(&future);
    }

    #[test]
    fn retry_policy_defaults_are_not_idempotent() {
        let policy = super::RetryPolicy::default();
        assert!(!policy.idempotent);
        assert_eq!(policy.max_total, std::time::Duration::from_secs(2));
    }
}
