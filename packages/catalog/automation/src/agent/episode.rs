use super::computer_use::AgentStep;
use super::executor::{BatchOutcome, DesktopRunner, Terminal, describe_for_log, run_batch};
use super::history::{
    KEEP_OBSERVATIONS, PRUNE_TRIGGER, ToolCallRequest, assistant_text, image, prune_observations,
    text, tool_calls, tool_results, user,
};
use super::loop_guard::{Progress, Status, StuckDetector, exhausted, screen_changed};
use super::observe::{Observation, Perception, Shot, Target, ZoomView, observe, settle};
use super::prompt::{NUDGE, no_effect_note, observation_header};
use super::tools::ForbiddenText;
use crate::llm::truncate_on_char_boundary;
use crate::types::handles::AutomationSession;
use flow_like::flow::execution::{LogLevel, context::ExecutionContext};
use flow_like_model_provider::llm::CompletionModelHandle;
use flow_like_types::tokio_util::sync::CancellationToken;
use rig::OneOrMany;
use rig::message::{AssistantContent, Message, UserContent};
use std::time::{Duration, Instant};

pub(crate) type ModelAgent = rig::agent::Agent<CompletionModelHandle<'static>>;

const MAX_TEXT_REPLIES: u32 = 3;
const ASSISTANT_TEXT_LIMIT: usize = 2_000;
const RETRY_DELAY: Duration = Duration::from_secs(2);

pub(crate) struct Settings {
    pub perception: Perception,
    pub max_steps: u32,
    pub max_duration: Duration,
    pub max_actions: usize,
    pub settle: Duration,
    pub forbidden: ForbiddenText,
}

pub(crate) struct Outcome {
    pub status: Status,
    pub answer: String,
    pub steps: Vec<AgentStep>,
    pub final_shot: Shot,
}

struct State {
    messages: Vec<Message>,
    observation: Observation,
    steps: Vec<AgentStep>,
    detector: StuckDetector,
    text_replies: u32,
    started: Instant,
}

struct Finish {
    status: Status,
    answer: String,
    shot: Option<Shot>,
}

enum Turn {
    Continue,
    Finish(Finish),
}

struct Reply {
    choice: OneOrMany<AssistantContent>,
    message_id: Option<String>,
}

fn cancelled(context: &ExecutionContext) -> bool {
    context
        .get_cancellation_token()
        .is_some_and(|token| token.is_cancelled())
}

async fn cancellation(token: Option<CancellationToken>) {
    match token {
        Some(token) => token.cancelled().await,
        None => std::future::pending().await,
    }
}

fn budget_answer(status: Status, settings: &Settings, steps: usize) -> String {
    match status {
        Status::Timeout => format!(
            "Stopped after {} s without finishing the task",
            settings.max_duration.as_secs()
        ),
        _ => format!("Stopped after {steps} steps without finishing the task"),
    }
}

fn observation_content(observation: &Observation, zoom: Option<&ZoomView>) -> Vec<UserContent> {
    let mut parts = Vec::new();
    if let Some(elements) = &observation.element_text {
        parts.push(text(elements.clone()));
    }
    parts.push(image(&observation.image));
    if let Some(zoom) = zoom {
        parts.push(text(zoom.label.clone()));
        parts.push(image(&zoom.image));
    }
    parts
}

enum Attempt {
    Reply(Reply),
    TimedOut,
    Failed(String),
}

async fn attempt(
    agent: &ModelAgent,
    prompt: &Message,
    history: &[Message],
    remaining: Duration,
    token: Option<CancellationToken>,
) -> flow_like_types::Result<Attempt> {
    use rig::completion::Completion;

    let request = async {
        agent
            .completion(prompt.clone(), history.to_vec())
            .await?
            .send()
            .await
    };
    let result = tokio::select! {
        biased;
        _ = cancellation(token) => {
            return Err(flow_like_types::anyhow!("Execution was cancelled"));
        }
        result = tokio::time::timeout(remaining, request) => result,
    };
    Ok(match result {
        Err(_) => Attempt::TimedOut,
        Ok(Ok(response)) => Attempt::Reply(Reply {
            choice: response.choice,
            message_id: response.message_id,
        }),
        Ok(Err(error)) => Attempt::Failed(error.to_string()),
    })
}

/// Sends the conversation; the last message is the prompt. `None` when the time budget ran out
/// first. A failed request is retried once.
async fn ask(
    context: &mut ExecutionContext,
    agent: &ModelAgent,
    messages: &[Message],
    deadline: Instant,
) -> flow_like_types::Result<Option<Reply>> {
    let Some((prompt, history)) = messages.split_last() else {
        return Err(flow_like_types::anyhow!(
            "The computer use conversation is empty"
        ));
    };
    let token = context.get_cancellation_token();
    let mut retried = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        match attempt(agent, prompt, history, remaining, token.clone()).await? {
            Attempt::Reply(reply) => return Ok(Some(reply)),
            Attempt::TimedOut => return Ok(None),
            Attempt::Failed(error) if !retried => {
                retried = true;
                context.log_message(
                    &format!("Computer use model request failed, retrying once: {error}"),
                    LogLevel::Warn,
                );
                crate::rpa::branch::delay(context, RETRY_DELAY.min(remaining)).await?;
            }
            Attempt::Failed(error) => {
                return Err(flow_like_types::anyhow!("Model request failed: {}", error));
            }
        }
    }
}

fn finish_step(state: &mut State, started: Instant) {
    if let Some(step) = state.steps.last_mut() {
        step.duration_ms = started.elapsed().as_millis() as u64;
    }
}

async fn final_shot(
    context: &ExecutionContext,
    target: &Target,
    settle_for: Duration,
) -> Option<Shot> {
    let located = target.locate().await.ok()?;
    settle(context, located.display_index, settle_for)
        .await
        .ok()
}

fn record_reply(
    context: &mut ExecutionContext,
    state: &mut State,
    index: u32,
    reply: Reply,
) -> (Vec<ToolCallRequest>, String) {
    let calls = tool_calls(&reply.choice);
    let assistant = assistant_text(&reply.choice);
    state.messages.push(Message::Assistant {
        id: reply.message_id,
        content: reply.choice,
    });
    if !assistant.is_empty() {
        context.log_message(
            &format!("Computer use step {index} model note: {assistant}"),
            LogLevel::Debug,
        );
    }
    state.steps.push(AgentStep {
        index,
        observation: state.observation.summary(),
        assistant: truncate_on_char_boundary(&assistant, ASSISTANT_TEXT_LIMIT).to_string(),
        actions: Vec::new(),
        duration_ms: 0,
    });
    (calls, assistant)
}

/// A reply without tool calls: nudge the model, or give up after repeated text replies.
fn text_reply(state: &mut State, assistant: String) -> Turn {
    state.text_replies += 1;
    if state.text_replies < MAX_TEXT_REPLIES {
        state.messages.push(user(text(NUDGE), Vec::new()));
        return Turn::Continue;
    }
    let answer = if assistant.is_empty() {
        "The model stopped calling tools".to_string()
    } else {
        format!("The model stopped calling tools: {assistant}")
    };
    Turn::Finish(Finish {
        status: Status::Failed,
        answer,
        shot: None,
    })
}

async fn execute(
    context: &mut ExecutionContext,
    session: &AutomationSession,
    settings: &Settings,
    observation: &Observation,
    index: u32,
    calls: &[ToolCallRequest],
) -> (BatchOutcome, Option<ZoomView>) {
    let (batch, zoom) = {
        let mut runner = DesktopRunner::new(context, session, observation, &settings.forbidden);
        let batch = run_batch(
            calls,
            settings.max_actions,
            settings.perception.marks(),
            &mut runner,
        )
        .await;
        (batch, runner.zoom)
    };
    context.log_message(
        &format!(
            "Computer use step {index}: {}",
            describe_for_log(&batch.actions)
        ),
        LogLevel::Info,
    );
    for failed in batch.actions.iter().filter(|a| !a.ok && !a.skipped) {
        context.log_message(
            &format!(
                "Computer use step {index}: {} failed: {}",
                failed.tool, failed.message
            ),
            LogLevel::Debug,
        );
    }
    (batch, zoom)
}

fn terminal_finish(terminal: Terminal, shot: Option<Shot>) -> Finish {
    let (status, answer) = match terminal {
        Terminal::Done {
            success: true,
            answer,
        } => (Status::Done, answer),
        Terminal::Done {
            success: false,
            answer,
        } => (Status::Failed, answer),
        Terminal::AskUser { question } => (Status::NeedsInput, question),
    };
    Finish {
        status,
        answer,
        shot,
    }
}

/// Waits for the screen to settle, checks for progress and sends the next observation.
async fn next_observation(
    context: &ExecutionContext,
    target: &Target,
    settings: &Settings,
    state: &mut State,
    index: u32,
    acted: bool,
    zoom: Option<ZoomView>,
) -> flow_like_types::Result<Turn> {
    let located = target.locate().await?;
    let shot = settle(context, located.display_index, settings.settle).await?;
    let changed = screen_changed(&state.observation.shot.fingerprint, &shot.fingerprint);
    let progress = state.detector.record(acted, changed);
    if let Progress::Stuck(turns) = progress {
        return Ok(Turn::Finish(Finish {
            status: Status::Stuck,
            answer: format!(
                "Stopped: the actions of the last {turns} turns had no visible effect on the screen"
            ),
            shot: Some(shot),
        }));
    }
    let observation = observe(shot, &located, settings.perception).await?;
    let mut notes = Vec::new();
    if let Progress::NoEffect(turns) = progress {
        notes.push(no_effect_note(turns));
    }
    notes.extend(located.warning);
    let header = observation_header(
        &observation.summary(),
        Some(index),
        settings.max_steps.saturating_sub(index),
        &notes,
    );
    state.messages.push(user(
        text(header),
        observation_content(&observation, zoom.as_ref()),
    ));
    prune_observations(&mut state.messages, PRUNE_TRIGGER, KEEP_OBSERVATIONS);
    state.observation = observation;
    Ok(Turn::Continue)
}

async fn turn(
    context: &mut ExecutionContext,
    session: &AutomationSession,
    agent: &ModelAgent,
    target: &Target,
    settings: &Settings,
    state: &mut State,
) -> flow_like_types::Result<Turn> {
    session.ensure_active(context).await?;
    let started = Instant::now();
    let index = state.steps.len() as u32 + 1;
    let deadline = state.started + settings.max_duration;
    let Some(reply) = ask(context, agent, &state.messages, deadline).await? else {
        return Ok(Turn::Finish(Finish {
            status: Status::Timeout,
            answer: budget_answer(Status::Timeout, settings, state.steps.len()),
            shot: None,
        }));
    };
    let (calls, assistant) = record_reply(context, state, index, reply);
    if calls.is_empty() {
        finish_step(state, started);
        return Ok(text_reply(state, assistant));
    }
    state.text_replies = 0;

    let (batch, zoom) = execute(
        context,
        session,
        settings,
        &state.observation,
        index,
        &calls,
    )
    .await;
    let results = tool_results(&calls, &batch.actions);
    if let Some(step) = state.steps.last_mut() {
        step.actions = batch.actions;
    }
    let turn = match batch.terminal {
        Some(terminal) => {
            let shot = if batch.acted {
                final_shot(context, target, settings.settle).await
            } else {
                None
            };
            Turn::Finish(terminal_finish(terminal, shot))
        }
        None => {
            state.messages.extend(results);
            next_observation(context, target, settings, state, index, batch.acted, zoom).await?
        }
    };
    finish_step(state, started);
    Ok(turn)
}

fn opening(task: String, observation: &Observation, max_steps: u32) -> Message {
    let header = observation_header(&observation.summary(), None, max_steps, &[]);
    let mut parts = vec![text(header)];
    parts.extend(observation_content(observation, None));
    user(text(task), parts)
}

fn stopped_by(context: &mut ExecutionContext, error: flow_like_types::Error) -> Finish {
    context.log_message(
        &format!("Computer use agent stopped after an error: {error:#}"),
        LogLevel::Error,
    );
    Finish {
        status: Status::Failed,
        answer: format!("Stopped after an error: {error:#}"),
        shot: None,
    }
}

/// Runs observe → act → verify turns until the model finishes, a budget runs out or the
/// agent is stuck. Errors inside the loop end it as failed; only cancellation is an `Err`.
pub(crate) async fn run(
    context: &mut ExecutionContext,
    session: &AutomationSession,
    agent: &ModelAgent,
    target: &Target,
    settings: &Settings,
    task: String,
    observation: Observation,
) -> flow_like_types::Result<Outcome> {
    let mut state = State {
        messages: vec![opening(task, &observation, settings.max_steps)],
        observation,
        steps: Vec::new(),
        detector: StuckDetector::default(),
        text_replies: 0,
        started: Instant::now(),
    };
    let finish = loop {
        if cancelled(context) {
            return Err(flow_like_types::anyhow!("Execution was cancelled"));
        }
        let steps = state.steps.len();
        let elapsed = state.started.elapsed();
        if let Some(status) = exhausted(
            steps as u32,
            settings.max_steps,
            elapsed,
            settings.max_duration,
        ) {
            break Finish {
                status,
                answer: budget_answer(status, settings, steps),
                shot: None,
            };
        }
        match turn(context, session, agent, target, settings, &mut state).await {
            Ok(Turn::Continue) => {}
            Ok(Turn::Finish(finish)) => break finish,
            Err(error) if cancelled(context) => return Err(error),
            Err(error) => break stopped_by(context, error),
        }
    };
    Ok(Outcome {
        status: finish.status,
        answer: finish.answer,
        steps: state.steps,
        final_shot: finish.shot.unwrap_or(state.observation.shot),
    })
}
