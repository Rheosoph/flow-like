use super::DeviceHub;
use crate::e2e::E2eHost;
use flow_like_device_crypto::inbox::{InboxKeyVault, InboxKeyring};
use flow_like_types::bot_turn::BotRunCheckpoint;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

#[derive(Default)]
pub(super) struct Vault(Mutex<HashMap<String, Vec<u8>>>);

impl InboxKeyVault for Vault {
    fn load(&self, owner: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(owner)
            .cloned()
            .map(Zeroizing::new))
    }

    fn store(&self, owner: &str, secret: &[u8]) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(owner.to_owned(), secret.to_vec());
        Ok(())
    }

    fn remove(&self, owner: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(owner);
        Ok(())
    }
}

#[tokio::test]
async fn sealed_checkpoint_survives_restart_and_key_rotation_without_plaintext_on_hub() {
    let vault = Arc::new(Vault::default());
    let keys = InboxKeyring::new(vault.clone());
    keys.ensure("bot").unwrap();
    let host = E2eHost::new(Arc::new(DeviceHub::default()), vault.clone(), "host".into());
    let checkpoint = BotRunCheckpoint {
        revision: 7,
        started_at_ms: 10,
        deadline_at_ms: 1000,
        state: json!({"version": 1, "objective": "private source material"}),
    };
    let sealed = host
        .seal_checkpoint("bot", "room", "turn", &checkpoint)
        .await
        .unwrap();
    assert!(
        !serde_json::to_string(&sealed)
            .unwrap()
            .contains("private source material")
    );
    keys.rotate("bot").unwrap();
    drop(host);
    let restarted = E2eHost::new(Arc::new(DeviceHub::default()), vault, "restarted".into());
    assert_eq!(
        restarted
            .open_checkpoint("bot", "room", "turn", &sealed)
            .await
            .unwrap(),
        checkpoint
    );
    for (bot, room, turn) in [
        ("other", "room", "turn"),
        ("bot", "other", "turn"),
        ("bot", "room", "other"),
    ] {
        assert!(
            restarted
                .open_checkpoint(bot, room, turn, &sealed)
                .await
                .is_err()
        );
    }
    for altered in [
        BotRunCheckpoint {
            revision: 8,
            ..sealed.clone()
        },
        checkpoint,
    ] {
        assert!(
            restarted
                .open_checkpoint("bot", "room", "turn", &altered)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn rebased_header_opens_with_the_stored_start_and_deadline() {
    let vault = Arc::new(Vault::default());
    InboxKeyring::new(vault.clone()).ensure("bot").unwrap();
    let host = E2eHost::new(Arc::new(DeviceHub::default()), vault, "host".into());
    let checkpoint = BotRunCheckpoint {
        revision: 3,
        started_at_ms: 10,
        deadline_at_ms: 1000,
        state: json!({"version": 1, "objective": "wait for approval"}),
    };
    let sealed = host
        .seal_checkpoint("bot", "room", "turn", &checkpoint)
        .await
        .unwrap();
    let rebased = BotRunCheckpoint {
        started_at_ms: 500,
        deadline_at_ms: 1500,
        ..sealed.clone()
    };
    let opened = host
        .open_checkpoint("bot", "room", "turn", &rebased)
        .await
        .unwrap();
    assert_eq!(
        (opened.started_at_ms, opened.deadline_at_ms, opened.revision),
        (500, 1500, 3)
    );
    assert_eq!(opened.state, checkpoint.state);
    let moved_revision = BotRunCheckpoint {
        revision: 4,
        ..rebased
    };
    assert!(
        host.open_checkpoint("bot", "room", "turn", &moved_revision)
            .await
            .is_err()
    );
}
