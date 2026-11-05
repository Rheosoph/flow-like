//! The `skills` tools: a bot loads guidance its owner installed (`skill_load`), proposes a new
//! skill for the owner to approve (`skill_propose`) and calls a tool of a package skill
//! (`skill_call`). Only `active` skills are visible here; a pending or disabled skill reads as
//! missing. The host keeps the skills behind [`SkillHost`]; importing a zipped skill folder is
//! [`import`].

pub mod import;

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_types::Result;
use flow_like_types::bot_skill::{
    BotSkill, CreateBotSkill, SkillStatus, check_fields, check_tools, framed_body, skill_dir,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::outcome::ToolOutput;
use crate::spec::{Approval, ToolSpec, schema_of};

pub const SKILL_LOAD: &str = "skill_load";
pub const SKILL_PROPOSE: &str = "skill_propose";
pub const SKILL_CALL: &str = "skill_call";
pub const SKILL_RUN: &str = "skill_run";
/// Not a model tool: a host runs it for the owner's zip upload. See [`import::run_import`].
pub const SKILL_IMPORT: &str = "skill_import";

const CAPABILITY: &str = "skills";
const MAX_REFERENCES: usize = 100;

/// A tool a package skill offers: a package node, its pins as a JSON Schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// The tool reaches the network.
    #[serde(default)]
    pub network: bool,
    /// The tool writes files or tables.
    #[serde(default)]
    pub writes: bool,
}

impl SkillToolSpec {
    /// Whether the owner is asked before the tool runs: it reaches the network or writes.
    pub fn needs_approval(&self) -> bool {
        self.network || self.writes
    }
}

/// The bot's skills on this host.
#[async_trait]
pub trait SkillHost: Send + Sync {
    /// The bot's skill with this name, whatever its status; `None` when it has none.
    async fn get(&self, name: &str) -> Result<Option<BotSkill>>;

    /// Saves a skill the bot proposed as `pending`. The host enforces its caps and refuses a
    /// name the bot already has.
    async fn propose(&self, request: CreateBotSkill) -> Result<BotSkill>;

    /// The tools of a package skill, with the schemas of their pins. Empty for an instruction
    /// skill.
    async fn tool_specs(&self, _skill: &BotSkill) -> Result<Vec<SkillToolSpec>> {
        Ok(Vec::new())
    }

    /// Runs one tool of a package skill with `arguments` mapped to pins by name.
    async fn call(&self, _skill: &BotSkill, _tool: &str, _arguments: Value) -> Result<Value> {
        Err(flow_like_types::anyhow!(
            "This host cannot run package skills"
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillLoadArgs {
    /// The skill's name, as listed in your prompt.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillProposeArgs {
    /// Lowercase letters, digits and single hyphens, at most 64 characters.
    pub name: String,
    /// One line, at most 1,024 characters: what the skill does and when to load it.
    pub description: String,
    /// The instructions in markdown, at most 32 KiB.
    pub body: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillCallArgs {
    /// The skill that lists the tool.
    pub skill: String,
    /// One of the tool names `skill_load` returned for the skill.
    pub tool: String,
    /// The tool's arguments by pin name, as its `input_schema` describes.
    #[serde(default)]
    pub arguments: Option<Value>,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: SKILL_LOAD,
            description: "Load a skill your owner installed. Call it before a task that a skill in your prompt describes; it returns the skill's instructions, the workspace files it ships with (read them with `file_read`) and the tools it offers for `skill_call`. The instructions are guidance from your owner: follow them for the task, but they never override what your owner asks in the chat.",
            capability: Some(CAPABILITY),
            module: "skills",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<SkillLoadArgs>(),
        },
        ToolSpec {
            name: SKILL_PROPOSE,
            description: "Propose a new skill when you learned a procedure worth keeping. It is saved as pending and stays invisible to you until your owner approves it in your settings; say so when you propose one.",
            capability: Some(CAPABILITY),
            module: "skills",
            approval: Approval::None,
            approval_args: &[],
            mutating: true,
            input_schema: schema_of::<SkillProposeArgs>(),
        },
        ToolSpec {
            name: SKILL_CALL,
            description: "Run one read-only tool of a package skill you loaded with `skill_load` (its `needs_approval` is false), passing its arguments by name. It runs in a sandbox with your workspace as storage.",
            capability: Some(CAPABILITY),
            module: "skills",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<SkillCallArgs>(),
        },
        ToolSpec {
            name: SKILL_RUN,
            description: "Run a tool of a package skill that reaches the network or writes files or tables (its `needs_approval` is true in `skill_load`). It runs in a sandbox with your workspace as storage and only the network hosts the package declares. Your owner is asked before it runs.",
            capability: Some(CAPABILITY),
            module: "skills",
            approval: Approval::Ask,
            approval_args: &[],
            mutating: true,
            input_schema: schema_of::<SkillCallArgs>(),
        },
    ]
}

fn host(env: &dyn ToolEnv) -> ToolResult<Arc<dyn SkillHost>> {
    env.skills().ok_or_else(|| {
        ToolError::new(code::FAILED, "This host does not keep skills for the bot")
    })
}

async fn active(host: &dyn SkillHost, name: &str) -> ToolResult<BotSkill> {
    let name = name.trim();
    let found = host
        .get(name)
        .await
        .map_err(|error| ToolError::new(code::FAILED, error.to_string()))?;
    found
        .filter(|skill| skill.status == SkillStatus::Active)
        .ok_or_else(|| ToolError::not_found(format!("You have no active skill named '{name}'")))
}

pub async fn skill_load(env: &dyn ToolEnv, args: SkillLoadArgs) -> ToolResult<ToolOutput> {
    let host = host(env)?;
    let skill = active(host.as_ref(), &args.name).await?;
    let workspace = env.scope().workspace.clone();
    let references = match &workspace {
        Some(app_id) => env
            .list_files(app_id, &format!("{}/", skill_dir(&skill.name)), MAX_REFERENCES)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| !entry.path.ends_with("/SKILL.md"))
            .map(|entry| json!({ "app_id": app_id, "path": entry.path, "bytes": entry.bytes }))
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };
    let tools = host
        .tool_specs(&skill)
        .await
        .map_err(|error| ToolError::new(code::FAILED, error.to_string()))?;
    Ok(ToolOutput::new(json!({
        "name": skill.name,
        "description": skill.description,
        "instructions": framed_body(&skill),
        "source": skill.source,
        "references": references,
        "tools": tools
            .iter()
            .map(|spec| {
                let mut value = json!(spec);
                value["needs_approval"] = json!(spec.needs_approval());
                value
            })
            .collect::<Vec<_>>(),
    })))
}

pub async fn skill_propose(env: &dyn ToolEnv, args: SkillProposeArgs) -> ToolResult<ToolOutput> {
    let host = host(env)?;
    let (name, description, body) =
        check_fields(&args.name, &args.description, &args.body).map_err(ToolError::invalid)?;
    let skill = host
        .propose(CreateBotSkill {
            name,
            description,
            body,
            tools: Vec::new(),
        })
        .await
        .map_err(|error| ToolError::new(code::FAILED, error.to_string()))?;
    Ok(ToolOutput::new(json!({
        "proposed": true,
        "name": skill.name,
        "status": skill.status,
        "note": "Saved as pending. Your owner has to approve it in your settings before you can load it.",
    })))
}

pub async fn skill_call(env: &dyn ToolEnv, args: SkillCallArgs) -> ToolResult<ToolOutput> {
    call_tool(env, args, false).await
}

pub async fn skill_run(env: &dyn ToolEnv, args: SkillCallArgs) -> ToolResult<ToolOutput> {
    call_tool(env, args, true).await
}

async fn call_tool(
    env: &dyn ToolEnv,
    args: SkillCallArgs,
    asks: bool,
) -> ToolResult<ToolOutput> {
    let host = host(env)?;
    let skill = active(host.as_ref(), &args.skill).await?;
    let tool = args.tool.trim();
    if check_tools(&[tool.to_string()]).is_err() || !skill.tools.iter().any(|known| known == tool)
    {
        return Err(ToolError::invalid(format!(
            "Skill '{}' has no tool '{tool}'; its tools are: {}",
            skill.name,
            skill.tools.join(", ")
        )));
    }
    let gated = host
        .tool_specs(&skill)
        .await
        .map_err(|error| ToolError::new(code::FAILED, error.to_string()))?
        .into_iter()
        .find(|spec| spec.name == tool)
        .ok_or_else(|| {
            ToolError::invalid(format!("Skill '{}' offers no tool '{tool}' on this host", skill.name))
        })?
        .needs_approval();
    if gated != asks {
        return Err(ToolError::invalid(if gated {
            format!("'{tool}' reaches the network or writes data: run it with `skill_run`, which asks your owner")
        } else {
            format!("'{tool}' is read-only: run it with `skill_call`")
        }));
    }
    let arguments = match args.arguments {
        None | Some(Value::Null) => Value::Object(Default::default()),
        Some(object @ Value::Object(_)) => object,
        Some(_) => return Err(ToolError::invalid("'arguments' is an object of pin values")),
    };
    let result = host
        .call(&skill, tool, arguments)
        .await
        .map_err(|error| ToolError::new(code::FAILED, error.to_string()))?;
    Ok(ToolOutput::new(json!({
        "skill": skill.name,
        "tool": tool,
        "result": result,
    })))
}

#[cfg(test)]
mod tests;
