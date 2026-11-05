//! Skills of local bots. The same rules as the API's: every text is checked, a skill the bot
//! proposed or the person imported starts pending, and only an active skill is visible to the
//! bot (`active_skill_index`, `skill_by_name` for the tools).

use crate::{
    LocalBotStore,
    bots::{bot_exists, bot_not_found},
    error::{Error, Result},
    store::{now_micros, timestamp},
};
use flow_like_types::{
    bot_skill::{
        self as contract, BotSkill, CreateBotSkill, MAX_SKILLS_PER_BOT, SkillIndexEntry,
        SkillSource, SkillStatus, UpdateBotSkill,
    },
    create_id,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

const SKILLS_CAPABILITY: &str = "skills";
const MAX_PENDING_PROPOSALS: i64 = 20;
const COLUMNS: &str = "id, bot_id, name, description, body, source, status, tools, updated_at";

/// How a new skill gets in.
pub struct SkillOrigin {
    pub source: SkillSource,
    pub status: SkillStatus,
}

impl SkillOrigin {
    pub fn owner() -> Self {
        Self {
            source: SkillSource::Owner,
            status: SkillStatus::Active,
        }
    }

    pub fn proposed_by_bot() -> Self {
        Self {
            source: SkillSource::BotProposed,
            status: SkillStatus::Pending,
        }
    }

    pub fn import() -> Self {
        Self {
            source: SkillSource::Import,
            status: SkillStatus::Pending,
        }
    }
}

fn read_skill(row: &Row<'_>) -> rusqlite::Result<BotSkill> {
    let source: String = row.get(5)?;
    let status: String = row.get(6)?;
    let tools: String = row.get(7)?;
    Ok(BotSkill {
        id: row.get(0)?,
        bot_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        body: row.get(4)?,
        source: SkillSource::parse(&source).unwrap_or(SkillSource::Import),
        status: SkillStatus::parse(&status).unwrap_or(SkillStatus::Disabled),
        tools: serde_json::from_str(&tools).unwrap_or_default(),
        updated_at: timestamp(row.get(8)?).to_rfc3339(),
    })
}

fn holds_capability(db: &Connection, bot_id: &str) -> Result<bool> {
    let stored: Option<String> = db
        .prepare_cached("SELECT capabilities FROM bots WHERE id = ?1")?
        .query_row([bot_id], |row| row.get(0))
        .optional()?;
    let ids: Vec<String> = stored
        .and_then(|stored| serde_json::from_str(&stored).ok())
        .unwrap_or_default();
    Ok(ids.iter().any(|id| id == SKILLS_CAPABILITY))
}

fn skill_not_found(skill_id: &str) -> Error {
    Error::not_found(format!("Skill '{skill_id}' not found"))
}

fn find(db: &Connection, bot_id: &str, skill_id: &str) -> Result<BotSkill> {
    db.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM skills WHERE id = ?1 AND bot_id = ?2"
    ))?
    .query_row(params![skill_id, bot_id], read_skill)
    .optional()?
    .ok_or_else(|| skill_not_found(skill_id))
}

fn find_named(db: &Connection, bot_id: &str, name: &str) -> Result<Option<BotSkill>> {
    Ok(db
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM skills WHERE bot_id = ?1 AND name = ?2"
        ))?
        .query_row(params![bot_id, name], read_skill)
        .optional()?)
}

fn require_bot(db: &Connection, bot_id: &str) -> Result<()> {
    if bot_exists(db, bot_id)? {
        Ok(())
    } else {
        Err(bot_not_found(bot_id))
    }
}

fn refuse_package_edit(skill: &BotSkill) -> Result<()> {
    if skill.source.package_id().is_some() {
        return Err(Error::invalid(format!(
            "Skill '{}' comes from a package; update the package instead of editing it",
            skill.name
        )));
    }
    Ok(())
}

impl LocalBotStore {
    /// The bot's skills with their status, by name.
    pub fn skills(&self, bot_id: &str) -> Result<Vec<BotSkill>> {
        self.read(|db| {
            require_bot(db, bot_id)?;
            let mut statement = db.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM skills WHERE bot_id = ?1 ORDER BY name"
            ))?;
            let rows = statement
                .query_map([bot_id], read_skill)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// What the bot's prompt names: its active skills.
    pub fn active_skill_index(&self, bot_id: &str) -> Result<Vec<SkillIndexEntry>> {
        self.read(|db| {
            let mut statement = db.prepare_cached(
                "SELECT name, description FROM skills WHERE bot_id = ?1 AND status = 'ACTIVE' ORDER BY name",
            )?;
            let rows = statement
                .query_map([bot_id], |row| {
                    Ok(SkillIndexEntry {
                        name: row.get(0)?,
                        description: row.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// The bot's skill of this name, whatever its status.
    pub fn skill_by_name(&self, bot_id: &str, name: &str) -> Result<Option<BotSkill>> {
        self.read(|db| find_named(db, bot_id, name))
    }

    pub fn create_skill(
        &self,
        bot_id: &str,
        request: CreateBotSkill,
        origin: SkillOrigin,
    ) -> Result<BotSkill> {
        self.write(|tx, _changes| {
            require_bot(tx, bot_id)?;
            if !holds_capability(tx, bot_id)? {
                return Err(Error::invalid(format!(
                    "Bot '{bot_id}' doesn't have the skills capability switched on"
                )));
            }
            let (name, description, body) =
                contract::check_fields(&request.name, &request.description, &request.body)
                    .map_err(Error::invalid)?;
            let tools = contract::check_tools(&request.tools).map_err(Error::invalid)?;
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM skills WHERE bot_id = ?1",
                [bot_id],
                |row| row.get(0),
            )?;
            if count >= MAX_SKILLS_PER_BOT as i64 {
                return Err(Error::invalid(format!(
                    "Bot '{bot_id}' already has {MAX_SKILLS_PER_BOT} skills; delete one first"
                )));
            }
            if origin.source == SkillSource::BotProposed {
                let pending: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM skills WHERE bot_id = ?1 AND status = 'PENDING'",
                    [bot_id],
                    |row| row.get(0),
                )?;
                if pending >= MAX_PENDING_PROPOSALS {
                    return Err(Error::invalid(format!(
                        "Bot '{bot_id}' already has {MAX_PENDING_PROPOSALS} skills waiting for approval"
                    )));
                }
            }
            if find_named(tx, bot_id, &name)?.is_some() {
                return Err(Error::invalid(format!(
                    "Bot '{bot_id}' already has a skill named '{name}'"
                )));
            }
            let id = format!("skill_{}", create_id());
            let now = now_micros();
            tx.execute(
                "INSERT INTO skills (id, bot_id, name, description, body, source, status, tools,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![
                    id,
                    bot_id,
                    name,
                    description,
                    body,
                    origin.source.stored(),
                    origin.status.stored(),
                    serde_json::json!(tools).to_string(),
                    now
                ],
            )?;
            find(tx, bot_id, &id)
        })
    }

    /// A skill read from a zip: new, or replacing an earlier one of the same name, which goes
    /// back to pending because its text changed.
    pub fn reimport_skill(&self, bot_id: &str, request: CreateBotSkill) -> Result<BotSkill> {
        let existing = self.skill_by_name(bot_id, request.name.trim())?;
        let Some(existing) = existing else {
            return self.create_skill(bot_id, request, SkillOrigin::import());
        };
        refuse_package_edit(&existing)?;
        let (_, description, body) =
            contract::check_fields(&request.name, &request.description, &request.body)
                .map_err(Error::invalid)?;
        self.write(|tx, _changes| {
            tx.execute(
                "UPDATE skills SET description = ?1, body = ?2, source = ?3, status = 'PENDING',
                     updated_at = ?4 WHERE id = ?5",
                params![description, body, SkillSource::Import.stored(), now_micros(), existing.id],
            )?;
            find(tx, bot_id, &existing.id)
        })
    }

    pub fn update_skill(
        &self,
        bot_id: &str,
        skill_id: &str,
        request: UpdateBotSkill,
    ) -> Result<BotSkill> {
        self.write(|tx, _changes| {
            let skill = find(tx, bot_id, skill_id)?;
            refuse_package_edit(&skill)?;
            let (_, description, body) = contract::check_fields(
                &skill.name,
                request.description.as_deref().unwrap_or(&skill.description),
                request.body.as_deref().unwrap_or(&skill.body),
            )
            .map_err(Error::invalid)?;
            let tools = match &request.tools {
                Some(tools) => contract::check_tools(tools).map_err(Error::invalid)?,
                None => skill.tools.clone(),
            };
            tx.execute(
                "UPDATE skills SET description = ?1, body = ?2, tools = ?3, updated_at = ?4
                 WHERE id = ?5",
                params![
                    description,
                    body,
                    serde_json::json!(tools).to_string(),
                    now_micros(),
                    skill_id
                ],
            )?;
            find(tx, bot_id, skill_id)
        })
    }

    /// Approving re-reads the stored text against the skill rules before it goes live.
    pub fn set_skill_status(
        &self,
        bot_id: &str,
        skill_id: &str,
        status: SkillStatus,
    ) -> Result<BotSkill> {
        self.write(|tx, _changes| {
            let skill = find(tx, bot_id, skill_id)?;
            if status == SkillStatus::Active {
                contract::check_fields(&skill.name, &skill.description, &skill.body)
                    .map_err(Error::invalid)?;
            }
            tx.execute(
                "UPDATE skills SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.stored(), now_micros(), skill_id],
            )?;
            find(tx, bot_id, skill_id)
        })
    }

    /// Deletes the skill and returns it, so the caller can remove the files it brought.
    pub fn delete_skill(&self, bot_id: &str, skill_id: &str) -> Result<BotSkill> {
        self.write(|tx, _changes| {
            let skill = find(tx, bot_id, skill_id)?;
            tx.execute("DELETE FROM skills WHERE id = ?1", [skill_id])?;
            Ok(skill)
        })
    }

    /// Saves the skills of a package as pending: refused when the package needs capabilities
    /// the bot lacks (`uncovered_tags`), or a skill of that name came from elsewhere. A skill
    /// the package brought before is replaced and goes back to pending. `installed_here` marks
    /// a package this device installed for the skills, so the last skill's removal can undo it.
    pub fn install_package_skills(
        &self,
        bot_id: &str,
        package_id: &str,
        version: &str,
        entries: Vec<CreateBotSkill>,
        tags: &[String],
        installed_here: bool,
    ) -> Result<Vec<BotSkill>> {
        self.write(|tx, _changes| {
            require_bot(tx, bot_id)?;
            let stored: Option<String> = tx
                .query_row("SELECT capabilities FROM bots WHERE id = ?1", [bot_id], |row| row.get(0))
                .optional()?;
            let held: Vec<String> = stored
                .and_then(|stored| serde_json::from_str(&stored).ok())
                .unwrap_or_default();
            if !held.iter().any(|id| id == SKILLS_CAPABILITY) {
                return Err(Error::invalid(format!(
                    "Bot '{bot_id}' doesn't have the skills capability switched on"
                )));
            }
            let missing = contract::uncovered_tags(tags, &held);
            if !missing.is_empty() {
                return Err(Error::invalid(format!(
                    "'{package_id}' needs what bot '{bot_id}' does not have switched on: {}",
                    missing.join(", ")
                )));
            }
            if entries.is_empty() {
                return Err(Error::invalid(format!(
                    "Version '{version}' of '{package_id}' ships no skills"
                )));
            }
            let source = SkillSource::Package {
                id: package_id.to_string(),
                version: version.to_string(),
            };
            let now = now_micros();
            let mut saved = Vec::with_capacity(entries.len());
            for entry in entries {
                let (name, description, body) =
                    contract::check_fields(&entry.name, &entry.description, &entry.body)
                        .map_err(Error::invalid)?;
                let tools = contract::check_tools(&entry.tools).map_err(Error::invalid)?;
                let tools_json = serde_json::json!(tools).to_string();
                match find_named(tx, bot_id, &name)? {
                    Some(existing) if existing.source.package_id() == Some(package_id) => {
                        tx.execute(
                            "UPDATE skills SET description = ?1, body = ?2, source = ?3,
                                 status = 'PENDING', tools = ?4, updated_at = ?5 WHERE id = ?6",
                            params![description, body, source.stored(), tools_json, now, existing.id],
                        )?;
                        saved.push(find(tx, bot_id, &existing.id)?);
                    }
                    Some(_) => {
                        return Err(Error::invalid(format!(
                            "Bot '{bot_id}' already has a skill named '{name}'"
                        )));
                    }
                    None => {
                        let id = format!("skill_{}", create_id());
                        tx.execute(
                            "INSERT INTO skills (id, bot_id, name, description, body, source, status,
                                 tools, created_at, updated_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'PENDING', ?7, ?8, ?8)",
                            params![id, bot_id, name, description, body, source.stored(), tools_json, now],
                        )?;
                        saved.push(find(tx, bot_id, &id)?);
                    }
                }
            }
            if installed_here {
                tx.execute(
                    "INSERT OR IGNORE INTO skill_package_installs (package_id, installed_at) VALUES (?1, ?2)",
                    params![package_id, now],
                )?;
            }
            Ok(saved)
        })
    }

    /// Whether the package is no longer used by any skill of any bot and this device installed
    /// it for skills: the caller uninstalls it, and the mark is dropped.
    pub fn release_package(&self, package_id: &str) -> Result<bool> {
        self.write(|tx, _changes| {
            let used: i64 = tx.query_row(
                "SELECT COUNT(*) FROM skills WHERE source LIKE ?1",
                [format!("package:{package_id}@%")],
                |row| row.get(0),
            )?;
            if used > 0 {
                return Ok(false);
            }
            let removed = tx.execute(
                "DELETE FROM skill_package_installs WHERE package_id = ?1",
                [package_id],
            )?;
            Ok(removed > 0)
        })
    }
}
