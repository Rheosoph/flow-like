use super::*;
use flow_like_device_protocol::PlaygroundInput;
use std::sync::{Arc, Mutex};
use tokio::io::DuplexStream;

const PACE: Pace = Pace {
    frame: Duration::from_millis(10),
    refresh: Duration::from_millis(60),
};

#[derive(Default)]
struct Inner {
    frame: Option<LiveFrame>,
    taken: bool,
    calls: Vec<String>,
    refuse_input: bool,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Inner>>);

fn picture(bytes: &[u8]) -> LiveFrame {
    LiveFrame {
        jpeg: bytes.to_vec(),
        width: 640,
        height: 400,
        kind: PlaygroundFrameKind::Computer,
        caption: "Desktop".into(),
        taken: Instant::now(),
        observation: None,
    }
}

impl Fake {
    fn show(&self, bytes: &[u8]) {
        self.0.lock().unwrap().frame = Some(picture(bytes));
    }

    fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().calls.clone()
    }

    fn count(&self, name: &str) -> usize {
        self.calls()
            .iter()
            .filter(|call| call.as_str() == name)
            .count()
    }

    fn note(&self, call: impl Into<String>) {
        self.0.lock().unwrap().calls.push(call.into());
    }
}

impl Playground for Fake {
    fn latest(&self, _: &str) -> Option<LiveFrame> {
        self.0.lock().unwrap().frame.clone()
    }

    async fn refresh(&self, bot_id: &str) -> Option<LiveFrame> {
        self.note("refresh");
        self.latest(bot_id)
    }

    async fn capture(&self, bot_id: &str) -> Option<LiveFrame> {
        self.note("capture");
        self.latest(bot_id)
    }

    fn is_taken_over(&self, _: &str) -> bool {
        self.0.lock().unwrap().taken
    }

    fn take_over(&self, bot_id: &str) -> Result<()> {
        self.note(format!("take_over:{bot_id}"));
        self.0.lock().unwrap().taken = true;
        Ok(())
    }

    fn hand_back(&self, bot_id: &str) -> Result<()> {
        self.note(format!("hand_back:{bot_id}"));
        self.0.lock().unwrap().taken = false;
        Ok(())
    }

    async fn inject(&self, _: &str, event: PlaygroundInput) -> Result<()> {
        if self.0.lock().unwrap().refuse_input {
            anyhow::bail!("watch only");
        }
        self.note(format!("inject:{}", serde_json::to_string(&event).unwrap()));
        Ok(())
    }

    async fn stop(&self, bot_id: &str) {
        self.note(format!("stop:{bot_id}"));
        let mut inner = self.0.lock().unwrap();
        inner.taken = false;
        inner.frame = None;
    }

    fn display(&self) -> PlaygroundDisplay {
        PlaygroundDisplay::Virtual
    }
}

struct Viewer {
    io: DuplexStream,
    decoder: PlaygroundDecoder,
}

impl Viewer {
    async fn event(&mut self) -> PlaygroundEvent {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(event) = self.decoder.next_event().unwrap() {
                    return event;
                }
                let mut chunk = [0u8; 4096];
                let count = self.io.read(&mut chunk).await.unwrap();
                assert!(count > 0, "the device closed the stream");
                self.decoder.push(&chunk[..count]);
            }
        })
        .await
        .expect("an event within 5 s")
    }

    async fn frame(&mut self) -> PlaygroundFrame {
        loop {
            if let PlaygroundEvent::Frame(frame) = self.event().await {
                return frame;
            }
        }
    }

    async fn send(&mut self, control: PlaygroundControl) {
        self.io.write_all(&control.encode().unwrap()).await.unwrap();
    }
}

fn attach(host: Fake) -> (Viewer, tokio::task::JoinHandle<Result<()>>) {
    attach_observations(host, true)
}

fn attach_observations(
    host: Fake,
    observations: bool,
) -> (Viewer, tokio::task::JoinHandle<Result<()>>) {
    let (near, far) = tokio::io::duplex(256 * 1024);
    let task = tokio::spawn(serve(far, host, "bot_1".into(), observations, PACE));
    (
        Viewer {
            io: near,
            decoder: PlaygroundDecoder::default(),
        },
        task,
    )
}

#[tokio::test]
async fn a_screen_is_sent_once_until_it_changes() {
    let host = Fake::default();
    host.show(b"one");
    let (mut viewer, _task) = attach(host.clone());
    let PlaygroundEvent::State(state) = viewer.event().await else {
        panic!("state first");
    };
    assert!(!state.asleep && !state.taken_over);
    assert_eq!(state.kind, Some(PlaygroundFrameKind::Computer));
    assert_eq!(viewer.frame().await.jpeg, b"one");

    host.show(b"two");
    let next = viewer.frame().await;
    assert_eq!(next.jpeg, b"two");
    assert_eq!((next.meta.width, next.meta.height), (640, 400));
    assert_eq!(next.meta.caption, "Desktop");
}

#[tokio::test]
async fn a_bot_without_a_screen_is_asleep() {
    let (mut viewer, _task) = attach(Fake::default());
    let PlaygroundEvent::State(state) = viewer.event().await else {
        panic!("state first");
    };
    assert!(state.asleep);
    assert_eq!(state.display, PlaygroundDisplay::Virtual);
}

#[tokio::test]
async fn frames_follow_the_pace_and_idle_screens_are_refreshed() {
    let host = Fake::default();
    host.show(b"idle");
    let (mut viewer, task) = attach(host.clone());
    viewer.frame().await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let refreshes = host.count("refresh");
    assert!(
        (2..=6).contains(&refreshes),
        "an idle screen refreshes every ~60 ms here, saw {refreshes}"
    );
    assert_eq!(host.count("capture"), 0);
    task.abort();
}

#[tokio::test]
async fn control_maps_to_the_machine_host() {
    let host = Fake::default();
    host.show(b"x");
    let (mut viewer, _task) = attach(host.clone());
    viewer.event().await;

    viewer.send(PlaygroundControl::TakeOver).await;
    loop {
        if let PlaygroundEvent::State(state) = viewer.event().await
            && state.taken_over
        {
            break;
        }
    }
    while host.count("capture") == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    viewer
        .send(PlaygroundControl::Input {
            event: PlaygroundInput::Key {
                chord: "Enter".into(),
            },
        })
        .await;
    viewer.send(PlaygroundControl::HandBack).await;
    loop {
        if let PlaygroundEvent::State(state) = viewer.event().await
            && !state.taken_over
        {
            break;
        }
    }
    viewer.send(PlaygroundControl::Stop).await;
    loop {
        if let PlaygroundEvent::State(state) = viewer.event().await
            && state.asleep
        {
            break;
        }
    }
    let calls = host.calls();
    let position = |name: &str| calls.iter().position(|call| call.starts_with(name));
    assert!(position("take_over:bot_1") < position("inject:"));
    assert!(position("inject:") < position("hand_back:bot_1"));
    assert!(position("hand_back:bot_1") < position("stop:bot_1"));
    assert!(calls.iter().any(|call| call.contains(r#""chord":"Enter""#)));
    assert!(
        host.count("capture") > 0,
        "the screen is captured while the owner is in control"
    );
}

#[tokio::test]
async fn refused_input_is_reported_and_the_stream_stays_open() {
    let host = Fake::default();
    host.show(b"x");
    host.0.lock().unwrap().refuse_input = true;
    let (mut viewer, _task) = attach(host.clone());
    viewer
        .send(PlaygroundControl::Input {
            event: PlaygroundInput::Text { text: "hi".into() },
        })
        .await;
    loop {
        if let PlaygroundEvent::Error { code, message } = viewer.event().await {
            assert_eq!(code, "input_refused");
            assert!(message.contains("watch only"));
            break;
        }
    }
    host.show(b"y");
    assert_eq!(viewer.frame().await.jpeg, b"y");
}

#[tokio::test]
async fn a_viewer_that_leaves_in_control_hands_the_bot_back() {
    let host = Fake::default();
    host.show(b"x");
    let (mut viewer, task) = attach(host.clone());
    viewer.send(PlaygroundControl::TakeOver).await;
    loop {
        if let PlaygroundEvent::State(state) = viewer.event().await
            && state.taken_over
        {
            break;
        }
    }
    drop(viewer);
    task.await.unwrap().unwrap();
    assert_eq!(host.count("hand_back:bot_1"), 1);
    assert!(!host.is_taken_over("bot_1"));
}

#[tokio::test]
async fn a_malformed_message_ends_the_stream() {
    let (mut viewer, task) = attach(Fake::default());
    viewer
        .io
        .write_all(&[99, 0, 0, 0, 2, b'{', b'}'])
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the stream ends")
        .unwrap();
    assert!(result.is_err());
}

fn observation(at: u64) -> InteractionObservation {
    InteractionObservation {
        captured_at_ms: at,
        width: 640,
        height: 400,
        pointer: None,
        focus: None,
        action: None,
        mapping: None,
    }
}

#[tokio::test]
async fn changed_observations_arrive_even_when_the_pixels_do_not_change() {
    let host = Fake::default();
    host.show(b"same pixels");
    host.0.lock().unwrap().frame.as_mut().unwrap().observation = Some(observation(1000));
    let (mut viewer, task) = attach(host.clone());
    let first = viewer.frame().await;
    assert_eq!(first.meta.observation, Some(observation(1000)));
    host.0.lock().unwrap().frame.as_mut().unwrap().observation = Some(observation(2000));
    let next = viewer.frame().await;
    assert_eq!(next.jpeg, first.jpeg);
    assert_eq!(next.meta.observation, Some(observation(2000)));
    task.abort();
}

#[tokio::test]
async fn legacy_viewers_receive_no_observation_extension() {
    let host = Fake::default();
    host.show(b"legacy pixels");
    host.0.lock().unwrap().frame.as_mut().unwrap().observation = Some(observation(1000));
    let (mut viewer, task) = attach_observations(host, false);
    let frame = viewer.frame().await;
    assert!(frame.meta.observation.is_none());
    assert!(
        serde_json::to_value(&frame.meta)
            .unwrap()
            .get("observation")
            .is_none()
    );
    task.abort();
}
