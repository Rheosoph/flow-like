use flow_like_types_contracts::dispatch::{
    DispatchPayload, dispatch_payload_hash, lambda_tenant_id,
};
use std::time::{Duration, SystemTime};

const CLEANUP_GRACE: Duration = Duration::from_secs(30);
const RUNTIME_RESPONSE_GRACE: Duration = Duration::from_secs(5);

/// These values must come from a verified executor JWT.
pub struct VerifiedIdentity<'a> {
    pub subject: &'a str,
    pub run_id: &'a str,
    pub app_id: &'a str,
    pub board_id: &'a str,
    pub callback_url: &'a str,
    pub dispatch_hash: Option<&'a str>,
}

pub fn validate_dispatch(
    payload: &DispatchPayload,
    identity: VerifiedIdentity<'_>,
    tenant_id: Option<&str>,
) -> Result<(), &'static str> {
    if identity.subject.is_empty() || tenant_id != Some(lambda_tenant_id(identity.subject).as_str())
    {
        return Err("Lambda tenant does not match the authenticated execution subject");
    }
    if payload.user_id != identity.subject
        || payload.run_id != identity.run_id
        || payload.app_id != identity.app_id
        || payload.board_id != identity.board_id
        || payload.callback_url != identity.callback_url
        || payload.job_id.is_empty()
    {
        return Err("Execution payload does not match its authenticated identity");
    }
    let actual = dispatch_payload_hash(payload)
        .map_err(|_| "Failed to calculate execution payload integrity hash")?;
    if identity.dispatch_hash != Some(actual.as_str()) {
        return Err("Executor JWT does not bind the complete execution payload");
    }
    Ok(())
}

/// Both durations start at `now`. Cleanup must end before Lambda's hard stop.
pub fn invocation_budget(
    deadline: SystemTime,
    now: SystemTime,
    configured_timeout: Duration,
) -> Result<(Duration, Duration), &'static str> {
    let usable = deadline
        .duration_since(now)
        .ok()
        .and_then(|remaining| remaining.checked_sub(RUNTIME_RESPONSE_GRACE))
        .ok_or("Lambda invocation has no remaining execution time")?;
    let execution = usable
        .checked_sub(CLEANUP_GRACE)
        .map(|remaining| remaining.min(configured_timeout))
        .filter(|remaining| !remaining.is_zero())
        .ok_or("Lambda invocation has no remaining execution time")?;
    Ok((execution, execution + CLEANUP_GRACE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> DispatchPayload {
        serde_json::from_value(serde_json::json!({
            "job_id": "job-1", "run_id": "run-1", "app_id": "app-1",
            "board_id": "board-1", "node_id": "node-1", "user_id": "auth0|owner",
            "credentials": {}, "executor_jwt": "verified-by-caller",
            "callback_url": "https://api.example", "stream_state": false
        }))
        .unwrap()
    }

    fn identity(hash: &str) -> VerifiedIdentity<'_> {
        VerifiedIdentity {
            subject: "auth0|owner",
            run_id: "run-1",
            app_id: "app-1",
            board_id: "board-1",
            callback_url: "https://api.example",
            dispatch_hash: Some(hash),
        }
    }

    #[test]
    fn accepted_dispatch_requires_the_aws_tenant_of_the_verified_subject() {
        let payload = payload();
        let hash = dispatch_payload_hash(&payload).unwrap();
        let tenant = lambda_tenant_id("auth0|owner");
        assert!(validate_dispatch(&payload, identity(&hash), Some(&tenant)).is_ok());
        assert!(validate_dispatch(&payload, identity(&hash), None).is_err());
        let other_tenant = lambda_tenant_id("another-owner");
        assert!(validate_dispatch(&payload, identity(&hash), Some(&other_tenant)).is_err());
    }

    #[test]
    fn a_valid_payload_hash_cannot_override_the_signed_identity() {
        let tenant = lambda_tenant_id("auth0|owner");
        for field in ["user_id", "run_id", "app_id", "board_id", "callback_url"] {
            let mut value = serde_json::to_value(payload()).unwrap();
            value[field] = serde_json::json!("different");
            let changed: DispatchPayload = serde_json::from_value(value).unwrap();
            let hash = dispatch_payload_hash(&changed).unwrap();
            assert!(
                validate_dispatch(&changed, identity(&hash), Some(&tenant)).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn changing_unsigned_inputs_or_omitting_the_hash_is_rejected() {
        let mut payload = payload();
        let hash = dispatch_payload_hash(&payload).unwrap();
        let tenant = lambda_tenant_id("auth0|owner");
        payload.payload = Some(serde_json::json!({"altered": true}));
        assert!(validate_dispatch(&payload, identity(&hash), Some(&tenant)).is_err());
        let mut unsigned = identity(&hash);
        unsigned.dispatch_hash = None;
        assert!(validate_dispatch(&payload, unsigned, Some(&tenant)).is_err());
    }

    #[test]
    fn deadlines_include_setup_and_reserve_cleanup_before_the_hard_stop() {
        let now = SystemTime::UNIX_EPOCH;
        assert_eq!(
            invocation_budget(
                now + Duration::from_secs(900),
                now,
                Duration::from_secs(840)
            ),
            Ok((Duration::from_secs(840), Duration::from_secs(870)))
        );
        assert_eq!(
            invocation_budget(now + Duration::from_secs(90), now, Duration::from_secs(840)),
            Ok((Duration::from_secs(55), Duration::from_secs(85)))
        );
        for seconds in [0, 5, 34, 35] {
            assert!(
                invocation_budget(
                    now + Duration::from_secs(seconds),
                    now,
                    Duration::from_secs(840)
                )
                .is_err()
            );
        }
        assert!(
            invocation_budget(now, now + Duration::from_secs(1), Duration::from_secs(840)).is_err()
        );
    }
}
