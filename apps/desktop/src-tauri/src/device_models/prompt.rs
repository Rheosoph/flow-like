use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::{sync::watch, time::Instant};

use super::link::{PromptHost, UnlockPrompt};

/// Declines are remembered this long, which outlasts any run that asks.
const DECLINE_MEMORY: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_DECLINES: usize = 1024;

/// A run that wants a locked device. Without a run label nothing is remembered.
pub(crate) struct Question {
    pub(crate) device_id: String,
    pub(crate) device_name: Option<String>,
    pub(crate) model_name: String,
    pub(crate) run_label: Option<String>,
}

/// One prompt per device at a time: runs that want the same locked device wait for its answer.
/// A decline, a timeout or a missing window holds for the rest of each waiting run, which is
/// not asked again for any device, so a run waits for at most one unanswered prompt.
pub(crate) struct Prompts<H> {
    host: H,
    timeout: Duration,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    pending: HashMap<String, Pending>,
    declined: HashMap<String, Instant>,
}

struct Pending {
    prompt: UnlockPrompt,
    runs: Vec<String>,
    answer: watch::Sender<Option<bool>>,
}

enum Joined {
    Declined,
    Waiting(watch::Receiver<Option<bool>>),
    Nobody,
}

impl Pending {
    fn join(&mut self, run_label: Option<String>) -> watch::Receiver<Option<bool>> {
        if let Some(run) = run_label
            && !self.runs.contains(&run)
        {
            self.runs.push(run);
        }
        self.answer.subscribe()
    }
}

impl State {
    fn is_declined(&self, run_label: Option<&str>) -> bool {
        run_label.is_some_and(|run| self.declined.contains_key(run))
    }

    fn decline(&mut self, run: &str) {
        let now = Instant::now();
        self.declined
            .retain(|_, at| now.duration_since(*at) < DECLINE_MEMORY);
        if self.declined.len() >= MAX_DECLINES
            && let Some(oldest) = self
                .declined
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(key, _)| key.clone())
        {
            self.declined.remove(&oldest);
        }
        self.declined.insert(run.to_owned(), now);
    }
}

impl<H: PromptHost> Prompts<H> {
    pub(crate) fn new(host: H, timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            host,
            timeout,
            state: Mutex::default(),
        })
    }

    pub(crate) fn host(&self) -> &H {
        &self.host
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// True once the device was unlocked; false for a decline, a timeout or no window to ask in.
    pub(crate) async fn ask(self: &Arc<Self>, question: Question) -> bool {
        let receiver = match self.join(&question) {
            Joined::Declined => return false,
            Joined::Waiting(receiver) => receiver,
            Joined::Nobody if !self.host.can_prompt() => {
                if let Some(run) = &question.run_label {
                    self.state().decline(run);
                }
                return false;
            }
            Joined::Nobody => self.open(question),
        };
        wait(receiver).await
    }

    fn join(&self, question: &Question) -> Joined {
        let mut state = self.state();
        if state.is_declined(question.run_label.as_deref()) {
            return Joined::Declined;
        }
        match state.pending.get_mut(&question.device_id) {
            Some(pending) => Joined::Waiting(pending.join(question.run_label.clone())),
            None => Joined::Nobody,
        }
    }

    fn open(self: &Arc<Self>, question: Question) -> watch::Receiver<Option<bool>> {
        let run_name = question
            .run_label
            .as_deref()
            .and_then(|run| self.host.run_name(run));
        let prompt = UnlockPrompt {
            id: uuid::Uuid::new_v4().to_string(),
            device_id: question.device_id,
            device_name: question.device_name,
            model_name: question.model_name,
            run_id: question.run_label.clone(),
            run_name,
            expires_at: unix_millis().saturating_add(self.timeout.as_millis() as u64),
        };
        let receiver = {
            let mut state = self.state();
            if let Some(pending) = state.pending.get_mut(&prompt.device_id) {
                return pending.join(question.run_label);
            }
            let (answer, receiver) = watch::channel(None);
            state.pending.insert(
                prompt.device_id.clone(),
                Pending {
                    prompt: prompt.clone(),
                    runs: question.run_label.into_iter().collect(),
                    answer,
                },
            );
            receiver
        };
        self.host.show(&prompt);
        let prompts = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(prompts.timeout).await;
            prompts.settle(&prompt.device_id, Some(&prompt.id), false);
        });
        receiver
    }

    /// Answers the prompt of the device, or only the prompt `prompt_id` when given; false when
    /// that prompt already ended.
    fn settle(&self, device_id: &str, prompt_id: Option<&str>, unlocked: bool) -> bool {
        let pending = {
            let mut state = self.state();
            let current = state
                .pending
                .get(device_id)
                .is_some_and(|pending| prompt_id.is_none_or(|id| pending.prompt.id == id));
            let Some(pending) = current.then(|| state.pending.remove(device_id)).flatten() else {
                return false;
            };
            if !unlocked {
                for run in &pending.runs {
                    state.decline(run);
                }
            }
            pending
        };
        pending.answer.send_replace(Some(unlocked));
        self.host.close(&pending.prompt.id);
        true
    }

    /// The device is unlocked, so every prompt for it is answered.
    pub(crate) fn unlocked(&self, device_id: &str) {
        self.settle(device_id, None, true);
    }

    pub(crate) fn decline(&self, prompt_id: &str) -> bool {
        let device = self
            .state()
            .pending
            .iter()
            .find(|(_, pending)| pending.prompt.id == prompt_id)
            .map(|(device, _)| device.clone());
        device.is_some_and(|device| self.settle(&device, Some(prompt_id), false))
    }

    /// Open prompts, for a window that starts listening after they were shown.
    pub(crate) fn pending(&self) -> Vec<UnlockPrompt> {
        self.state()
            .pending
            .values()
            .map(|pending| pending.prompt.clone())
            .collect()
    }
}

async fn wait(mut receiver: watch::Receiver<Option<bool>>) -> bool {
    receiver
        .wait_for(Option::is_some)
        .await
        .is_ok_and(|answer| *answer == Some(true))
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
