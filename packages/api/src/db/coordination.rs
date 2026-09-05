//! Coordination for short transactions that read an aggregate before writing.
//!
//! Every participant writes the same retained row before reading its protected
//! state. PostgreSQL waits for that write; optimistic engines abort a conflicting
//! transaction so the retry starts with a fresh snapshot. No external work belongs
//! inside the transaction.

use sea_orm::{DatabaseTransaction, DbErr};

/// Domain and length prefixes keep distinct resources in separate namespaces.
/// A hash collision only serializes unrelated operations; it cannot weaken exclusion.
pub(crate) fn transaction_lock_id(domain: &str, parts: &[&str]) -> i64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.transaction-lock/v1\0");
    for part in std::iter::once(domain).chain(parts.iter().copied()) {
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    i64::from_be_bytes(hasher.finalize().as_bytes()[..8].try_into().unwrap())
}

pub(crate) async fn coordinate(
    txn: &DatabaseTransaction,
    domain: &str,
    parts: &[&str],
) -> Result<(), DbErr> {
    super::lease::touch_lock_row(txn, transaction_lock_id(domain, parts)).await
}

#[cfg(test)]
mod tests {
    use super::transaction_lock_id;

    #[test]
    fn namespaces_and_component_boundaries_are_distinct() {
        let root = transaction_lock_id("audit-root", &[]);
        assert_ne!(root, transaction_lock_id("audit-branch", &[""]));
        assert_ne!(root, transaction_lock_id("audit-branch", &["root"]));
        assert_ne!(
            transaction_lock_id("realtime", &["ab", "c"]),
            transaction_lock_id("realtime", &["a", "bc"])
        );
        assert_eq!(root, transaction_lock_id("audit-root", &[]));
    }
}
