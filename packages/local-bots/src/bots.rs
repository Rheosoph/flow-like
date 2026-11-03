//! Local bots (Assistant's and Flux's rows among them), their apps and secrets, duplicates, the
//! bundle they move with, and the profile remap.

use crate::{
    LOCAL_OWNER_ID, cipher,
    error::{Error, Result, codes},
    inbox,
    model::{
        Bot, BotApp, BotSecretEncryption, BotSecretInfo, CreateBotRequest, ExportedSecret,
        LocalBotBundle, OpenedSecrets, SecretValue, UpdateBotRequest,
    },
    store::{Json, KeyError, LocalBotStore, STEP_MICROS, now_micros, timestamp},
    validation::{
        self, BotSettings, MAX_APPS_PER_BOT, MAX_LOCAL_BOTS, MAX_SECRETS_PER_BOT, NAME_MAX_CHARS,
    },
};
use chrono::DateTime;
use flow_like_types::{
    bot::{BotCapability, BotLocation, BotModelPolicy, BotSystemKind},
    bot_restrictions::BotRestrictions,
    create_id,
    data_classification::{DataClassification, DataSensitivity},
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;
use zeroize::Zeroizing;

const BOT_COLUMNS: &str = "id, system, preset_id, name, description, instructions, appearance, \
                           model, embedding_model, classification, pii, capabilities, profile_id, \
                           workspace_app_id, created_at, updated_at";

const OWNER_ROLE: &str = "owner";

/// Assistant's and Flux's names until the person renames them, like the API's.
const GENERAL_NAME: &str = "Assistant";
const FLUX_NAME: &str = "Flux";

pub(crate) struct StoredBot {
    id: String,
    pub(crate) system: Option<BotSystemKind>,
    preset_id: Option<String>,
    settings: BotSettings,
    pub(crate) workspace_app_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl StoredBot {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        let system: Option<String> = row.get("system")?;
        let classification: Option<String> = row.get("classification")?;
        let capabilities: Json<Vec<String>> = row.get("capabilities")?;
        Ok(Self {
            id: row.get("id")?,
            system: system.as_deref().and_then(BotSystemKind::parse),
            preset_id: row.get("preset_id")?,
            settings: BotSettings {
                name: row.get("name")?,
                description: row.get("description")?,
                instructions: row.get("instructions")?,
                appearance: row.get::<_, Json<Value>>("appearance")?.0,
                model: row.get::<_, Json<BotModelPolicy>>("model")?.0,
                embedding_model: row.get("embedding_model")?,
                classification: DataSensitivity {
                    classification: classification
                        .as_deref()
                        .and_then(DataClassification::parse),
                    pii: row.get("pii")?,
                },
                capabilities: validation::stored_capabilities(&capabilities.0),
                profile_id: row.get("profile_id")?,
            },
            workspace_app_id: row.get("workspace_app_id")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    /// The name shown: the stored one, or a built-in's default while it was never renamed.
    pub(crate) fn display_name(&self) -> &str {
        self.settings.name.as_deref().unwrap_or(match self.system {
            Some(BotSystemKind::General) => GENERAL_NAME,
            Some(BotSystemKind::Flux) => FLUX_NAME,
            None => "Bot",
        })
    }

    fn into_bot(self) -> Bot {
        let settings = self.settings;
        Bot {
            id: self.id,
            owner_id: LOCAL_OWNER_ID.to_string(),
            system: self.system,
            preset_id: self.preset_id,
            name: settings.name,
            description: settings.description,
            instructions: settings.instructions,
            appearance: settings.appearance,
            model: settings.model,
            embedding_model: settings.embedding_model,
            classification: settings.classification,
            capabilities: settings.capabilities,
            profile_id: settings.profile_id,
            location: BotLocation::Desktop,
            device_id: None,
            workspace_app_id: self.workspace_app_id,
            created_at: timestamp(self.created_at),
            updated_at: timestamp(self.updated_at),
            local: true,
        }
    }
}

/// The settings as stored: JSON texts and the classification's wire name.
struct SettingsColumns {
    appearance: String,
    model: String,
    classification: Option<&'static str>,
    capabilities: String,
}

impl SettingsColumns {
    fn of(settings: &BotSettings) -> Result<Self> {
        Ok(Self {
            appearance: serde_json::to_string(&settings.appearance)?,
            model: serde_json::to_string(&settings.model)?,
            classification: settings
                .classification
                .classification
                .map(DataClassification::as_str),
            capabilities: serde_json::to_string(&validation::capability_ids(
                &settings.capabilities,
            ))?,
        })
    }
}

pub(crate) fn bot_not_found(bot_id: &str) -> Error {
    Error::not_found(format!("Bot '{bot_id}' not found"))
}

fn stored_bot(db: &Connection, bot_id: &str) -> Result<Option<StoredBot>> {
    Ok(db
        .query_row(
            &format!("SELECT {BOT_COLUMNS} FROM bots WHERE id = ?1"),
            [bot_id],
            StoredBot::read,
        )
        .optional()?)
}

pub(crate) fn required_bot(db: &Connection, bot_id: &str) -> Result<StoredBot> {
    stored_bot(db, bot_id)?.ok_or_else(|| bot_not_found(bot_id))
}

pub(crate) fn bot_exists(db: &Connection, bot_id: &str) -> Result<bool> {
    Ok(db
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM bots WHERE id = ?1)")?
        .query_row([bot_id], |row| row.get(0))?)
}

fn require_bot(db: &Connection, bot_id: &str) -> Result<()> {
    if bot_exists(db, bot_id)? {
        Ok(())
    } else {
        Err(bot_not_found(bot_id))
    }
}

fn count(db: &Connection, sql: &str, bot_id: &str) -> Result<i64> {
    Ok(db.query_row(sql, [bot_id], |row| row.get(0))?)
}

fn new_bot_id() -> String {
    format!("bot_{}", create_id())
}

/// Room for one more bot: at most 50 of the person's own live on a device; Assistant and Flux
/// don't count.
fn admit_new_bot(db: &Connection) -> Result<()> {
    let held: i64 = db.query_row(
        "SELECT COUNT(*) FROM bots WHERE system IS NULL",
        [],
        |row| row.get(0),
    )?;
    if held >= MAX_LOCAL_BOTS {
        return Err(Error::new(
            codes::BOT_LIMIT_REACHED,
            format!(
                "You already have {MAX_LOCAL_BOTS} bots on this device; delete one before creating another"
            ),
        ));
    }
    Ok(())
}

const COPY_MARKER: &str = " copy";

/// "<name> copy", the name shortened at its end so the marker always fits, like the API's.
fn copy_name(name: &str) -> String {
    let room = NAME_MAX_CHARS - COPY_MARKER.chars().count();
    let base: String = name.chars().take(room).collect();
    format!("{}{COPY_MARKER}", base.trim_end())
}

/// A new row; `system` is set for Assistant and Flux only.
struct NewBot<'a> {
    id: &'a str,
    system: Option<BotSystemKind>,
    preset_id: Option<&'a str>,
    settings: &'a BotSettings,
}

fn insert_bot(db: &Connection, bot: NewBot<'_>, now: i64) -> Result<()> {
    let settings = bot.settings;
    let columns = SettingsColumns::of(settings)?;
    db.execute(
        "INSERT INTO bots (id, system, preset_id, name, description, instructions, appearance,
             model, embedding_model, classification, pii, capabilities, profile_id, created_at,
             updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
        params![
            bot.id,
            bot.system.map(BotSystemKind::as_str),
            bot.preset_id,
            settings.name,
            settings.description,
            settings.instructions,
            columns.appearance,
            columns.model,
            settings.embedding_model,
            columns.classification,
            settings.classification.pii,
            columns.capabilities,
            settings.profile_id,
            now,
        ],
    )?;
    Ok(())
}

/// `updated_at` moves strictly forward with every change, even two in one millisecond, so a
/// forget conditional on an exported `updated_at` sees each of them.
fn save_settings(db: &Connection, bot_id: &str, settings: &BotSettings, now: i64) -> Result<()> {
    let columns = SettingsColumns::of(settings)?;
    db.execute(
        "UPDATE bots SET name = ?2, description = ?3, instructions = ?4, appearance = ?5,
             model = ?6, embedding_model = ?7, classification = ?8, pii = ?9, capabilities = ?10,
             profile_id = ?11, updated_at = MAX(?12, updated_at + ?13)
         WHERE id = ?1",
        params![
            bot_id,
            settings.name,
            settings.description,
            settings.instructions,
            columns.appearance,
            columns.model,
            settings.embedding_model,
            columns.classification,
            settings.classification.pii,
            columns.capabilities,
            settings.profile_id,
            now,
            STEP_MICROS,
        ],
    )?;
    Ok(())
}

/// Rows for Assistant and Flux with their defaults, where none exist yet; whether it made any.
fn insert_built_ins(db: &Connection) -> Result<bool> {
    let now = now_micros();
    let mut inserted = false;
    for kind in BotSystemKind::ALL {
        if built_in_exists(db, kind)? {
            continue;
        }
        let settings = validation::built_in(kind);
        let bot = NewBot {
            id: &new_bot_id(),
            system: Some(kind),
            preset_id: None,
            settings: &settings,
        };
        insert_bot(db, bot, now)?;
        inserted = true;
    }
    Ok(inserted)
}

fn built_in_exists(db: &Connection, kind: BotSystemKind) -> Result<bool> {
    Ok(db
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM bots WHERE system = ?1)")?
        .query_row([kind.as_str()], |row| row.get(0))?)
}

/// The source's capabilities and what its kind always holds (Flux: `flowpilot`, also on a row
/// stored without it), in the settings' order.
fn copied_capabilities(source: &StoredBot) -> Vec<BotCapability> {
    let fixed = source
        .system
        .map_or(&[][..], BotSystemKind::fixed_capabilities);
    BotCapability::ALL
        .into_iter()
        .filter(|capability| source.settings.has(*capability) || fixed.contains(capability))
        .collect()
}

/// A built-in that joins a chat counts as changed from its defaults, so after a sign-in it is
/// among the bots to move even if the person never edited it. Whether its row changed.
pub(crate) fn mark_built_in_used(db: &Connection, bot_id: &str) -> Result<bool> {
    let changed = db
        .prepare_cached(
            "UPDATE bots SET updated_at = MAX(?2, updated_at + ?3)
             WHERE id = ?1 AND system IS NOT NULL AND updated_at = created_at",
        )?
        .execute(params![bot_id, now_micros(), STEP_MICROS])?;
    Ok(changed > 0)
}

/// Assistant and Flux stay; the person's edits of them can be undone instead.
fn refuse_built_in(bot: &StoredBot) -> Result<()> {
    match bot.system {
        Some(kind) => Err(Error::new(
            codes::BOT_SYSTEM_UNDELETABLE,
            format!("The built-in bot '{}' can't be deleted", kind.as_str()),
        )),
        None => Ok(()),
    }
}

/// A change of the bot's apps or secrets moves its `updated_at` like a settings change.
fn touch_bot(db: &Connection, bot_id: &str) -> Result<()> {
    db.prepare_cached("UPDATE bots SET updated_at = MAX(?2, updated_at + ?3) WHERE id = ?1")?
        .execute(params![bot_id, now_micros(), STEP_MICROS])?;
    Ok(())
}

fn stored_updated_at(db: &Connection, bot_id: &str) -> Result<Option<i64>> {
    Ok(db
        .prepare_cached("SELECT updated_at FROM bots WHERE id = ?1")?
        .query_row([bot_id], |row| row.get(0))
        .optional()?)
}

/// An exported `updated_at` (RFC 3339) as stored.
fn parse_updated_at(value: &str) -> Result<i64> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|at| at.timestamp_micros())
        .map_err(|_| {
            Error::invalid(format!(
                "expected_updated_at '{}' is not an RFC 3339 time",
                value.chars().take(80).collect::<String>()
            ))
        })
}

fn restrictions_of(mask: Option<i64>, scopes: &str) -> BotRestrictions {
    BotRestrictions::from_bits(mask, serde_json::from_str(scopes).unwrap_or_default())
}

fn app_not_attached(bot_id: &str, app_id: &str) -> Error {
    Error::new(
        codes::NOT_FOUND,
        format!("Bot '{bot_id}' doesn't use app '{app_id}'"),
    )
}

fn attached_restrictions(db: &Connection, bot_id: &str, app_id: &str) -> Result<BotRestrictions> {
    db.prepare_cached(
        "SELECT permission_mask, scopes FROM bot_apps WHERE bot_id = ?1 AND app_id = ?2",
    )?
    .query_row(params![bot_id, app_id], |row| {
        Ok(restrictions_of(row.get(0)?, &row.get::<_, String>(1)?))
    })
    .optional()?
    .ok_or_else(|| app_not_attached(bot_id, app_id))
}

fn attached_apps(db: &Connection, bot_id: &str) -> Result<Vec<BotApp>> {
    let mut statement = db.prepare_cached(
        "SELECT app_id, added_at, permission_mask, scopes FROM bot_apps WHERE bot_id = ?1 ORDER BY added_at, app_id",
    )?;
    let apps = statement
        .query_map([bot_id], |row| {
            Ok(BotApp {
                app_id: row.get("app_id")?,
                added_at: timestamp(row.get("added_at")?),
                owner_role: Some(OWNER_ROLE.to_string()),
                restrictions: restrictions_of(
                    row.get("permission_mask")?,
                    &row.get::<_, String>("scopes")?,
                ),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(apps)
}

fn secret_info(row: &Row<'_>) -> rusqlite::Result<BotSecretInfo> {
    Ok(BotSecretInfo {
        name: row.get("name")?,
        encryption: BotSecretEncryption::Device,
        description: row.get("description")?,
        created_at: timestamp(row.get("created_at")?),
        updated_at: timestamp(row.get("updated_at")?),
        last_used_at: row.get::<_, Option<i64>>("last_used_at")?.map(timestamp),
    })
}

const SECRET_INFO_COLUMNS: &str = "name, description, created_at, updated_at, last_used_at";

struct SealedSecret {
    name: String,
    ciphertext: Vec<u8>,
    description: Option<String>,
}

fn sealed_secrets(db: &Connection, bot_id: &str) -> Result<Vec<SealedSecret>> {
    let mut statement = db.prepare_cached(
        "SELECT name, ciphertext, description FROM bot_secrets WHERE bot_id = ?1 ORDER BY name",
    )?;
    let secrets = statement
        .query_map([bot_id], |row| {
            Ok(SealedSecret {
                name: row.get("name")?,
                ciphertext: row.get("ciphertext")?,
                description: row.get("description")?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(secrets)
}

/// One validated secret write: the sealed value and, unless omitted, the new description.
struct SecretWrite<'a> {
    bot_id: &'a str,
    name: &'a str,
    ciphertext: Vec<u8>,
    description: Option<Option<String>>,
}

/// Stores or replaces the secret; a new one counts toward [`MAX_SECRETS_PER_BOT`]. Returns its
/// metadata and whether an earlier value was overwritten.
fn store_secret(
    db: &Connection,
    write: SecretWrite<'_>,
    now: i64,
) -> Result<(BotSecretInfo, bool)> {
    let SecretWrite {
        bot_id,
        name,
        ciphertext,
        description,
    } = write;
    let replaced = db.execute(
        "UPDATE bot_secrets SET ciphertext = ?3, updated_at = ?4,
             description = CASE WHEN ?5 THEN ?6 ELSE description END
         WHERE bot_id = ?1 AND name = ?2",
        params![
            bot_id,
            name,
            ciphertext,
            now,
            description.is_some(),
            description.clone().flatten(),
        ],
    )?;
    if replaced == 0 {
        let held = count(
            db,
            "SELECT COUNT(*) FROM bot_secrets WHERE bot_id = ?1",
            bot_id,
        )?;
        if held >= MAX_SECRETS_PER_BOT {
            return Err(Error::new(
                codes::BOT_SECRET_LIMIT_REACHED,
                format!(
                    "Bot '{bot_id}' already has {MAX_SECRETS_PER_BOT} secrets; delete one before adding '{name}'"
                ),
            ));
        }
        db.execute(
            "INSERT INTO bot_secrets (bot_id, name, ciphertext, description, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![bot_id, name, ciphertext, description.flatten(), now],
        )?;
    }
    let info = db.query_row(
        &format!("SELECT {SECRET_INFO_COLUMNS} FROM bot_secrets WHERE bot_id = ?1 AND name = ?2"),
        params![bot_id, name],
        secret_info,
    )?;
    Ok((info, replaced > 0))
}

/// Why no key seals a new secret. While secrets are stored, a missing key is never replaced:
/// a second key would leave them sealed under the lost one for good.
fn sealing_refused(error: KeyError, stored: i64) -> Error {
    let message = match error {
        KeyError::Missing(reason) if stored > 0 => format!(
            "{reason}, so the secrets saved before ({stored} on this device) can't be opened; delete the old secrets first"
        ),
        KeyError::Missing(reason) | KeyError::Unavailable(reason) => reason,
    };
    Error::new(codes::SECRET_KEY, message)
}

/// Each secret with its value where this device's key opens it.
type Opened = Vec<(SealedSecret, Option<Zeroizing<String>>)>;

impl LocalBotStore {
    /// Makes the rows of Assistant and Flux with their defaults where they are missing, like the
    /// API's first list, so the person can edit them; a built-in a move took along comes back
    /// as new.
    pub fn ensure_built_ins(&self) -> Result<()> {
        let complete = self.read(|db| {
            for kind in BotSystemKind::ALL {
                if !built_in_exists(db, kind)? {
                    return Ok(false);
                }
            }
            Ok(true)
        })?;
        if complete {
            return Ok(());
        }
        self.write(|tx, changes| {
            if insert_built_ins(tx)? {
                changes.bots();
            }
            Ok(())
        })
    }

    /// Assistant and Flux (once [`Self::ensure_built_ins`] made them), then the person's own
    /// bots, newest first.
    pub fn list_bots(&self) -> Result<Vec<Bot>> {
        self.read(|db| {
            let mut statement = db.prepare_cached(&format!(
                "SELECT {BOT_COLUMNS} FROM bots
                 ORDER BY CASE system WHEN 'general' THEN 0 WHEN 'flux' THEN 1 ELSE 2 END,
                     created_at DESC, id DESC"
            ))?;
            let bots = statement
                .query_map([], StoredBot::read)?
                .map(|bot| bot.map(StoredBot::into_bot))
                .collect::<rusqlite::Result<_>>()?;
            Ok(bots)
        })
    }

    pub fn get_bot(&self, bot_id: &str) -> Result<Option<Bot>> {
        self.read(|db| Ok(stored_bot(db, bot_id)?.map(StoredBot::into_bot)))
    }

    /// The row of Assistant or Flux, `None` before [`Self::ensure_built_ins`] made it. A read:
    /// a run uses the person's edits of a built-in without making rows.
    pub fn built_in_bot(&self, kind: BotSystemKind) -> Result<Option<Bot>> {
        self.read(|db| {
            Ok(db
                .query_row(
                    &format!("SELECT {BOT_COLUMNS} FROM bots WHERE system = ?1"),
                    [kind.as_str()],
                    StoredBot::read,
                )
                .optional()?
                .map(StoredBot::into_bot))
        })
    }

    /// A new bot on this desktop, unless there already are 50.
    pub fn create_bot(&self, request: CreateBotRequest) -> Result<Bot> {
        let (preset_id, settings) = validation::create_settings(request)?;
        let bot_id = new_bot_id();
        self.write(|tx, changes| {
            admit_new_bot(tx)?;
            let bot = NewBot {
                id: &bot_id,
                system: None,
                preset_id: preset_id.as_deref(),
                settings: &settings,
            };
            insert_bot(tx, bot, now_micros())?;
            changes.bot(&bot_id);
            Ok(required_bot(tx, &bot_id)?.into_bot())
        })
    }

    /// A new bot of the person's own with the source's name (marked as a copy), look, texts,
    /// models, classification, capabilities (`flowpilot` included: a copy of Flux builds too),
    /// profile, preset and apps, like the API's duplicate. Its secrets, workspace, memory and
    /// chats stay with the source.
    pub fn clone_bot(&self, bot_id: &str) -> Result<Bot> {
        let copy_id = new_bot_id();
        self.write(|tx, changes| {
            let source = required_bot(tx, bot_id)?;
            admit_new_bot(tx)?;
            let mut settings = source.settings.clone();
            settings.name = Some(copy_name(source.display_name()));
            settings.capabilities = copied_capabilities(&source);
            let now = now_micros();
            let copy = NewBot {
                id: &copy_id,
                system: None,
                preset_id: source.preset_id.as_deref(),
                settings: &settings,
            };
            insert_bot(tx, copy, now)?;
            tx.execute(
                "INSERT INTO bot_apps (bot_id, app_id, added_at, permission_mask, scopes)
                 SELECT ?2, app_id, ?3, permission_mask, scopes FROM bot_apps WHERE bot_id = ?1",
                params![bot_id, copy_id, now],
            )?;
            changes.bot(&copy_id);
            Ok(required_bot(tx, &copy_id)?.into_bot())
        })
    }

    /// Merges `request` into the stored bot; omitted fields stay, `null` clears (a built-in's
    /// name too, which brings back its default).
    pub fn update_bot(&self, bot_id: &str, request: UpdateBotRequest) -> Result<Bot> {
        self.write(|tx, changes| {
            let current = required_bot(tx, bot_id)?;
            let next = validation::apply_patch(&current.settings, current.system, request)?;
            if next == current.settings {
                return Ok(current.into_bot());
            }
            save_settings(tx, bot_id, &next, now_micros())?;
            changes.bot(bot_id);
            Ok(required_bot(tx, bot_id)?.into_bot())
        })
    }

    /// Deletes the bot with its apps, secrets and direct conversations; it leaves its groups.
    /// Assistant and Flux can't be deleted. Returns the workspace app it had, which only the
    /// host can delete: hosts call [`Self::delete_bot_with_workspace`].
    pub(crate) fn delete_bot(&self, bot_id: &str) -> Result<Option<String>> {
        self.write(|tx, changes| {
            let bot = required_bot(tx, bot_id)?;
            refuse_built_in(&bot)?;
            inbox::retire_bot(tx, bot_id, changes)?;
            tx.execute("DELETE FROM bots WHERE id = ?1", [bot_id])?;
            changes.bot(bot_id);
            changes.scrub();
            Ok(bot.workspace_app_id)
        })
    }

    /// Fails like [`Self::delete_bot`] would, without deleting; otherwise the bot's workspace
    /// app, if it has one.
    pub(crate) fn deletable_bot(&self, bot_id: &str) -> Result<Option<String>> {
        self.read(|db| {
            let bot = required_bot(db, bot_id)?;
            refuse_built_in(&bot)?;
            Ok(bot.workspace_app_id)
        })
    }

    /// Removes the bot, its apps and secrets once it moved to an account; its conversations
    /// stay. With `expected_updated_at`, the `updated_at` of the bot that moved, a bot changed
    /// since stays and the call fails with [`codes::LOCAL_CHANGED`]. Forgetting a bot that is
    /// gone already changes nothing.
    pub fn forget_bot(&self, bot_id: &str, expected_updated_at: Option<&str>) -> Result<()> {
        let expected = expected_updated_at.map(parse_updated_at).transpose()?;
        self.write(|tx, changes| {
            let Some(updated_at) = stored_updated_at(tx, bot_id)? else {
                return Ok(());
            };
            if expected.is_some_and(|expected| expected != updated_at) {
                return Err(Error::local_changed(format!(
                    "Bot '{bot_id}' changed on this device after it was exported, so it stays here; move it again"
                )));
            }
            tx.execute("DELETE FROM bots WHERE id = ?1", [bot_id])?;
            changes.bot(bot_id);
            changes.scrub();
            Ok(())
        })
    }

    /// The bot's apps, oldest attachment first.
    pub fn bot_apps(&self, bot_id: &str) -> Result<Vec<BotApp>> {
        self.read(|db| {
            require_bot(db, bot_id)?;
            attached_apps(db, bot_id)
        })
    }

    /// Attaches the app unless it already is; a bot uses at most 200 apps.
    pub fn attach_app(&self, bot_id: &str, app_id: &str) -> Result<()> {
        validation::app_id(app_id)?;
        self.write(|tx, changes| {
            require_bot(tx, bot_id)?;
            let attached: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM bot_apps WHERE bot_id = ?1 AND app_id = ?2)",
                params![bot_id, app_id],
                |row| row.get(0),
            )?;
            if attached {
                return Ok(());
            }
            let held = count(tx, "SELECT COUNT(*) FROM bot_apps WHERE bot_id = ?1", bot_id)?;
            if held >= MAX_APPS_PER_BOT {
                return Err(Error::new(
                    codes::BOT_APP_LIMIT_REACHED,
                    format!(
                        "Bot '{bot_id}' already uses {MAX_APPS_PER_BOT} apps; detach one before attaching '{app_id}'"
                    ),
                ));
            }
            tx.execute(
                "INSERT INTO bot_apps (bot_id, app_id, added_at) VALUES (?1, ?2, ?3)",
                params![bot_id, app_id, now_micros()],
            )?;
            touch_bot(tx, bot_id)?;
            changes.bot(bot_id);
            Ok(())
        })
    }

    /// What the person lets the bot do in the attached app; it applies in every chat.
    pub fn app_restrictions(&self, bot_id: &str, app_id: &str) -> Result<BotRestrictions> {
        self.read(|db| {
            require_bot(db, bot_id)?;
            attached_restrictions(db, bot_id, app_id)
        })
    }

    pub fn set_app_restrictions(
        &self,
        bot_id: &str,
        app_id: &str,
        restrictions: BotRestrictions,
    ) -> Result<BotRestrictions> {
        restrictions.validate().map_err(Error::invalid)?;
        let scopes = serde_json::to_string(&restrictions.scopes)?;
        self.write(|tx, changes| {
            require_bot(tx, bot_id)?;
            let updated = tx.execute(
                "UPDATE bot_apps SET permission_mask = ?3, scopes = ?4 WHERE bot_id = ?1 AND app_id = ?2",
                params![bot_id, app_id, restrictions.permission_bits(), scopes],
            )?;
            if updated == 0 {
                return Err(app_not_attached(bot_id, app_id));
            }
            touch_bot(tx, bot_id)?;
            changes.bot(bot_id);
            Ok(restrictions.clone())
        })
    }

    pub fn detach_app(&self, bot_id: &str, app_id: &str) -> Result<()> {
        self.write(|tx, changes| {
            require_bot(tx, bot_id)?;
            let removed = tx.execute(
                "DELETE FROM bot_apps WHERE bot_id = ?1 AND app_id = ?2",
                params![bot_id, app_id],
            )?;
            if removed > 0 {
                touch_bot(tx, bot_id)?;
                changes.bot(bot_id);
                changes.scrub();
            }
            Ok(())
        })
    }

    /// Names and metadata of the bot's secrets, sorted by name; never their values.
    pub fn secrets(&self, bot_id: &str) -> Result<Vec<BotSecretInfo>> {
        self.read(|db| {
            require_bot(db, bot_id)?;
            let mut statement = db.prepare_cached(&format!(
                "SELECT {SECRET_INFO_COLUMNS} FROM bot_secrets WHERE bot_id = ?1 ORDER BY name"
            ))?;
            let secrets = statement
                .query_map([bot_id], secret_info)?
                .collect::<rusqlite::Result<_>>()?;
            Ok(secrets)
        })
    }

    /// Seals and stores `value` as secret `name`, replacing an earlier value. `description`:
    /// `None` keeps the current one, `Some(None)` or an empty text clears it. A key is only
    /// created while no secret is stored, so it is never replaced under sealed values.
    pub fn set_secret(
        &self,
        bot_id: &str,
        name: &str,
        value: &str,
        description: Option<Option<String>>,
    ) -> Result<BotSecretInfo> {
        validation::secret_name(name)?;
        validation::secret_value(name, value)?;
        let description = description
            .map(validation::secret_description)
            .transpose()?;
        let stored: i64 = self.read(|db| {
            require_bot(db, bot_id)?;
            Ok(db.query_row("SELECT COUNT(*) FROM bot_secrets", [], |row| row.get(0))?)
        })?;
        let ciphertext = {
            let key = self
                .secret_key(stored == 0)
                .map_err(|error| sealing_refused(error, stored))?;
            cipher::seal(&key, bot_id, name, value)?
        };
        self.write(|tx, changes| {
            require_bot(tx, bot_id)?;
            let write = SecretWrite {
                bot_id,
                name,
                ciphertext,
                description,
            };
            let (info, replaced) = store_secret(tx, write, now_micros())?;
            touch_bot(tx, bot_id)?;
            changes.bot(bot_id);
            if replaced {
                changes.scrub();
            }
            Ok(info)
        })
    }

    /// Deletes the secret; one that doesn't exist is already gone.
    pub fn delete_secret(&self, bot_id: &str, name: &str) -> Result<()> {
        self.write(|tx, changes| {
            require_bot(tx, bot_id)?;
            let removed = tx.execute(
                "DELETE FROM bot_secrets WHERE bot_id = ?1 AND name = ?2",
                params![bot_id, name],
            )?;
            if removed > 0 {
                touch_bot(tx, bot_id)?;
                changes.bot(bot_id);
                changes.scrub();
            }
            Ok(())
        })
    }

    /// Opens the secrets with this device's key; a value stays `None` where it doesn't open.
    /// A missing key opens none. A keychain that can't be asked now fails with `SECRET_KEY`,
    /// since asking again later may work. The key is only read when there are secrets.
    fn open_sealed(&self, bot_id: &str, sealed: Vec<SealedSecret>) -> Result<Opened> {
        if sealed.is_empty() {
            return Ok(Vec::new());
        }
        let key = match self.secret_key(false) {
            Ok(key) => Some(key),
            Err(KeyError::Missing(_)) => None,
            Err(KeyError::Unavailable(reason)) => {
                return Err(Error::new(codes::SECRET_KEY, reason));
            }
        };
        Ok(sealed
            .into_iter()
            .map(|secret| {
                let value = key.as_ref().and_then(|key| {
                    cipher::open(key, bot_id, &secret.name, &secret.ciphertext).ok()
                });
                (secret, value)
            })
            .collect())
    }

    /// The bot with its apps, secrets and conversations, for the move to an account. Secrets
    /// carry no values, only whether this device's key opens them.
    pub fn export_bot(&self, bot_id: &str) -> Result<LocalBotBundle> {
        let (bot, app_ids, sealed, conversations) = self.read(|db| {
            let bot = required_bot(db, bot_id)?.into_bot();
            let app_ids = attached_apps(db, bot_id)?
                .into_iter()
                .map(|app| app.app_id)
                .collect();
            Ok((
                bot,
                app_ids,
                sealed_secrets(db, bot_id)?,
                inbox::bot_conversations(db, bot_id)?,
            ))
        })?;
        let secrets = self
            .open_sealed(bot_id, sealed)?
            .into_iter()
            .map(|(secret, value)| ExportedSecret {
                name: secret.name,
                description: secret.description,
                readable: value.is_some(),
            })
            .collect();
        Ok(LocalBotBundle {
            bot,
            app_ids,
            secrets,
            conversations,
        })
    }

    /// The bot's secret values for the upload to its account, and the names of the secrets
    /// this device's key doesn't open.
    pub fn open_secrets(&self, bot_id: &str) -> Result<OpenedSecrets> {
        let sealed = self.read(|db| {
            require_bot(db, bot_id)?;
            sealed_secrets(db, bot_id)
        })?;
        let mut opened = OpenedSecrets::default();
        for (secret, value) in self.open_sealed(bot_id, sealed)? {
            match value {
                Some(value) => opened.readable.push(SecretValue {
                    name: secret.name,
                    value,
                    description: secret.description,
                }),
                None => opened.unreadable.push(secret.name),
            }
        }
        Ok(opened)
    }

    /// Points the bots that use profile `local_id` at `server_id` once the profile synced;
    /// returns how many moved.
    pub fn remap_profile(&self, local_id: &str, server_id: &str) -> Result<usize> {
        if local_id == server_id {
            return Ok(0);
        }
        self.write(|tx, changes| {
            let moved = tx.execute(
                "UPDATE bots SET profile_id = ?2, updated_at = MAX(?3, updated_at + ?4)
                 WHERE profile_id = ?1",
                params![local_id, server_id, now_micros(), STEP_MICROS],
            )?;
            if moved > 0 {
                changes.bots();
            }
            Ok(moved)
        })
    }
}
