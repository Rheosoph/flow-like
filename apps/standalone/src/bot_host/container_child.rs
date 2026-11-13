//! The complete bot host inside its container. Only lifecycle requests cross the parent pipe.

use super::{
    container_wire::{self as wire, Frame},
    e2e::DeviceKeyVault,
    hub::DeviceApi,
};
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use flow_like_device_crypto::inbox::InboxKeyVault;
use reqwest::Method;
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Semaphore, mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<(u16, Vec<u8>)>>>>;
type Views = Arc<Mutex<HashMap<u64, mpsc::Sender<Vec<u8>>>>>;

struct BrokerApi {
    base_url: String,
    device_id: String,
    bot_id: String,
    state_dir: PathBuf,
    tx: mpsc::Sender<Frame>,
    pending: Pending,
    next: AtomicU64,
    slots: Semaphore,
}

#[async_trait]
impl DeviceApi for BrokerApi {
    fn base_url(&self) -> &str {
        &self.base_url
    }
    fn device_id(&self) -> &str {
        &self.device_id
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Vec<u8>)> {
        let _slot = self.slots.acquire().await?;
        if path.ends_with("/inbox-keys") {
            let vault = DeviceKeyVault::open(&self.state_dir)?;
            if let Some(keys) = vault.load(&self.bot_id).map_err(anyhow::Error::msg)? {
                self.tx
                    .send(Frame::Keys {
                        data: wire::encode(&keys),
                    })
                    .await?;
            }
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, receive) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, answer);
        let sent = self
            .tx
            .send(Frame::Call {
                id,
                method: method.to_string(),
                path: path.into(),
                body: body.cloned(),
            })
            .await;
        let result = match sent {
            Ok(()) => match tokio::time::timeout(Duration::from_secs(60), receive).await {
                Ok(answer) => answer.context("Device broker disconnected"),
                Err(_) => Err(anyhow::anyhow!("Device broker timed out")),
            },
            Err(_) => Err(anyhow::anyhow!("Device broker disconnected")),
        };
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        result
    }
}

#[cfg(all(
    feature = "playground",
    any(target_os = "linux", target_os = "macos", windows)
))]
fn open_view(
    id: u64,
    observations: bool,
    bot_id: String,
    tx: mpsc::Sender<Frame>,
    views: Views,
    cancel: CancellationToken,
) -> Result<()> {
    let host =
        crate::playground::shared_machine_host().context("The bot's machine has not started")?;
    let (incoming, mut rx) = mpsc::channel::<Vec<u8>>(32);
    {
        let mut views = views.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(
            views.len() < 4 && !views.contains_key(&id),
            "Too many bot screen viewers"
        );
        views.insert(id, incoming);
    }
    tokio::spawn(async move {
        let (relay, view) = tokio::io::duplex(wire::PIPE_BYTES);
        let (mut read, mut write) = tokio::io::split(relay);
        let input = async {
            while let Some(data) = rx.recv().await {
                write.write_all(&data).await?;
            }
            Ok::<_, std::io::Error>(())
        };
        let output = async {
            let mut bytes = [0u8; 16 * 1024];
            loop {
                let count = read.read(&mut bytes).await?;
                if count == 0 {
                    break;
                }
                tx.send(Frame::ViewData {
                    id,
                    data: wire::encode(&bytes[..count]),
                })
                .await
                .map_err(|_| std::io::Error::other("Device broker disconnected"))?;
            }
            Ok::<_, std::io::Error>(())
        };
        tokio::select! {
            _ = cancel.cancelled() => {},
            _ = input => {},
            _ = output => {},
            _ = crate::management::playground_stream::serve(view, host, bot_id, observations, Default::default()) => {},
        }
        views.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        let _ = tx.send(Frame::CloseView { id }).await;
    });
    Ok(())
}

#[cfg(not(all(
    feature = "playground",
    any(target_os = "linux", target_os = "macos", windows)
)))]
fn open_view(
    _: u64,
    _: bool,
    _: String,
    _: mpsc::Sender<Frame>,
    _: Views,
    _: CancellationToken,
) -> Result<()> {
    anyhow::bail!("This worker has no browser or computer support")
}

/// This hidden command never opens a device session or starts the normal device supervisor.
pub async fn run() -> Result<()> {
    let mut reader = tokio::io::stdin();
    let init = tokio::time::timeout(Duration::from_secs(30), wire::read(&mut reader))
        .await
        .context("The device did not configure its bot container in time")??
        .context("The device sent no container configuration")?;
    let Frame::Init {
        base_url,
        device_id,
        bot_id,
        keys,
    } = init
    else {
        anyhow::bail!("The first container message must configure its bot");
    };
    ensure!(
        !bot_id.is_empty() && bot_id.len() <= 128,
        "Invalid container bot id"
    );
    let state_dir = Path::new("/var/lib/flow-like").to_path_buf();
    reset_browser_lock(&state_dir, &bot_id)?;
    let vault = DeviceKeyVault::open(&state_dir)?;
    if let Some(keys) = keys {
        let bytes = wire::decode(&keys)?;
        ensure!(
            bytes.len() <= 1024 * 1024,
            "Bot inbox key backup is too large"
        );
        vault.store(&bot_id, &bytes).map_err(anyhow::Error::msg)?;
    }
    let cancel = CancellationToken::new();
    let (tx, mut outgoing) = mpsc::channel::<Frame>(8);
    let pending = Pending::default();
    let pending_reader = pending.clone();
    let views = Views::default();
    let api = Arc::new(BrokerApi {
        base_url,
        device_id,
        bot_id: bot_id.clone(),
        state_dir: state_dir.clone(),
        tx: tx.clone(),
        pending: pending.clone(),
        next: AtomicU64::new(1),
        slots: Semaphore::new(32),
    });
    let writer_cancel = cancel.clone();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(frame) = outgoing.recv().await {
            if wire::write(&mut stdout, &frame).await.is_err() {
                break;
            }
        }
        writer_cancel.cancel();
    });
    let read_cancel = cancel.clone();
    let (last_parent, parent_seen) = tokio::sync::watch::channel(tokio::time::Instant::now());
    let reader_task = tokio::spawn(async move {
        while let Ok(Some(frame)) = wire::read(&mut reader).await {
            last_parent.send_replace(tokio::time::Instant::now());
            match frame {
                Frame::Reply { id, status, body } => {
                    let Ok(body) = wire::decode(&body) else {
                        break;
                    };
                    if let Some(answer) = pending_reader
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id)
                    {
                        let _ = answer.send((status, body));
                    }
                }
                Frame::OpenView { id, observations } => {
                    if open_view(
                        id,
                        observations,
                        bot_id.clone(),
                        tx.clone(),
                        views.clone(),
                        read_cancel.child_token(),
                    )
                    .is_err()
                    {
                        let _ = tx.send(Frame::CloseView { id }).await;
                    }
                }
                Frame::ViewData { id, data } => {
                    let Ok(data) = wire::decode(&data) else {
                        break;
                    };
                    ensure_small_view(&views, id, data);
                }
                Frame::CloseView { id } => {
                    views.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                }
                Frame::Shutdown => break,
                Frame::Ping => {}
                _ => break,
            }
        }
        pending_reader
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        views.lock().unwrap_or_else(|e| e.into_inner()).clear();
        read_cancel.cancel();
    });
    let watchdog = tokio::spawn(parent_watchdog(parent_seen, cancel.clone()));
    let host = super::run_with_api(state_dir, api, cancel.clone());
    tokio::pin!(host);
    let result = tokio::select! {
        result = &mut host => result,
        _ = cancel.cancelled() => {
            pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
            tokio::time::timeout(Duration::from_secs(5), &mut host).await.unwrap_or(Ok(()))
        }
    };
    cancel.cancel();
    reader_task.abort();
    writer.abort();
    watchdog.abort();
    result
}

fn ensure_small_view(views: &Views, id: u64, data: Vec<u8>) {
    let mut views = views.lock().unwrap_or_else(|e| e.into_inner());
    let failed = data.len() > 16 * 1024
        || views
            .get(&id)
            .is_some_and(|view| view.try_send(data).is_err());
    if failed {
        views.remove(&id);
    }
}

#[cfg(all(
    feature = "playground",
    any(target_os = "linux", target_os = "macos", windows)
))]
fn reset_browser_lock(state_dir: &Path, bot: &str) -> Result<()> {
    let profile =
        flow_like_bot_machine::bot_dir(&state_dir.join("bot-machine"), bot).join("browser-profile");
    // The parent stopped the previous container before giving this worker its private volume.
    // Chromium's old hostname/PID lock cannot identify a live browser in this new PID namespace.
    for name in [
        "SingletonLock",
        "SingletonCookie",
        "SingletonSocket",
        "DevToolsActivePort",
    ] {
        match std::fs::remove_file(profile.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Remove the bot browser's stale {name}"));
            }
        }
    }
    Ok(())
}

#[cfg(not(all(
    feature = "playground",
    any(target_os = "linux", target_os = "macos", windows)
)))]
fn reset_browser_lock(_: &Path, _: &str) -> Result<()> {
    Ok(())
}

async fn parent_watchdog(
    mut seen: tokio::sync::watch::Receiver<tokio::time::Instant>,
    cancel: CancellationToken,
) {
    loop {
        let deadline = *seen.borrow() + Duration::from_secs(20);
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            result = seen.changed() => if result.is_err() { break; },
            _ = cancel.cancelled() => return,
        }
    }
    cancel.cancel();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn loss_of_the_parent_stops_a_worker_even_if_stdin_stays_open() {
        let (sender, receiver) = tokio::sync::watch::channel(tokio::time::Instant::now());
        let cancel = CancellationToken::new();
        let watcher = tokio::spawn(parent_watchdog(receiver, cancel.clone()));
        tokio::time::advance(Duration::from_secs(19)).await;
        assert!(!cancel.is_cancelled());
        sender.send_replace(tokio::time::Instant::now());
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(19)).await;
        assert!(!cancel.is_cancelled());
        tokio::time::advance(Duration::from_secs(2)).await;
        watcher.await.unwrap();
        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    async fn a_closed_parent_channel_stops_immediately() {
        let (sender, receiver) = tokio::sync::watch::channel(tokio::time::Instant::now());
        let cancel = CancellationToken::new();
        drop(sender);
        parent_watchdog(receiver, cancel.clone()).await;
        assert!(cancel.is_cancelled());
    }

    #[cfg(all(
        feature = "playground",
        any(target_os = "linux", target_os = "macos", windows)
    ))]
    #[test]
    fn restarting_a_container_clears_only_browser_coordination_files() {
        let state = tempfile::tempdir().unwrap();
        let profile = flow_like_bot_machine::bot_dir(&state.path().join("bot-machine"), "bot")
            .join("browser-profile");
        std::fs::create_dir_all(&profile).unwrap();
        for name in [
            "SingletonLock",
            "SingletonCookie",
            "SingletonSocket",
            "DevToolsActivePort",
            "Cookies",
            "Preferences",
        ] {
            std::fs::write(profile.join(name), b"kept").unwrap();
        }
        reset_browser_lock(state.path(), "bot").unwrap();
        assert!(!profile.join("SingletonLock").exists());
        assert!(!profile.join("DevToolsActivePort").exists());
        assert_eq!(std::fs::read(profile.join("Cookies")).unwrap(), b"kept");
        assert_eq!(std::fs::read(profile.join("Preferences")).unwrap(), b"kept");
    }
}
