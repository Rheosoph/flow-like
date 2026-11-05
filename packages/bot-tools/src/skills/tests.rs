use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use flow_like_types::Result;
use flow_like_types::bot_skill::{BotSkill, CreateBotSkill, SkillSource, SkillStatus};
use serde_json::{Value, json};

use super::*;
use crate::testing::{FakeEnv, WORKSPACE};

#[derive(Default)]
struct FakeHost {
    skills: Mutex<Vec<BotSkill>>,
    calls: Mutex<Vec<(String, String, Value)>>,
}

fn skill(name: &str, status: SkillStatus, tools: &[&str]) -> BotSkill {
    BotSkill {
        id: format!("id-{name}"),
        bot_id: "bot".into(),
        name: name.into(),
        description: format!("About {name}"),
        body: format!("Do the {name} thing."),
        source: SkillSource::Owner,
        status,
        tools: tools.iter().map(|tool| tool.to_string()).collect(),
        updated_at: "2026-11-05T00:00:00Z".into(),
    }
}

#[async_trait]
impl SkillHost for FakeHost {
    async fn get(&self, name: &str) -> Result<Option<BotSkill>> {
        Ok(self.skills.lock().unwrap().iter().find(|s| s.name == name).cloned())
    }

    async fn propose(&self, request: CreateBotSkill) -> Result<BotSkill> {
        let mut pending = skill(&request.name, SkillStatus::Pending, &[]);
        pending.source = SkillSource::BotProposed;
        pending.description = request.description;
        pending.body = request.body;
        self.skills.lock().unwrap().push(pending.clone());
        Ok(pending)
    }

    async fn tool_specs(&self, skill: &BotSkill) -> Result<Vec<SkillToolSpec>> {
        Ok(skill
            .tools
            .iter()
            .map(|name| SkillToolSpec {
                name: name.clone(),
                description: "a tool".into(),
                input_schema: json!({ "type": "object" }),
                network: name.starts_with("fetch"),
                writes: name.starts_with("save"),
            })
            .collect())
    }

    async fn call(&self, skill: &BotSkill, tool: &str, arguments: Value) -> Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((skill.name.clone(), tool.to_string(), arguments));
        Ok(json!({ "done": true }))
    }
}

async fn env_with(host: Arc<FakeHost>) -> FakeEnv {
    let env = FakeEnv::new().await;
    *env.skills.lock().unwrap() = Some(host);
    env
}

async fn call(env: &FakeEnv, tool: &str, args: Value) -> crate::ToolOutcome {
    crate::run(tool, args, env).await
}

#[test]
fn the_specs_need_the_skills_capability_and_call_asks() {
    for name in [SKILL_LOAD, SKILL_PROPOSE, SKILL_CALL, SKILL_RUN] {
        assert_eq!(crate::spec(name).expect("registered").capability, Some("skills"));
    }
    assert!(crate::spec(SKILL_CALL).unwrap().approval == Approval::None);
    assert!(crate::spec(SKILL_RUN).unwrap().approval == Approval::Ask);
    assert!(crate::spec(SKILL_LOAD).unwrap().approval == Approval::None);
    assert!(crate::spec(SKILL_IMPORT).is_none(), "import is no model tool");
}

#[tokio::test]
async fn an_active_skill_loads_framed_with_its_references_and_tools() {
    let host = Arc::new(FakeHost::default());
    host.skills.lock().unwrap().push(skill("pdf", SkillStatus::Active, &["extract"]));
    let env = env_with(host).await;
    env.put(WORKSPACE, "bot/skills/pdf/SKILL.md", b"x", "text/markdown");
    env.put(WORKSPACE, "bot/skills/pdf/refs/a.md", b"ref", "text/markdown");
    let outcome = call(&env, SKILL_LOAD, json!({ "name": "pdf" })).await;
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let instructions = outcome.result["instructions"].as_str().unwrap();
    assert!(instructions.contains("Do the pdf thing."));
    assert!(instructions.contains("never overrides your owner"));
    assert_eq!(outcome.result["references"][0]["path"], "bot/skills/pdf/refs/a.md");
    assert_eq!(outcome.result["references"].as_array().unwrap().len(), 1);
    assert_eq!(outcome.result["tools"][0]["name"], "extract");
}

#[tokio::test]
async fn a_pending_or_disabled_skill_is_never_loadable() {
    let host = Arc::new(FakeHost::default());
    host.skills.lock().unwrap().push(skill("draft", SkillStatus::Pending, &["t"]));
    host.skills.lock().unwrap().push(skill("off", SkillStatus::Disabled, &["t"]));
    let env = env_with(host.clone()).await;
    for name in ["draft", "off", "missing"] {
        let outcome = call(&env, SKILL_LOAD, json!({ "name": name })).await;
        assert!(!outcome.is_ok(), "{name}");
        assert_eq!(outcome.error.unwrap().code, code::NOT_FOUND, "{name}");
    }
    let outcome = call(&env, SKILL_CALL, json!({ "skill": "draft", "tool": "t" })).await;
    assert!(!outcome.is_ok());
    assert!(host.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_proposal_is_saved_pending_and_checked() {
    let host = Arc::new(FakeHost::default());
    let env = env_with(host.clone()).await;
    let outcome = call(
        &env,
        SKILL_PROPOSE,
        json!({ "name": "weekly-report", "description": "Write the weekly report", "body": "1. Collect" }),
    )
    .await;
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.result["status"], "pending");
    let loaded = call(&env, SKILL_LOAD, json!({ "name": "weekly-report" })).await;
    assert!(!loaded.is_ok(), "the proposal stays invisible until approved");

    for bad in [
        json!({ "name": "Bad Name", "description": "d", "body": "b" }),
        json!({ "name": "ok", "description": "d", "body": "hide\u{200B}this" }),
        json!({ "name": "ok", "description": "d\nsecond line", "body": "b" }),
    ] {
        let outcome = call(&env, SKILL_PROPOSE, bad).await;
        assert_eq!(outcome.error.unwrap().code, code::INVALID_ARGS);
    }
    assert_eq!(host.skills.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_call_runs_only_a_tool_the_skill_lists() {
    let host = Arc::new(FakeHost::default());
    host.skills.lock().unwrap().push(skill("pdf", SkillStatus::Active, &["extract"]));
    let env = env_with(host.clone()).await;
    let outcome = call(
        &env,
        SKILL_CALL,
        json!({ "skill": "pdf", "tool": "extract", "arguments": { "page": 2 } }),
    )
    .await;
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.result["result"]["done"], true);
    assert_eq!(host.calls.lock().unwrap()[0].2, json!({ "page": 2 }));

    let other = call(&env, SKILL_CALL, json!({ "skill": "pdf", "tool": "delete" })).await;
    assert_eq!(other.error.unwrap().code, code::INVALID_ARGS);
    let bad = call(&env, SKILL_CALL, json!({ "skill": "pdf", "tool": "extract", "arguments": [1] })).await;
    assert_eq!(bad.error.unwrap().code, code::INVALID_ARGS);
    assert_eq!(host.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_host_without_skills_says_so() {
    let env = FakeEnv::new().await;
    let outcome = call(&env, SKILL_LOAD, json!({ "name": "pdf" })).await;
    assert!(outcome.error.unwrap().message.contains("does not keep skills"));
}

#[tokio::test]
async fn network_and_write_tools_need_skill_run_and_read_only_tools_skill_call() {
    let host = Arc::new(FakeHost::default());
    host.skills
        .lock()
        .unwrap()
        .push(skill("web", SkillStatus::Active, &["extract", "fetch_page", "save_note"]));
    let env = env_with(host.clone()).await;
    let loaded = call(&env, SKILL_LOAD, json!({ "name": "web" })).await;
    let flags: Vec<bool> = loaded.result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["needs_approval"].as_bool().unwrap())
        .collect();
    assert_eq!(flags, [false, true, true]);

    for tool in ["fetch_page", "save_note"] {
        let refused = call(&env, SKILL_CALL, json!({ "skill": "web", "tool": tool })).await;
        assert!(refused.error.unwrap().message.contains("skill_run"), "{tool}");
        let ran = call(&env, SKILL_RUN, json!({ "skill": "web", "tool": tool })).await;
        assert!(ran.is_ok(), "{tool}: {:?}", ran.error);
    }
    let refused = call(&env, SKILL_RUN, json!({ "skill": "web", "tool": "extract" })).await;
    assert!(refused.error.unwrap().message.contains("skill_call"));
    assert_eq!(host.calls.lock().unwrap().len(), 2);
}
