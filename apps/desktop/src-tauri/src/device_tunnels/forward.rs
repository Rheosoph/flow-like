use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError, Weak},
    time::{Duration, Instant},
};

use flow_like_device_client::{LoopbackForward, TunnelMode, TunnelOpen, TunnelTarget};
use serde::{Deserialize, Serialize};

const MAX_LISTENERS: usize = 8;
/// The page touches every 10 s; a port it stopped touching closes after this long.
const LEASE: Duration = Duration::from_secs(30);
const LEASE_CHECK: Duration = Duration::from_secs(1);

pub(super) static FORWARDS: LazyLock<Arc<Forwards<LoopbackForward>>> = LazyLock::new(Forwards::new);

/// The deployed service listener a local port reaches.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ForwardTarget {
    device_id: String,
    placement_id: String,
    service_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListenerInfo {
    id: String,
    token: String,
    port: u16,
}

pub(super) trait Listener: Send + Sync + 'static {
    fn close(&self);
}

impl Listener for LoopbackForward {
    fn close(&self) {
        LoopbackForward::close(self);
    }
}

struct Entry<L> {
    owner: String,
    token: String,
    device_id: String,
    listener: L,
    touched: Mutex<Instant>,
}

impl<L> Entry<L> {
    fn expired(&self) -> bool {
        guard(&self.touched).elapsed() >= LEASE
    }
}

pub(super) struct Forwards<L> {
    entries: Mutex<HashMap<String, Arc<Entry<L>>>>,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn full() -> String {
    format!("At most {MAX_LISTENERS} local device ports can be open at once; close one first.")
}

impl<L: Listener> Forwards<L> {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::default(),
        })
    }

    fn has_room(&self) -> Result<(), String> {
        if guard(&self.entries).len() >= MAX_LISTENERS {
            return Err(full());
        }
        Ok(())
    }

    /// Takes over a bound listener; it closes when its lease runs out.
    pub(super) fn insert(
        self: &Arc<Self>,
        owner: &str,
        device_id: &str,
        listener: L,
        port: u16,
    ) -> Result<ListenerInfo, String> {
        let mut entries = guard(&self.entries);
        if entries.len() >= MAX_LISTENERS {
            listener.close();
            return Err(full());
        }
        let info = ListenerInfo {
            id: uuid::Uuid::new_v4().to_string(),
            token: uuid::Uuid::new_v4().to_string(),
            port,
        };
        entries.insert(
            info.id.clone(),
            Arc::new(Entry {
                owner: owner.to_owned(),
                token: info.token.clone(),
                device_id: device_id.to_owned(),
                listener,
                touched: Mutex::new(Instant::now()),
            }),
        );
        tokio::spawn(watch_lease(Arc::downgrade(self), info.id.clone()));
        Ok(info)
    }

    fn entry(&self, owner: &str, id: &str, token: &str) -> Result<Arc<Entry<L>>, String> {
        let entry = guard(&self.entries)
            .get(id)
            .filter(|entry| entry.owner == owner && entry.token == token)
            .cloned()
            .ok_or_else(|| format!("The local device port {id} is not open."))?;
        if entry.expired() {
            return Err(format!("The local device port {id} expired."));
        }
        Ok(entry)
    }

    pub(super) fn touch(&self, owner: &str, id: &str, token: &str) -> Result<(), String> {
        *guard(&self.entry(owner, id, token)?.touched) = Instant::now();
        Ok(())
    }

    pub(super) fn close(&self, owner: &str, id: &str, token: &str) -> Result<(), String> {
        self.entry(owner, id, token)?;
        self.remove(id);
        Ok(())
    }

    fn remove(&self, id: &str) {
        if let Some(entry) = guard(&self.entries).remove(id) {
            entry.listener.close();
        }
    }

    fn close_where(&self, closes: impl Fn(&Entry<L>) -> bool) {
        guard(&self.entries).retain(|_, entry| {
            let closing = closes(entry);
            if closing {
                entry.listener.close();
            }
            !closing
        });
    }

    /// Every port of `owner`, or every port.
    pub(super) fn close_owner(&self, owner: Option<&str>) {
        self.close_where(|entry| owner.is_none_or(|owner| owner == entry.owner));
    }

    pub(super) fn close_device(&self, device_id: &str) {
        self.close_where(|entry| entry.device_id == device_id);
    }
}

async fn watch_lease<L: Listener>(forwards: Weak<Forwards<L>>, id: String) {
    loop {
        tokio::time::sleep(LEASE_CHECK).await;
        let Some(forwards) = forwards.upgrade() else {
            return;
        };
        let Some(entry) = guard(&forwards.entries).get(&id).cloned() else {
            return;
        };
        if entry.expired() {
            forwards.remove(&id);
            return;
        }
    }
}

/// The device checks the grant and the listener with one probe stream before the port exists.
pub(super) async fn listen(
    owner: &str,
    target: ForwardTarget,
    port: u16,
) -> Result<ListenerInfo, String> {
    FORWARDS.has_room()?;
    let ForwardTarget {
        device_id,
        placement_id,
        service_id,
    } = target;
    let session = crate::device_models::session(&device_id).await?;
    let open = TunnelOpen {
        placement_id,
        service_id,
        mode: TunnelMode::Tcp,
        target: TunnelTarget::Service,
    };
    let refused = |error: flow_like_device_client::Error| {
        format!(
            "Device {device_id} refused a connection to service {}/{}: {error}",
            open.placement_id, open.service_id
        )
    };
    drop(session.open_stream(open.clone()).await.map_err(refused)?);
    let forward = LoopbackForward::bind(session, open, port)
        .await
        .map_err(|error| error.to_string())?;
    let port = forward.local_addr().port();
    FORWARDS.insert(owner, &device_id, forward, port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, Default)]
    struct FakeListener(Arc<AtomicUsize>);

    impl Listener for FakeListener {
        fn close(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn open(
        forwards: &Arc<Forwards<FakeListener>>,
        owner: &str,
        device: &str,
    ) -> (ListenerInfo, FakeListener) {
        let listener = FakeListener::default();
        let info = forwards
            .insert(owner, device, listener.clone(), 41_234)
            .unwrap();
        (info, listener)
    }

    fn closes(listener: &FakeListener) -> usize {
        listener.0.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn only_the_owning_page_with_its_token_touches_or_closes_a_port() {
        let forwards = Forwards::new();
        let (info, listener) = open(&forwards, "main", "device-1");
        assert_eq!(info.port, 41_234);

        assert!(forwards.touch("widget", &info.id, &info.token).is_err());
        assert!(forwards.touch("main", &info.id, "wrong").is_err());
        assert!(forwards.close("widget", &info.id, &info.token).is_err());
        assert_eq!(closes(&listener), 0);

        forwards.touch("main", &info.id, &info.token).unwrap();
        forwards.close("main", &info.id, &info.token).unwrap();
        assert_eq!(closes(&listener), 1);
        assert!(forwards.touch("main", &info.id, &info.token).is_err());
    }

    #[tokio::test]
    async fn the_listener_cap_refuses_and_closes_the_extra_port() {
        let forwards = Forwards::new();
        for _ in 0..MAX_LISTENERS {
            open(&forwards, "main", "device-1");
        }
        assert!(forwards.has_room().is_err());
        let extra = FakeListener::default();
        assert!(
            forwards
                .insert("main", "device-1", extra.clone(), 1)
                .is_err()
        );
        assert_eq!(closes(&extra), 1);
    }

    #[tokio::test]
    async fn pages_and_devices_close_only_their_own_ports() {
        let forwards = Forwards::new();
        let (_, first) = open(&forwards, "main", "device-1");
        let (_, second) = open(&forwards, "main", "device-2");
        let (_, other) = open(&forwards, "other", "device-1");

        forwards.close_device("device-2");
        assert_eq!((closes(&first), closes(&second), closes(&other)), (0, 1, 0));

        forwards.close_owner(Some("main"));
        assert_eq!((closes(&first), closes(&other)), (1, 0));

        forwards.close_owner(None);
        assert_eq!(closes(&other), 1);
        assert!(guard(&forwards.entries).is_empty());
    }

    #[tokio::test]
    async fn an_untouched_port_expires_and_cannot_be_revived() {
        let forwards = Forwards::new();
        let (info, listener) = open(&forwards, "main", "device-1");
        let entry = forwards.entry("main", &info.id, &info.token).unwrap();
        *guard(&entry.touched) = Instant::now() - LEASE;

        assert!(forwards.touch("main", &info.id, &info.token).is_err());
        tokio::time::timeout(Duration::from_secs(3), async {
            while closes(&listener) == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the lease watcher closes the port");
        assert!(guard(&forwards.entries).is_empty());
    }
}
