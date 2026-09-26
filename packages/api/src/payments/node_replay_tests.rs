use super::*;
use flow_like::hub::PaymentsConfig;

fn input() -> CreatePayment {
    serde_json::from_value(json!({
        "nodeId": "node",
        "nonce": "invocation",
        "idempotencyKey": "order-123",
        "amountMinor": 100,
        "currency": "eur",
        "productName": "Access",
        "productTaxCode": "txcd_10000000",
        "shippingCountries": [],
        "description": "",
        "reference": null,
        "ttlSeconds": 300
    }))
    .unwrap()
}

fn config() -> PaymentsConfig {
    PaymentsConfig {
        platform_account_id: Some("acct_platform".into()),
        ..PaymentsConfig::default()
    }
}

#[test]
fn keyed_restarts_reuse_identity_after_nonce_run_and_timeout_change() {
    let original = input();
    let claims = tests::executor_fixture();
    let expected = request_identity(&original, &claims, &config()).unwrap();
    let mut retry = original;
    retry.nonce = "new-invocation".into();
    retry.ttl_seconds = 60;
    retry.idempotency_key = Some("  order-123  ".into());
    let mut restarted = claims;
    restarted.run_id = "restarted-run".into();
    assert_eq!(
        request_identity(&retry, &restarted, &config()).unwrap(),
        expected
    );
}

#[test]
fn keyed_replay_rejects_changed_commercial_fields() {
    let original = input();
    let claims = tests::executor_fixture();
    let (id, digest) = request_identity(&original, &claims, &config()).unwrap();
    let mut row = tests::request_fixture();
    row.request_digest = digest;
    for (field, value) in [
        ("amountMinor", json!(200)),
        ("currency", json!("usd")),
        ("productName", json!("Different product")),
        ("productTaxCode", json!("txcd_20000000")),
        ("shippingCountries", json!(["DE"])),
        ("description", json!("Different description")),
        ("reference", json!("different-reference")),
    ] {
        let mut value_input = serde_json::to_value(&original).unwrap();
        value_input[field] = value;
        let changed: CreatePayment = serde_json::from_value(value_input).unwrap();
        let (changed_id, changed_digest) = request_identity(&changed, &claims, &config()).unwrap();
        assert_eq!(changed_id, id, "{field} must not create a second payment");
        assert_ne!(changed_digest, row.request_digest, "{field}");
        assert_eq!(
            replay_scope(&claims, &changed, &row, &changed_digest)
                .unwrap_err()
                .status(),
            axum::http::StatusCode::CONFLICT,
            "{field}"
        );
    }
}

#[test]
fn payment_keys_are_scoped_to_payer_workflow_node_and_payment_environment() {
    let original = input();
    let claims = tests::executor_fixture();
    let configuration = config();
    let (id, _) = request_identity(&original, &claims, &configuration).unwrap();
    for field in ["sub", "app_id", "board_id", "event_id"] {
        let mut value = serde_json::to_value(&claims).unwrap();
        value[field] = json!("other");
        if field == "sub" {
            value["payer_sub"] = json!("other");
        }
        let changed: ExecutionClaims = serde_json::from_value(value).unwrap();
        assert_ne!(
            request_identity(&original, &changed, &configuration)
                .unwrap()
                .0,
            id,
            "{field}"
        );
    }
    let mut changed = original.clone();
    changed.node_id = "another-node".into();
    assert_ne!(
        request_identity(&changed, &claims, &configuration)
            .unwrap()
            .0,
        id
    );
    changed = original.clone();
    changed.idempotency_key = Some("order-124".into());
    assert_ne!(
        request_identity(&changed, &claims, &configuration)
            .unwrap()
            .0,
        id
    );
    let mut changed_config = configuration.clone();
    changed_config.platform_account_id = Some("acct_other".into());
    assert_ne!(
        request_identity(&original, &claims, &changed_config)
            .unwrap()
            .0,
        id
    );
    changed_config = configuration;
    changed_config.livemode = true;
    assert_ne!(
        request_identity(&original, &claims, &changed_config)
            .unwrap()
            .0,
        id
    );
}

#[test]
fn replay_scope_preserves_payer_app_board_event_and_node_boundaries() {
    let input = input();
    let claims = tests::executor_fixture();
    let (_, digest) = request_identity(&input, &claims, &config()).unwrap();
    let mut row = tests::request_fixture();
    row.request_digest = digest.clone();
    for field in ["app_id", "board_id", "event_id", "payer_sub"] {
        let mut value = serde_json::to_value(&claims).unwrap();
        value[field] = json!("other");
        let changed: ExecutionClaims = serde_json::from_value(value).unwrap();
        assert_eq!(
            replay_scope(&changed, &input, &row, &digest)
                .unwrap_err()
                .status(),
            axum::http::StatusCode::NOT_FOUND,
            "{field}"
        );
    }
    let mut wrong_node = input.clone();
    wrong_node.node_id = "other-node".into();
    assert_eq!(
        replay_scope(&claims, &wrong_node, &row, &digest)
            .unwrap_err()
            .status(),
        axum::http::StatusCode::NOT_FOUND
    );
    let mut missing_payer = claims;
    missing_payer.payer_sub = None;
    assert!(replay_scope(&missing_payer, &input, &row, &digest).is_err());
}

#[test]
fn cross_run_replay_preserves_paid_skipped_and_pending_outcomes() {
    let input = input();
    let mut restarted = tests::executor_fixture();
    restarted.run_id = "restarted-run".into();
    let (_, digest) = request_identity(&input, &restarted, &config()).unwrap();
    let mut row = tests::request_fixture();
    row.request_digest = digest.clone();
    for status in [
        "PAID",
        "CANCELED",
        "EXPIRED",
        "FAILED",
        "CREATED",
        "OPENING",
        "OPEN",
        "PROCESSING",
        "CANCEL_PENDING",
    ] {
        row.status = status.into();
        let recorded = row.clone();
        assert!(replay_scope(&restarted, &input, &row, &digest).is_ok());
        assert_eq!(row, recorded);
        assert_eq!(
            executor_scope(&restarted, &row).unwrap_err().status(),
            axum::http::StatusCode::NOT_FOUND,
            "replaying {status} must not authorize cancellation from another run"
        );
    }
}

#[test]
fn omitted_and_blank_keys_preserve_legacy_identity_and_digest() {
    let mut input = input();
    input.idempotency_key = None;
    let claims = tests::executor_fixture();
    let legacy_json = r#"{"nodeId":"node","nonce":"invocation","amountMinor":100,"currency":"eur","productName":"Access","productTaxCode":"txcd_10000000","shippingCountries":[],"description":"","reference":null,"ttlSeconds":300}"#;
    assert_eq!(serde_json::to_string(&input).unwrap(), legacy_json);
    let expected = (
        blake3::hash(br#"["run","node","invocation"]"#)
            .to_hex()
            .to_string(),
        blake3::hash(legacy_json.as_bytes()).to_hex().to_string(),
    );
    assert_eq!(
        request_identity(&input, &claims, &config()).unwrap(),
        expected
    );
    for key in ["", "   "] {
        input.idempotency_key = Some(key.into());
        assert_eq!(
            request_identity(&input, &claims, &config()).unwrap(),
            expected
        );
    }
    let mut restarted = claims.clone();
    restarted.run_id = "restarted-run".into();
    assert_ne!(
        request_identity(&input, &restarted, &config()).unwrap().0,
        expected.0
    );
    let mut row = tests::request_fixture();
    row.request_digest = expected.1.clone();
    assert!(replay_scope(&claims, &input, &row, &expected.1).is_ok());
    assert_eq!(
        replay_scope(&restarted, &input, &row, &expected.1)
            .unwrap_err()
            .status(),
        axum::http::StatusCode::NOT_FOUND
    );
}

#[test]
fn idempotency_keys_reject_control_characters_and_more_than_200_bytes() {
    let mut input = input();
    for key in ["order:123", "", "   "] {
        input.idempotency_key = Some(key.into());
        assert!(validate(&input).is_ok());
    }
    input.idempotency_key = Some("a".repeat(200));
    assert!(validate(&input).is_ok());
    for key in [
        "a".repeat(201),
        "é".repeat(101),
        "order\n123".into(),
        "order\t123".into(),
        "order\0123".into(),
    ] {
        input.idempotency_key = Some(key);
        assert!(validate(&input).is_err());
    }
}
