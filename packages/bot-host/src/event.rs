//! Shared event triage for account, device and local hosts.

use crate::host::{CODE_SETUP, RunError, TurnJob, TurnRunner};
use flow_like::flow::copilot::{
    decision,
    decision_event::{self, EventDecision},
};
use flow_like_local_bots::InboxEntry;
use flow_like_types::inbox::{InboxEntryKind, TriggerBody};
use serde_json::json;
use std::sync::Arc;

pub fn attach_delivery(
    job: &mut TurnJob,
    delivery: decision_event::EventDelivery,
) -> Option<Arc<decision_event::EventRecovery>> {
    job.suppress_owner_notifications = true;
    let inner = job.recovery.take()?;
    let recovery = Arc::new(decision_event::EventRecovery::new(inner, delivery));
    job.recovery = Some(recovery.clone());
    Some(recovery)
}

pub async fn failed_execution(job: &TurnJob) -> bool {
    match &job.recovery {
        Some(recovery) => match recovery.load().await {
            Ok(checkpoint) => {
                checkpoint.is_some_and(|checkpoint| decision_event::failed_execution(&checkpoint))
            }
            Err(error) => {
                tracing::warn!(%error, "Event execution state could not be read; keeping the result visible");
                true
            }
        },
        None => false,
    }
}

pub fn trigger(entries: &[InboxEntry], message_id: &str) -> Option<TriggerBody> {
    let entry = entries
        .iter()
        .find(|entry| entry.message_id == message_id)?;
    (entry.kind == InboxEntryKind::Trigger)
        .then(|| TriggerBody::parse(entry.envelope.get("body")?).ok())
        .flatten()
}

pub fn baseline_note(previous: Option<&str>) -> String {
    match previous {
        Some(previous) => format!("Previous successful monitoring result, retained privately as comparison evidence:\n{}", decision::excerpt(previous, 4_000)),
        None => "No prior successful monitoring result is available. A missing baseline is not evidence that nothing changed. Establish the current state; stay quiet only when the current evidence itself shows no owner action or meaningful update is needed.".into(),
    }
}

pub async fn classify(
    runner: &dyn TurnRunner,
    job: &TurnJob,
    trigger: &TriggerBody,
    result: Option<&str>,
) -> Result<Option<EventDecision>, RunError> {
    let history: Vec<_> = job.history.iter().rev().take(12).rev()
        .map(|message| json!({"role":message.role, "text":decision::excerpt(&message.content, 1_000)}))
        .collect();
    let request = decision_event::request(
        trigger,
        json!({
            "bot_id":job.bot_id, "prompt":decision::excerpt(&job.prompt, 4_000), "history":history,
        }),
        result,
    )
    .map_err(|error| RunError::Failed {
        code: CODE_SETUP.into(),
        message: error.to_string(),
    })?;
    let Some(request) = request else {
        return Ok(None);
    };
    let Some(response) = runner.classify(job, &request).await? else {
        return Ok(None);
    };
    decision_event::parse(&request, &response)
        .map(Some)
        .map_err(|error| RunError::Failed {
            code: CODE_SETUP.into(),
            message: error.to_string(),
        })
}
