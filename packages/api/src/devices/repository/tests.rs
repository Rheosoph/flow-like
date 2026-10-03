use super::*;
use flow_like_device_protocol::{PROTOCOL_VERSION, SigningKey};
use futures::future::join_all;
use sea_orm::{ConnectOptions, Database, TransactionTrait};

fn template(owner: &str) -> OnboardingManifest {
    let now = chrono::Utc::now().timestamp();
    OnboardingManifest {
        version: PROTOCOL_VERSION,
        enrollment_id: uuid::Uuid::new_v4().to_string(),
        device_id: uuid::Uuid::new_v4().to_string(),
        owner_id: owner.into(),
        name: "Concurrent enrollment fixture".into(),
        api_base_url: "https://api.example/api/v1".into(),
        bootstrap_key: SigningKey::generate().public_key(),
        controller_key: SigningKey::generate().public_key(),
        owner_invitation_key: SigningKey::generate().public_key(),
        issued_at: now,
        expires_at: now + 3600,
    }
}

fn receipt(manifest: &OnboardingManifest) -> DeviceReceipt {
    DeviceReceipt {
        enrollment_id: manifest.enrollment_id.clone(),
        device_id: manifest.device_id.clone(),
        owner_id: manifest.owner_id.clone(),
        name: manifest.name.clone(),
        identity: DeviceIdentity {
            auth_key: SigningKey::generate().public_key(),
            management_key: rand::random(),
            telemetry_key: SigningKey::generate().public_key(),
        },
        // Repository tests exercise transactions. The service verifies real
        // signed envelopes before calling this persistence boundary.
        manifest_jws: "signed-manifest-fixture".into(),
        binding_jws: "signed-binding-fixture".into(),
        registered_at: chrono::Utc::now().timestamp(),
        auth_epoch: 1,
    }
}

/// Run against a disposable PostgreSQL server. Each invocation creates its own
/// schema, and all pool connections select it before issuing any test query.
#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
async fn authoritative_enrollment_and_replay() {
    let url = std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL")
        .expect("set FLOW_LIKE_DEVICE_TEST_DATABASE_URL to a disposable PostgreSQL server");
    let admin = Database::connect(&url).await.unwrap();
    let schema = format!("device_test_{}", uuid::Uuid::new_v4().simple());
    admin
        .execute_unprepared(&format!("CREATE SCHEMA {schema}"))
        .await
        .unwrap();
    let mut scoped_url = reqwest::Url::parse(&url).unwrap();
    scoped_url
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let mut options = ConnectOptions::new(scoped_url.to_string());
    options.max_connections(16).min_connections(1);
    let db = Database::connect(options).await.unwrap();
    for statement in [
        include_str!("../../../prisma/migrations/20260921120000_standalone_devices/migration.sql"),
        include_str!("../../../prisma/migrations/20260921140000_instance_resources/migration.sql"),
        include_str!("../../../prisma/migrations/20260922120000_device_management/migration.sql"),
        include_str!("../../../prisma/migrations/20261001120000_device_console/migration.sql"),
        include_str!("../../../prisma/migrations/20261002120000_device_schedules/migration.sql"),
    ]
    .into_iter()
    .flat_map(|migration| migration.split(';'))
    .filter(|statement| !statement.trim().is_empty())
    {
        db.execute_unprepared(statement).await.unwrap();
    }
    db.execute_unprepared(r#"CREATE TABLE "User" (id TEXT PRIMARY KEY, status TEXT NOT NULL, "updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now())"#).await.unwrap();
    db.execute_unprepared(
        r#"INSERT INTO "User" (id,status) VALUES ('owner','ACTIVE'),('other','ACTIVE')"#,
    )
    .await
    .unwrap();
    let repository = Repository {
        db: &db,
        dialect: DbDialect::Postgres,
    };
    let now = chrono::Utc::now().timestamp();

    // A per-owner write prevents concurrent issuers from all observing zero
    // pending rows and exceeding the account's limit.
    let templates: Vec<_> = (0..8).map(|_| template("owner")).collect();
    let creates = join_all(
        templates
            .iter()
            .map(|manifest| repository.create_enrollment(manifest, &manifest.enrollment_id, 4, 2)),
    )
    .await;
    assert_eq!(creates.iter().filter(|result| result.is_ok()).count(), 2);
    assert!(
        creates
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|error| error.status() == axum::http::StatusCode::TOO_MANY_REQUESTS)
    );
    let mut winners = templates
        .iter()
        .zip(&creates)
        .filter(|(_, result)| result.is_ok())
        .map(|(manifest, _)| manifest);
    let manifest = winners.next().unwrap();
    let cancelled = winners.next().unwrap();
    assert_eq!(
        repository
            .cancel_enrollment("other", &cancelled.enrollment_id)
            .await
            .unwrap_err()
            .status(),
        axum::http::StatusCode::NOT_FOUND
    );
    repository
        .cancel_enrollment("owner", &cancelled.enrollment_id)
        .await
        .unwrap();
    assert!(
        repository
            .challenge(
                &cancelled.enrollment_id,
                &cancelled.enrollment_id,
                "challenge",
                now + 60,
                now
            )
            .await
            .is_err()
    );

    assert_eq!(
        repository
            .challenge(
                &manifest.enrollment_id,
                &manifest.enrollment_id,
                "challenge",
                now + 60,
                now,
            )
            .await
            .unwrap(),
        ("challenge".to_string(), now + 60)
    );
    // Repeating the request, as an enrollment token holder without the
    // bootstrap key can, returns the outstanding challenge instead of replacing it.
    assert_eq!(
        repository
            .challenge(
                &manifest.enrollment_id,
                &manifest.enrollment_id,
                "replacement",
                now + 90,
                now + 1,
            )
            .await
            .unwrap(),
        ("challenge".to_string(), now + 60)
    );
    let nonce_hash = compact_digest(&super::super::challenge_nonce("challenge"));
    let first = receipt(manifest);
    assert!(
        repository
            .redeem(
                &first,
                &manifest.enrollment_id,
                "challenge",
                "wrong-nonce-hash",
                now + 60,
                4
            )
            .await
            .is_err()
    );
    assert_eq!(
        repository
            .enrollment(&manifest.enrollment_id)
            .await
            .unwrap()
            .status,
        "pending"
    );
    assert!(repository.device(&manifest.device_id).await.is_err());

    // Competing permanent identities cannot both consume the same authorization.
    let second = receipt(manifest);
    let redemptions = join_all([&first, &second].into_iter().map(|candidate| {
        repository.redeem(
            candidate,
            &manifest.enrollment_id,
            "challenge",
            &nonce_hash,
            now + 60,
            4,
        )
    }))
    .await;
    assert_eq!(
        redemptions.iter().filter(|result| result.is_ok()).count(),
        1
    );
    let accepted = redemptions.into_iter().find_map(Result::ok).unwrap();
    let registered = repository.device(&manifest.device_id).await.unwrap();
    assert_eq!(registered.receipt, accepted);
    assert_eq!(
        repository
            .enrollment(&manifest.enrollment_id)
            .await
            .unwrap()
            .status,
        "consumed"
    );

    // A long revocation history must not push a still-active device out of the
    // bounded inventory response, even if those history entries are newer.
    db.execute_raw(sql(
        r#"INSERT INTO "ManagedDevice" (id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt")
        SELECT 'history-' || sequence, 'owner', 'Revoked history', 'revoked', 2, $1, $2, $3
        FROM generate_series(1,1000) AS sequence"#,
        [
            serde_json::to_string(&accepted.identity).unwrap().into(),
            serde_json::to_string(&accepted).unwrap().into(),
            (now + 100).into(),
        ],
    ))
    .await
    .unwrap();
    crate::backend_jwt::init_for_tests();
    let policy = flow_like::hub::StandaloneConfig {
        enabled: true,
        ..Default::default()
    };
    let context = super::super::DeviceContext {
        db: &db,
        dialect: DbDialect::Postgres,
        config: &policy,
        domain: "api.example",
        secure: true,
    };
    let inventory = super::super::view::list(&context, "owner").await.unwrap();
    assert_eq!(inventory.len(), 1000);
    assert_eq!(inventory[0].device_id, manifest.device_id);
    assert_eq!(inventory[0].status, DeviceRegistrationStatus::Active);
    assert_eq!(
        super::super::view::visible_devices(&context, "owner")
            .await
            .unwrap()
            .len(),
        1000
    );

    // Replay protection is in the database shared by all API replicas.
    let uses = join_all((0..8).map(|_| {
        repository.authorize_proof(&manifest.device_id, 1, "same-proof-id", now + 60, true)
    }))
    .await;
    assert_eq!(uses.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(
        uses.iter()
            .filter_map(|result| result.as_ref().err())
            .all(
                |error| error.status() == axum::http::StatusCode::UNAUTHORIZED
                    && error.public_code() == super::super::DEVICE_PROOF_INVALID
            )
    );
    assert!(
        repository
            .device(&manifest.device_id)
            .await
            .unwrap()
            .status
            .last_seen_at
            .is_some_and(|last_seen| last_seen >= now)
    );
    // Another API replica may have a slightly faster clock. Presence remains
    // monotonic when a later request is admitted by this replica.
    let last_seen = chrono::Utc::now().timestamp() + 5;
    db.execute_raw(sql(
        r#"UPDATE "ManagedDevice" SET "lastSeenAt" = $1 WHERE id = $2"#,
        [last_seen.into(), manifest.device_id.clone().into()],
    ))
    .await
    .unwrap();
    assert!(
        repository
            .authorize_proof(&manifest.device_id, 1, "monotonic-presence", now + 60, true,)
            .await
            .unwrap()
            .status
            .last_seen_at
            .is_some_and(|observed| observed >= last_seen)
    );
    // A valid proof may expire while another replica holds the device row. It
    // must not authorize after the wait, and its failed transaction must not
    // leave a replay claim behind.
    let guard = db.begin().await.unwrap();
    guard
        .execute_raw(sql(
            r#"UPDATE "ManagedDevice" SET "authEpoch" = "authEpoch" WHERE id = $1"#,
            [manifest.device_id.clone().into()],
        ))
        .await
        .unwrap();
    let expiring = chrono::Utc::now().timestamp() + 1;
    let (waited, ()) = futures::join!(
        repository.authorize_proof(
            &manifest.device_id,
            1,
            "expired-during-lock",
            expiring,
            false
        ),
        async {
            flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
            guard.commit().await.unwrap();
        }
    );
    let waited = waited.err().unwrap();
    assert_eq!(waited.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(waited.public_code(), super::super::DEVICE_PROOF_INVALID);
    repository
        .authorize_proof(
            &manifest.device_id,
            1,
            "expired-during-lock",
            chrono::Utc::now().timestamp() + 60,
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        repository
            .revoke("other", &manifest.device_id)
            .await
            .unwrap_err()
            .status(),
        axum::http::StatusCode::NOT_FOUND
    );
    let revoked_at = || async {
        db.query_one_raw(sql(
            r#"SELECT "revokedAt" FROM "ManagedDevice" WHERE id = $1"#,
            [manifest.device_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<Option<i64>>("", "revokedAt")
        .unwrap()
    };
    assert_eq!(revoked_at().await, None);
    repository
        .revoke("owner", &manifest.device_id)
        .await
        .unwrap();
    assert!(revoked_at().await.is_some_and(|at| at >= now));
    // A repeated revocation finds nothing to revoke and keeps the recorded time.
    db.execute_raw(sql(
        r#"UPDATE "ManagedDevice" SET "revokedAt" = 1 WHERE id = $1"#,
        [manifest.device_id.clone().into()],
    ))
    .await
    .unwrap();
    assert_eq!(
        repository
            .revoke("owner", &manifest.device_id)
            .await
            .unwrap_err()
            .status(),
        axum::http::StatusCode::NOT_FOUND
    );
    assert_eq!(revoked_at().await, Some(1));
    assert!(
        repository
            .authorize_proof(
                &manifest.device_id,
                1,
                "fresh-proof-after-revocation",
                now + 60,
                false
            )
            .await
            .is_err()
    );
    assert_eq!(
        repository
            .device(&manifest.device_id)
            .await
            .unwrap()
            .status
            .auth_epoch,
        2
    );

    let expired = template("owner");
    repository
        .create_enrollment(&expired, "expired-jwt", 4, 2)
        .await
        .unwrap();
    repository
        .challenge(
            &expired.enrollment_id,
            "expired-jwt",
            "expired-challenge",
            now + 60,
            now,
        )
        .await
        .unwrap();
    let expired_nonce = compact_digest(&super::super::challenge_nonce("expired-challenge"));
    db.execute_raw(sql(
        r#"UPDATE "DeviceChallenge" SET "expiresAt" = $1 WHERE "enrollmentId" = $2"#,
        [(now - 1).into(), expired.enrollment_id.clone().into()],
    ))
    .await
    .unwrap();
    assert!(
        repository
            .redeem(
                &receipt(&expired),
                "expired-jwt",
                "expired-challenge",
                &expired_nonce,
                now + 60,
                4
            )
            .await
            .is_err()
    );
    assert_eq!(
        repository
            .enrollment(&expired.enrollment_id)
            .await
            .unwrap()
            .status,
        "pending"
    );
    db.execute_raw(sql(
        r#"UPDATE "DeviceEnrollment" SET "expiresAt" = $1 WHERE id = $2"#,
        [now.into(), expired.enrollment_id.clone().into()],
    ))
    .await
    .unwrap();
    assert!(
        repository
            .challenge(
                &expired.enrollment_id,
                "expired-jwt",
                "fresh-challenge",
                now + 60,
                now
            )
            .await
            .is_err()
    );

    // The new template was prepared before the previous reservation expired.
    // Quota admission must count pending rows at admission time, not issue time.
    db.execute_unprepared(r#"INSERT INTO "User" (id,status) VALUES ('quota-clock','ACTIVE')"#)
        .await
        .unwrap();
    let quota_now = chrono::Utc::now().timestamp();
    let mut old_reservation = template("quota-clock");
    old_reservation.issued_at = quota_now - 120;
    repository
        .create_enrollment(&old_reservation, "old-quota-reservation", 1, 1)
        .await
        .unwrap();
    old_reservation.expires_at = quota_now - 30;
    db.execute_raw(sql(
        r#"UPDATE "DeviceEnrollment" SET "expiresAt" = $1, manifest = $2 WHERE id = $3"#,
        [
            old_reservation.expires_at.into(),
            serde_json::to_string(&old_reservation).unwrap().into(),
            old_reservation.enrollment_id.into(),
        ],
    ))
    .await
    .unwrap();
    let mut prepared_earlier = template("quota-clock");
    prepared_earlier.issued_at = quota_now - 60;
    repository
        .create_enrollment(&prepared_earlier, "new-quota-reservation", 1, 1)
        .await
        .unwrap();

    // Create-then-cancel loops are bounded per day, and abandoned packages with
    // their challenges are pruned after the grace period.
    db.execute_unprepared(r#"INSERT INTO "User" (id,status) VALUES ('churn','ACTIVE')"#)
        .await
        .unwrap();
    let mut abandoned = Vec::new();
    for index in 0..4 {
        let package = template("churn");
        let jwt = format!("churn-{index}");
        repository
            .create_enrollment(&package, &jwt, 1, 1)
            .await
            .unwrap();
        repository
            .challenge(
                &package.enrollment_id,
                &jwt,
                &format!("churn-challenge-{index}"),
                now + 60,
                now,
            )
            .await
            .unwrap();
        repository
            .cancel_enrollment("churn", &package.enrollment_id)
            .await
            .unwrap();
        abandoned.push(package.enrollment_id);
    }
    let limited = repository
        .create_enrollment(&template("churn"), "churn-limited", 1, 1)
        .await
        .unwrap_err();
    assert_eq!(limited.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert!(
        limited
            .public_message()
            .is_some_and(|message| message.contains("4 per day"))
    );
    // The usage view reads these counts, so it shows the allowance as spent.
    assert_eq!(
        enrollment_counts(&db, "churn", chrono::Utc::now().timestamp())
            .await
            .unwrap(),
        EnrollmentCounts {
            pending: 0,
            active_devices: 0,
            last_day: daily_enrollment_limit(1, 1),
        }
    );
    let lapsed = now - ABANDONED_ENROLLMENT_GRACE_SECONDS - 1;
    db.execute_raw(sql(
        r#"UPDATE "DeviceEnrollment" SET "expiresAt" = $1, "createdAt" = $1 WHERE "ownerId" = 'churn'"#,
        [lapsed.into()],
    ))
    .await
    .unwrap();
    assert_eq!(
        prune_abandoned_enrollments(&db, DbDialect::Postgres, now)
            .await
            .unwrap(),
        4
    );
    for id in &abandoned {
        assert!(repository.enrollment(id).await.is_err());
    }
    let challenges = db
        .query_one_raw(sql(
            r#"SELECT COUNT(*) AS count FROM "DeviceChallenge" WHERE id LIKE 'churn-challenge-%'"#,
            [],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(challenges, 0);
    assert_eq!(
        repository
            .enrollment(&manifest.enrollment_id)
            .await
            .unwrap()
            .status,
        "consumed"
    );
    repository
        .create_enrollment(&template("churn"), "churn-after-prune", 1, 1)
        .await
        .unwrap();
    assert_eq!(
        enrollment_counts(&db, "churn", chrono::Utc::now().timestamp())
            .await
            .unwrap(),
        EnrollmentCounts {
            pending: 1,
            active_devices: 0,
            last_day: 1,
        }
    );

    db.execute_unprepared(r#"UPDATE "User" SET status = 'SUSPENDED' WHERE id = 'owner'"#)
        .await
        .unwrap();
    assert!(repository.active_account("owner").await.is_err());
    assert!(
        repository
            .create_enrollment(&template("owner"), "suspended-jwt", 4, 2)
            .await
            .is_err()
    );
    db.close().await.unwrap();
    admin
        .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    admin.close().await.unwrap();
}

#[test]
fn a_package_is_refused_at_each_limit_the_usage_view_reports() {
    let counts = |pending, active_devices, last_day| EnrollmentCounts {
        pending,
        active_devices,
        last_day,
    };
    assert_eq!(daily_enrollment_limit(6, 2), 16);
    assert!(counts(1, 4, 15).admit(6, 2).is_ok());
    for (refused, reason) in [
        (counts(2, 0, 2), "limit reached"),
        (counts(1, 5, 1), "limit reached"),
        (counts(0, 0, 16), "limited to 16 per day; 16 were created"),
    ] {
        let error = refused.admit(6, 2).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert!(
            error
                .public_message()
                .is_some_and(|message| message.contains(reason)),
            "{refused:?}"
        );
    }
}

#[test]
fn rejection_codes_are_stored_and_served_under_one_name() {
    for code in [
        AuthRejectionCode::ClockSkew,
        AuthRejectionCode::RevokedCredential,
    ] {
        assert_eq!(AuthRejectionCode::parse(code.as_str()), Some(code));
        assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
    }
    assert_eq!(AuthRejectionCode::parse("reason_of_a_newer_hub"), None);
}

#[test]
fn only_device_proofs_report_a_lapse_during_the_database_wait_as_clock_skew() {
    let lapsed = chrono::Utc::now().timestamp() - 1;
    let proof = require_live_proof(lapsed).unwrap_err();
    assert_eq!(proof.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(proof.public_code(), super::super::DEVICE_PROOF_INVALID);
    let enrollment = require_live(lapsed).unwrap_err();
    assert_eq!(
        enrollment.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    assert_ne!(enrollment.public_code(), super::super::DEVICE_PROOF_INVALID);
    assert!(
        enrollment
            .public_message()
            .is_some_and(|message| !message.contains("clock"))
    );
    let live = chrono::Utc::now().timestamp() + 60;
    assert!(require_live(live).is_ok() && require_live_proof(live).is_ok());
}
