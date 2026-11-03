//! Standing grants between bots: `from_bot_id` may ask `to_bot_id` without asking the person
//! again. A grant points one way and goes when either bot goes. Built-in bots need their row
//! (`ensure_built_ins`) before a grant can name them.

use crate::{
    BotPermission, LOCAL_OWNER_ID, LocalBotStore,
    bots::{bot_exists, bot_not_found},
    error::{Error, Result},
    store::{now_micros, timestamp},
};
use flow_like_types::create_id;
use rusqlite::{Connection, OptionalExtension, Row, params};

const COLUMNS: &str = "id, from_bot_id, to_bot_id, granted_at, granted_by";

fn read(row: &Row<'_>) -> rusqlite::Result<BotPermission> {
    Ok(BotPermission {
        id: row.get("id")?,
        owner_id: LOCAL_OWNER_ID.to_string(),
        from_bot_id: row.get("from_bot_id")?,
        to_bot_id: row.get("to_bot_id")?,
        granted_at: timestamp(row.get("granted_at")?),
        granted_by: row.get("granted_by")?,
    })
}

fn find(db: &Connection, from_bot_id: &str, to_bot_id: &str) -> Result<Option<BotPermission>> {
    Ok(db
        .query_row(
            &format!("SELECT {COLUMNS} FROM permissions WHERE from_bot_id = ?1 AND to_bot_id = ?2"),
            params![from_bot_id, to_bot_id],
            read,
        )
        .optional()?)
}

impl LocalBotStore {
    /// Every grant, oldest first.
    pub fn permissions(&self) -> Result<Vec<BotPermission>> {
        self.read(|db| {
            let mut statement = db.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM permissions ORDER BY granted_at, id"
            ))?;
            let grants = statement
                .query_map([], read)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(grants)
        })
    }

    /// Whether `from_bot_id` may ask `to_bot_id` without asking the person.
    pub fn has_permission(&self, from_bot_id: &str, to_bot_id: &str) -> Result<bool> {
        self.read(|db| Ok(find(db, from_bot_id, to_bot_id)?.is_some()))
    }

    /// Lets `from_bot_id` ask `to_bot_id`; granting again returns the first grant.
    pub fn grant_permission(&self, from_bot_id: &str, to_bot_id: &str) -> Result<BotPermission> {
        if from_bot_id == to_bot_id {
            return Err(Error::invalid(
                "A bot doesn't need permission to ask itself",
            ));
        }
        self.write(|tx, changes| {
            for bot_id in [from_bot_id, to_bot_id] {
                if !bot_exists(tx, bot_id)? {
                    return Err(bot_not_found(bot_id));
                }
            }
            if let Some(existing) = find(tx, from_bot_id, to_bot_id)? {
                return Ok(existing);
            }
            tx.execute(
                "INSERT INTO permissions (id, from_bot_id, to_bot_id, granted_at, granted_by)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    format!("grant_{}", create_id()),
                    from_bot_id,
                    to_bot_id,
                    now_micros(),
                    LOCAL_OWNER_ID
                ],
            )?;
            changes.bot(from_bot_id);
            changes.bot(to_bot_id);
            find(tx, from_bot_id, to_bot_id)?.ok_or_else(|| Error::storage("The grant vanished"))
        })
    }

    /// Takes the grant back; `false` when there was none.
    pub fn revoke_permission(&self, from_bot_id: &str, to_bot_id: &str) -> Result<bool> {
        self.write(|tx, changes| {
            let removed = tx.execute(
                "DELETE FROM permissions WHERE from_bot_id = ?1 AND to_bot_id = ?2",
                params![from_bot_id, to_bot_id],
            )?;
            if removed > 0 {
                changes.bot(from_bot_id);
                changes.bot(to_bot_id);
            }
            Ok(removed > 0)
        })
    }
}
