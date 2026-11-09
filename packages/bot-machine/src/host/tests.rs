use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use flow_like_bot_tools::browser::{
    BrowserBackend, BrowserScreen, BrowserStep, Download, PageState, SnapshotRequest, SnapshotView,
};
use flow_like_bot_tools::computer::{
    ComputerAction, ComputerBackend, PointerButton, Screen, WindowBounds, WindowOp, WindowSummary,
};
use flow_like_bot_tools::error::{ToolError, code};
use flow_like_types::interaction_observation::{
    InteractionAction, InteractionObservation, InteractionPhase, InteractionPoint,
    InteractionPointer, PointerSource,
};
use flow_like_types::{Bytes, Result};
use tokio::io::duplex;

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::frames::Shot;
use crate::rpc::{Spawner, Transport, serve};

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::new(width, height))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

#[derive(Default)]
struct FakeBrowser {
    opened: Mutex<Vec<String>>,
    steps: Mutex<Vec<BrowserStep>>,
    observation: Mutex<Option<InteractionObservation>>,
}

fn page(url: &str) -> PageState {
    PageState {
        url: url.into(),
        title: "Title".into(),
    }
}

#[async_trait]
impl BrowserBackend for FakeBrowser {
    async fn open(&self, url: &str) -> Result<PageState> {
        self.opened.lock().unwrap().push(url.into());
        Ok(page(url))
    }
    async fn snapshot(&self, request: SnapshotRequest) -> Result<SnapshotView> {
        Ok(SnapshotView {
            page: page("https://a.example/"),
            text: format!("limit {}", request.max_chars),
            truncated: request.interactive_only,
        })
    }
    async fn act(&self, step: &BrowserStep) -> Result<PageState> {
        self.steps.lock().unwrap().push(step.clone());
        Ok(page("https://a.example/next"))
    }
    async fn screenshot(&self) -> Result<Vec<u8>> {
        Ok(png(20, 10))
    }
    async fn capture(&self) -> Result<BrowserScreen> {
        Ok(BrowserScreen {
            png: self.screenshot().await?,
            observation: self.observation.lock().unwrap().clone(),
        })
    }
    async fn observation(&self) -> Result<Option<InteractionObservation>> {
        Ok(self.observation.lock().unwrap().clone())
    }
    async fn current(&self) -> Result<Option<PageState>> {
        Ok(Some(page("https://a.example/")))
    }
    async fn leave(&self) -> Result<()> {
        Ok(())
    }
    async fn take_downloads(&self) -> Result<Vec<Download>> {
        Ok(vec![Download {
            name: "a.txt".into(),
            bytes: Bytes::from_static(b"hello"),
        }])
    }
    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct FakeScreen {
    actions: Mutex<Vec<ComputerAction>>,
    observation: Mutex<Option<InteractionObservation>>,
}

#[async_trait]
impl ComputerBackend for FakeScreen {
    async fn screenshot(&self) -> Result<Screen> {
        let mut screen = Screen::new(png(64, 40), 64, 40);
        screen.observation = self.observation.lock().unwrap().clone();
        Ok(screen)
    }
    async fn act(&self, action: &ComputerAction) -> Result<String> {
        self.actions.lock().unwrap().push(action.clone());
        Ok("did it".into())
    }
    async fn windows(&self) -> Result<Vec<WindowSummary>> {
        Ok(vec![WindowSummary {
            id: "1".into(),
            title: "Term".into(),
            app: None,
            x: 1,
            y: 2,
            width: 3,
            height: 4,
            focused: true,
            minimized: false,
        }])
    }
    async fn window(&self, window: &str, op: WindowOp, bounds: WindowBounds) -> Result<String> {
        if window == "gone" {
            return Err(flow_like_types::Error::new(ToolError::not_found(
                "No window matches 'gone'",
            )));
        }
        Ok(format!("{op:?} {:?}", bounds.x))
    }
}

#[derive(Default)]
struct FakePlayground {
    browser: Arc<FakeBrowser>,
    screen: Arc<FakeScreen>,
    injected: Mutex<Vec<(InputEvent, u32, u32)>>,
    stops: AtomicUsize,
    shot: Mutex<Option<Shot>>,
}

#[async_trait]
impl Playground for FakePlayground {
    fn browser(&self, _conversation: Option<&str>) -> Arc<dyn BrowserBackend> {
        self.browser.clone()
    }
    fn computer(&self) -> Arc<dyn ComputerBackend> {
        self.screen.clone()
    }
    async fn capture(&self) -> Result<Option<Shot>> {
        Ok(self.shot.lock().unwrap().clone())
    }
    async fn inject(&self, event: &InputEvent, width: u32, height: u32) -> Result<()> {
        self.injected
            .lock()
            .unwrap()
            .push((event.clone(), width, height));
        Ok(())
    }
    async fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
    async fn sweep(&self, _idle: Duration) {}
}

fn host_over(playground: Arc<FakePlayground>, display: DisplayKind) -> MachineHost {
    MachineHost::with_display(display, move |_| playground.clone() as Arc<dyn Playground>)
}

fn local_ish(playground: Arc<FakePlayground>) -> MachineHost {
    host_over(playground, DisplayKind::Virtual)
}

#[tokio::test]
async fn a_machine_offers_only_what_was_asked_for() {
    let host = local_ish(Arc::default());
    let none = host.machine("b", None, false, false);
    assert!(none.browser.is_none() && none.computer.is_none());
    let both = host.machine("b", Some("c"), true, true);
    assert!(both.browser.is_some() && both.computer.is_some());
}

#[tokio::test]
async fn while_taken_over_every_computer_and_browser_call_is_refused_and_the_note_comes_once() {
    let playground = Arc::new(FakePlayground::default());
    let host = local_ish(playground.clone());
    let machine = host.machine("b", None, true, true);
    let (browser, computer) = (machine.browser.unwrap(), machine.computer.unwrap());
    host.take_over("b").unwrap();
    assert!(host.is_taken_over("b"));

    let taken = |error: flow_like_types::Error| ToolError::from(error).code == code::TAKEN_OVER;
    assert!(taken(computer.screenshot().await.unwrap_err()));
    assert!(taken(computer.windows().await.unwrap_err()));
    assert!(taken(
        computer
            .act(&ComputerAction::Key {
                chord: "Enter".into()
            })
            .await
            .unwrap_err()
    ));
    assert!(taken(computer.launch("x", &[]).await.unwrap_err()));
    assert!(taken(browser.open("https://a.example/").await.unwrap_err()));
    assert!(taken(
        browser
            .act(&BrowserStep::Click {
                reference: "e1".into()
            })
            .await
            .unwrap_err()
    ));
    assert!(playground.screen.actions.lock().unwrap().is_empty());
    assert!(playground.browser.opened.lock().unwrap().is_empty());

    host.hand_back("b").unwrap();
    let first = computer.screenshot().await.unwrap();
    assert!(
        first
            .note
            .as_deref()
            .unwrap()
            .contains("The owner used the computer")
    );
    assert!(computer.screenshot().await.unwrap().note.is_none());
}

#[tokio::test]
async fn after_a_hand_back_the_browser_needs_a_fresh_snapshot_before_it_acts() {
    let host = local_ish(Arc::default());
    let browser = host.machine("b", None, true, false).browser.unwrap();
    host.take_over("b").unwrap();
    host.hand_back("b").unwrap();
    let step = BrowserStep::Click {
        reference: "e1".into(),
    };
    let error = ToolError::from(browser.act(&step).await.unwrap_err());
    assert_eq!(error.code, code::STALE_PAGE);
    assert!(error.message.contains("fresh"));
    assert_eq!(
        ToolError::from(browser.act(&step).await.unwrap_err()).code,
        code::STALE_PAGE
    );

    host.take_over("b").unwrap();
    host.hand_back("b").unwrap();
    let snapshot = browser
        .snapshot(SnapshotRequest {
            max_chars: 10,
            interactive_only: false,
        })
        .await
        .unwrap();
    assert!(snapshot.text.starts_with("[The owner used the computer"));
    assert!(browser.act(&step).await.is_ok());
}

#[tokio::test]
async fn after_a_hand_back_window_operations_and_launch_need_a_fresh_screenshot() {
    let host = local_ish(Arc::default());
    let computer = host.machine("b", None, false, true).computer.unwrap();
    host.take_over("b").unwrap();
    host.hand_back("b").unwrap();
    let stale = |error: flow_like_types::Error| ToolError::from(error).code == code::STALE_PAGE;
    assert!(stale(
        computer
            .window("1", WindowOp::Close, WindowBounds::default())
            .await
            .unwrap_err()
    ));
    assert!(stale(computer.launch("x", &[]).await.unwrap_err()));
    assert!(computer.windows().await.is_ok());

    computer.screenshot().await.unwrap();
    assert!(
        computer
            .window("1", WindowOp::Focus, WindowBounds::default())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn input_is_injected_only_while_taken_over_and_only_into_a_computer_frame() {
    let playground = Arc::new(FakePlayground::default());
    let host = local_ish(playground.clone());
    let computer = host.machine("b", None, false, true).computer.unwrap();
    let event = InputEvent::PointerDown {
        x: 5.0,
        y: 6.0,
        button: PointerButton::Left,
    };

    assert!(host.inject("b", event.clone()).await.is_err());
    host.take_over("b").unwrap();
    let refused = ToolError::from(host.inject("b", event.clone()).await.unwrap_err());
    assert_eq!(
        refused.code,
        code::NOT_SUPPORTED,
        "no frame yet means nothing to click in"
    );

    host.hand_back("b").unwrap();
    computer.screenshot().await.unwrap();
    host.take_over("b").unwrap();
    host.inject("b", event.clone()).await.unwrap();
    assert_eq!(
        playground.injected.lock().unwrap()[0],
        (event.clone(), 64, 40)
    );

    host.hand_back("b").unwrap();
    assert!(host.inject("b", event).await.is_err());
    assert_eq!(playground.injected.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_browser_frame_is_watch_only_and_an_in_process_host_never_injects() {
    let playground = Arc::new(FakePlayground::default());
    let host = local_ish(playground.clone());
    let browser = host.machine("b", None, true, false).browser.unwrap();
    browser.screenshot().await.unwrap();
    host.take_over("b").unwrap();
    let event = InputEvent::Text { text: "a".into() };
    let error = ToolError::from(host.inject("b", event.clone()).await.unwrap_err());
    assert_eq!(error.code, code::NOT_SUPPORTED);
    assert!(error.message.contains("watch only"));

    let local = MachineHost::new(std::env::temp_dir(), Mode::InProcess);
    local.take_over("b").unwrap();
    let error = ToolError::from(local.inject("b", event).await.unwrap_err());
    assert_eq!(error.code, code::NOT_SUPPORTED);
}

#[tokio::test]
async fn screenshots_feed_the_frame_stream_and_stop_clears_it() {
    let playground = Arc::new(FakePlayground::default());
    let host = local_ish(playground.clone());
    let stream = host.frames("b");
    assert!(stream.latest().is_none());
    let computer = host.machine("b", None, false, true).computer.unwrap();
    computer.screenshot().await.unwrap();
    let frame = stream.latest().unwrap();
    assert_eq!(
        (frame.kind, frame.width, frame.height),
        (FrameKind::Computer, 64, 40)
    );

    *playground.shot.lock().unwrap() = Some(Shot {
        png: png(8, 8),
        kind: FrameKind::Browser,
        caption: "https://x/".into(),
        observation: None,
    });
    tokio::time::sleep(Duration::from_millis(1600)).await;
    let refreshed = host.refresh_frame("b").await.unwrap();
    assert_eq!(refreshed.kind, FrameKind::Browser);

    host.stop("b").await;
    assert_eq!(playground.stops.load(Ordering::SeqCst), 1);
    assert!(stream.latest().is_none());
    assert!(!host.is_taken_over("b"));
}

#[tokio::test]
async fn a_machine_without_a_display_says_what_to_install() {
    let host = host_over(
        Arc::default(),
        DisplayKind::None("computer use needs a display: install xvfb".into()),
    );
    let machine = host.machine("b", None, true, true);
    let error = ToolError::from(machine.computer.unwrap().screenshot().await.unwrap_err());
    assert_eq!(error.code, code::NO_DISPLAY);
    assert!(error.message.contains("install xvfb"));
    assert!(
        machine
            .browser
            .unwrap()
            .open("https://a.example/")
            .await
            .is_ok()
    );
}

struct PipeSpawner {
    playground: Arc<dyn Playground>,
    spawned: AtomicUsize,
}

#[async_trait]
impl Spawner for PipeSpawner {
    async fn spawn(&self) -> Result<Transport> {
        self.spawned.fetch_add(1, Ordering::SeqCst);
        let (agent_end, worker_end) = duplex(1 << 20);
        let (worker_read, worker_write) = tokio::io::split(worker_end);
        tokio::spawn(serve(worker_read, worker_write, self.playground.clone()));
        let (agent_read, agent_write) = tokio::io::split(agent_end);
        Ok(Transport {
            reader: Box::new(agent_read),
            writer: Box::new(agent_write),
            guard: Box::new(()),
        })
    }
}

fn remote(playground: Arc<FakePlayground>) -> (Arc<RemotePlayground>, Arc<PipeSpawner>) {
    let spawner = Arc::new(PipeSpawner {
        playground,
        spawned: AtomicUsize::new(0),
    });
    (RemotePlayground::new(spawner.clone()), spawner)
}

#[tokio::test]
async fn interaction_feedback_survives_worker_transport_and_viewer_frames() {
    let observation = InteractionObservation {
        captured_at_ms: 100,
        width: 64,
        height: 40,
        pointer: Some(InteractionPointer {
            position: InteractionPoint { x: 0.7, y: 0.3 },
            source: PointerSource::Observed,
        }),
        focus: None,
        action: Some(InteractionAction {
            kind: "click".into(),
            phase: InteractionPhase::Completed,
            at_ms: 90,
            target: Some(InteractionPoint { x: 0.6, y: 0.3 }),
            target_bounds: None,
        }),
        mapping: None,
    };
    let playground = Arc::new(FakePlayground::default());
    *playground.screen.observation.lock().unwrap() = Some(observation.clone());
    *playground.browser.observation.lock().unwrap() = Some(observation.clone());
    *playground.shot.lock().unwrap() = Some(Shot {
        png: png(64, 40),
        kind: FrameKind::Computer,
        caption: "Screen".into(),
        observation: Some(observation.clone()),
    });
    let (remote, _) = remote(playground);
    let remote: Arc<dyn Playground> = Arc::new(remote);
    assert_eq!(
        remote.computer().screenshot().await.unwrap().observation,
        Some(observation.clone())
    );
    assert_eq!(
        remote.browser(None).capture().await.unwrap().observation,
        Some(observation.clone())
    );
    assert_eq!(
        remote.browser(None).observation().await.unwrap(),
        Some(observation.clone())
    );
    let shot = remote.capture().await.unwrap().unwrap();
    let frame = Frame::from_shot(&shot).unwrap();
    assert_eq!(frame.observation, Some(observation));
}

#[tokio::test]
async fn handback_requires_each_surface_to_be_observed_even_after_repeated_actions() {
    let host = local_ish(Arc::default());
    let machine = host.machine("b", None, true, true);
    let computer = machine.computer.unwrap();
    let browser = machine.browser.unwrap();
    host.take_over("b").unwrap();
    host.hand_back("b").unwrap();
    let action = ComputerAction::Key {
        chord: "Enter".into(),
    };
    for _ in 0..2 {
        assert_eq!(
            ToolError::from(computer.act(&action).await.unwrap_err()).code,
            code::STALE_PAGE
        );
    }
    computer.windows().await.unwrap();
    assert!(computer.act(&action).await.is_err());
    computer.screenshot().await.unwrap();
    computer.act(&action).await.unwrap();
    let step = BrowserStep::Click {
        reference: "e1".into(),
    };
    assert_eq!(
        ToolError::from(browser.act(&step).await.unwrap_err()).code,
        code::STALE_PAGE
    );
    browser
        .snapshot(SnapshotRequest {
            max_chars: 100,
            interactive_only: false,
        })
        .await
        .unwrap();
    browser.act(&step).await.unwrap();
}

#[tokio::test]
async fn the_worker_protocol_round_trips_browser_and_computer_calls() {
    let playground = Arc::new(FakePlayground::default());
    let (remote, spawner) = remote(playground.clone());
    let remote: Arc<dyn Playground> = Arc::new(remote);

    let browser = remote.browser(Some("conv"));
    assert!(
        browser.current().await.unwrap().is_none(),
        "asking before any call starts nothing"
    );
    assert_eq!(spawner.spawned.load(Ordering::SeqCst), 0);

    assert_eq!(
        browser.open("https://a.example/").await.unwrap().url,
        "https://a.example/"
    );
    let view = browser
        .snapshot(SnapshotRequest {
            max_chars: 77,
            interactive_only: true,
        })
        .await
        .unwrap();
    assert_eq!((view.text.as_str(), view.truncated), ("limit 77", true));
    let step = BrowserStep::Type {
        reference: "e2".into(),
        text: "hi".into(),
    };
    browser.act(&step).await.unwrap();
    assert_eq!(playground.browser.steps.lock().unwrap()[0], step);
    assert_eq!(browser.screenshot().await.unwrap(), png(20, 10));
    let files = browser.take_downloads().await.unwrap();
    assert_eq!(
        (files[0].name.as_str(), &files[0].bytes[..]),
        ("a.txt", &b"hello"[..])
    );

    let computer = remote.computer();
    let screen = computer.screenshot().await.unwrap();
    assert_eq!(
        (screen.width, screen.height, screen.png),
        (64, 40, png(64, 40))
    );
    let action = ComputerAction::Click {
        x: 1.0,
        y: 2.0,
        button: PointerButton::Right,
        count: 2,
    };
    assert_eq!(computer.act(&action).await.unwrap(), "did it");
    assert_eq!(playground.screen.actions.lock().unwrap()[0], action);
    assert_eq!(computer.windows().await.unwrap()[0].title, "Term");
    let bounds = WindowBounds {
        x: Some(9),
        ..Default::default()
    };
    assert_eq!(
        computer
            .window("Term", WindowOp::Move, bounds)
            .await
            .unwrap(),
        "Move Some(9)"
    );
    assert_eq!(spawner.spawned.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn error_codes_cross_the_pipe_and_unsupported_methods_stay_unsupported() {
    let (remote, _) = remote(Arc::default());
    let remote: Arc<dyn Playground> = Arc::new(remote);
    let computer = remote.computer();
    let error = ToolError::from(
        computer
            .window("gone", WindowOp::Focus, WindowBounds::default())
            .await
            .unwrap_err(),
    );
    assert_eq!(error.code, code::NOT_FOUND);
    let error = ToolError::from(computer.launch("x", &[]).await.unwrap_err());
    assert_eq!(error.code, code::NOT_SUPPORTED);
}

#[tokio::test]
async fn capture_and_input_cross_the_pipe_and_stop_ends_the_worker_connection() {
    let playground = Arc::new(FakePlayground::default());
    *playground.shot.lock().unwrap() = Some(Shot {
        png: png(4, 4),
        kind: FrameKind::Computer,
        caption: "Screen".into(),
        observation: None,
    });
    let (remote, spawner) = remote(playground.clone());
    let remote: Arc<dyn Playground> = Arc::new(remote);
    assert!(
        remote.capture().await.unwrap().is_none(),
        "no worker, nothing to capture, nothing started"
    );
    assert_eq!(spawner.spawned.load(Ordering::SeqCst), 0);

    remote.computer().screenshot().await.unwrap();
    let shot = remote.capture().await.unwrap().unwrap();
    assert_eq!(
        (shot.kind, shot.caption.as_str()),
        (FrameKind::Computer, "Screen")
    );
    let event = InputEvent::Key {
        chord: "Enter".into(),
    };
    remote.inject(&event, 10, 20).await.unwrap();
    assert_eq!(playground.injected.lock().unwrap()[0], (event, 10, 20));

    remote.stop().await;
    assert!(playground.stops.load(Ordering::SeqCst) >= 1);
    remote.computer().screenshot().await.unwrap();
    assert_eq!(
        spawner.spawned.load(Ordering::SeqCst),
        2,
        "the next call starts a new worker"
    );
}

#[tokio::test]
async fn a_dead_worker_is_a_clear_error_and_the_next_call_respawns() {
    struct Dying;
    #[async_trait]
    impl Spawner for Dying {
        async fn spawn(&self) -> Result<Transport> {
            let (agent_end, worker_end) = duplex(1024);
            drop(worker_end);
            let (r, w) = tokio::io::split(agent_end);
            Ok(Transport {
                reader: Box::new(r),
                writer: Box::new(w),
                guard: Box::new(()),
            })
        }
    }
    let crashing = RemotePlayground::new(Arc::new(Dying));
    let crashing: Arc<dyn Playground> = Arc::new(crashing);
    let error = crashing.computer().screenshot().await.unwrap_err();
    assert!(
        format!("{error:#}").contains("playground worker stopped"),
        "{error:#}"
    );
    let again = crashing.computer().screenshot().await.unwrap_err();
    assert!(format!("{again:#}").contains("playground worker stopped"));
}

#[tokio::test]
async fn session_mode_workers_share_the_desk_between_bots() {
    let own = MachineHost::with_display(DisplayKind::Session, |_| {
        Arc::new(FakePlayground::default()) as Arc<dyn Playground>
    });
    let a = own.machine("a", None, false, true).computer.unwrap();
    let b = own.machine("b", None, false, true).computer.unwrap();
    a.screenshot().await.unwrap();
    let waiting = tokio::time::timeout(Duration::from_millis(300), b.screenshot()).await;
    assert!(waiting.is_err(), "bot b queues while a holds the desk");
    own.stop("a").await;
    assert!(b.screenshot().await.is_ok());
}
