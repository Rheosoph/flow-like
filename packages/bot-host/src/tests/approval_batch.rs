use super::*;
use flow_like::flow::copilot::run_recovery::{CallScope, resumes_held_round, scope_call};
use flow_like_types::bot_turn::BotRunCheckpoint;
use flow_like_types::tool_grant::ONCE_PREFIX;

const CALLS: usize = 12;

fn fetch(n: usize) -> Value {
    json!({ "url": format!("https://cisa.gov/news-events/alerts?p={n}") })
}

/// The round of twelve web requests as the core saves it before any of them ran.
fn round_of_fetches() -> BotRunCheckpoint {
    let now = Utc::now().timestamp_millis();
    let prompt = "Read the twelve alert pages";
    BotRunCheckpoint {
        revision: 1,
        started_at_ms: now,
        deadline_at_ms: now + 60_000,
        state: json!({
            "version": 1,
            "phase": "tools_in_flight",
            "original_request": prompt,
            "history": [],
            "prompt": { "role": "user", "content": [{ "type": "text", "text": prompt }] },
            "reference_fingerprints": [],
            "work_plan": null,
            "steering": [],
            "iteration": 0,
            "model_calls_reserved": 1,
            "max_tool_rounds": 8,
            "plan_step_counter": 0,
            "empty_continuations": 0,
            "progress_seen": [],
            "stats": flow_like::flow_like_model_provider::response::LLMUsageStats::default(),
            "web": null,
            "delegated_web": null,
            "local_app_discovery_returned": false,
            "room_refreshes_remaining": 0,
            "recoveries": 0,
            "approval_resume": false,
            "pending_calls": (0..CALLS)
                .map(|n| json!({
                    "id": format!("fetch-{n}"),
                    "name": "web_request",
                    "arguments": fetch(n),
                    "output": null,
                }))
                .collect::<Vec<_>>(),
            "answer": null,
        }),
    }
}

/// Runs the saved round's twelve calls through the turn's gate, as the core's lanes do.
#[derive(Default)]
struct Fetcher {
    prompts: Mutex<Vec<String>>,
    ran: Mutex<Vec<usize>>,
}

#[async_trait]
impl TurnRunner for Fetcher {
    async fn run(&self, job: TurnJob) -> Result<String, RunError> {
        self.prompts.lock().unwrap().push(job.prompt.clone());
        let mut ran = 0;
        for n in 0..CALLS {
            let call = CallScope {
                id: format!("fetch-{n}"),
                round_web_content: false,
                held_replay: false,
            };
            let description = format!(
                "The bot wants to send a GET request to cisa.gov/news-events/alerts?p={n}."
            );
            let cleared = scope_call(
                call,
                job.gate.clear("web_request", &fetch(n), None, &description),
            )
            .await;
            ran += usize::from(cleared.is_ok());
        }
        self.ran.lock().unwrap().push(ran);
        Ok(if ran == CALLS {
            format!("Read {ran} pages")
        } else {
            String::new()
        })
    }

    fn cancel(&self, _: &str) {}
}

async fn until(what: &str, check: impl Fn() -> bool, hub: &DeviceHub) {
    for _ in 0..600 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for {what}: {:?}", hub.calls());
}

#[tokio::test]
async fn twelve_asking_calls_park_once_and_the_resume_runs_all_twelve() {
    let hub = Arc::new(DeviceHub {
        recovery_enabled: true,
        ..Default::default()
    });
    *hub.checkpoint.lock().unwrap() = Some(round_of_fetches());
    *hub.entries.lock().unwrap() = vec![plain("Read the twelve alert pages")];
    hub.turns.lock().unwrap().push(turn("bt_1"));
    let runner = Arc::new(Fetcher::default());
    let host = AccountHost::new(
        hub.clone(),
        runner.clone(),
        Arc::new(Quiet),
        Arc::new(Semaphore::new(2)),
        config(),
    );
    tokio::spawn(host.clone().run());
    let parked = || host.turns().first().map(|turn| turn.status) == Some(TurnStatus::Waiting);
    until("the park", parked, &hub).await;

    let cards = hub.cards.lock().unwrap().clone();
    assert_eq!(cards.len(), 1, "one card for the whole round");
    assert_eq!(cards[0].items.len(), CALLS);
    let saved = hub.checkpoint.lock().unwrap().clone().unwrap();
    assert!(
        resumes_held_round(&saved),
        "every call of the round is held"
    );
    assert_eq!(saved.state["approval_resume"], true);

    hub.grants
        .lock()
        .unwrap()
        .extend(cards[0].items.iter().map(|item| {
            (
                item.tool.clone(),
                format!("{ONCE_PREFIX}{}", item.args_hash),
            )
        }));
    hub.turns.lock().unwrap().push(turn("bt_1"));
    let replied = || host.turns().first().map(|turn| turn.status) == Some(TurnStatus::Done);
    until("the resumed reply", replied, &hub).await;

    assert_eq!(*runner.ran.lock().unwrap(), [0, CALLS]);
    let prompts = runner.prompts.lock().unwrap().clone();
    assert!(
        !prompts[1].contains("Earlier in this turn"),
        "a resumed held round runs its calls without the resume note: {}",
        prompts[1]
    );
    let parks = hub
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::Park(_)))
        .count();
    assert_eq!(parks, 1);
    let sent: Vec<Value> = hub
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Send(message) => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["envelope"]["body"]["text"], "Read 12 pages");
    assert!(
        hub.grants.lock().unwrap().is_empty(),
        "every answer is spent"
    );
}
