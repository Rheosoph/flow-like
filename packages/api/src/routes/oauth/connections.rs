use super::{OAuthProxyError, TokenResponse};
use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::Duration;

pub(super) const HANDLE_PREFIX: &str = "fl_oauth_";
const REFRESH_WAIT: Duration = Duration::from_secs(5);
const REFRESH_UNCERTAIN_AFTER: i64 = 45;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn storage_error(error: impl std::fmt::Display) -> OAuthProxyError {
    tracing::error!(%error, "OAuth credential persistence failed");
    OAuthProxyError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "OAuth credential storage is unavailable; retry shortly",
    )
}

pub(super) fn reconnect() -> OAuthProxyError {
    OAuthProxyError::coded(
        StatusCode::BAD_REQUEST,
        "oauth_reauthorization_required",
        "Reconnect this OAuth account before running the workflow",
    )
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredToken {
    response: TokenResponse,
    expires_at: Option<i64>,
    #[serde(default)]
    scoped: std::collections::HashMap<String, ScopedToken>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ScopedToken {
    response: TokenResponse,
    expires_at: Option<i64>,
}

impl StoredToken {
    fn from_response(response: TokenResponse) -> Result<Self, OAuthProxyError> {
        let expires_at = response
            .expires_in
            .map(|seconds| now().saturating_add(seconds));
        Ok(Self {
            response,
            expires_at,
            scoped: Default::default(),
        })
    }

    fn selected(&self, scope: Option<&str>) -> Option<(&TokenResponse, Option<i64>)> {
        match scope {
            Some(scope) => self
                .scoped
                .get(scope)
                .map(|token| (&token.response, token.expires_at)),
            None => Some((&self.response, self.expires_at)),
        }
    }

    fn is_current(&self, scope: Option<&str>) -> bool {
        self.selected(scope).is_some_and(|(response, expires_at)| {
            !response.access_token.is_empty() && expires_at.is_none_or(|expiry| expiry > now() + 60)
        })
    }

    fn public_response(
        &self,
        id: &str,
        scope: Option<&str>,
    ) -> Result<TokenResponse, OAuthProxyError> {
        let (response, expires_at) = self.selected(scope).ok_or_else(reconnect)?;
        let mut response = response.clone();
        response.refresh_token = Some(id.to_owned());
        response.expires_in = expires_at.map(|expiry| expiry.saturating_sub(now()).max(0));
        Ok(response)
    }
}

pub(super) struct ListedConnection {
    pub handle: String,
    pub provider_id: Option<String>,
    pub state: String,
    pub scopes: Vec<String>,
    pub updated_at: i64,
}

struct Connection {
    id: String,
    state: String,
    generation: i64,
    updated_at: i64,
    token: StoredToken,
}

/// A stable bearer handle names the one durable owner of a provider's rotating grant.
/// HTTP callers also prove their hub identity; sinks hold the handle in encrypted storage.
pub(super) struct Connections<'a> {
    db: &'a DatabaseConnection,
    key: &'a [u8; 32],
    fingerprint: String,
    client_id: String,
    owner: Option<&'a str>,
}

impl<'a> Connections<'a> {
    pub(super) fn new(
        db: &'a DatabaseConnection,
        key: &'a [u8; 32],
        provider_id: &str,
        client_id: &str,
        token_url: &str,
        owner: Option<&'a str>,
    ) -> Self {
        let identity = serde_json::to_vec(&(provider_id, client_id, token_url)).unwrap();
        Self {
            db,
            key,
            fingerprint: blake3::hash(&identity).to_hex().to_string(),
            client_id: client_id.to_owned(),
            owner,
        }
    }

    /// The owner's grants with the provider each belongs to, recomputed from the fingerprint over
    /// the configured providers `(id, token_url)`. Handles and scopes only, never a token.
    pub(super) async fn list_owned(
        db: &DatabaseConnection,
        key: &[u8; 32],
        owner: &str,
        providers: &[(String, String)],
    ) -> Result<Vec<ListedConnection>, OAuthProxyError> {
        let rows = db
            .query_all_raw(Statement::from_sql_and_values(
                db.get_database_backend(),
                r#"SELECT "id", "providerFingerprint", "clientId", "tokenEncrypted", "state", "updatedAt" FROM "OAuthConnection" WHERE "ownerSub" = $1 ORDER BY "updatedAt" DESC LIMIT 200"#,
                [owner.into()],
            ))
            .await
            .map_err(storage_error)?;
        let mut listed = Vec::with_capacity(rows.len());
        for row in rows {
            let fingerprint: String = row
                .try_get("", "providerFingerprint")
                .map_err(storage_error)?;
            let client_id: Option<String> = row.try_get("", "clientId").ok();
            let provider_id = providers.iter().find_map(|(provider_id, token_url)| {
                let identity = serde_json::to_vec(&(
                    provider_id.as_str(),
                    client_id.as_deref().unwrap_or_default(),
                    token_url.as_str(),
                ))
                .ok()?;
                (blake3::hash(&identity).to_hex().to_string() == fingerprint)
                    .then(|| provider_id.clone())
            });
            let encrypted: String = row.try_get("", "tokenEncrypted").map_err(storage_error)?;
            let scopes = crate::utils::crypto::decrypt_secret(&encrypted, key)
                .and_then(|json| serde_json::from_str::<StoredToken>(&json).ok())
                .and_then(|token| token.response.scope)
                .map(|scope| scope.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default();
            listed.push(ListedConnection {
                handle: row.try_get("", "id").map_err(storage_error)?,
                provider_id,
                state: row.try_get("", "state").map_err(storage_error)?,
                scopes,
                updated_at: row.try_get("", "updatedAt").map_err(storage_error)?,
            });
        }
        Ok(listed)
    }

    pub(super) async fn client_id(
        db: &DatabaseConnection,
        id: &str,
    ) -> Result<String, OAuthProxyError> {
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                db.get_database_backend(),
                r#"SELECT "clientId" FROM "OAuthConnection" WHERE "id" = $1"#,
                [id.into()],
            ))
            .await
            .map_err(storage_error)?
            .ok_or_else(reconnect)?;
        row.try_get("", "clientId").map_err(storage_error)
    }

    fn statement(&self, sql: &str, values: impl IntoIterator<Item = sea_orm::Value>) -> Statement {
        Statement::from_sql_and_values(self.db.get_database_backend(), sql, values)
    }

    fn handle(&self, token: &str) -> String {
        if token.starts_with(HANDLE_PREFIX) {
            return token.to_owned();
        }
        // Retained original-token aliases let old browser/sink copies converge. The keyed
        // digest is also an unguessable capability; no raw refresh token enters an index.
        let input = serde_json::to_vec(&(&self.fingerprint, token)).unwrap();
        format!(
            "{HANDLE_PREFIX}{}",
            blake3::keyed_hash(self.key, &input).to_hex()
        )
    }

    fn encrypt(&self, token: &StoredToken) -> Result<String, OAuthProxyError> {
        let json = serde_json::to_string(token).map_err(storage_error)?;
        Ok(crate::utils::crypto::encrypt_secret(&json, self.key))
    }

    async fn read(&self, id: &str) -> Result<Connection, OAuthProxyError> {
        let row = self.db.query_one_raw(self.statement(
            r#"SELECT "providerFingerprint", "ownerSub", "tokenEncrypted", "state", "generation", "updatedAt" FROM "OAuthConnection" WHERE "id" = $1"#,
            [id.into()],
        )).await.map_err(storage_error)?.ok_or_else(reconnect)?;
        let fingerprint: String = row
            .try_get("", "providerFingerprint")
            .map_err(storage_error)?;
        let owner: Option<String> = row.try_get("", "ownerSub").map_err(storage_error)?;
        if fingerprint != self.fingerprint
            || self
                .owner
                .is_some_and(|expected| owner.as_deref() != Some(expected))
        {
            return Err(reconnect());
        }
        let encrypted: String = row.try_get("", "tokenEncrypted").map_err(storage_error)?;
        let json =
            crate::utils::crypto::decrypt_secret(&encrypted, self.key).ok_or_else(reconnect)?;
        Ok(Connection {
            id: id.to_owned(),
            state: row.try_get("", "state").map_err(storage_error)?,
            generation: row.try_get("", "generation").map_err(storage_error)?,
            updated_at: row.try_get("", "updatedAt").map_err(storage_error)?,
            token: serde_json::from_str(&json).map_err(storage_error)?,
        })
    }

    pub(super) async fn adopt(&self, response: TokenResponse) -> Result<String, OAuthProxyError> {
        let refresh = response
            .refresh_token
            .as_deref()
            .filter(|token| !token.is_empty())
            .ok_or_else(reconnect)?;
        let id = self.handle(refresh);
        if !refresh.starts_with(HANDLE_PREFIX) {
            // A legacy refresh-only call deliberately seeds an already-expired access token.
            let token = StoredToken::from_response(response)?;
            self.db.execute_raw(self.statement(
                r#"INSERT INTO "OAuthConnection" ("id", "providerFingerprint", "ownerSub", "tokenEncrypted", "state", "generation", "updatedAt", "clientId") VALUES ($1, $2, $3, $4, 'ready', 0, $5, $6) ON CONFLICT ("id") DO NOTHING"#,
                [id.clone().into(), self.fingerprint.clone().into(), self.owner.map(str::to_owned).into(), self.encrypt(&token)?.into(), now().into(), self.client_id.clone().into()],
            )).await.map_err(storage_error)?;
        }
        if let Some(owner) = self.owner {
            // A sink can migrate a legacy token first. Possession of its original token or
            // handle plus an authenticated hub identity claims the previously unowned grant.
            self.db.execute_raw(self.statement(
                r#"UPDATE "OAuthConnection" SET "ownerSub" = $2 WHERE "id" = $1 AND "ownerSub" IS NULL AND "providerFingerprint" = $3"#,
                [id.clone().into(), owner.into(), self.fingerprint.clone().into()],
            )).await.map_err(storage_error)?;
        }
        self.read(&id).await?;
        Ok(id)
    }

    /// Only a successful code/device exchange observed by this server can repair
    /// an uncertain grant. Legacy snapshots must continue through `adopt`.
    pub(super) async fn reauthorize(
        &self,
        response: TokenResponse,
    ) -> Result<String, OAuthProxyError> {
        let id = self.adopt(response.clone()).await?;
        let current = self.read(&id).await?;
        if current.state == "refreshing"
            && now().saturating_sub(current.updated_at) < REFRESH_UNCERTAIN_AFTER
        {
            return Err(OAuthProxyError::coded(
                StatusCode::SERVICE_UNAVAILABLE,
                "oauth_refresh_pending",
                "OAuth refresh is in progress; retry shortly",
            ));
        }
        let token = StoredToken::from_response(response)?;
        if token.response.access_token.is_empty()
            || token.expires_at.is_some_and(|expiry| expiry <= now())
        {
            return Err(reconnect());
        }
        let updated = self.db.execute_raw(self.statement(
            r#"UPDATE "OAuthConnection" SET "tokenEncrypted" = $3, "state" = 'ready', "generation" = "generation" + 1, "updatedAt" = $4 WHERE "id" = $1 AND "generation" = $2 AND "state" = $5"#,
            [id.clone().into(), current.generation.into(), self.encrypt(&token)?.into(), now().into(), current.state.into()],
        )).await.map_err(storage_error)?;
        if updated.rows_affected() != 1 {
            return Err(reconnect());
        }
        Ok(id)
    }

    pub(super) async fn resolve<F, Fut>(
        &self,
        id: &str,
        refresh: F,
    ) -> Result<TokenResponse, OAuthProxyError>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<TokenResponse, OAuthProxyError>>,
    {
        self.resolve_scoped(id, None, refresh).await
    }

    pub(super) async fn resolve_scoped<F, Fut>(
        &self,
        id: &str,
        scope: Option<&str>,
        refresh: F,
    ) -> Result<TokenResponse, OAuthProxyError>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<TokenResponse, OAuthProxyError>>,
    {
        let deadline = std::time::Instant::now() + REFRESH_WAIT;
        let connection = loop {
            let connection = self.read(id).await?;
            match connection.state.as_str() {
                "ready" => {
                    if connection.token.is_current(scope) {
                        return connection.token.public_response(id, scope);
                    }
                    let claimed = self.db.execute_raw(self.statement(
                        r#"UPDATE "OAuthConnection" SET "state" = 'refreshing', "generation" = "generation" + 1, "updatedAt" = $3 WHERE "id" = $1 AND "generation" = $2 AND "state" = 'ready'"#,
                        [id.into(), connection.generation.into(), now().into()],
                    )).await.map_err(storage_error)?;
                    if claimed.rows_affected() == 1 {
                        break connection;
                    }
                }
                "refreshing"
                    if now().saturating_sub(connection.updated_at) < REFRESH_UNCERTAIN_AFTER => {}
                _ => return Err(reconnect()),
            }
            if std::time::Instant::now() >= deadline {
                return Err(OAuthProxyError::coded(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "oauth_refresh_pending",
                    "OAuth refresh is in progress; retry shortly",
                ));
            }
            flow_like_types::tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let generation = connection.generation + 1;
        let old = connection.token.response.clone();
        let provider_refresh = old
            .refresh_token
            .clone()
            .filter(|token| !token.is_empty())
            .ok_or_else(reconnect)?;
        // Never retry this external side effect after a timeout, crash, or failed commit.
        // The durable 'refreshing' state prevents another replica replaying the old token.
        let response = match refresh(provider_refresh.clone()).await {
            Ok(mut response) => {
                response.refresh_token = response.refresh_token.or(Some(provider_refresh));
                response.scope = response
                    .scope
                    .or_else(|| scope.map(str::to_owned).or(old.scope));
                response.workspace_id = response.workspace_id.or(old.workspace_id);
                response.workspace_name = response.workspace_name.or(old.workspace_name);
                response.workspace_icon = response.workspace_icon.or(old.workspace_icon);
                response.bot_id = response.bot_id.or(old.bot_id);
                response
            }
            Err(error)
                if matches!(
                    error.code,
                    "oauth_scope_rejected" | "oauth_provider_rejected"
                ) =>
            {
                // A definitive scope or request-configuration error issued no tokens.
                // Preserve the grant so the caller can correct the request and retry.
                let released = self.db.execute_raw(self.statement(
                    r#"UPDATE "OAuthConnection" SET "state" = 'ready', "updatedAt" = $3 WHERE "id" = $1 AND "generation" = $2 AND "state" = 'refreshing'"#,
                    [id.into(), generation.into(), now().into()],
                )).await.map_err(storage_error)?;
                if released.rows_affected() != 1 {
                    return Err(reconnect());
                }
                return Err(error);
            }
            Err(_) => {
                self.db.execute_raw(self.statement(
                    r#"UPDATE "OAuthConnection" SET "state" = 'failed', "updatedAt" = $3 WHERE "id" = $1 AND "generation" = $2 AND "state" = 'refreshing'"#,
                    [id.into(), generation.into(), now().into()],
                )).await.map_err(storage_error)?;
                return Err(reconnect());
            }
        };
        if response.access_token.is_empty()
            || response.expires_in.is_some_and(|seconds| seconds <= 0)
        {
            return Err(reconnect());
        }
        let mut token = StoredToken::from_response(response)?;
        if let Some(scope) = scope {
            // A Dataverse resource token must not replace the discovery token. Only the
            // rotating refresh credential is shared across audience-specific access tokens.
            let mut connection_token = connection.token;
            connection_token.response.refresh_token = token.response.refresh_token.take();
            // Bound the cache even when a workflow visits many resource audiences.
            if connection_token.scoped.len() >= 32 && !connection_token.scoped.contains_key(scope) {
                if let Some(oldest) = connection_token
                    .scoped
                    .iter()
                    .min_by_key(|(_, value)| value.expires_at.unwrap_or(i64::MAX))
                    .map(|(key, _)| key.clone())
                {
                    connection_token.scoped.remove(&oldest);
                }
            }
            connection_token.scoped.insert(
                scope.to_owned(),
                ScopedToken {
                    response: token.response,
                    expires_at: token.expires_at,
                },
            );
            token = connection_token;
        } else {
            token.scoped = connection.token.scoped;
        }
        let stored = self.db.execute_raw(self.statement(
            r#"UPDATE "OAuthConnection" SET "tokenEncrypted" = $3, "state" = 'ready', "updatedAt" = $4 WHERE "id" = $1 AND "generation" = $2 AND "state" = 'refreshing'"#,
            [connection.id.into(), generation.into(), self.encrypt(&token)?.into(), now().into()],
        )).await.map_err(storage_error)?;
        if stored.rows_affected() != 1 {
            return Err(reconnect());
        }
        token.public_response(id, scope)
    }

    pub(super) async fn revoke(&self, token: &str) -> Result<String, OAuthProxyError> {
        let id = self.handle(token);
        let connection = self.read(&id).await?;
        self.db.execute_raw(self.statement(
            r#"UPDATE "OAuthConnection" SET "state" = 'revoked', "updatedAt" = $2 WHERE "id" = $1"#,
            [id.into(), now().into()],
        )).await.map_err(storage_error)?;
        connection
            .token
            .response
            .refresh_token
            .ok_or_else(reconnect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    const KEY: [u8; 32] = [42; 32];

    async fn database() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_raw(Statement::from_string(
            db.get_database_backend(),
            include_str!(
                "../../../prisma/migrations/20261015120000_oauth_connections/migration.sql"
            ),
        ))
        .await
        .unwrap();
        db
    }

    fn store(db: &DatabaseConnection) -> Connections<'_> {
        Connections::new(
            db,
            &KEY,
            "provider",
            "client",
            "https://issuer.test/token",
            Some("owner"),
        )
    }

    fn expired() -> TokenResponse {
        TokenResponse {
            access_token: "expired-access".into(),
            refresh_token: Some("original-provider-refresh".into()),
            expires_in: Some(-1),
            scope: Some("read write".into()),
            ..Default::default()
        }
    }

    fn rotated() -> TokenResponse {
        TokenResponse {
            access_token: "new-access".into(),
            refresh_token: Some("rotated-provider-refresh".into()),
            expires_in: Some(3600),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn copied_grants_share_one_refresh_and_keep_replacement_private() {
        let db = database().await;
        let a = store(&db);
        let b = store(&db);
        let id = a.adopt(expired()).await.unwrap();
        assert!(id.starts_with(HANDLE_PREFIX));
        assert_eq!(b.adopt(expired()).await.unwrap(), id);
        let calls = Arc::new(AtomicUsize::new(0));
        let refresh = |token: String| {
            let calls = calls.clone();
            async move {
                assert_eq!(token, "original-provider-refresh");
                calls.fetch_add(1, Ordering::SeqCst);
                flow_like_types::tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(rotated())
            }
        };
        let (first, second) =
            flow_like_types::tokio::join!(a.resolve(&id, refresh), b.resolve(&id, refresh));
        for result in [first.unwrap(), second.unwrap()] {
            assert_eq!(result.access_token, "new-access");
            assert_eq!(result.refresh_token.as_deref(), Some(id.as_str()));
            assert_eq!(result.scope.as_deref(), Some("read write"));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Saving an old browser/sink snapshot cannot overwrite the rotated grant.
        assert_eq!(b.adopt(expired()).await.unwrap(), id);
        let restored = b
            .resolve(&id, |_| async { panic!("must use durable replacement") })
            .await
            .unwrap();
        assert_eq!(restored.access_token, "new-access");
        let record = a.read(&id).await.unwrap();
        assert_eq!(
            record.token.response.refresh_token.as_deref(),
            Some("rotated-provider-refresh")
        );
    }

    #[tokio::test]
    async fn resource_tokens_share_rotation_without_replacing_the_base_audience() {
        let db = database().await;
        let store = store(&db);
        let id = store
            .adopt(TokenResponse {
                access_token: "discovery-access".into(),
                expires_in: Some(3600),
                ..expired()
            })
            .await
            .unwrap();
        let resource = store
            .resolve_scoped(&id, Some("resource-a"), |refresh| async move {
                assert_eq!(refresh, "original-provider-refresh");
                Ok(TokenResponse {
                    access_token: "resource-a-access".into(),
                    ..rotated()
                })
            })
            .await
            .unwrap();
        assert_eq!(resource.scope.as_deref(), Some("resource-a"));
        assert_eq!(resource.refresh_token.as_deref(), Some(id.as_str()));
        let base = store
            .resolve(&id, |_| async { panic!("base token is current") })
            .await
            .unwrap();
        assert_eq!(base.access_token, "discovery-access");
        store
            .resolve_scoped(&id, Some("resource-b"), |refresh| async move {
                assert_eq!(refresh, "rotated-provider-refresh");
                Ok(TokenResponse {
                    access_token: "resource-b-access".into(),
                    refresh_token: Some("second-rotation".into()),
                    ..rotated()
                })
            })
            .await
            .unwrap();
        let cached = store
            .resolve_scoped(&id, Some("resource-a"), |_| async {
                panic!("reuse audience token")
            })
            .await
            .unwrap();
        assert_eq!(cached.access_token, "resource-a-access");
        assert_eq!(
            store
                .read(&id)
                .await
                .unwrap()
                .token
                .response
                .refresh_token
                .as_deref(),
            Some("second-rotation")
        );
        assert_eq!(Connections::client_id(&db, &id).await.unwrap(), "client");
    }

    #[tokio::test]
    async fn definitive_provider_rejections_keep_the_existing_grant_usable() {
        let db = database().await;
        let store = store(&db);
        let id = store
            .adopt(TokenResponse {
                expires_in: Some(3600),
                ..expired()
            })
            .await
            .unwrap();
        for code in ["oauth_scope_rejected", "oauth_provider_rejected"] {
            let error = store
                .resolve_scoped(&id, Some("invalid-resource"), |_| async move {
                    Err(OAuthProxyError::coded(
                        StatusCode::BAD_REQUEST,
                        code,
                        "request rejected",
                    ))
                })
                .await
                .unwrap_err();
            assert_eq!(error.code, code);
        }
        let base = store
            .resolve(&id, |_| async { panic!("base token is current") })
            .await
            .unwrap();
        assert_eq!(base.access_token, "expired-access");
        assert_eq!(
            store
                .read(&id)
                .await
                .unwrap()
                .token
                .response
                .refresh_token
                .as_deref(),
            Some("original-provider-refresh")
        );
    }

    #[tokio::test]
    async fn owner_and_provider_configuration_are_bound_to_the_handle() {
        let db = database().await;
        let id = store(&db).adopt(expired()).await.unwrap();
        let other_owner = Connections::new(
            &db,
            &KEY,
            "provider",
            "client",
            "https://issuer.test/token",
            Some("other"),
        );
        assert!(other_owner.adopt(expired()).await.is_err());
        for (provider, client, url) in [
            ("other", "client", "https://issuer.test/token"),
            ("provider", "other", "https://issuer.test/token"),
            ("provider", "client", "https://other.test/token"),
        ] {
            let other = Connections::new(&db, &KEY, provider, client, url, Some("owner"));
            assert!(other.read(&id).await.is_err());
        }
        // Server execution uses the encrypted delegated handle without a browser identity.
        let sink = Connections::new(
            &db,
            &KEY,
            "provider",
            "client",
            "https://issuer.test/token",
            None,
        );
        assert!(sink.read(&id).await.is_ok());
    }

    #[tokio::test]
    async fn uncertain_refresh_is_never_replayed() {
        let db = database().await;
        let store = store(&db);
        let id = store.adopt(expired()).await.unwrap();
        let error = store
            .resolve(&id, |_| async {
                Err(OAuthProxyError::new(StatusCode::BAD_GATEWAY, "timeout"))
            })
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "oauth_reauthorization_required");
        assert!(
            store
                .resolve(&id, |_| async {
                    panic!("provider token must not be reused")
                })
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn failed_persistence_does_not_publish_or_replay_a_rotated_token() {
        let db = database().await;
        let store = store(&db);
        let id = store.adopt(expired()).await.unwrap();
        db.execute_raw(Statement::from_string(db.get_database_backend(), r#"CREATE TRIGGER reject_rotation BEFORE UPDATE ON "OAuthConnection" WHEN NEW.state = 'ready' BEGIN SELECT RAISE(FAIL, 'storage unavailable'); END"#)).await.unwrap();
        assert!(
            store
                .resolve(&id, |_| async { Ok(rotated()) })
                .await
                .is_err()
        );
        assert_eq!(store.read(&id).await.unwrap().state, "refreshing");
        db.execute_raw(Statement::from_string(
            db.get_database_backend(),
            r#"UPDATE "OAuthConnection" SET "updatedAt" = 0"#,
        ))
        .await
        .unwrap();
        assert!(
            store
                .resolve(&id, |_| async {
                    panic!("uncertain rotation must never repeat")
                })
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn only_confirmed_reauthorization_can_restore_a_revoked_grant() {
        let db = database().await;
        let store = store(&db);
        let id = store.adopt(expired()).await.unwrap();
        store.revoke(&id).await.unwrap();
        store.adopt(expired()).await.unwrap();
        assert_eq!(store.read(&id).await.unwrap().state, "revoked");
        let fresh = TokenResponse {
            access_token: "fresh-authorization".into(),
            expires_in: Some(3600),
            ..expired()
        };
        assert_eq!(store.reauthorize(fresh).await.unwrap(), id);
        let response = store
            .resolve(&id, |_| async { panic!("fresh grant") })
            .await
            .unwrap();
        assert_eq!(response.access_token, "fresh-authorization");
        let expanded = TokenResponse {
            access_token: "fresh-authorization".into(),
            scope: Some("read write admin".into()),
            expires_in: Some(3600),
            ..expired()
        };
        store.reauthorize(expanded).await.unwrap();
        let response = store
            .resolve(&id, |_| async { panic!("updated grant") })
            .await
            .unwrap();
        assert_eq!(response.access_token, "fresh-authorization");
        assert_eq!(response.scope.as_deref(), Some("read write admin"));
    }

    #[tokio::test]
    async fn omitted_refresh_token_is_retained_and_revocation_stops_sink_access() {
        let db = database().await;
        let store = store(&db);
        let id = store.adopt(expired()).await.unwrap();
        store
            .resolve(&id, |_| async {
                Ok(TokenResponse {
                    refresh_token: None,
                    ..rotated()
                })
            })
            .await
            .unwrap();
        assert_eq!(
            store.revoke(&id).await.unwrap(),
            "original-provider-refresh"
        );
        assert!(
            store
                .resolve(&id, |_| async { panic!("revoked connection") })
                .await
                .is_err()
        );
    }
}
