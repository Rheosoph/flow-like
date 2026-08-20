//! Batch limits shared by the per-prefix data routes.
//!
//! Upload and download both sign one URL per prefix, and both used to cap the
//! work with `payload.prefixes.iter().take(MAX_PREFIXES)`. That returned 200 OK
//! with fewer results than were asked for: a client uploading a folder saw
//! success while every file past the hundredth was silently dropped, and a
//! client downloading a selection got a short list with nothing to distinguish
//! it from the full one. The cap is now a precondition rather than a silent
//! truncation, and it lives here so the two routes cannot drift apart.

use crate::error::ApiError;

/// Prefixes one request may carry.
///
/// A larger batch is not free: every entry costs a signature on the way out and
/// several hundred bytes in the response, and the request body carrying the
/// paths has to fit inside the deployment's limit — 6 MB on Lambda, and as
/// little as 1 MB behind an `ingress-nginx` default. Clients working through a
/// folder are expected to split it across requests.
pub const MAX_PREFIXES: usize = 100;

/// Signatures computed at once.
///
/// Signing is independent per prefix, and on providers that sign through an IAM
/// call rather than a local key it is a network round trip. Serially, a full
/// batch cost the sum of every round trip.
pub const SIGN_CONCURRENCY: usize = 16;

/// Rejects a batch the caller cannot be served in full.
pub fn validate_batch(prefixes: &[String]) -> Result<(), ApiError> {
    if prefixes.is_empty() {
        return Err(ApiError::bad_request(
            "prefixes must contain at least one entry".to_string(),
        ));
    }

    if prefixes.len() > MAX_PREFIXES {
        return Err(ApiError::bad_request(format!(
            "Too many prefixes: {} requested, at most {} per request. Split the request into batches of {}.",
            prefixes.len(),
            MAX_PREFIXES,
            MAX_PREFIXES
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefixes(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("folder/file-{i}.bin")).collect()
    }

    #[test]
    fn a_batch_at_the_cap_is_accepted() {
        assert!(validate_batch(&prefixes(MAX_PREFIXES)).is_ok());
        assert!(validate_batch(&prefixes(1)).is_ok());
    }

    #[test]
    fn an_oversized_batch_is_rejected_rather_than_truncated() {
        let error = validate_batch(&prefixes(MAX_PREFIXES + 1))
            .expect_err("a batch over the cap must not be served");
        let message = format!("{error:?}");
        assert!(
            message.contains("Too many prefixes"),
            "expected an explicit cap error, got: {message}"
        );
    }

    #[test]
    fn an_empty_batch_is_rejected() {
        assert!(validate_batch(&[]).is_err());
    }
}
