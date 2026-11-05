//! Connectors of local bots: the MCP servers and OAuth accounts the person attached. The same
//! rules as the API's, except the URL may be http and point anywhere (this is the person's own
//! machine) and the account's sign-in stays in the desktop's native OAuth store, so no grant is
//! kept here.

use std::collections::BTreeMap;

use crate::{
    LocalBotStore,
    bots::{bot_exists, bot_not_found},
    error::{Error, Result},
    store::{now_micros, timestamp},
};
use flow_like_types::{
    bot_connector::{
        BotConnector, ConnectorAuth, ConnectorKind, ConnectorPolicy, ConnectorToolInfo,
        CreateBotConnector, MAX_CONNECTORS_PER_BOT, MAX_SCOPES, MAX_TOOLS_SNAPSHOT, MAX_URL_CHARS,
        UpdateBotConnector, validate_label, validate_tool_policies,
    },
    create_id,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

const COLUMNS: &str = "id, bot_id, kind, label, provider_id, url, auth, scopes, default_policy, tool_policies, tools_snapshot, updated_at, confirmed_hashes";

fn json_or_default<T: serde::de::DeserializeOwned + Default>(stored: &str) -> T {
    serde_json::from_str(stored).unwrap_or_default()
}

fn read_connector(row: &Row<'_>) -> rusqlite::Result<BotConnector> {
    let kind: String = row.get(2)?;
    let auth: String = row.get(6)?;
    let scopes: String = row.get(7)?;
    let policy: String = row.get(8)?;
    let policies: String = row.get(9)?;
    let tools: String = row.get(10)?;
    Ok(BotConnector {
        id: row.get(0)?,
        bot_id: row.get(1)?,
        kind: ConnectorKind::from_column(&kind).unwrap_or(ConnectorKind::Mcp),
        label: row.get(3)?,
        provider_id: row.get(4)?,
        url: row.get(5)?,
        auth: json_or_default::<ConnectorAuth>(&auth),
        has_grant: false,
        scopes: json_or_default::<Vec<String>>(&scopes),
        default_policy: ConnectorPolicy::from_column(&policy).unwrap_or(ConnectorPolicy::Ask),
        tool_policies: json_or_default::<BTreeMap<String, ConnectorPolicy>>(&policies),
        tools_snapshot: json_or_default::<Vec<ConnectorToolInfo>>(&tools),
        updated_at: timestamp(row.get(11)?).to_rfc3339(),
        confirmed_hashes: json_or_default::<BTreeMap<String, String>>(&row.get::<_, String>(12)?),
    })
}

fn require_bot(db: &Connection, bot_id: &str) -> Result<()> {
    if bot_exists(db, bot_id)? {
        Ok(())
    } else {
        Err(bot_not_found(bot_id))
    }
}

fn find(db: &Connection, bot_id: &str, connector_id: &str) -> Result<BotConnector> {
    db.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM bot_connectors WHERE id = ?1 AND bot_id = ?2"
    ))?
    .query_row(params![connector_id, bot_id], read_connector)
    .optional()?
    .ok_or_else(|| Error::not_found(format!("Connector '{connector_id}' not found")))
}

fn label_free(db: &Connection, bot_id: &str, label: &str, except: Option<&str>) -> Result<()> {
    let taken: i64 = db.query_row(
        "SELECT COUNT(*) FROM bot_connectors WHERE bot_id = ?1 AND lower(label) = lower(?2) AND id <> ?3",
        params![bot_id, label, except.unwrap_or_default()],
        |row| row.get(0),
    )?;
    if taken > 0 {
        return Err(Error::invalid(format!(
            "Bot '{bot_id}' already has a connector called '{label}'"
        )));
    }
    Ok(())
}

fn checked_url(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let invalid = |why: &str| Error::invalid(format!("The server URL {why}"));
    if raw.chars().count() > MAX_URL_CHARS {
        return Err(invalid("is too long"));
    }
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| invalid("must start with http:// or https://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if !matches!(scheme, "http" | "https") || authority.is_empty() || authority.contains('@') {
        return Err(invalid(
            "must be an http or https URL without a user name or password",
        ));
    }
    Ok(raw.to_string())
}

impl LocalBotStore {
    pub fn connectors(&self, bot_id: &str) -> Result<Vec<BotConnector>> {
        self.read(|db| {
            require_bot(db, bot_id)?;
            let mut statement = db.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM bot_connectors WHERE bot_id = ?1 ORDER BY created_at, id"
            ))?;
            let rows = statement
                .query_map([bot_id], read_connector)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn create_connector(
        &self,
        bot_id: &str,
        request: CreateBotConnector,
    ) -> Result<BotConnector> {
        let label = validate_label(&request.label).map_err(Error::invalid)?;
        if request.scopes.len() > MAX_SCOPES {
            return Err(Error::invalid(format!("At most {MAX_SCOPES} scopes")));
        }
        let (url, provider_id) = match (request.kind, &request.auth) {
            (ConnectorKind::Mcp, auth) => {
                let url = checked_url(
                    request
                        .url
                        .as_deref()
                        .ok_or_else(|| Error::invalid("An MCP server needs a URL"))?,
                )?;
                let provider = match auth {
                    ConnectorAuth::Oauth { provider_id } => Some(provider_id.clone()),
                    _ => None,
                };
                (Some(url), provider)
            }
            (ConnectorKind::Oauth, ConnectorAuth::Oauth { provider_id }) => {
                (None, Some(provider_id.clone()))
            }
            (ConnectorKind::Oauth, _) => {
                return Err(Error::invalid(
                    "An account needs a provider: use auth type oauth",
                ));
            }
        };
        if let ConnectorAuth::Secret { name } = &request.auth
            && !flow_like_types::bot_message::is_secret_name(name)
        {
            return Err(Error::invalid("The secret name is invalid"));
        }
        self.write(|tx, _changes| {
            require_bot(tx, bot_id)?;
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM bot_connectors WHERE bot_id = ?1",
                [bot_id],
                |row| row.get(0),
            )?;
            if count >= MAX_CONNECTORS_PER_BOT as i64 {
                return Err(Error::invalid(format!(
                    "Bot '{bot_id}' already has {MAX_CONNECTORS_PER_BOT} connectors"
                )));
            }
            label_free(tx, bot_id, &label, None)?;
            let id = format!("bc_{}", create_id());
            let now = now_micros();
            tx.execute(
                "INSERT INTO bot_connectors (id, bot_id, kind, label, provider_id, url, auth, scopes,
                     default_policy, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    id,
                    bot_id,
                    request.kind.column(),
                    label,
                    provider_id,
                    url,
                    serde_json::to_string(&request.auth).unwrap_or_default(),
                    serde_json::json!(request.scopes).to_string(),
                    request
                        .default_policy
                        .unwrap_or(ConnectorPolicy::Ask)
                        .column(),
                    now
                ],
            )?;
            find(tx, bot_id, &id)
        })
    }

    pub fn update_connector(
        &self,
        bot_id: &str,
        connector_id: &str,
        request: UpdateBotConnector,
    ) -> Result<BotConnector> {
        self.write(|tx, _changes| {
            let current = find(tx, bot_id, connector_id)?;
            let label = match &request.label {
                Some(label) => {
                    let label = validate_label(label).map_err(Error::invalid)?;
                    label_free(tx, bot_id, &label, Some(connector_id))?;
                    label
                }
                None => current.label.clone(),
            };
            let policies = match &request.tool_policies {
                Some(policies) => {
                    validate_tool_policies(policies).map_err(Error::invalid)?;
                    policies.clone()
                }
                None => current.tool_policies.clone(),
            };
            let mut hashes = current.confirmed_hashes.clone();
            let hash_of = |name: &str| {
                current
                    .tools_snapshot
                    .iter()
                    .find(|tool| tool.name == name)
                    .map(|tool| tool.schema_hash.clone())
            };
            if request.tool_policies.is_some() {
                for (name, policy) in &policies {
                    if current.tool_policies.get(name) != Some(policy)
                        && let Some(hash) = hash_of(name)
                    {
                        hashes.insert(name.clone(), hash);
                    }
                }
                hashes.retain(|name, _| policies.contains_key(name));
            }
            for name in &request.confirm_tools {
                if let Some(hash) = hash_of(name) {
                    hashes.insert(name.clone(), hash);
                }
            }
            let scopes = match &request.scopes {
                Some(scopes) if scopes.len() > MAX_SCOPES => {
                    return Err(Error::invalid(format!("At most {MAX_SCOPES} scopes")));
                }
                Some(scopes) => scopes.clone(),
                None => current.scopes.clone(),
            };
            tx.execute(
                "UPDATE bot_connectors SET label = ?1, default_policy = ?2, tool_policies = ?3,
                     scopes = ?4, updated_at = ?5, confirmed_hashes = ?7 WHERE id = ?6",
                params![
                    label,
                    request
                        .default_policy
                        .unwrap_or(current.default_policy)
                        .column(),
                    serde_json::json!(policies).to_string(),
                    serde_json::json!(scopes).to_string(),
                    now_micros(),
                    connector_id,
                    serde_json::json!(hashes).to_string()
                ],
            )?;
            find(tx, bot_id, connector_id)
        })
    }

    /// Stores the tool list an MCP refresh returned.
    pub fn set_connector_tools(
        &self,
        bot_id: &str,
        connector_id: &str,
        mut tools: Vec<ConnectorToolInfo>,
    ) -> Result<BotConnector> {
        tools.truncate(MAX_TOOLS_SNAPSHOT);
        self.write(|tx, _changes| {
            find(tx, bot_id, connector_id)?;
            tx.execute(
                "UPDATE bot_connectors SET tools_snapshot = ?1, updated_at = ?2 WHERE id = ?3",
                params![
                    serde_json::json!(tools).to_string(),
                    now_micros(),
                    connector_id
                ],
            )?;
            find(tx, bot_id, connector_id)
        })
    }

    pub fn delete_connector(&self, bot_id: &str, connector_id: &str) -> Result<()> {
        self.write(|tx, _changes| {
            find(tx, bot_id, connector_id)?;
            tx.execute(
                "DELETE FROM bot_connectors WHERE id = ?1 AND bot_id = ?2",
                params![connector_id, bot_id],
            )?;
            Ok(())
        })
    }
}
