//! Redis cache backend with native TTL.
//!
//! Entries with a TTL are written with `SET ... EX` so Redis evicts them without help.
//! Entries without a TTL are written with a plain `SET` and persist until deleted, which
//! is what makes the cache usable as cheap configuration storage.

use super::types::*;
use async_trait::async_trait;
use chrono::Utc;
use futures::lock::Mutex;
use redis::{AsyncCommands, Client, aio::MultiplexedConnection};
use std::sync::Arc;

const KEY_PREFIX: &str = "flowcache:";
/// Set membership per app, so an app teardown can find its keys without `SCAN`.
const APP_INDEX_PREFIX: &str = "flowcache:app:";

#[derive(Debug)]
pub struct RedisCacheStore {
    conn: Arc<Mutex<MultiplexedConnection>>,
}

impl RedisCacheStore {
    pub async fn new(url: &str) -> Result<Self, CacheStoreError> {
        let client = Client::open(url).map_err(|e| CacheStoreError::Connection(e.to_string()))?;
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| CacheStoreError::Connection(e.to_string()))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn from_env() -> Result<Self, CacheStoreError> {
        let url = std::env::var("CACHE_REDIS_URL")
            .or_else(|_| std::env::var("REDIS_URL"))
            .unwrap_or_else(|_| "redis://localhost:6379".to_string());
        Self::new(&url).await
    }

    fn entry_key(key: &CacheKey) -> String {
        format!("{KEY_PREFIX}{}:{}", key.app_id, key.sort_key())
    }

    /// Every Redis key of one `(app, scope, user, namespace)` slice starts with this,
    /// because the sort key hashes its segments into fixed-width blocks — so namespace
    /// invalidation is a literal `starts_with`, no parsing and no glob escaping.
    fn namespace_key_prefix(
        app_id: &str,
        scope: flow_like_types::cache::CacheScope,
        user_id: &str,
        namespace: &str,
    ) -> String {
        format!(
            "{KEY_PREFIX}{}:{}",
            app_id,
            CacheKey::namespace_sort_prefix(scope, user_id, namespace)
        )
    }

    fn app_index_key(app_id: &str) -> String {
        format!("{APP_INDEX_PREFIX}{app_id}")
    }

    /// Whole seconds remaining, floored at 1 so a nearly-elapsed TTL never rounds to a
    /// non-expiring `SET`.
    fn ttl_seconds(expires_at: Option<i64>, now_ms: i64) -> Option<u64> {
        expires_at.map(|expires| (((expires - now_ms) + 999) / 1_000).max(1) as u64)
    }

    /// One `SSCAN` page of an index set.
    ///
    /// Cursor-based on purpose: a busy app's index can hold many members, and a single
    /// `SMEMBERS` would block single-threaded Redis (and every other cache request on
    /// this shared connection) for the whole materialization.
    async fn sscan_page(
        conn: &mut MultiplexedConnection,
        index_key: &str,
        cursor: u64,
    ) -> Result<(u64, Vec<String>), CacheStoreError> {
        redis::cmd("SSCAN")
            .arg(index_key)
            .arg(cursor)
            .arg("COUNT")
            .arg(SCAN_BATCH)
            .query_async(&mut *conn)
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))
    }
}

/// Members handled per `SSCAN`/`SCAN` page.
const SCAN_BATCH: usize = 500;

/// Atomic "insert if nothing live" honouring the *stored* expiry, not just Redis's.
///
/// TTLs are rounded up to whole seconds on write, so for up to a second an entry can be
/// physically present while its stored `expires_at` has already lapsed. A plain
/// `SET NX` refuses to write in that window although the trait contract says a lapsed
/// entry does not count as present — `get_or_set` would then burn its retries in
/// milliseconds and surface a spurious conflict.
///
/// KEYS[1] = entry key, ARGV[1] = payload, ARGV[2] = now in epoch ms,
/// ARGV[3] = TTL in seconds or "" for no expiry. Returns 1 when this call wrote.
const TRY_INSERT_SCRIPT: &str = r#"
local cur = redis.call('GET', KEYS[1])
if cur then
  local ok, rec = pcall(cjson.decode, cur)
  if ok and rec ~= nil then
    local exp = rec['expires_at']
    if type(exp) ~= 'number' or exp > tonumber(ARGV[2]) then
      return 0
    end
  end
end
if ARGV[3] ~= '' then
  redis.call('SET', KEYS[1], ARGV[1], 'EX', tonumber(ARGV[3]))
else
  redis.call('SET', KEYS[1], ARGV[1])
end
return 1
"#;

#[async_trait]
impl CacheStore for RedisCacheStore {
    fn backend_name(&self) -> &'static str {
        "redis"
    }

    async fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, CacheStoreError> {
        let redis_key = Self::entry_key(key);
        let mut conn = self.conn.lock().await;
        let raw: Option<String> = conn
            .get(&redis_key)
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        let Some(raw) = raw else {
            return Ok(None);
        };

        let entry: CacheEntry = serde_json::from_str(&raw)
            .map_err(|e| CacheStoreError::Serialization(e.to_string()))?;

        // Redis eviction is not instantaneous; honour the stored expiry regardless.
        if entry.is_expired_at(Utc::now().timestamp_millis()) {
            return Ok(None);
        }

        Ok(Some(entry))
    }

    async fn exists(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        // Redis owns expiry here, so EXISTS is the whole answer and the value never
        // crosses the wire. Note the TTL was rounded up to whole seconds on write, so
        // within the last second of an entry's life this can still report `true` while
        // `get` already reports a miss. Callers that cannot tolerate that gap should use
        // `get_or_set`, which decides atomically.
        let redis_key = Self::entry_key(key);
        let mut conn = self.conn.lock().await;
        let exists: bool = conn
            .exists(&redis_key)
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(exists)
    }

    async fn set(&self, entry: SetCacheEntry) -> Result<CacheEntry, CacheStoreError> {
        let now_ms = Utc::now().timestamp_millis();
        let record = CacheEntry {
            key: entry.key.key.clone(),
            value: entry.value,
            expires_at: entry.expires_at,
            updated_at: now_ms,
        };

        let payload = serde_json::to_string(&record)
            .map_err(|e| CacheStoreError::Serialization(e.to_string()))?;
        let redis_key = Self::entry_key(&entry.key);
        let index_key = Self::app_index_key(&entry.key.app_id);

        let mut conn = self.conn.lock().await;
        let mut pipe = redis::pipe();
        match Self::ttl_seconds(entry.expires_at, now_ms) {
            Some(ttl) => pipe.set_ex(&redis_key, &payload, ttl),
            None => pipe.set(&redis_key, &payload),
        };
        // The index has no TTL of its own; stale members are pruned lazily by
        // `delete_expired`, which is cheaper than re-expiring the set on every write.
        pipe.sadd(&index_key, &redis_key);

        pipe.query_async::<()>(&mut *conn)
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(record)
    }

    async fn try_insert(
        &self,
        entry: SetCacheEntry,
    ) -> Result<Option<CacheEntry>, CacheStoreError> {
        let now_ms = Utc::now().timestamp_millis();
        let record = CacheEntry {
            key: entry.key.key.clone(),
            value: entry.value,
            expires_at: entry.expires_at,
            updated_at: now_ms,
        };

        let payload = serde_json::to_string(&record)
            .map_err(|e| CacheStoreError::Serialization(e.to_string()))?;
        let redis_key = Self::entry_key(&entry.key);
        let index_key = Self::app_index_key(&entry.key.app_id);

        // A Lua script rather than `SET NX`: the script honours the *stored* expiry, so
        // an entry whose lifetime lapsed but which Redis has not physically evicted yet
        // (TTLs are rounded up to whole seconds) still counts as absent, exactly as the
        // trait contract requires.
        let ttl_arg = Self::ttl_seconds(entry.expires_at, now_ms)
            .map(|ttl| ttl.to_string())
            .unwrap_or_default();

        let mut conn = self.conn.lock().await;
        let wrote: i64 = redis::Script::new(TRY_INSERT_SCRIPT)
            .key(&redis_key)
            .arg(&payload)
            .arg(now_ms)
            .arg(ttl_arg)
            .invoke_async(&mut *conn)
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        if wrote == 0 {
            return Ok(None);
        }

        let _: i64 = conn
            .sadd(&index_key, &redis_key)
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(Some(record))
    }

    async fn delete(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        let redis_key = Self::entry_key(key);
        let index_key = Self::app_index_key(&key.app_id);

        let mut conn = self.conn.lock().await;
        let removed: i64 = conn
            .del(&redis_key)
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;
        let _: i64 = conn
            .srem(&index_key, &redis_key)
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(removed > 0)
    }

    async fn delete_namespace(
        &self,
        app_id: &str,
        scope: flow_like_types::cache::CacheScope,
        user_id: &str,
        namespace: &str,
    ) -> Result<i64, CacheStoreError> {
        if namespace.is_empty() {
            return Err(CacheStoreError::InvalidInput(
                "Namespace invalidation requires a non-empty namespace".to_string(),
            ));
        }

        // The per-app index names every key this app has written, so no keyspace SCAN
        // is needed; SSCAN keeps each round trip small.
        let prefix = Self::namespace_key_prefix(app_id, scope, user_id, namespace);
        let index_key = Self::app_index_key(app_id);
        let mut conn = self.conn.lock().await;

        let mut removed = 0i64;
        let mut cursor = 0u64;
        loop {
            let (next, members) = Self::sscan_page(&mut conn, &index_key, cursor).await?;

            let matching: Vec<String> = members
                .into_iter()
                .filter(|member| member.starts_with(&prefix))
                .collect();

            if !matching.is_empty() {
                // DEL counts only keys that still existed; expired members are pruned
                // from the index all the same.
                let deleted: i64 = conn
                    .del(matching.clone())
                    .await
                    .map_err(|e| CacheStoreError::Database(e.to_string()))?;
                removed += deleted;
                let _: i64 = conn
                    .srem(&index_key, matching)
                    .await
                    .map_err(|e| CacheStoreError::Database(e.to_string()))?;
            }

            cursor = next;
            if cursor == 0 {
                break;
            }
        }

        Ok(removed)
    }

    async fn delete_app(&self, app_id: &str) -> Result<i64, CacheStoreError> {
        let index_key = Self::app_index_key(app_id);
        let mut conn = self.conn.lock().await;

        let mut removed = 0i64;
        let mut cursor = 0u64;
        loop {
            let (next, members) = Self::sscan_page(&mut conn, &index_key, cursor).await?;

            if !members.is_empty() {
                let deleted: i64 = conn
                    .del(members)
                    .await
                    .map_err(|e| CacheStoreError::Database(e.to_string()))?;
                removed += deleted;
            }

            cursor = next;
            if cursor == 0 {
                break;
            }
        }

        let _: i64 = conn
            .del(&index_key)
            .await
            .map_err(|e| CacheStoreError::Database(e.to_string()))?;

        Ok(removed)
    }

    async fn delete_expired(&self) -> Result<i64, CacheStoreError> {
        // Redis evicts the entries themselves, but nothing else removes their members
        // from the per-app index sets — without this sweep a busy app's index grows
        // without bound. Walk every index, check which members still exist, and drop
        // the dead ones. Returns the number of members reclaimed.
        let mut conn = self.conn.lock().await;

        let mut index_keys: Vec<String> = Vec::new();
        let mut cursor = 0u64;
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(format!("{APP_INDEX_PREFIX}*"))
                .arg("COUNT")
                .arg(SCAN_BATCH)
                .query_async(&mut *conn)
                .await
                .map_err(|e| CacheStoreError::Database(e.to_string()))?;
            index_keys.extend(keys);
            cursor = next;
            if cursor == 0 {
                break;
            }
        }

        let mut pruned = 0i64;
        for index_key in index_keys {
            let mut cursor = 0u64;
            loop {
                let (next, members) = Self::sscan_page(&mut conn, &index_key, cursor).await?;

                if !members.is_empty() {
                    let mut pipe = redis::pipe();
                    for member in &members {
                        pipe.exists(member);
                    }
                    let alive: Vec<bool> = pipe
                        .query_async(&mut *conn)
                        .await
                        .map_err(|e| CacheStoreError::Database(e.to_string()))?;

                    let dead: Vec<String> = members
                        .into_iter()
                        .zip(alive)
                        .filter(|(_, alive)| !alive)
                        .map(|(member, _)| member)
                        .collect();

                    if !dead.is_empty() {
                        let dropped: i64 = conn
                            .srem(&index_key, dead)
                            .await
                            .map_err(|e| CacheStoreError::Database(e.to_string()))?;
                        pruned += dropped;
                    }
                }

                cursor = next;
                if cursor == 0 {
                    break;
                }
            }
        }

        Ok(pruned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_prefix_covers_exactly_its_entries() {
        use flow_like_types::cache::CacheScope;

        let entry = RedisCacheStore::entry_key(&CacheKey::app("app-1", "reports", "daily"));

        let own = RedisCacheStore::namespace_key_prefix("app-1", CacheScope::App, "", "reports");
        assert!(entry.starts_with(&own));

        let other_ns =
            RedisCacheStore::namespace_key_prefix("app-1", CacheScope::App, "", "billing");
        assert!(!entry.starts_with(&other_ns));

        let other_app =
            RedisCacheStore::namespace_key_prefix("app-2", CacheScope::App, "", "reports");
        assert!(!entry.starts_with(&other_app));

        let user_scope =
            RedisCacheStore::namespace_key_prefix("app-1", CacheScope::User, "alice", "reports");
        assert!(!entry.starts_with(&user_scope));
    }

    #[test]
    fn ttl_rounds_up_and_never_reaches_zero() {
        // 1 ms left must still be a 1 second TTL, not a non-expiring SET.
        assert_eq!(RedisCacheStore::ttl_seconds(Some(1), 0), Some(1));
        assert_eq!(RedisCacheStore::ttl_seconds(Some(1_000), 0), Some(1));
        assert_eq!(RedisCacheStore::ttl_seconds(Some(1_001), 0), Some(2));
        // Already elapsed: still clamp to 1 so Redis drops it promptly.
        assert_eq!(RedisCacheStore::ttl_seconds(Some(0), 5_000), Some(1));
        assert_eq!(RedisCacheStore::ttl_seconds(None, 0), None);
    }
}
