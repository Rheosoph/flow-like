//! Account bots running on this desktop asking each other. The crew, the grants, the exchange and
//! the callee's turn all live in the hub (`/bots/{id}/talk`); this host only carries the question
//! there and brings the answer back. A callee that is late or offline answers `pending`, and its
//! answer wakes the asker later in its own conversation with the owner.

use super::hub::{HubFailure, ProfileHub};
use async_trait::async_trait;
use flow_like::flow::copilot::bot_talk::{
    BotTalkHost, CrewMember, NestedBotTurn, PendingAnswer,
};
use flow_like_types::anyhow;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Mutex;
use tauri::AppHandle;

pub(crate) struct AccountTalkHost {
    hub: ProfileHub,
    run_id: String,
    callee: Mutex<Option<String>>,
    reply_id: Mutex<Option<String>>,
}

#[derive(Deserialize)]
struct Roster {
    members: Vec<CrewMember>,
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
    pub(crate) fn new(app: AppHandle, run_id: &str) -> Self {
        Self {
            hub: ProfileHub::new(app),
            run_id: run_id.to_string(),
            callee: Mutex::default(),
            reply_id: Mutex::default(),
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

#[async_trait]
impl BotTalkHost for AccountTalkHost {
    async fn roster(&self, caller: &CrewMember) -> flow_like_types::Result<Vec<CrewMember>> {
        let roster: Roster = self
            .call(Method::GET, format!("bots/{}/talk/roster", caller.id), None)
            .await?;
        Ok(roster.members)
    }

    async fn has_grant(&self, from: &CrewMember, to: &CrewMember) -> flow_like_types::Result<bool> {
        let permissions: Permissions = self
            .call(Method::GET, format!("bots/{}/permissions", from.id), None)
            .await?;
        Ok(permissions
            .granted
            .iter()
            .any(|grant| grant.to_bot_id == to.id))
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
