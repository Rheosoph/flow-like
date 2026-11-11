//! Standing tool approvals of local bots: `(tool, scope)` rows per bot, and beside them the
//! short-lived `once:` and `deny:` tokens an approval answer leaves for the turn that resumes.
//! Which scopes a tool call needs is core's rule (`flow_like::flow::copilot::tool_grants`); the
//! store only keeps the pairs.

use crate::{
    LOCAL_OWNER_ID, LocalBotStore,
    bots::{bot_exists, bot_not_found},
    error::{Error, Result},
    store::{now_micros, timestamp},
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::tool_grant::{
    DENY_PREFIX, GRANT_SLIDE_HOURS, GRANT_TTL_DAYS, ONCE_PREFIX, TOKEN_TTL_HOURS, ToolGrant,
    ToolGrantKey, check_key, is_token_scope,
};
use rusqlite::{Connection, params};

const HOUR_MICROS: i64 = 3_600_000_000;

fn token_cutoff() -> i64 {
    now_micros() - TOKEN_TTL_HOURS * HOUR_MICROS
}

fn grant_expiry(now: i64) -> i64 {
    now + GRANT_TTL_DAYS * 24 * HOUR_MICROS
}

fn slide_threshold(now: i64) -> i64 {
    grant_expiry(now) - GRANT_SLIDE_HOURS * HOUR_MICROS
}

/// A turn parked behind an approval card: it runs again once the person answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkedTurn {
    pub turn_id: String,
    pub bot_id: String,
    pub conversation_id: String,
    pub trigger_message_id: String,
    pub approval_id: String,
    pub parked_at: DateTime<FixedOffset>,
}

pub(crate) fn purge_tokens(db: &Connection) -> Result<usize> {
    let tokens = db.execute(
        "DELETE FROM tool_grants WHERE granted_at < ?1 AND (scope LIKE ?2 OR scope LIKE ?3)",
        params![
            token_cutoff(),
            format!("{ONCE_PREFIX}%"),
            format!("{DENY_PREFIX}%")
        ],
    )?;
    let lapsed = db.execute(
        "DELETE FROM tool_grants WHERE expires_at IS NOT NULL AND expires_at <= ?1",
        [now_micros()],
    )?;
    Ok(tokens + lapsed)
}

fn checked(key: &ToolGrantKey) -> Result<()> {
    check_key(key).map_err(Error::invalid)
}

impl LocalBotStore {
    /// What the person allowed the bot, oldest first; answer tokens stay out.
    pub fn tool_grants(&self, bot_id: &str) -> Result<Vec<ToolGrant>> {
        self.read(|db| {
            let mut statement = db.prepare_cached(
                "SELECT tool, scope, granted_at, granted_by, expires_at FROM tool_grants
                 WHERE bot_id = ?1 AND (expires_at IS NULL OR expires_at > ?2)
                 ORDER BY granted_at, tool, scope",
            )?;
            let rows = statement
                .query_map(params![bot_id, now_micros()], |row| {
                    Ok(ToolGrant {
                        tool: row.get(0)?,
                        scope: row.get(1)?,
                        granted_at: timestamp(row.get(2)?).to_rfc3339(),
                        granted_by: row.get(3)?,
                        expires_at: row
                            .get::<_, Option<i64>>(4)?
                            .map(|micros| timestamp(micros).to_rfc3339()),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows
                .into_iter()
                .filter(|grant| !is_token_scope(&grant.scope))
                .collect())
        })
    }

    /// Allows `tool` for `scope`; allowing again changes nothing.
    pub fn grant_tool(&self, bot_id: &str, key: &ToolGrantKey) -> Result<ToolGrant> {
        checked(key)?;
        self.put_tool_key(bot_id, &key.tool, &key.scope)?;
        self.tool_grants(bot_id)?
            .into_iter()
            .find(|grant| grant.tool == key.tool && grant.scope == key.scope)
            .ok_or_else(|| Error::storage("The grant vanished"))
    }

    /// Takes the grant back; false when there was none.
    pub fn revoke_tool(&self, bot_id: &str, key: &ToolGrantKey) -> Result<bool> {
        checked(key)?;
        self.delete_tool_key(bot_id, &key.tool, &key.scope)
    }

    /// Every stored pair of the bot, tokens included and expired tokens dropped.
    pub fn tool_keys(&self, bot_id: &str) -> Result<Vec<(String, String)>> {
        self.write(|tx, _| {
            purge_tokens(tx)?;
            let mut statement = tx.prepare_cached(
                "SELECT tool, scope FROM tool_grants
                 WHERE bot_id = ?1 AND (expires_at IS NULL OR expires_at > ?2)",
            )?;
            let rows = statement
                .query_map(params![bot_id, now_micros()], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn put_tool_key(&self, bot_id: &str, tool: &str, scope: &str) -> Result<()> {
        self.write(|tx, changes| {
            if !bot_exists(tx, bot_id)? {
                return Err(bot_not_found(bot_id));
            }
            let now = now_micros();
            tx.execute(
                "INSERT INTO tool_grants (bot_id, tool, scope, granted_at, granted_by, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (bot_id, tool, scope) DO UPDATE SET
                     granted_at = CASE WHEN ?7 OR expires_at <= ?4 THEN ?4 ELSE granted_at END,
                     expires_at = ?6
                 WHERE ?7 OR expires_at IS NULL OR expires_at <= ?4 OR expires_at < ?8",
                params![
                    bot_id,
                    tool,
                    scope,
                    now,
                    LOCAL_OWNER_ID,
                    grant_expiry(now),
                    is_token_scope(scope),
                    slide_threshold(now)
                ],
            )?;
            changes.bot(bot_id);
            Ok(())
        })
    }

    /// Moves the expiry of grants the bot used forward, at most once a day each; a grant that
    /// is gone or lapsed stays so.
    pub fn touch_tool_keys(&self, bot_id: &str, keys: &[(String, String)]) -> Result<()> {
        self.write(|tx, _| {
            let now = now_micros();
            for (tool, scope) in keys {
                tx.execute(
                    "UPDATE tool_grants SET expires_at = ?1
                     WHERE bot_id = ?2 AND tool = ?3 AND scope = ?4
                       AND (expires_at IS NULL OR (expires_at > ?5 AND expires_at < ?6))",
                    params![grant_expiry(now), bot_id, tool, scope, now, slide_threshold(now)],
                )?;
            }
            Ok(())
        })
    }

    pub fn delete_tool_key(&self, bot_id: &str, tool: &str, scope: &str) -> Result<bool> {
        self.write(|tx, changes| {
            let removed = tx.execute(
                "DELETE FROM tool_grants WHERE bot_id = ?1 AND tool = ?2 AND scope = ?3",
                params![bot_id, tool, scope],
            )?;
            if removed > 0 {
                changes.bot(bot_id);
            }
            Ok(removed > 0)
        })
    }

    /// Turns waiting for an answer, oldest first.
    pub fn parked_turns(&self) -> Result<Vec<ParkedTurn>> {
        self.read(|db| {
            let mut statement = db.prepare_cached(
                "SELECT turn_id, bot_id, conversation_id, trigger_message_id, approval_id, parked_at
                 FROM parked_turns ORDER BY parked_at, turn_id",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok(ParkedTurn {
                        turn_id: row.get(0)?,
                        bot_id: row.get(1)?,
                        conversation_id: row.get(2)?,
                        trigger_message_id: row.get(3)?,
                        approval_id: row.get(4)?,
                        parked_at: timestamp(row.get(5)?),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }
}
