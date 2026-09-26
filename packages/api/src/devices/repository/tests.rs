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
    for statement in
        include_str!("../../../prisma/migrations/20260921120000_standalone_devices/migration.sql")
            .split(';')
    {
        if !statement.trim().is_empty() {
            db.execute_unprepared(statement).await.unwrap();
        }
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
                "nonce-hash",
                now + 60,
                now
            )
            .await
            .is_err()
    );

    repository
        .challenge(
            &manifest.enrollment_id,
            &manifest.enrollment_id,
            "challenge",
            "nonce-hash",
            now + 60,
            now,
        )
        .await
        .unwrap();
    let first = receipt(manifest);
    assert!(
        repository
            .redeem(
                &first,
                &manifest.enrollment_id,
                "challenge",
                "wrong-nonce",
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
            "nonce-hash",
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
    let inventory = repository.list("owner").await.unwrap();
    assert_eq!(inventory.len(), 1000);
    assert_eq!(inventory[0].device_id, manifest.device_id);
    assert_eq!(inventory[0].status, DeviceRegistrationStatus::Active);

    // Replay protection is in the database shared by all API replicas.
    let uses = join_all((0..8).map(|_| {
        repository.authorize_proof(&manifest.device_id, 1, "same-proof-id", now + 60, true)
    }))
    .await;
    assert_eq!(uses.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(
        uses.iter()
            .filter_map(|result| result.as_ref().err())
            .all(|error| error.status() == axum::http::StatusCode::UNAUTHORIZED)
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
    assert_eq!(
        waited.err().unwrap().status(),
        axum::http::StatusCode::UNAUTHORIZED
    );
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
    repository
        .revoke("owner", &manifest.device_id)
        .await
        .unwrap();
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
            "nonce",
            now + 60,
            now,
        )
        .await
        .unwrap();
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
                "nonce",
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
                "nonce",
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
    drop(repository);
    db.close().await.unwrap();
    admin
        .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    admin.close().await.unwrap();
}
