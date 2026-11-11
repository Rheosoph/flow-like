//! A bounded observation from the same scheduled instruction, independent of optional memory.

use crate::{
    entity::{bot_turn, bot_turn_checkpoint, inbox_entry},
    error::ApiError,
};
use flow_like_types::{
    bot_turn::{BotRunCheckpoint, STORED_DONE, turn_id},
    inbox::{TriggerBody, TriggerSource},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

pub(crate) async fn is_monitoring<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
) -> Result<bool, sea_orm::DbErr> {
    let trigger = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(&turn.bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(&turn.conversation_id))
        .filter(inbox_entry::Column::MessageId.eq(&turn.trigger_message_id))
        .filter(inbox_entry::Column::Kind.eq("TRIGGER"))
        .one(db)
        .await?;
    Ok(trigger
        .and_then(|row| TriggerBody::parse(&row.envelope["body"]).ok())
        .is_some_and(|body| body.source == TriggerSource::Schedule && body.monitoring))
}

/// A run that stopped early, or whose tool calls ended in an error, says nothing reliable about
/// the monitored state and must not replace the last good baseline.
fn reported_failure(saved: &BotRunCheckpoint) -> bool {
    flow_like::flow::copilot::decision_event::failed_execution(saved)
        || saved.state["pending_calls"]
            .as_array()
            .is_some_and(|calls| calls.iter().any(call_failed))
}

fn call_failed(call: &serde_json::Value) -> bool {
    let Some(output) = call["output"].as_str() else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output) else {
        return false;
    };
    matches!(parsed["status"].as_str(), Some("error" | "failed"))
        || parsed["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
        || parsed["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty())
}

/// Encrypted triggers cannot prove their schedule identity here, and encrypted checkpoints
/// stay encrypted. Both cases leave the monitor without a prior observation.
pub(crate) async fn previous_result_for_turn<C: ConnectionTrait>(
    db: &C,
    current: &bot_turn::Model,
) -> Result<Option<String>, ApiError> {
    let Some(trigger) = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(&current.bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(&current.conversation_id))
        .filter(inbox_entry::Column::MessageId.eq(&current.trigger_message_id))
        .filter(inbox_entry::Column::Kind.eq("TRIGGER"))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let Some(seq) = trigger.seq else {
        return Ok(None);
    };
    if trigger.sender_id != current.owner_id || trigger.envelope["body"]["deleted"] == true {
        return Ok(None);
    }
    let Ok(body) = TriggerBody::parse(&trigger.envelope["body"]) else {
        return Ok(None);
    };
    if body.source != TriggerSource::Schedule || !body.monitoring {
        return Ok(None);
    }
    let candidates = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(&current.bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(&current.conversation_id))
        .filter(inbox_entry::Column::Kind.eq("TRIGGER"))
        .filter(inbox_entry::Column::SenderId.eq(&current.owner_id))
        .filter(inbox_entry::Column::Seq.lt(seq))
        .order_by_desc(inbox_entry::Column::Seq)
        .limit(64)
        .all(db)
        .await?;
    for candidate in candidates {
        if candidate.envelope["body"]["deleted"] == true {
            continue;
        }
        let Ok(previous) = TriggerBody::parse(&candidate.envelope["body"]) else {
            continue;
        };
        if previous.source != TriggerSource::Schedule
            || !previous.monitoring
            || previous.ref_id != body.ref_id
            || previous.text != body.text
        {
            continue;
        }
        let prior_id = turn_id(&current.bot_id, &candidate.message_id);
        let completed = bot_turn::Entity::find_by_id(&prior_id)
            .filter(bot_turn::Column::BotId.eq(&current.bot_id))
            .filter(bot_turn::Column::ConversationId.eq(&current.conversation_id))
            .filter(bot_turn::Column::OwnerId.eq(&current.owner_id))
            .filter(bot_turn::Column::Status.eq(STORED_DONE))
            .one(db)
            .await?;
        if completed.is_none() {
            continue;
        }
        let Some(row) = bot_turn_checkpoint::Entity::find_by_id(&prior_id)
            .one(db)
            .await?
        else {
            continue;
        };
        let saved = BotRunCheckpoint {
            revision: u64::try_from(row.revision).unwrap_or(0),
            started_at_ms: row.started_at_ms,
            deadline_at_ms: row.deadline_at_ms,
            state: row.state,
        };
        if saved.state.get("sealed_checkpoint").is_some() {
            return Ok(None);
        }
        if reported_failure(&saved) {
            continue;
        }
        if let Ok(Some(result)) = flow_like::flow::copilot::run_recovery::finished_reply(&saved)
            && !result.trim().is_empty()
        {
            return Ok(Some(result.chars().take(4_000).collect()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_host::{
        fixture::{at, conversation, database, delivery, entry, insert_bot, recipient},
        ledger,
        recovery::{self, Claim},
    };
    use flow_like_types::{bot::BotLocation, inbox::ConversationKind};
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, sea_query::Expr};
    use serde_json::json;

    async fn trigger(
        db: &DatabaseConnection,
        id: &str,
        task: &str,
        text: &str,
        seq: i64,
    ) -> bot_turn::Model {
        entry(db, "room", id, "user-a", seq, "TRIGGER", json!({
            "source":"schedule", "monitoring":true, "ref_id":task, "title":"Monitor", "text":text
        }), &["bot_1"]).await;
        let mut bot = recipient("bot_1", "user-a", BotLocation::Server);
        bot.conversation_id = "room".into();
        bot.trigger_message_id = id.into();
        ledger::enqueue_in(db, &delivery(ConversationKind::Direct, vec![bot]), at(seq))
            .await
            .unwrap();
        bot_turn::Entity::find_by_id(turn_id("bot_1", id))
            .one(db)
            .await
            .unwrap()
            .unwrap()
    }

    fn finished(output: &str) -> BotRunCheckpoint {
        BotRunCheckpoint {
            revision: 1,
            started_at_ms: 1000,
            deadline_at_ms: 31_000,
            state: json!({
                "version":1, "phase":"finished", "original_request":"Check report", "history":[],
                "prompt":{"role":"user","content":[{"type":"text","text":"Finished execution."}]},
                "reference_fingerprints":[], "work_plan":{}, "steering":[], "iteration":0,
                "model_calls_reserved":0, "max_tool_rounds":1, "plan_step_counter":0,
                "empty_continuations":0, "progress_seen":[],
                "stats":flow_like::flow_like_model_provider::response::LLMUsageStats::default(),
                "web":{}, "delegated_web":{}, "local_app_discovery_returned":false,
                "room_refreshes_remaining":0, "recoveries":0, "approval_resume":false, "pending_calls":[],
                "answer":{"output":output,"stop_reason":null}
            }),
        }
    }

    async fn complete(db: &DatabaseConnection, turn: &bot_turn::Model, saved: BotRunCheckpoint) {
        bot_turn::Entity::update_many()
            .col_expr(bot_turn::Column::Status, Expr::value(STORED_DONE))
            .col_expr(bot_turn::Column::StatusText, Expr::value(None::<String>))
            .filter(bot_turn::Column::Id.eq(&turn.id))
            .exec(db)
            .await
            .unwrap();
        bot_turn_checkpoint::ActiveModel {
            turn_id: Set(turn.id.clone()),
            revision: Set(saved.revision as i64),
            claim_attempt: Set(1),
            recovery_attempts: Set(0),
            started_at_ms: Set(saved.started_at_ms),
            deadline_at_ms: Set(saved.deadline_at_ms),
            state: Set(saved.state),
            updated_at: Set(at(10)),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn setup() -> DatabaseConnection {
        let db = database().await;
        insert_bot(&db, "bot_1", "user-a", "SERVER").await;
        conversation(&db, "room", "DIRECT", "PLAIN", &["user-a", "bot_1"]).await;
        db
    }

    #[tokio::test]
    async fn baseline_requires_the_same_instruction_and_a_live_claim_and_is_bounded() {
        let db = setup().await;
        let old = trigger(&db, "old", "task-a", "Check report", 1).await;
        let saved = finished(&"🙂".repeat(5_000));
        assert!(
            flow_like::flow::copilot::run_recovery::finished_reply(&saved)
                .unwrap()
                .is_some()
        );
        complete(&db, &old, saved).await;
        let current = trigger(&db, "current", "task-a", "Check report", 2).await;
        assert!(
            ledger::take_in(&db, &current.id, "host", at(10))
                .await
                .unwrap()
        );
        let claim = Claim {
            turn_id: current.id.clone(),
            owner_id: "user-a".into(),
            host_id: "host".into(),
            attempt: 1,
        };
        assert_eq!(
            recovery::previous_monitor_result_in(&db, &claim, at(11))
                .await
                .unwrap()
                .unwrap()
                .chars()
                .count(),
            4_000
        );
        let stale = Claim {
            attempt: 2,
            ..claim.clone()
        };
        assert!(
            recovery::previous_monitor_result_in(&db, &stale, at(11))
                .await
                .is_err()
        );
        let foreign = Claim {
            owner_id: "user-b".into(),
            ..claim
        };
        assert!(
            recovery::previous_monitor_result_in(&db, &foreign, at(11))
                .await
                .is_err()
        );
        let changed = trigger(&db, "changed", "task-a", "Check another report", 3).await;
        assert_eq!(previous_result_for_turn(&db, &changed).await.unwrap(), None);
        let different = trigger(&db, "different", "task-b", "Check report", 4).await;
        assert_eq!(
            previous_result_for_turn(&db, &different).await.unwrap(),
            None
        );
        let mut another_room = current.clone();
        another_room.conversation_id = "foreign-room".into();
        assert_eq!(
            previous_result_for_turn(&db, &another_room).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn incomplete_failed_and_encrypted_results_are_not_baselines() {
        let db = setup().await;
        let old = trigger(&db, "old", "task-a", "Check report", 1).await;
        let current = trigger(&db, "current", "task-a", "Check report", 2).await;
        assert_eq!(previous_result_for_turn(&db, &current).await.unwrap(), None);
        let mut saved = finished("The check failed");
        saved.state["answer"]["stop_reason"] = json!("tool_round_limit");
        complete(&db, &old, saved).await;
        assert_eq!(previous_result_for_turn(&db, &current).await.unwrap(), None);
        bot_turn_checkpoint::Entity::update_many()
            .col_expr(
                bot_turn_checkpoint::Column::State,
                Expr::value(json!({"sealed_checkpoint":1,"envelope":{"ciphertext":"private"}})),
            )
            .filter(bot_turn_checkpoint::Column::TurnId.eq(&old.id))
            .exec(&db)
            .await
            .unwrap();
        assert_eq!(previous_result_for_turn(&db, &current).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_run_with_a_failed_tool_call_keeps_the_previous_good_baseline() {
        let db = setup().await;
        let good = trigger(&db, "good", "task-a", "Check report", 1).await;
        complete(&db, &good, finished("All quiet")).await;
        let failed = trigger(&db, "failed", "task-a", "Check report", 2).await;
        let mut saved = finished("Report unreachable");
        saved.state["pending_calls"] = json!([{
            "id":"call-1", "name":"fetch", "arguments":{},
            "output":"{\"status\":\"error\",\"code\":\"bot_tool_unavailable_headless\"}"
        }]);
        complete(&db, &failed, saved).await;
        let current = trigger(&db, "current", "task-a", "Check report", 3).await;
        assert_eq!(
            previous_result_for_turn(&db, &current).await.unwrap(),
            Some("All quiet".to_string())
        );
    }
}
