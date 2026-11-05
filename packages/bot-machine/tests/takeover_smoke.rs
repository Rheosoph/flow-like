//! Needs Linux, Xvfb on PATH and the agent binary in `FLOW_LIKE_GATE_AGENT`. Run by the gate:
//! `FLOW_LIKE_GATE_AGENT=/target/debug/flow-like-standalone FLOW_LIKE_PLAYGROUND=virtual \
//!  cargo test -p flow-like-bot-machine --test takeover_smoke -- --ignored --nocapture`.

#![cfg(target_os = "linux")]

use std::path::PathBuf;

use flow_like_bot_machine::{InputEvent, MachineHost, Mode};
use flow_like_bot_tools::computer::{ComputerAction, PointerButton};
use flow_like_bot_tools::error::{ToolError, code};

fn host(dir: &std::path::Path) -> MachineHost {
    let program = PathBuf::from(
        std::env::var("FLOW_LIKE_GATE_AGENT").expect("FLOW_LIKE_GATE_AGENT names the agent binary"),
    );
    MachineHost::new(dir.to_path_buf(), Mode::Worker { program })
}

fn click(x: f64, y: f64) -> ComputerAction {
    ComputerAction::Click {
        x,
        y,
        button: PointerButton::Left,
        count: 1,
    }
}

#[tokio::test]
#[ignore = "needs Xvfb and the agent binary"]
async fn the_owner_takes_over_a_bots_virtual_display_and_hands_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let host = host(dir.path());
    let computer = host
        .machine("takeover-bot", None, false, true)
        .computer
        .expect("computer backend");

    let before = computer.screenshot().await.expect("bot screenshot");
    assert_eq!((before.width, before.height), (1280, 800));
    assert!(before.note.is_none());

    host.take_over("takeover-bot").expect("take over");
    assert!(host.is_taken_over("takeover-bot"));
    let refused = computer.act(&click(10.0, 10.0)).await.unwrap_err();
    assert_eq!(ToolError::from(refused).code, code::TAKEN_OVER);
    let refused = computer.screenshot().await.unwrap_err();
    assert_eq!(ToolError::from(refused).code, code::TAKEN_OVER);

    host.inject(
        "takeover-bot",
        InputEvent::PointerDown {
            x: 100.0,
            y: 100.0,
            button: PointerButton::Left,
        },
    )
    .await
    .expect("owner click down");
    host.inject(
        "takeover-bot",
        InputEvent::PointerUp {
            x: 100.0,
            y: 100.0,
            button: PointerButton::Left,
        },
    )
    .await
    .expect("owner click up");

    host.hand_back("takeover-bot").expect("hand back");
    assert!(!host.is_taken_over("takeover-bot"));
    let after = computer.screenshot().await.expect("screenshot after hand back");
    let note = after.note.expect("a note about the owner's time");
    assert!(note.contains("The owner used the computer"), "note was: {note}");
    assert!(
        host.inject(
            "takeover-bot",
            InputEvent::PointerMove { x: 1.0, y: 1.0 }
        )
        .await
        .is_err(),
        "input is refused once the bot has the computer back"
    );
    host.shutdown().await;
}

#[tokio::test]
#[ignore = "needs Xvfb and the agent binary"]
async fn two_bots_get_two_displays() {
    let dir = tempfile::tempdir().unwrap();
    let host = host(dir.path());
    let a = host.machine("duo-a", None, false, true).computer.unwrap();
    let b = host.machine("duo-b", None, false, true).computer.unwrap();
    a.screenshot().await.expect("a");
    b.screenshot().await.expect("b");
    let sockets = std::fs::read_dir("/tmp/.X11-unix")
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("X9"))
        .count();
    assert!(sockets >= 2, "two Xvfb sockets expected, saw {sockets}");
    host.shutdown().await;
}
