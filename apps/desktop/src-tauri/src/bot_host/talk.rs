//! Account bots running on this desktop asking each other. The crew, the grants, the exchange and
//! the callee's turn all live in the hub (`/bots/{id}/talk`); this host only carries the question
//! there and brings the answer back. A callee that is late or offline answers `pending`, and its
//! answer wakes the asker later in its own conversation with the owner.

use super::{
    gate::ApprovalGate,
    hub::{HubFailure, ProfileHub},
};
use async_trait::async_trait;
use flow_like::flow::copilot::bot_talk::{
    BotTalkHost, CrewMember, HeadlessPermission, NestedBotTurn, PendingAnswer,
};
use flow_like_types::{anyhow, bot_team::TeamBrief};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

pub(crate) struct AccountTalkHost {
    hub: ProfileHub,
    run_id: String,
    gate: Option<Arc<ApprovalGate>>,
    callee: Mutex<Option<String>>,
    reply_id: Mutex<Option<String>>,
    /// The team brief the last roster read carried, with the id of the bot it was read for.
    brief: Mutex<Option<(String, Option<TeamBrief>)>>,
}

#[derive(Deserialize)]
struct Roster {
    members: Vec<CrewMember>,
    #[serde(default)]
    team: Option<TeamBrief>,
}

#[derive(Deserialize)]
struct Exchange {
    exchange_id: String,
}

#[derive(Deserialize)]
struct Permissions {
    granted: Vec<Granted>,
}

#[derive(Deserialize)]
struct Granted {
    to_bot_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Answered,
    Pending,
    Failed,
}

#[derive(Deserialize)]
struct Answer {
    status: Status,
    text: String,
    ask_id: String,
    reply_id: Option<String>,
}

impl AccountTalkHost {
    pub(crate) fn new(app: AppHandle, run_id: &str, gate: Option<Arc<ApprovalGate>>) -> Self {
        Self {
            hub: ProfileHub::new(app),
            run_id: run_id.to_string(),
            gate,
            callee: Mutex::default(),
            reply_id: Mutex::default(),
            brief: Mutex::default(),
        }
    }

    async fn fetch_roster(&self, caller: &CrewMember) -> flow_like_types::Result<Roster> {
        self.call(Method::GET, format!("bots/{}/talk/roster", caller.id), None)
            .await
    }

    async fn settle_card_answer(
        &self,
        from: &CrewMember,
        to: &CrewMember,
    ) -> flow_like_types::Result<bool> {
        match &self.gate {
            Some(gate) => gate.settle_ask_card(to, self.store_grant(from, to)).await,
            None => Ok(false),
        }
    }

    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: Method,
        path: String,
        body: Option<Value>,
    ) -> flow_like_types::Result<T> {
        let value = self
            .hub
            .request(method, &path, body)
            .await
            .map_err(failed)?;
        serde_json::from_value(value)
            .map_err(|_| anyhow!("The hub's answer to {path} is not in the expected shape"))
    }
}

fn failed(failure: HubFailure) -> flow_like_types::Error {
    anyhow!("{failure}")
}

fn slot<T: Clone>(cell: &Mutex<Option<T>>) -> Option<T> {
    cell.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn put<T>(cell: &Mutex<Option<T>>, value: T) {
    *cell.lock().unwrap_or_else(|p| p.into_inner()) = Some(value);
}

/// The brief a roster read cached for `bot_id`, taken out of `cell`. One cached for another bot is
/// dropped: a roster read to resolve a callee leaves its caller's brief behind.
fn take_brief(
    cell: &Mutex<Option<(String, Option<TeamBrief>)>>,
    bot_id: &str,
) -> Option<Option<TeamBrief>> {
    match cell.lock().unwrap_or_else(|p| p.into_inner()).take() {
        Some((cached_for, brief)) if cached_for == bot_id => Some(brief),
        _ => None,
    }
}

#[async_trait]
impl BotTalkHost for AccountTalkHost {
    async fn roster(&self, caller: &CrewMember) -> flow_like_types::Result<Vec<CrewMember>> {
        let roster = self.fetch_roster(caller).await?;
        put(&self.brief, (caller.id.clone(), roster.team));
        Ok(roster.members)
    }

    async fn team_brief(&self, caller: &CrewMember) -> flow_like_types::Result<Option<TeamBrief>> {
        match take_brief(&self.brief, &caller.id) {
            Some(brief) => Ok(brief),
            None => Ok(self.fetch_roster(caller).await?.team),
        }
    }

    async fn has_grant(&self, from: &CrewMember, to: &CrewMember) -> flow_like_types::Result<bool> {
        let permissions: Permissions = self
            .call(Method::GET, format!("bots/{}/permissions", from.id), None)
            .await?;
        if permissions
            .granted
            .iter()
            .any(|grant| grant.to_bot_id == to.id)
        {
            return Ok(true);
        }
        self.settle_card_answer(from, to).await
    }

    async fn headless_permission(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
        question: &str,
    ) -> HeadlessPermission {
        match &self.gate {
            Some(gate) => gate.ask_bot(caller, callee, question).await,
            None => HeadlessPermission::Unavailable,
        }
    }

    async fn store_grant(&self, from: &CrewMember, to: &CrewMember) -> flow_like_types::Result<()> {
        let _: Value = self
            .call(
                Method::PUT,
                format!("bots/{}/permissions/{}", from.id, to.id),
                None,
            )
            .await?;
        Ok(())
    }

    async fn exchange_conversation(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
    ) -> flow_like_types::Result<String> {
        let exchange: Exchange = self
            .call(
                Method::POST,
                format!("bots/{}/talk/exchange", caller.id),
                Some(json!({ "callee_id": callee.id })),
            )
            .await?;
        put(&self.callee, callee.id.clone());
        Ok(exchange.exchange_id)
    }

    async fn record(
        &self,
        _conversation_id: &str,
        sender: &CrewMember,
        _text: &str,
    ) -> flow_like_types::Result<String> {
        if slot(&self.callee).as_deref() == Some(sender.id.as_str()) {
            return slot(&self.reply_id)
                .ok_or_else(|| anyhow!("{} has no answer in the exchange yet", sender.name));
        }
        Ok(self.run_id.clone())
    }

    async fn run_turn(&self, turn: NestedBotTurn) -> flow_like_types::Result<String> {
        let answer: Answer = self
            .call(
                Method::POST,
                format!("bots/{}/talk", turn.caller.id),
                Some(json!({
                    "callee_id": turn.callee.id,
                    "question": turn.question,
                    "run_id": self.run_id,
                    "round": turn.round,
                })),
            )
            .await?;
        match answer.status {
            Status::Answered => {
                put(&self.reply_id, answer.reply_id.unwrap_or(answer.ask_id));
                Ok(answer.text)
            }
            Status::Pending => Err(PendingAnswer(answer.text).into()),
            Status::Failed => Err(anyhow!("{}", answer.text)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brief(bot_name: &str) -> TeamBrief {
        TeamBrief {
            bot_name: bot_name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_cached_brief_serves_only_the_bot_it_was_read_for_and_only_once() {
        let cell = Mutex::default();
        put(&cell, ("patch".to_string(), Some(brief("Patch"))));
        assert_eq!(take_brief(&cell, "sentinel"), None);
        assert_eq!(take_brief(&cell, "patch"), None);

        put(&cell, ("patch".to_string(), Some(brief("Patch"))));
        let taken = take_brief(&cell, "patch").flatten().unwrap();
        assert_eq!(taken.bot_name, "Patch");
        assert_eq!(take_brief(&cell, "patch"), None);

        put(&cell, ("scout".to_string(), None));
        assert_eq!(take_brief(&cell, "scout"), Some(None));
    }
}
