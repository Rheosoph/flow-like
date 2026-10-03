use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use flow_like_bots::StreamEvent;
use flow_like_bots::reply::{Answer, Link, MAX_TEXT_BYTES, ReplySink, ReplyStream, SendError};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
struct Shown {
    answer: Answer,
    done: bool,
    at: Instant,
}

struct Sink {
    shows: Arc<Mutex<Vec<Shown>>>,
    interval: Duration,
    /// Answers for the next calls; `Ok` once they are used up.
    script: Vec<Result<(), SendError>>,
}

#[async_trait]
impl ReplySink for Sink {
    async fn show(&mut self, answer: &Answer, done: bool) -> Result<(), SendError> {
        self.shows.lock().unwrap().push(Shown {
            answer: answer.clone(),
            done,
            at: Instant::now(),
        });
        if self.script.is_empty() {
            Ok(())
        } else {
            self.script.remove(0)
        }
    }

    fn edit_interval(&self, _shown: u32) -> Duration {
        self.interval
    }
}

fn partial(content: &str) -> StreamEvent {
    StreamEvent::new(
        "chat_stream_partial",
        json!({"chunk":{"id":"c1","choices":[{"index":0,"delta":{"role":"assistant","content":content}}]},
               "actions":[],"attachments":[],"plan":null,"widgets":[]}),
    )
}

fn planned(current: u32) -> StreamEvent {
    StreamEvent::new(
        "chat_stream_partial",
        json!({"chunk":null,"actions":[],"attachments":[],
               "plan":{"plan":[[0,"Think"],[1,"Answer"]],"current_step":current,"current_message":"reading"},
               "widgets":[]}),
    )
}

fn response(kind: &str, content: &str) -> StreamEvent {
    StreamEvent::new(
        kind,
        json!({"response":{"choices":[{"index":0,"finish_reason":"stop",
                 "message":{"role":"assistant","content":content,"tool_calls":[]}}],
                 "usage":{"completion_tokens":1,"prompt_tokens":1,"total_tokens":2}},
               "local_session":{},"global_session":{},"actions":[],"attachments":[],"model_id":null,"widgets":[]}),
    )
}

async fn follow(
    events: Vec<(u64, StreamEvent)>,
    interval: Duration,
    script: Vec<Result<(), SendError>>,
) -> (Vec<Shown>, bool, bool) {
    let shows = Arc::new(Mutex::new(Vec::new()));
    let sink = Sink {
        shows: shows.clone(),
        interval,
        script,
    };
    let (sender, receiver) = mpsc::channel(64);
    let done = CancellationToken::new();
    let run = CancellationToken::new();
    let following =
        tokio::spawn(ReplyStream::new(Box::new(sink)).follow(receiver, done.clone(), run.clone()));
    for (wait, event) in events {
        tokio::time::sleep(Duration::from_millis(wait)).await;
        sender.send(event).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    done.cancel();
    let asked = following.await.unwrap();
    let shows = shows.lock().unwrap().clone();
    (shows, asked, run.is_cancelled())
}

fn finals(shows: &[Shown]) -> Vec<&Shown> {
    shows.iter().filter(|shown| shown.done).collect()
}

#[tokio::test(start_paused = true)]
async fn edits_are_throttled_and_the_final_text_comes_once() {
    let start = Instant::now();
    let events: Vec<(u64, StreamEvent)> = (0..30)
        .map(|index| (100, partial(&format!("{index} "))))
        .collect();
    let (shows, asked, cancelled) = follow(events, Duration::from_secs(1), vec![]).await;
    assert!(!asked && !cancelled);
    let edits: Vec<&Shown> = shows.iter().filter(|shown| !shown.done).collect();
    assert!(
        edits.len() >= 3 && edits.len() <= 4,
        "{} edits in 3 s",
        edits.len()
    );
    for pair in edits.windows(2) {
        assert!(pair[1].at - pair[0].at >= Duration::from_secs(1));
    }
    assert!(edits[0].at - start <= Duration::from_millis(150));
    let finals = finals(&shows);
    assert_eq!(finals.len(), 1);
    let expected: String = (0..30).map(|index| format!("{index} ")).collect();
    assert_eq!(finals[0].answer.text, expected);
    assert!(finals[0].answer.plan.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_final_text_without_push_response() {
    let (shows, _, _) = follow(
        vec![(0, partial("Grüße ")), (10, partial("👋 東京"))],
        Duration::from_secs(1),
        vec![],
    )
    .await;
    let finals = finals(&shows);
    assert_eq!(finals.len(), 1);
    assert_eq!(finals[0].answer.text, "Grüße 👋 東京");
}

#[tokio::test(start_paused = true)]
async fn push_response_and_chat_out_set_the_text() {
    let (shows, _, _) = follow(
        vec![
            (0, partial("draft")),
            (10, response("chat_stream", "Hello there")),
        ],
        Duration::from_secs(1),
        vec![],
    )
    .await;
    assert_eq!(finals(&shows)[0].answer.text, "Hello there");

    let (shows, _, _) = follow(
        vec![
            (0, partial("draft")),
            (10, response("chat_stream", "pushed")),
            (10, response("chat_out", "completed")),
        ],
        Duration::from_secs(1),
        vec![],
    )
    .await;
    let completed = finals(&shows);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].answer.text, "completed");

    let (shows, _, _) = follow(
        vec![(0, partial("kept")), (10, response("chat_out", ""))],
        Duration::from_secs(1),
        vec![],
    )
    .await;
    assert_eq!(finals(&shows)[0].answer.text, "kept");
}

#[tokio::test(start_paused = true)]
async fn the_plan_shows_while_running_and_not_at_the_end() {
    let (shows, _, _) = follow(
        vec![(0, planned(0)), (1_500, planned(1)), (10, partial("done"))],
        Duration::from_secs(1),
        vec![],
    )
    .await;
    let first = &shows[0];
    assert!(!first.done);
    assert_eq!(first.answer.plan.as_ref().unwrap().current_step, 0);
    assert!(shows.iter().any(|shown| {
        !shown.done
            && shown
                .answer
                .plan
                .as_ref()
                .is_some_and(|plan| plan.current_step == 1)
    }));
    let finals = finals(&shows);
    assert_eq!(finals.len(), 1);
    assert!(finals[0].answer.plan.is_none());
    assert_eq!(finals[0].answer.text, "done");
}

#[tokio::test(start_paused = true)]
async fn files_become_web_links_only() {
    let attachments = StreamEvent::new(
        "chat_stream_partial",
        json!({"chunk":null,"actions":[],"plan":null,"widgets":[],"attachments":[
            "https://example.com/a.png",
            {"url":"https://example.com/r.pdf","name":"Report","preview_text":null,"thumbnail_url":null,
             "size":null,"type":null,"anchor":null,"page":null},
            "data:image/png;base64,AAAA",
            {"url":"file:///etc/passwd"},
            "https://example.com/a.png"]}),
    );
    let (shows, _, _) = follow(
        vec![(0, partial("see")), (10, attachments)],
        Duration::from_secs(1),
        vec![],
    )
    .await;
    assert_eq!(
        finals(&shows)[0].answer.links,
        [
            Link {
                url: "https://example.com/a.png".into(),
                name: None
            },
            Link {
                url: "https://example.com/r.pdf".into(),
                name: Some("Report".into())
            }
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_question_cancels_the_run() {
    let (shows, asked, cancelled) = follow(
        vec![
            (0, partial("one moment")),
            (
                10,
                StreamEvent::new("interaction_request", json!({"id":"q1"})),
            ),
        ],
        Duration::from_secs(1),
        vec![],
    )
    .await;
    assert!(asked && cancelled);
    assert_eq!(finals(&shows).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_final_answer_waits_out_retry_after() {
    let (shows, _, _) = follow(
        vec![(0, partial("hi"))],
        Duration::from_secs(1),
        vec![Ok(()), Err(SendError::RetryAfter(Duration::from_secs(3)))],
    )
    .await;
    let finals = finals(&shows);
    assert_eq!(finals.len(), 2);
    assert!(finals[1].at - finals[0].at >= Duration::from_secs(3));
}

#[tokio::test(start_paused = true)]
async fn a_runaway_answer_is_cut_on_a_character_boundary() {
    let sink = || {
        Box::new(Sink {
            shows: Arc::new(Mutex::new(Vec::new())),
            interval: Duration::from_secs(1),
            script: vec![],
        })
    };
    let mut stream = ReplyStream::new(sink());
    for _ in 0..30_000 {
        stream.take(&partial("Grüße👋"));
    }
    let text = &stream.answer().text;
    assert!(text.len() <= MAX_TEXT_BYTES);
    assert!(text.len() > MAX_TEXT_BYTES - 16);
    assert!(text.ends_with('…'));
    stream.take(&partial("more"));
    assert!(stream.answer().text.ends_with('…'));

    let mut pushed = ReplyStream::new(sink());
    pushed.take(&response("chat_stream", &"東".repeat(MAX_TEXT_BYTES)));
    let text = &pushed.answer().text;
    assert!(text.len() <= MAX_TEXT_BYTES && text.ends_with('…'));
}

#[tokio::test(start_paused = true)]
async fn nothing_is_sent_for_a_silent_run() {
    let (shows, asked, _) = follow(
        vec![(0, StreamEvent::new("chat_local_session", Value::Null))],
        Duration::from_secs(1),
        vec![],
    )
    .await;
    assert!(shows.is_empty() && !asked);
    let mut stream = ReplyStream::new(Box::new(Sink {
        shows: Arc::new(Mutex::new(Vec::new())),
        interval: Duration::from_secs(1),
        script: vec![],
    }));
    assert!(stream.take(&StreamEvent::new("chat_stream_partial", json!("garbage"))));
    assert!(stream.take(&StreamEvent::new("chat_stream", json!({"response": 7}))));
    assert!(stream.answer().is_empty());
}
