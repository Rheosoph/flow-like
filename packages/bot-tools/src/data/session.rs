use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flow_like_storage::contracts::column_metadata::ColumnReader;
use flow_like_storage::lancedb::Connection;

use crate::env::{Scope, ToolEnv};
use crate::error::{ToolError, ToolResult};

pub const WORKSPACE_SCHEMA: &str = "ws";
const SESSION_NAME: &str = "data.apps";
const SESSION_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub struct OpenApp {
    pub connection: Connection,
    pub reader: ColumnReader,
    pub writable: bool,
}

/// The table handles a conversation has opened, kept in the host's session cache.
#[derive(Default)]
pub struct DataSession {
    apps: Mutex<HashMap<String, OpenApp>>,
}

fn alias_for(app_id: &str) -> String {
    let id: String = app_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect::<String>()
        .to_lowercase();
    format!("app_{id}")
}

fn sanitize_alias(alias: &str) -> String {
    let clean: String = alias
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if clean.starts_with(|c: char| c.is_ascii_lowercase()) {
        clean
    } else {
        format!("a_{clean}")
    }
}

/// Each app the bot was given as a SQL schema name, unique and never `ws`.
pub fn app_aliases(scope: &Scope) -> Vec<(String, String)> {
    let mut taken = vec![WORKSPACE_SCHEMA.to_string()];
    let mut aliases = Vec::new();
    for grant in &scope.apps {
        if scope.is_workspace(&grant.app_id) {
            continue;
        }
        let base = grant
            .alias
            .as_deref()
            .map(sanitize_alias)
            .unwrap_or_else(|| alias_for(&grant.app_id));
        let mut alias = base.clone();
        let mut suffix = 2;
        while taken.contains(&alias) {
            alias = format!("{base}_{suffix}");
            suffix += 1;
        }
        taken.push(alias.clone());
        aliases.push((alias, grant.app_id.clone()));
    }
    aliases
}

pub fn session_of(env: &dyn ToolEnv) -> Arc<DataSession> {
    let Some(cache) = env.sessions() else {
        return Arc::new(DataSession::default());
    };
    if let Some(found) = cache
        .get(SESSION_NAME)
        .and_then(|value| value.downcast::<DataSession>().ok())
    {
        cache.put(SESSION_NAME, found.clone(), SESSION_TTL);
        return found;
    }
    let fresh = Arc::new(DataSession::default());
    cache.put(SESSION_NAME, fresh.clone(), SESSION_TTL);
    fresh
}

impl DataSession {
    /// The tables of the workspace or of an app the bot was given.
    pub async fn open(&self, env: &dyn ToolEnv, app_id: &str) -> ToolResult<OpenApp> {
        if !env.scope().can_read_tables(app_id) {
            return Err(ToolError::refused(format!(
                "App '{app_id}' is not one of the bot's apps, so its tables cannot be read"
            )));
        }
        if let Some(open) = self
            .apps
            .lock()
            .ok()
            .and_then(|apps| apps.get(app_id).cloned())
        {
            return Ok(open);
        }
        let tables = env.app_tables(app_id).await?;
        let open = OpenApp {
            connection: tables.connection,
            reader: tables.reader,
            writable: tables.writable,
        };
        if let Ok(mut apps) = self.apps.lock() {
            apps.insert(app_id.to_string(), open.clone());
        }
        Ok(open)
    }
}

/// Which source a name or id refers to.
pub enum Source {
    Workspace,
    App { alias: String, app_id: String },
}

pub fn resolve_source(scope: &Scope, source: &str) -> ToolResult<Source> {
    let source = source.trim();
    if source.eq_ignore_ascii_case(WORKSPACE_SCHEMA) || scope.is_workspace(source) {
        return Ok(Source::Workspace);
    }
    let aliases = app_aliases(scope);
    if let Some((alias, app_id)) = aliases
        .iter()
        .find(|(alias, app_id)| alias.eq_ignore_ascii_case(source) || app_id == source)
    {
        return Ok(Source::App {
            alias: alias.clone(),
            app_id: app_id.clone(),
        });
    }
    Ok(Source::App {
        alias: source.to_string(),
        app_id: source.to_string(),
    })
}
