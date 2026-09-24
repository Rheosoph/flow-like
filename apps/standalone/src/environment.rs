use crate::{
    config::{PlacementConfig, ProjectSource},
    runtime, secrets, vault,
};
use anyhow::{Context, Result, ensure};
use flow_like_runtime::{app::App, flow::variable::Variable};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};
use zeroize::Zeroizing;

struct Definition {
    variable: Variable,
    refs: HashMap<String, String>,
    default: Option<Value>,
}

/// Hex encoding preserves the full placement and variable IDs without name collisions.
fn key(placement: &str, variable: &str) -> String {
    fn hex(value: &str) -> String {
        value
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect()
    }
    format!("FLOW_LIKE_VAR_{}_{}", hex(placement), hex(variable))
}

async fn definitions(config: &PlacementConfig) -> Result<BTreeMap<String, Definition>> {
    config.validate()?;
    ensure!(
        config.source == ProjectSource::Offline,
        "Local environment export requires an offline project snapshot"
    );
    let state = runtime::inspection_state(&config.project_path, None).await?;
    let app = App::load(config.project_id.clone(), state).await?;
    ensure!(
        app.id == config.project_id
            && matches!(
                app.visibility,
                flow_like_runtime::app::AppVisibility::Offline
            ),
        "Environment definitions require the matching offline project"
    );
    let mut definitions: BTreeMap<String, Definition> = BTreeMap::new();
    for binding in &config.events {
        let version = |v: [u32; 3]| (v[0], v[1], v[2]);
        let event = app
            .get_event(&binding.event_id, Some(version(binding.event_version)))
            .await?;
        ensure!(
            event.id == binding.event_id
                && event.event_version == version(binding.event_version)
                && event.board_version == Some(version(binding.board_version)),
            "Event does not match its pin"
        );
        let board = app
            .open_board_authoritative(event.board_id.clone(), Some(version(binding.board_version)))
            .await?;
        let board = board.lock().await;
        ensure!(
            board.id == event.board_id && board.version == version(binding.board_version),
            "Board does not match its pin"
        );
        for variable in board.variables.values().chain(
            board
                .layers
                .values()
                .flat_map(|layer| layer.variables.values()),
        ) {
            if !variable.exposed && !variable.runtime_configured {
                continue;
            }
            let default = if variable.secret {
                None
            } else {
                config.variables.get(&variable.id).cloned().or_else(|| {
                    event
                        .variables
                        .get(&variable.id)
                        .unwrap_or(variable)
                        .default_value
                        .as_ref()
                        .and_then(|bytes| serde_json::from_slice(bytes).ok())
                })
            };
            let name = key(&config.id, &variable.id);
            if let Some(previous) = definitions.get(&name) {
                ensure!(
                    previous.variable == *variable
                        && previous.refs == board.refs
                        && previous.default == default,
                    "Selected events disagree about an exposed variable; use separate placements"
                );
            } else {
                definitions.insert(
                    name,
                    Definition {
                        variable: variable.clone(),
                        refs: board.refs.clone(),
                        default,
                    },
                );
            }
        }
    }
    Ok(definitions)
}

pub async fn export(config: &PlacementConfig, output: &Path) -> Result<()> {
    let definitions = definitions(config).await?;
    let mut text = String::from(
        "# Placement workflow variables. Values contain single-quoted JSON.\n# Import with apply <manifest> --variables-env <this-file>.\n# Secret values are omitted; uncomment a key after supplying its value.\n",
    );
    for (key, definition) in definitions {
        let label: String = definition
            .variable
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        text.push_str(&format!(
            "\n# {} ({label}), {:?}/{:?}{}\n",
            serde_json::to_string(&definition.variable.id)?,
            definition.variable.data_type,
            definition.variable.value_type,
            if definition.variable.secret {
                ", secret"
            } else {
                ""
            }
        ));
        let valid_default = definition.default.as_ref().filter(|value| {
            runtime::validate_override(&definition.variable, &definition.refs, value).is_ok()
        });
        if let Some(default) = valid_default {
            text.push_str(&format!(
                "{key}='{}'\n",
                serde_json::to_string(default)?.replace('\'', "\\u0027")
            ));
        } else {
            text.push_str(&format!("# {key}='null'\n"));
        }
    }
    vault::write_new_private(output, text.as_bytes())
        .context("Create environment template without overwriting an existing file")
}

fn parse(bytes: &[u8]) -> Result<BTreeMap<String, Value>> {
    ensure!(
        bytes.len() <= 1024 * 1024,
        "Variable environment file exceeds 1 MiB"
    );
    let text = std::str::from_utf8(bytes).context("Variable environment file must use UTF-8")?;
    let mut result = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .context("Invalid variable environment assignment")?;
        let key = key.trim();
        ensure!(
            key.starts_with("FLOW_LIKE_VAR_")
                && key
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'),
            "Invalid variable environment key"
        );
        let value = value.trim();
        let value = if value.starts_with('\'') {
            value
                .strip_prefix('\'')
                .and_then(|v| v.strip_suffix('\''))
                .context("Invalid literal JSON quoting")?
        } else {
            value
        };
        // Parse literals directly. Dollar references never read the agent's process environment.
        let value = serde_json::from_str(value)
            .map_err(|_| anyhow::anyhow!("Variable environment values must contain JSON"))?;
        ensure!(
            result.insert(key.to_owned(), value).is_none(),
            "Repeated variable environment key"
        );
        ensure!(
            result.len() <= 1024,
            "Too many variable environment entries"
        );
    }
    Ok(result)
}

pub async fn apply(config: &mut PlacementConfig, path: &Path) -> Result<()> {
    let bytes = vault::read_private(path)?;
    let values = parse(&bytes)?;
    let definitions = definitions(config).await?;
    let mut updates = Vec::new();
    for (key, value) in values {
        let definition = definitions
            .get(&key)
            .context("Environment key does not match an exposed variable in this placement")?;
        ensure!(
            runtime::validate_override(&definition.variable, &definition.refs, &value).is_ok(),
            "Environment value does not satisfy its pinned variable schema"
        );
        let bytes = Zeroizing::new(serde_json::to_vec(&value)?);
        ensure!(
            !definition.variable.secret || bytes.len() <= 4096,
            "Variable secret exceeds its size limit"
        );
        updates.push((
            definition.variable.id.clone(),
            definition.variable.secret,
            value,
            bytes,
        ));
    }
    // Validate every value before publishing files. Unique names leave running revisions untouched.
    for (id, secret, value, bytes) in updates {
        if secret {
            let name = format!("env-{}", uuid::Uuid::new_v4());
            secrets::install(config, &name, &bytes)?;
            config.variables.remove(&id);
            config.secret_overrides.insert(id, name);
        } else {
            config.secret_overrides.remove(&id);
            config.variables.insert(id, value);
        }
    }
    config.validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_keys_do_not_collide_and_values_never_expand_environment() {
        assert_ne!(key("a-b", "v"), key("a_b", "v"));
        assert_ne!(key("a", "v"), key("b", "v"));
        let key = key("placement", "variable");
        let entries = parse(format!("{key}='\"${{HOME}}\"'\n").as_bytes()).unwrap();
        assert_eq!(entries[&key], "${HOME}");
        assert!(parse(format!("{key}=false\n{key}=true").as_bytes()).is_err());
        assert!(parse(b"AWS_SECRET_ACCESS_KEY='\"value\"'").is_err());
        let error = parse(format!("{key}=private-non-json-secret").as_bytes()).unwrap_err();
        assert!(!error.to_string().contains("private-non-json-secret"));
    }
}
