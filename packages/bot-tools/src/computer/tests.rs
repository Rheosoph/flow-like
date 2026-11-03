use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use flow_like_types::{Result, anyhow};
use serde_json::{Value, json};

use super::*;
use crate::browser::tests::CapEnv;
use crate::{Status, ToolOutcome, run};

#[derive(Default)]
struct FakeScreen {
    actions: Mutex<Vec<ComputerAction>>,
    shots: Mutex<u32>,
    fail_act: Mutex<Option<String>>,
}

#[async_trait]
impl ComputerBackend for FakeScreen {
    async fn screenshot(&self) -> Result<Screen> {
        *self.shots.lock().unwrap() += 1;
        Ok(Screen {
            png: vec![0x89, b'P', b'N', b'G'],
            width: 1280,
            height: 800,
        })
    }

    async fn act(&self, action: &ComputerAction) -> Result<String> {
        if let Some(message) = self.fail_act.lock().unwrap().clone() {
            return Err(anyhow!(message));
        }
        self.actions.lock().unwrap().push(action.clone());
        Ok("done".into())
    }
}

async fn setup() -> (Arc<FakeScreen>, CapEnv) {
    let screen = Arc::new(FakeScreen::default());
    let mut env = CapEnv::new().await;
    env.computer = Some(screen.clone());
    (screen, env)
}

async fn call(env: &CapEnv, tool: &str, args: Value) -> ToolOutcome {
    run(tool, args, env).await
}

#[tokio::test]
async fn a_screenshot_is_an_image_with_its_size() {
    let (_, env) = setup().await;
    let outcome = call(&env, "computer_screenshot", json!({})).await;
    assert!(outcome.is_ok());
    assert_eq!(outcome.images.len(), 1);
    assert_eq!(
        outcome.result["screen"],
        json!({ "width": 1280, "height": 800 })
    );
}

#[tokio::test]
async fn an_action_runs_and_shows_the_screen_after() {
    let (screen, env) = setup().await;
    let outcome = call(
        &env,
        "computer_act",
        json!({ "action": "double_click", "x": 10, "y": 20 }),
    )
    .await;
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(outcome.images.len(), 1);
    assert_eq!(
        screen.actions.lock().unwrap()[0],
        ComputerAction::Click {
            x: 10.0,
            y: 20.0,
            button: PointerButton::Left,
            count: 2
        }
    );
}

#[tokio::test]
async fn a_host_without_a_computer_refuses_every_tool_and_touches_nothing() {
    let env = CapEnv::new().await;
    for (tool, args) in [
        ("computer_screenshot", json!({})),
        ("computer_act", json!({ "action": "key", "text": "Enter" })),
        ("computer_wait", json!({ "ms": 1 })),
    ] {
        let outcome = call(&env, tool, args).await;
        assert_eq!(outcome.status, Status::Error, "{tool}");
        assert_eq!(outcome.error.unwrap().code, crate::error::code::NOT_YET);
    }
}

#[tokio::test]
async fn bad_actions_never_reach_the_screen() {
    let (screen, env) = setup().await;
    for args in [
        json!({ "action": "click" }),
        json!({ "action": "click", "x": -1, "y": 3 }),
        json!({ "action": "type" }),
        json!({ "action": "key", "text": "  " }),
        json!({ "action": "scroll", "x": 1, "y": 1, "text": "diagonal" }),
    ] {
        let outcome = call(&env, "computer_act", args.clone()).await;
        assert_eq!(
            outcome.error.as_ref().map(|e| e.code),
            Some(crate::error::code::INVALID_ARGS),
            "{args}"
        );
    }
    assert!(screen.actions.lock().unwrap().is_empty());
    assert_eq!(*screen.shots.lock().unwrap(), 0);
}

#[tokio::test]
async fn typed_secrets_are_filled_and_never_returned() {
    let (screen, env) = setup().await;
    env.inner
        .secrets
        .lock()
        .unwrap()
        .insert("VPN".into(), "tr0ub4dor".into());
    *screen.fail_act.lock().unwrap() = None;
    let outcome = call(
        &env,
        "computer_act",
        json!({ "action": "type", "text": "{{secret:VPN}}" }),
    )
    .await;
    assert!(outcome.is_ok());
    assert_eq!(
        screen.actions.lock().unwrap()[0],
        ComputerAction::Type {
            text: "tr0ub4dor".into()
        }
    );
    assert!(!outcome.envelope().to_string().contains("tr0ub4dor"));

    *screen.fail_act.lock().unwrap() = Some("could not type tr0ub4dor".into());
    let failed = call(
        &env,
        "computer_act",
        json!({ "action": "type", "text": "{{secret:VPN}}" }),
    )
    .await;
    assert!(!failed.is_ok());
    assert!(!failed.envelope().to_string().contains("tr0ub4dor"));
}

#[tokio::test]
async fn waiting_is_capped_and_cancellable() {
    let (_, env) = setup().await;
    env.inner
        .cancelled
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let outcome = call(&env, "computer_wait", json!({ "ms": 60_000 })).await;
    assert_eq!(outcome.error.unwrap().code, crate::error::code::CANCELLED);
}

#[test]
fn the_screen_asks_and_waiting_does_not() {
    let specs = super::specs();
    let approval = |name: &str| specs.iter().find(|s| s.name == name).unwrap().approval;
    assert_eq!(approval(COMPUTER_SCREENSHOT), crate::Approval::Ask);
    assert_eq!(approval(COMPUTER_ACT), crate::Approval::Ask);
    assert_eq!(approval(COMPUTER_WAIT), crate::Approval::None);
    assert!(specs.iter().all(|s| s.capability == Some("computer_use")));
}
