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

fn published(vault: &Arc<Vault>) -> serde_json::Value {
    let (key, _) = InboxKeyring::new(vault.clone()).ensure("bot").unwrap();
    json!({
        "principal_id": "bot",
        "key_id": key.key_id,
        "algorithm": key.algorithm,
        "public_key": key.public_key,
    })
}

/// Each device that runs the bot keeps its own key: what the phone saves, the desktop resumes.
#[tokio::test]
async fn a_checkpoint_opens_on_every_host_that_published_a_key_of_the_bot() {
    let (phone, desktop, stranger) = (
        Arc::new(Vault::default()),
        Arc::new(Vault::default()),
        Arc::new(Vault::default()),
    );
    let hub = Arc::new(DeviceHub {
        published_keys: Mutex::new(Some(vec![published(&desktop), published(&phone)])),
        ..DeviceHub::default()
    });
    InboxKeyring::new(stranger.clone()).ensure("bot").unwrap();
    let host = |vault: &Arc<Vault>, id: &str| E2eHost::new(hub.clone(), vault.clone(), id.into());
    let checkpoint = |revision| BotRunCheckpoint {
        revision,
        started_at_ms: 10,
        deadline_at_ms: 1000,
        state: json!({"version": 1, "objective": "private source material"}),
    };
    let on_phone = host(&phone, "phone");
    let sealed = on_phone
        .seal_checkpoint("bot", "room", "turn", &checkpoint(1))
        .await
        .unwrap();
    on_phone
        .seal_checkpoint("bot", "room", "turn", &checkpoint(2))
        .await
        .unwrap();
    assert_eq!(*hub.key_reads.lock().unwrap(), 1);
    assert!(
        !serde_json::to_string(&sealed)
            .unwrap()
            .contains("private source material")
    );
    for vault in [&phone, &desktop] {
        assert_eq!(
            host(vault, "resuming")
                .open_checkpoint("bot", "room", "turn", &sealed)
                .await
                .unwrap(),
            checkpoint(1)
        );
    }
    let refused = host(&stranger, "stranger")
        .open_checkpoint("bot", "room", "turn", &sealed)
        .await
        .unwrap_err();
    assert!(refused.message.contains("another device"), "{refused:?}");
}

/// A host that can't read the bot's other keys still saves a checkpoint it resumes itself.
#[tokio::test]
async fn a_checkpoint_is_sealed_to_this_host_while_the_hub_lists_no_usable_keys() {
    let vault = Arc::new(Vault::default());
    let broken = json!({
        "principal_id": "bot",
        "key_id": "ik_broken",
        "algorithm": published(&vault)["algorithm"],
        "public_key": "not a key",
    });
    let checkpoint = BotRunCheckpoint {
        revision: 1,
        started_at_ms: 10,
        deadline_at_ms: 1000,
        state: json!({"version": 1}),
    };
    for listed in [None, Some(vec![broken])] {
        let hub = Arc::new(DeviceHub {
            published_keys: Mutex::new(listed),
            ..DeviceHub::default()
        });
        let host = E2eHost::new(hub, vault.clone(), "host".into());
        let sealed = host
            .seal_checkpoint("bot", "room", "turn", &checkpoint)
            .await
            .unwrap();
        assert_eq!(
            host.open_checkpoint("bot", "room", "turn", &sealed)
                .await
                .unwrap(),
            checkpoint
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
