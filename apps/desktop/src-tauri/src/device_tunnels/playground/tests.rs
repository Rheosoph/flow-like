use super::*;
use flow_like_device_protocol::{PlaygroundFrameMeta, PlaygroundInput};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::DuplexStream;

#[derive(Default)]
struct FakeConnector {
    connects: AtomicUsize,
    devices: Mutex<Vec<DuplexStream>>,
}

impl Connector for Arc<FakeConnector> {
    fn connect(&self, _: &str, _: &str) -> Connecting {
        let this = self.clone();
        Box::pin(async move {
            this.connects.fetch_add(1, Ordering::SeqCst);
            let (near, far) = tokio::io::duplex(1 << 20);
            guard(&this.devices).push(far);
            Ok(Box::new(near) as Box<dyn Duplex>)
        })
    }
}

fn sessions() -> (Arc<Sessions<Arc<FakeConnector>>>, Arc<FakeConnector>) {
    let connector = Arc::new(FakeConnector::default());
    (Sessions::new(connector.clone()), connector)
}

fn state(taken_over: bool, asleep: bool, display: PlaygroundDisplay) -> PlaygroundState {
    PlaygroundState {
        taken_over,
        asleep,
        display,
        kind: Some(PlaygroundFrameKind::Computer),
    }
}

fn frame(kind: PlaygroundFrameKind, age_ms: u64) -> PlaygroundFrame {
    PlaygroundFrame {
        meta: PlaygroundFrameMeta {
            width: 1280,
            height: 800,
            kind,
            caption: "Desktop".into(),
            age_ms,
            observation: None,
        },
        jpeg: vec![0xff, 0xd8, 0xff],
    }
}

async fn settle<F: Fn() -> bool>(done: F) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition never held");
}

#[test]
fn a_frame_and_state_make_an_active_view_with_a_jpeg_data_url() {
    let (live, _controls) = Live::new();
    assert_eq!(live.view().state, ViewState::Asleep);
    live.apply(PlaygroundEvent::State(state(
        true,
        false,
        PlaygroundDisplay::Virtual,
    )));
    live.apply(PlaygroundEvent::Frame(frame(
        PlaygroundFrameKind::Computer,
        40,
    )));
    let view = live.view();
    assert_eq!(view.state, ViewState::Active);
    assert_eq!(view.kind, Some(ViewKind::Computer));
    assert!(view.taken_over && view.can_control);
    assert_eq!(view.image.as_deref(), Some("data:image/jpeg;base64,/9j/"));
    assert_eq!((view.width, view.height), (Some(1280), Some(800)));
    assert!(view.age_ms.unwrap() >= 40);
}

#[test]
fn browser_frames_and_displayless_devices_are_watch_only() {
    let (live, _controls) = Live::new();
    live.apply(PlaygroundEvent::State(state(
        false,
        false,
        PlaygroundDisplay::Virtual,
    )));
    live.apply(PlaygroundEvent::Frame(frame(
        PlaygroundFrameKind::Browser,
        0,
    )));
    let view = live.view();
    assert_eq!(view.kind, Some(ViewKind::Browser));
    assert!(!view.can_control);

    live.apply(PlaygroundEvent::State(state(
        false,
        false,
        PlaygroundDisplay::None,
    )));
    live.apply(PlaygroundEvent::Frame(frame(
        PlaygroundFrameKind::Computer,
        0,
    )));
    assert!(!live.view().can_control);
}

#[test]
fn an_asleep_bot_and_an_old_picture_show_no_image_but_keep_taken_over() {
    let (live, _controls) = Live::new();
    live.apply(PlaygroundEvent::Frame(frame(
        PlaygroundFrameKind::Computer,
        0,
    )));
    live.apply(PlaygroundEvent::State(state(
        true,
        true,
        PlaygroundDisplay::Session,
    )));
    let view = live.view();
    assert_eq!(view.state, ViewState::Asleep);
    assert!(view.image.is_none() && view.taken_over);

    live.apply(PlaygroundEvent::State(state(
        false,
        false,
        PlaygroundDisplay::Session,
    )));
    live.apply(PlaygroundEvent::Frame(frame(
        PlaygroundFrameKind::Computer,
        ACTIVE_FOR.as_millis() as u64 + 1_000,
    )));
    assert_eq!(live.view().state, ViewState::Asleep);
}

#[test]
fn a_refusal_reaches_the_panel_once() {
    let (live, _controls) = Live::new();
    live.apply(PlaygroundEvent::Error {
        code: "input_refused".into(),
        message: "watch only".into(),
    });
    assert_eq!(
        live.view().notice.as_deref(),
        Some("input_refused: watch only")
    );
    assert_eq!(live.view().notice, None);
}

#[test]
fn a_view_nobody_polls_expires_after_the_lease() {
    let (live, _controls) = Live::new();
    let now = Instant::now();
    assert!(!live.expired_at(now + LEASE - Duration::from_secs(1)));
    assert!(live.expired_at(now + LEASE + Duration::from_secs(1)));
    live.view();
    assert!(!live.expired_at(Instant::now() + LEASE - Duration::from_secs(1)));
    assert!(live.expired_at(Instant::now() + LEASE + Duration::from_secs(1)));
}

#[test]
fn an_ended_view_refuses_controls_with_its_reason() {
    let (live, _controls) = Live::new();
    live.end("the device closed the stream");
    let error = live.send(PlaygroundControl::Stop).unwrap_err();
    assert!(error.contains("the device closed the stream"));
}

#[tokio::test]
async fn one_stream_per_bot_carries_views_and_controls() {
    let (sessions, connector) = sessions();
    let first = sessions.view("dev", "bot").await.unwrap();
    assert_eq!(first.state, ViewState::Asleep);
    sessions.view("dev", "bot").await.unwrap();
    assert_eq!(connector.connects.load(Ordering::SeqCst), 1);

    let mut device = guard(&connector.devices).pop().unwrap();
    device
        .write_all(
            &PlaygroundEvent::State(state(false, false, PlaygroundDisplay::Virtual))
                .encode()
                .unwrap(),
        )
        .await
        .unwrap();
    device
        .write_all(
            &PlaygroundEvent::Frame(frame(PlaygroundFrameKind::Computer, 0))
                .encode()
                .unwrap(),
        )
        .await
        .unwrap();
    settle(|| {
        let live = guard(&sessions.entries).values().next().unwrap().clone();
        live.view().state == ViewState::Active
    })
    .await;

    sessions
        .control("dev", "bot", PlaygroundControl::TakeOver)
        .await
        .unwrap();
    sessions
        .control(
            "dev",
            "bot",
            PlaygroundControl::Input {
                event: PlaygroundInput::Text { text: "hi".into() },
            },
        )
        .await
        .unwrap();
    let mut decoder = PlaygroundDecoder::default();
    let mut seen = Vec::new();
    while seen.len() < 2 {
        let mut chunk = [0u8; 256];
        let count = device.read(&mut chunk).await.unwrap();
        decoder.push(&chunk[..count]);
        while let Some(control) = decoder.next_control().unwrap() {
            seen.push(control);
        }
    }
    assert_eq!(seen[0], PlaygroundControl::TakeOver);
    assert!(matches!(
        &seen[1],
        PlaygroundControl::Input {
            event: PlaygroundInput::Text { text }
        } if text == "hi"
    ));
}

#[tokio::test]
async fn a_stream_the_device_dropped_is_reopened_by_the_next_poll() {
    let (sessions, connector) = sessions();
    sessions.view("dev", "bot").await.unwrap();
    drop(guard(&connector.devices).pop());
    settle(|| guard(&sessions.entries).values().all(|live| live.ended())).await;
    sessions.view("dev", "bot").await.unwrap();
    assert_eq!(connector.connects.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_malformed_device_message_ends_the_view() {
    let (sessions, connector) = sessions();
    sessions.view("dev", "bot").await.unwrap();
    let mut device = guard(&connector.devices).pop().unwrap();
    device.write_all(&[99, 0, 0, 0, 0]).await.unwrap();
    settle(|| guard(&sessions.entries).values().all(|live| live.ended())).await;
    let live = guard(&sessions.entries).values().next().unwrap().clone();
    assert!(
        live.send(PlaygroundControl::Stop)
            .unwrap_err()
            .contains("malformed")
    );
}

#[tokio::test]
async fn closing_a_device_ends_only_its_views_and_the_number_of_views_is_capped() {
    let (sessions, _connector) = sessions();
    sessions.view("one", "bot").await.unwrap();
    sessions.view("two", "bot").await.unwrap();
    let one = guard(&sessions.entries)[&("one".to_owned(), "bot".to_owned())].clone();
    let two = guard(&sessions.entries)[&("two".to_owned(), "bot".to_owned())].clone();
    sessions.close_device("one");
    assert!(one.ended() && !two.ended());

    for index in 0..MAX_VIEWS {
        let _ = sessions.view("many", &format!("bot{index}")).await;
    }
    let refused = sessions.view("many", "one-too-many").await.unwrap_err();
    assert!(refused.contains("At most"));
    sessions.close_all();
    assert!(two.ended());
}

#[test]
fn the_inspection_names_the_playground_flag() {
    let response = |state: &str, result: serde_json::Value| ManagementResponse {
        operation_id: "op".into(),
        state: state.into(),
        result,
    };
    let with = response(
        "completed",
        serde_json::json!({"features":{"bot_playground":1}}),
    );
    let without = response(
        "completed",
        serde_json::json!({"features":{"model_host":1}}),
    );
    let denied = response("rejected", serde_json::json!({"code":"unauthorized"}));
    let broken = response(
        "rejected",
        serde_json::json!({"code":"other","error":"boom"}),
    );
    assert_eq!(inspected_flag(&with), Ok(Some(true)));
    assert_eq!(inspected_flag(&without), Ok(Some(false)));
    assert_eq!(inspected_flag(&denied), Ok(None));
    assert!(inspected_flag(&broken).unwrap_err().contains("boom"));
}

#[test]
fn observations_travel_with_their_image_and_are_cleared_for_an_asleep_view() {
    let (live, _controls) = Live::new();
    let mut picture = frame(PlaygroundFrameKind::Computer, 40);
    let observation = InteractionObservation {
        captured_at_ms: 1000,
        width: 2560,
        height: 1600,
        pointer: None,
        focus: None,
        action: None,
        mapping: None,
    };
    picture.meta.observation = Some(observation.clone());
    live.apply(PlaygroundEvent::Frame(picture));
    assert_eq!(live.view().observation, Some(observation));
    live.apply(PlaygroundEvent::State(state(
        false,
        true,
        PlaygroundDisplay::Session,
    )));
    assert!(live.view().observation.is_none());
}

#[test]
fn the_desktop_requests_observations_when_opening_a_view() {
    assert!(matches!(
        open_of("bot_1").target,
        TunnelTarget::BotPlayground {
            observations: true,
            ..
        }
    ));
}
