//! Lets the owner's other devices watch the computer view of bots that run in this desktop app.
//! Viewers find this host through the account's signaling topic and open an end-to-end encrypted
//! WebRTC data channel that carries the playground stream.

mod access;
mod answerer;
mod crypto;
mod host;
mod messages;
mod pairing;
mod records;
mod sealed;
mod signaling;
mod trust;

use pairing::{PairedDevice, PairingStart, Shared};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use trust::TrustStore;

const TRUST_FILE: &str = "bot-screen-trust.json";

pub(crate) fn host_peer_id(host_id: &str) -> String {
    let hash = blake3::hash(format!("flow-like.bot-screens-host/v1|{host_id}").as_bytes());
    hash.to_hex()[..32].to_owned()
}

/// The id viewers see as `from` of this host's `here` and `answer` messages.
#[tauri::command]
pub fn bot_screens_host_id(app: AppHandle) -> String {
    host_peer_id(&crate::bot_host::host_id(&app))
}

fn shared(app: &AppHandle) -> Result<Arc<Shared>, String> {
    app.try_state::<Arc<Shared>>()
        .map(|state| state.inner().clone())
        .ok_or_else(|| "The bot screen host has not started yet".to_owned())
}

/// A pairing link for the QR code; it works once, for five minutes.
#[tauri::command]
pub fn bot_screens_pairing_start(app: AppHandle) -> Result<PairingStart, String> {
    let shared = shared(&app)?;
    let (link, ttl) = shared.start(pairing::PENDING_TTL)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    Ok(PairingStart {
        link,
        expires_at: now + ttl.as_secs() as i64,
        host_label: shared.label.clone(),
    })
}

#[tauri::command]
pub fn bot_screens_paired(app: AppHandle) -> Result<Vec<PairedDevice>, String> {
    shared(&app)?.paired()
}

#[tauri::command(rename_all = "camelCase")]
pub fn bot_screens_unpair(app: AppHandle, key_id: String) -> Result<(), String> {
    shared(&app)?.unpair(&key_id)
}

pub(crate) fn start(app: &AppHandle) {
    let path = match app.path().app_data_dir() {
        Ok(dir) => dir.join(TRUST_FILE),
        Err(error) => {
            tracing::error!(%error, "Bot screens: no app data dir for the trust file, so other devices cannot watch this computer's bots");
            return;
        }
    };
    let peer = host_peer_id(&crate::bot_host::host_id(app));
    let shared = Arc::new(Shared::new(peer, TrustStore::new(path)));
    app.manage(shared.clone());
    tauri::async_runtime::spawn(host::run(app.clone(), shared));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_peer_id_is_32_hex_and_stable_per_host() {
        let id = host_peer_id("desktop:0123");
        assert_eq!(id.len(), 32);
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_eq!(id, host_peer_id("desktop:0123"));
        assert_ne!(id, host_peer_id("desktop:0124"));
    }
}
