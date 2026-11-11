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
    window_ops: Mutex<Vec<(String, WindowOp, WindowBounds)>>,
    launches: Mutex<Vec<(String, Vec<String>)>>,
}

#[async_trait]
impl ComputerBackend for FakeScreen {
    async fn screenshot(&self) -> Result<Screen> {
        *self.shots.lock().unwrap() += 1;
        Ok(Screen::new(vec![0x89, b'P', b'N', b'G'], 1280, 800))
    }

    async fn act(&self, action: &ComputerAction) -> Result<String> {
        if let Some(message) = self.fail_act.lock().unwrap().clone() {
            return Err(anyhow!(message));
        }
        self.actions.lock().unwrap().push(action.clone());
        Ok("done".into())
    }

    async fn screenshot_window(&self, window: &str) -> Result<Screen> {
        let mut screen = self.screenshot().await?;
        screen.note = Some(format!("window {window}"));
        Ok(screen)
    }

    async fn windows(&self) -> Result<Vec<WindowSummary>> {
        Ok(vec![WindowSummary {
            id: "7".into(),
            title: "Notes".into(),
            app: Some("notes".into()),
            x: 0,
            y: 0,
            width: 400,
            height: 300,
            focused: true,
            minimized: false,
            onscreen: true,
        }])
    }

    async fn window(&self, window: &str, op: WindowOp, bounds: WindowBounds) -> Result<String> {
        self.window_ops
            .lock()
            .unwrap()
            .push((window.into(), op, bounds));
        Ok("done".into())
    }

    async fn launch(&self, program: &str, args: &[String]) -> Result<String> {
        self.launches
            .lock()
            .unwrap()
            .push((program.into(), args.to_vec()));
        Ok("started".into())
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

#[test]
fn feedback_keeps_clean_pixels_and_explains_the_annotation_in_model_json() {
    use flow_like_types::image::{DynamicImage, ImageFormat, RgbaImage};
    use flow_like_types::interaction_observation::{
        InteractionObservation, InteractionPoint, InteractionPointer, PointerSource,
    };
    let mut png = Vec::new();
    DynamicImage::ImageRgba8(RgbaImage::new(80, 40))
        .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
        .unwrap();
    let mut screen = Screen::new(png.clone(), 80, 40);
    screen.observation = Some(InteractionObservation {
        captured_at_ms: 10,
        width: 80,
        height: 40,
        pointer: Some(InteractionPointer {
            position: InteractionPoint { x: 0.5, y: 0.5 },
            source: PointerSource::Observed,
        }),
        focus: None,
        action: None,
        mapping: None,
    });
    let output = screen_output(screen, "Screen", json!({"screen":{"width":80,"height":40}}));
    assert_eq!(output.images.len(), 2);
    assert_eq!(&output.images[0].data[..], png);
    assert_ne!(output.images[0].data, output.images[1].data);
    assert_eq!(
        output.result["observation"]["pointer"]["source"],
        "observed"
    );
    assert!(
        output.result["feedback_legend"]
            .as_str()
            .unwrap()
            .contains("do not prove")
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
        ("computer_windows", json!({})),
        (
            "computer_window",
            json!({ "action": "focus", "window": "Notes" }),
        ),
        ("computer_launch", json!({ "program": "notes" })),
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

#[tokio::test]
async fn windows_are_listed_and_a_screenshot_can_be_one_window() {
    let (_, env) = setup().await;
    let listed = call(&env, "computer_windows", json!({})).await;
    assert!(listed.is_ok());
    assert_eq!(listed.result["windows"][0]["title"], "Notes");
    let shot = call(&env, "computer_screenshot", json!({ "window": " Notes " })).await;
    assert_eq!(shot.images.len(), 1);
    assert_eq!(shot.result["note"], "window Notes");
}

#[tokio::test]
async fn window_actions_validate_their_arguments_before_the_backend_runs() {
    let (screen, env) = setup().await;
    for args in [
        json!({ "action": "move", "window": "Notes" }),
        json!({ "action": "resize", "window": "Notes" }),
        json!({ "action": "resize", "window": "Notes", "width": 0 }),
        json!({ "action": "focus", "window": "  " }),
        json!({ "action": "teleport", "window": "Notes" }),
    ] {
        let outcome = call(&env, "computer_window", args.clone()).await;
        assert_eq!(
            outcome.error.as_ref().map(|e| e.code),
            Some(crate::error::code::INVALID_ARGS),
            "{args}"
        );
    }
    assert!(screen.window_ops.lock().unwrap().is_empty());
    let moved = call(
        &env,
        "computer_window",
        json!({ "action": "move", "window": "Notes", "x": 5 }),
    )
    .await;
    assert!(moved.is_ok());
    assert_eq!(
        screen.window_ops.lock().unwrap()[0],
        (
            "Notes".into(),
            WindowOp::Move,
            WindowBounds {
                x: Some(5),
                ..Default::default()
            }
        )
    );
}

#[tokio::test]
async fn launch_arguments_are_never_secret_substituted_and_programs_are_validated() {
    let (screen, env) = setup().await;
    env.inner
        .secrets
        .lock()
        .unwrap()
        .insert("VPN".into(), "tr0ub4dor".into());
    let outcome = call(
        &env,
        "computer_launch",
        json!({ "program": "/usr/bin/tool", "args": ["--token", "{{secret:VPN}}"] }),
    )
    .await;
    assert!(outcome.is_ok());
    assert_eq!(
        screen.launches.lock().unwrap()[0],
        (
            "/usr/bin/tool".to_string(),
            vec!["--token".to_string(), "{{secret:VPN}}".to_string()]
        )
    );
    for args in [
        json!({ "program": "  " }),
        json!({ "program": "a\u{0000}b" }),
        json!({ "program": "x", "args": vec!["a"; 33] }),
    ] {
        let outcome = call(&env, "computer_launch", args.clone()).await;
        assert_eq!(
            outcome.error.as_ref().map(|e| e.code),
            Some(crate::error::code::INVALID_ARGS),
            "{args}"
        );
    }
    assert_eq!(screen.launches.lock().unwrap().len(), 1);
}

#[test]
fn a_launch_grant_names_the_program_file() {
    assert_eq!(launch_scope("/usr/bin/Firefox"), Some("firefox".into()));
    assert_eq!(launch_scope("C:\\Apps\\Code.exe"), Some("code.exe".into()));
    assert_eq!(launch_scope(" firefox "), Some("firefox".into()));
    assert_eq!(launch_scope("/"), None);
}

#[tokio::test]
async fn a_backend_that_cannot_manage_windows_says_so() {
    struct Plain;
    #[async_trait]
    impl ComputerBackend for Plain {
        async fn screenshot(&self) -> Result<Screen> {
            Ok(Screen::new(vec![1], 1, 1))
        }
        async fn act(&self, _: &ComputerAction) -> Result<String> {
            Ok(String::new())
        }
    }
    let mut env = CapEnv::new().await;
    env.computer = Some(Arc::new(Plain));
    let outcome = call(&env, "computer_windows", json!({})).await;
    assert_eq!(
        outcome.error.unwrap().code,
        crate::error::code::NOT_SUPPORTED
    );
}

#[test]
fn window_tools_ask_except_the_listing() {
    let specs = super::specs();
    let approval = |name: &str| specs.iter().find(|s| s.name == name).unwrap().approval;
    assert_eq!(approval(COMPUTER_WINDOWS), crate::Approval::None);
    assert_eq!(approval(COMPUTER_WINDOW), crate::Approval::Ask);
    assert_eq!(approval(COMPUTER_LAUNCH), crate::Approval::Ask);
}

#[tokio::test]
async fn waiting_while_the_owner_is_at_the_computer_succeeds_without_a_picture() {
    struct Owner;
    #[async_trait]
    impl ComputerBackend for Owner {
        async fn screenshot(&self) -> Result<Screen> {
            Err(flow_like_types::Error::new(ToolError::new(
                crate::error::code::TAKEN_OVER,
                "The owner is using the computer",
            )))
        }
        async fn act(&self, _: &ComputerAction) -> Result<String> {
            Ok(String::new())
        }
    }
    let mut env = CapEnv::new().await;
    env.computer = Some(Arc::new(Owner));
    let outcome = call(&env, "computer_wait", json!({ "ms": 1 })).await;
    assert!(outcome.is_ok());
    assert!(outcome.images.is_empty());
    assert_eq!(outcome.result["taken_over"], true);
    let act = call(&env, "computer_screenshot", json!({})).await;
    assert_eq!(act.error.unwrap().code, crate::error::code::TAKEN_OVER);
}
