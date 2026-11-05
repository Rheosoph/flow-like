//! The bot's secrets as the model sees them: names and descriptions to write as
//! `{{secret:NAME}}`, and a request to the owner for one it lacks. Values never reach the model:
//! the owner types them into a masked prompt that stores them in the bot's write-only store, and
//! hosts fill references in only where a tool hands the value to a page, a keyboard or a process.

use std::collections::BTreeSet;

use flow_like_types::bot_message::{MAX_SUMMARY_CHARS, is_secret_name};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult};
use crate::outcome::ToolOutput;
use crate::secret_text::{placeholder, secret_names};
use crate::spec::{Approval, ToolSpec, schema_of};

pub const SECRET_LIST: &str = "secret_list";
pub const SECRET_REQUEST: &str = flow_like_types::bot_message::SECRET_REQUEST_TOOL;

/// The tools that take a secret by name; the secret tools are offered beside them.
pub const SECRET_USERS: &[&str] = &[
    "shell_run",
    "browser_act",
    "computer_act",
    "data_sources",
    "web_request",
];

/// A stored secret as the model may see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SecretListArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SecretRequestArgs {
    /// The secret's name: A-Z first, then A-Z, 0-9 or _, e.g. GITHUB_TOKEN.
    pub name: String,
    /// One line the owner reads before typing the secret in: what it is and what you need it for.
    pub description: String,
    /// The stored value was rejected (expired, revoked or wrong); ask the owner for a new one.
    #[serde(default)]
    pub replace: bool,
}

const USAGE: &str = "Write {{secret:NAME}} where a tool takes a secret (the text of browser_act or computer_act, the secrets list of shell_run, a data connection's secret, a header or the URL of web_request); the host fills the value in and you never see it.";

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: SECRET_LIST,
            description: "List the names and descriptions of the secrets your owner stored for you (API keys, passwords, tokens). Values are never shown.",
            capability: None,
            module: "secrets",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<SecretListArgs>(),
        },
        ToolSpec {
            name: SECRET_REQUEST,
            description: "Ask your owner for a secret you need and don't have (call secret_list first), or for a new value when a stored one was rejected (replace: true). The owner types it into a masked field that stores it; you only get its {{secret:NAME}} reference. Never ask for a secret in plain chat.",
            capability: None,
            module: "secrets",
            approval: Approval::Ask,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<SecretRequestArgs>(),
        },
    ]
}

pub async fn secret_list(env: &dyn ToolEnv, _: SecretListArgs) -> ToolResult<ToolOutput> {
    let secrets = env.secrets().await?;
    Ok(ToolOutput::new(json!({
        "secrets": secrets,
        "usage": USAGE,
    })))
}

/// The secrets one call reads: `{{secret:NAME}}` in any text, the names `shell_run` passes and
/// the secret a data connection logs in with. A host that runs the call elsewhere sends only these.
pub fn referenced_secrets(args: &Value) -> BTreeSet<String> {
    fn collect(value: &Value, names: &mut BTreeSet<String>) {
        match value {
            Value::String(text) => names.extend(secret_names(text)),
            Value::Array(items) => items.iter().for_each(|item| collect(item, names)),
            Value::Object(fields) => fields.values().for_each(|field| collect(field, names)),
            _ => {}
        }
    }
    let mut names = BTreeSet::new();
    collect(args, &mut names);
    if let Some(listed) = args.get("secrets").and_then(Value::as_array) {
        names.extend(listed.iter().filter_map(Value::as_str).map(str::to_string));
    }
    if let Some(secret) = args
        .pointer("/attach/connection/secret")
        .and_then(Value::as_str)
    {
        names.insert(secret.to_string());
    }
    names
}

/// Checks the arguments a `secret_request` card is built from.
pub fn checked_request(args: &SecretRequestArgs) -> ToolResult<(String, String)> {
    let name = args.name.trim();
    if !is_secret_name(name) {
        return Err(ToolError::invalid(format!(
            "Secret name '{name}' must start with A-Z and continue with up to 63 of A-Z, 0-9 or _"
        )));
    }
    let description = args.description.trim();
    let chars = description.chars().count();
    if chars == 0 || chars > MAX_SUMMARY_CHARS || description.chars().any(char::is_control) {
        return Err(ToolError::invalid(format!(
            "'description' is one line of 1 to {MAX_SUMMARY_CHARS} characters"
        )));
    }
    Ok((name.to_string(), description.to_string()))
}

/// Runs once the owner answered the request: the secret is stored now, or they stored nothing.
pub async fn secret_request(env: &dyn ToolEnv, args: SecretRequestArgs) -> ToolResult<ToolOutput> {
    let (name, _) = checked_request(&args)?;
    let stored = env
        .secrets()
        .await?
        .into_iter()
        .any(|secret| secret.name == name);
    if !stored {
        return Err(ToolError::not_found(format!(
            "The owner answered but stored no secret named '{name}'. Don't ask again in this turn; tell them what you could not do."
        )));
    }
    Ok(ToolOutput::new(json!({
        "name": name,
        "reference": placeholder(&name),
        "usage": USAGE,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeEnv;

    fn request(name: &str, description: &str) -> SecretRequestArgs {
        SecretRequestArgs {
            name: name.into(),
            description: description.into(),
            replace: false,
        }
    }

    #[tokio::test]
    async fn the_list_names_secrets_and_never_values() {
        let env = FakeEnv::new().await;
        env.set_secret("GITHUB_TOKEN", "ghp_value");
        let output = secret_list(&env, SecretListArgs {}).await.unwrap();
        let text = output.result.to_string();
        assert!(text.contains("GITHUB_TOKEN"));
        assert!(!text.contains("ghp_value"));
    }

    #[tokio::test]
    async fn an_answered_request_returns_the_reference_only() {
        let env = FakeEnv::new().await;
        env.set_secret("API_KEY", "sk-live");
        let output = secret_request(&env, request("API_KEY", "Weather API"))
            .await
            .unwrap();
        assert_eq!(output.result["reference"], "{{secret:API_KEY}}");
        assert!(!output.result.to_string().contains("sk-live"));
    }

    #[tokio::test]
    async fn an_answer_without_a_stored_secret_is_not_found() {
        let env = FakeEnv::new().await;
        let error = secret_request(&env, request("API_KEY", "Weather API"))
            .await
            .unwrap_err();
        assert_eq!(error.code, crate::error::code::NOT_FOUND);
    }

    #[test]
    fn a_call_references_only_the_secrets_its_arguments_name() {
        let args = json!({
            "steps": [{ "text": "{{secret:LOGIN}} and {{secret:PIN}}" }],
            "secrets": ["API_KEY"],
            "attach": { "connection": { "secret": "PG_SALES", "host": "db" } },
            "note": "no secret here",
        });
        let names: Vec<String> = referenced_secrets(&args).into_iter().collect();
        assert_eq!(names, ["API_KEY", "LOGIN", "PG_SALES", "PIN"]);
        assert!(referenced_secrets(&json!({ "path": "a.txt" })).is_empty());
    }

    #[test]
    fn requests_keep_the_store_rule_and_one_line_descriptions() {
        assert!(checked_request(&request("API_KEY", "Weather API")).is_ok());
        for (name, description) in [
            ("api_key", "x"),
            ("API KEY", "x"),
            ("API_KEY", ""),
            ("API_KEY", "two\nlines"),
        ] {
            assert!(
                checked_request(&request(name, description)).is_err(),
                "{name}"
            );
        }
        let long = "d".repeat(MAX_SUMMARY_CHARS + 1);
        assert!(checked_request(&request("API_KEY", &long)).is_err());
    }
}
