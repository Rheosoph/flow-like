//! Reads new activity for a local bot while its room turn is running.

use super::store::HostStore;
use async_trait::async_trait;
use flow_like::flow::copilot::platform::RoomUpdates;
use flow_like_bot_host::room_updates::RoomUpdateTracker;
use flow_like_local_bots::{InboxEntry, LOCAL_OWNER_ID};
use flow_like_types::inbox::PrincipalKind;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

pub(super) struct LocalRoomUpdates {
    store: Arc<dyn HostStore>,
    conversation_id: String,
    bot_id: String,
    tracker: Mutex<RoomUpdateTracker>,
}

impl LocalRoomUpdates {
    pub(super) fn new(
        store: Arc<dyn HostStore>,
        bot_id: &str,
        trigger: &InboxEntry,
        names: HashMap<String, String>,
    ) -> Self {
        Self {
            store,
            conversation_id: trigger.conversation_id.clone(),
            bot_id: bot_id.to_owned(),
            tracker: Mutex::new(RoomUpdateTracker::new(
                bot_id,
                LOCAL_OWNER_ID,
                trigger,
                names,
            )),
        }
    }
}

impl std::fmt::Debug for LocalRoomUpdates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalRoomUpdates")
            .field("conversation_id", &self.conversation_id)
            .field("bot_id", &self.bot_id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl RoomUpdates for LocalRoomUpdates {
    async fn drain(&self) -> Vec<String> {
        let Ok(Some(conversation)) = self.store.conversation(&self.conversation_id).await else {
            return Vec::new();
        };
        if !conversation
            .members
            .iter()
            .any(|member| member.principal_id == self.bot_id && member.kind == PrincipalKind::Bot)
        {
            return Vec::new();
        }
        let entries = match self.store.newest(&self.conversation_id).await {
            Ok(entries) => entries,
            Err(error) => {
                tracing::debug!(%error, "Bot host: new local room activity could not be read");
                return Vec::new();
            }
        };
        self.tracker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .consume(&entries)
    }
}
