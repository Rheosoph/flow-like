//! Bot tokens of the Telegram and Discord sinks live in the system keychain. The sink database
//! keeps a reference to a token, so a copy of the database does not hold the bots.

use anyhow::{Context, Result, bail};
use flow_like_bots::Provider;
use rusqlite::{Connection, params};

const REFERENCE_PREFIX: &str = "kc1:";

#[cfg(all(not(test), not(target_os = "android")))]
const SERVICE: &str = "com.flow-like.event-sink-bots.v1";

trait Vault: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
}

#[cfg(all(not(test), not(target_os = "android")))]
struct Keychain;

#[cfg(all(not(test), not(target_os = "android")))]
impl Keychain {
    fn entry(account: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, account).context("Cannot open the system credential store")
    }
}

#[cfg(all(not(test), not(target_os = "android")))]
impl Vault for Keychain {
    fn get(&self, account: &str) -> Result<Option<String>> {
        match Self::entry(account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!("The system credential store is unavailable or locked"),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        Self::entry(account)?
            .set_password(secret)
            .context("Cannot save the bot token in the system credential store")
    }

    fn delete(&self, account: &str) -> Result<()> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => bail!("The system credential store is unavailable or locked"),
        }
    }
}

// Android has no OS keyring backend in this dependency, and a token must never fall back to a
// file next to the database.
#[cfg(all(not(test), target_os = "android"))]
struct Keychain;

#[cfg(all(not(test), target_os = "android"))]
impl Vault for Keychain {
    fn get(&self, _: &str) -> Result<Option<String>> {
        bail!("Bot tokens need a system keychain, which this device doesn't offer")
    }

    fn set(&self, _: &str, _: &str) -> Result<()> {
        bail!("Bot tokens need a system keychain, which this device doesn't offer")
    }

    fn delete(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(not(test))]
fn vault() -> &'static dyn Vault {
    &Keychain
}

#[cfg(test)]
pub(super) mod memory {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Mutex, PoisonError};

    #[derive(Default)]
    pub(in super::super) struct MemoryVault {
        secrets: Mutex<HashMap<String, String>>,
        refused: Mutex<HashSet<String>>,
    }

    impl MemoryVault {
        pub(in super::super) fn holds(&self, account: &str) -> bool {
            self.secrets
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key(account)
        }

        pub(in super::super) fn refuse(&self, account: &str) {
            self.refused
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(account.to_owned());
        }
    }

    impl Vault for MemoryVault {
        fn get(&self, account: &str) -> Result<Option<String>> {
            Ok(self
                .secrets
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(account)
                .cloned())
        }

        fn set(&self, account: &str, secret: &str) -> Result<()> {
            if self
                .refused
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(account)
            {
                bail!("The system credential store is unavailable or locked");
            }
            self.secrets
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(account.to_owned(), secret.to_owned());
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<()> {
            self.secrets
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(account);
            Ok(())
        }
    }

    pub(in super::super) static VAULT: std::sync::LazyLock<MemoryVault> =
        std::sync::LazyLock::new(MemoryVault::default);
}

#[cfg(test)]
fn vault() -> &'static dyn Vault {
    &*memory::VAULT
}

/// What the database stores in place of `token`: the same for the same token, and no way back
/// to it.
pub(super) fn reference(provider: Provider, token: &str) -> String {
    let digest = blake3::hash(token.as_bytes()).to_hex();
    format!("{REFERENCE_PREFIX}{}:{}", provider.as_str(), &digest[..32])
}

pub(super) fn is_reference(stored: &str) -> bool {
    stored.starts_with(REFERENCE_PREFIX)
}

/// Saves `token` in the keychain and returns the reference to store.
pub(super) fn keep(provider: Provider, token: &str) -> Result<String> {
    if token.trim().is_empty() {
        bail!("The {} bot token is empty", provider.name());
    }
    let reference = reference(provider, token);
    vault()
        .set(&reference, token)
        .with_context(|| format!("Cannot store the {} bot token", provider.name()))?;
    Ok(reference)
}

/// The token behind a stored value. A value saved before tokens moved to the keychain is the
/// token itself.
pub(super) fn token_of(stored: &str) -> Result<String> {
    if !is_reference(stored) {
        return Ok(stored.to_owned());
    }
    vault()
        .get(stored)?
        .context("The bot token is missing from the system credential store")
}

/// The key a bot is known by while it runs: its reference, whichever way it is stored.
pub(super) fn key_of(provider: Provider, stored: &str) -> String {
    if is_reference(stored) {
        stored.to_owned()
    } else {
        reference(provider, stored)
    }
}

/// Removes the token behind a stored reference from the keychain.
pub(super) fn forget(stored: &str) {
    if !is_reference(stored) {
        return;
    }
    if let Err(error) = vault().delete(stored) {
        tracing::warn!(%error, "Could not remove a bot token from the system credential store");
    }
}

/// Moves the tokens of `bots` that an earlier version saved in plain text into the keychain, and
/// points the rows of `handlers` at the references. `copied` lists the columns of a bot row that
/// carry over. A token whose move fails stays where it is, and the bot keeps running from it.
pub(super) fn move_plaintext_tokens(
    conn: &Connection,
    provider: Provider,
    bots: &str,
    handlers: &str,
    copied: &str,
) -> Result<()> {
    let plaintext: Vec<String> = conn
        .prepare(&format!(
            "SELECT token FROM {bots} WHERE token NOT LIKE '{REFERENCE_PREFIX}%'"
        ))?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for token in plaintext {
        let reference = match keep(provider, &token) {
            Ok(reference) => reference,
            Err(error) => {
                tracing::error!(
                    %error,
                    "A {} bot token could not be moved to the system credential store",
                    provider.name()
                );
                continue;
            }
        };
        let moved = conn.unchecked_transaction()?;
        moved.execute(
            &format!(
                "INSERT INTO {bots} (token, {copied})
                 SELECT ?2, {copied} FROM {bots} WHERE token = ?1
                 ON CONFLICT(token) DO NOTHING"
            ),
            params![token, reference],
        )?;
        moved.execute(
            &format!("UPDATE {handlers} SET bot_token = ?2 WHERE bot_token = ?1"),
            params![token, reference],
        )?;
        moved.execute(
            &format!("DELETE FROM {bots} WHERE token = ?1"),
            params![token],
        )?;
        moved.commit()?;
    }
    Ok(())
}

/// Blanks the bot tokens that stored registrations still hold in their config.
pub(super) fn forget_registration_tokens(conn: &Connection) -> Result<()> {
    let stored: Vec<(String, String)> = conn
        .prepare(
            "SELECT event_id, config FROM event_registrations
             WHERE config LIKE '%\"bot_token\"%' OR config LIKE '%\"token\"%'",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (event_id, config) in stored {
        let Ok(mut config) = serde_json::from_str::<serde_json::Value>(&config) else {
            continue;
        };
        let key = match config.get("sink_type").and_then(serde_json::Value::as_str) {
            Some("telegram") => "bot_token",
            Some("discord") => "token",
            _ => continue,
        };
        let blanked = config
            .get_mut(key)
            .filter(|token| token.as_str().is_some_and(|token| !token.is_empty()))
            .map(|token| *token = serde_json::Value::String(String::new()))
            .is_some();
        if blanked {
            conn.execute(
                "UPDATE event_registrations SET config = ?2 WHERE event_id = ?1",
                params![event_id, serde_json::to_string(&config)?],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_sink::EventConfig;
    use serde_json::json;

    fn registrations() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE event_registrations (
                event_id TEXT PRIMARY KEY,
                type TEXT NOT NULL,
                config TEXT NOT NULL
            )",
            [],
        )
        .unwrap();
        conn
    }

    fn save(conn: &Connection, event_id: &str, config: serde_json::Value) {
        conn.execute(
            "INSERT INTO event_registrations (event_id, type, config) VALUES (?1, 'x', ?2)",
            params![event_id, config.to_string()],
        )
        .unwrap();
    }

    fn config_of(conn: &Connection, event_id: &str) -> serde_json::Value {
        let config: String = conn
            .query_row(
                "SELECT config FROM event_registrations WHERE event_id = ?1",
                params![event_id],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_str(&config).unwrap()
    }

    #[test]
    fn stored_registrations_lose_their_bot_tokens_and_nothing_else() {
        let conn = registrations();
        save(
            &conn,
            "tg",
            json!({"sink_type": "telegram", "bot_token": "7:tg-secret", "bot_name": "Helper"}),
        );
        save(
            &conn,
            "dc",
            json!({"sink_type": "discord", "token": "dc.secret", "respond_to_dms": true}),
        );
        save(
            &conn,
            "other",
            json!({"sink_type": "slack", "bot_token": "xoxb-kept", "token": "kept"}),
        );

        forget_registration_tokens(&conn).unwrap();
        forget_registration_tokens(&conn).unwrap();

        assert_eq!(
            config_of(&conn, "tg"),
            json!({"sink_type": "telegram", "bot_token": "", "bot_name": "Helper"})
        );
        assert_eq!(
            config_of(&conn, "dc"),
            json!({"sink_type": "discord", "token": "", "respond_to_dms": true})
        );
        assert_eq!(config_of(&conn, "other")["bot_token"], "xoxb-kept");
    }

    #[test]
    fn a_registration_is_stored_without_its_token_and_still_reads_back() {
        for (config, secret) in [
            (
                json!({"sink_type": "telegram", "bot_token": "7:tg-secret"}),
                "tg-secret",
            ),
            (
                json!({"sink_type": "discord", "token": "dc.secret"}),
                "dc.secret",
            ),
        ] {
            let parsed: EventConfig = serde_json::from_value(config).unwrap();
            let stored = serde_json::to_string(&parsed.for_storage()).unwrap();

            assert!(!stored.contains(secret), "{stored}");
            assert!(serde_json::from_str::<EventConfig>(&stored).is_ok());
            assert!(serde_json::to_string(&parsed).unwrap().contains(secret));
        }
    }

    #[test]
    fn a_reference_neither_holds_nor_reveals_its_token() {
        let token = "7:abcdef-secret-token";
        let reference = reference(Provider::Telegram, token);

        assert!(is_reference(&reference));
        assert!(!reference.contains("abcdef"));
        assert_eq!(reference, super::reference(Provider::Telegram, token));
        assert!(!is_reference(token));
    }

    #[test]
    fn a_missing_keychain_entry_is_an_error_not_an_empty_token() {
        let reference = reference(Provider::Discord, "never-stored.token");
        assert!(token_of(&reference).is_err());
        assert_eq!(
            token_of("plain.legacy.token").unwrap(),
            "plain.legacy.token"
        );
    }
}
