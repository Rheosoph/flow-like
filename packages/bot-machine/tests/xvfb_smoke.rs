//! Needs Linux with `Xvfb` (and ideally openbox + xauth) on PATH. Run by the gate:
//! `cargo test -p flow-like-bot-machine --test xvfb_smoke -- --ignored`.

#![cfg(target_os = "linux")]

use flow_like_bot_machine::{LocalPlayground, Playground, Surface, VirtualDisplay};
use flow_like_bot_tools::computer::ComputerBackend;

#[tokio::test]
#[ignore = "needs Xvfb on PATH"]
async fn a_bot_sees_its_own_virtual_display() {
    let dir = tempfile::tempdir().unwrap();
    let display = VirtualDisplay::start(dir.path())
        .await
        .expect("Xvfb starts");
    assert!(display.number >= 90);
    for (key, value) in display.env() {
        unsafe { std::env::set_var(key, value) };
    }
    unsafe { std::env::remove_var("WAYLAND_DISPLAY") };

    let playground = LocalPlayground::new(dir.path(), "smoke-bot", Surface::VirtualDisplay);
    let computer = playground.computer();
    let screen = computer.screenshot().await.expect("screenshot of :N");
    assert_eq!((screen.width, screen.height), (1280, 800));
    assert!(computer.windows().await.is_ok());
    playground.stop().await;
}
