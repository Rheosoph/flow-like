use crate::history::{History, HistoryMessage};
use flow_like_types::Result;
use rig::completion::Message as RigMessage;

/// Converts a Flow-Like history through the canonical, multimodal conversion implementation.
pub fn history_to_rig_messages(history: &History) -> Result<Vec<RigMessage>> {
    history.to_rig_messages()
}

pub fn history_message_to_rig(msg: &HistoryMessage) -> Result<RigMessage> {
    msg.clone().try_into()
}

pub fn rig_message_to_history(msg: &RigMessage) -> Result<HistoryMessage> {
    Ok(msg.clone().into())
}

pub fn rig_messages_to_history(messages: Vec<RigMessage>, model: String) -> Result<History> {
    let mut history: History = messages.into();
    history.model = model;
    Ok(history)
}
