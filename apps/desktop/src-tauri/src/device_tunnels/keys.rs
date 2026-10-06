use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc, LazyLock, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::Deserialize;

use crate::device_models::VaultUnlock;

pub(super) static MIRRORS: LazyLock<Arc<Mirrors<ModelKeys>>> =
    LazyLock::new(|| Mirrors::new(ModelKeys));

/// How the device area let go of a device's keys.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LockMode {
    /// The user locked the device: its keys go everywhere in this app, kept or not.
    Lock,
    /// The area let go on its own (an idle lock, a page that is gone): keys kept for model
    /// access, or used by a run, stay.
    Release,
}

/// Where mirrored keys are held: the model connector's key store, or a fake in tests.
pub(super) trait KeyStore: Send + Sync + 'static {
    fn unlock(&self, unlock: VaultUnlock) -> impl Future<Output = Result<(), String>> + Send;
    fn lock(&self, device_id: &str) -> impl Future<Output = ()> + Send;
    fn release(&self, device_id: &str) -> impl Future<Output = ()> + Send;
    fn keep(&self, device_id: &str, keep: bool) -> impl Future<Output = ()> + Send;
    fn lock_all(&self) -> impl Future<Output = ()> + Send;
}

/// The device area and device models share one set of keys per device. The connector tracks
/// whether they are kept for model access, so a keep from a run's prompt survives the area.
pub(super) struct ModelKeys;

impl KeyStore for ModelKeys {
    async fn unlock(&self, unlock: VaultUnlock) -> Result<(), String> {
        crate::device_models::unlock_for_area(unlock)
            .await
            .map(|_| ())
    }

    async fn lock(&self, device_id: &str) {
        super::forward::FORWARDS.close_device(device_id);
        crate::device_models::lock(device_id).await;
    }

    async fn release(&self, device_id: &str) {
        super::forward::FORWARDS.close_device(device_id);
        crate::device_models::release(device_id).await;
    }

    async fn keep(&self, device_id: &str, keep: bool) {
        crate::device_models::keep(device_id, keep).await;
    }

    async fn lock_all(&self) {
        super::forward::FORWARDS.close_owner(None);
        crate::device_models::lock_every_device().await;
    }
}

/// A device the device area unlocked here, and the page that did.
struct Mirror {
    webview: String,
    page: u64,
}

/// Unlock, lock and cleanup of one device run in turn.
type Slot = Arc<tokio::sync::Mutex<Option<Mirror>>>;

pub(super) struct Mirrors<K> {
    store: K,
    /// Counts page loads; a mirror remembers the page that unlocked it.
    pages: AtomicU64,
    devices: Mutex<HashMap<String, Slot>>,
}

impl<K: KeyStore> Mirrors<K> {
    pub(super) fn new(store: K) -> Arc<Self> {
        Arc::new(Self {
            store,
            pages: AtomicU64::new(0),
            devices: Mutex::default(),
        })
    }

    fn slot(&self, device_id: &str) -> Slot {
        self.devices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(device_id.to_owned())
            .or_default()
            .clone()
    }

    fn slots(&self) -> Vec<(String, Slot)> {
        self.devices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(device_id, slot)| (device_id.clone(), slot.clone()))
            .collect()
    }

    pub(super) async fn unlock(&self, webview: &str, unlock: VaultUnlock) -> Result<(), String> {
        let page = self.pages.load(Ordering::Acquire);
        let slot = self.slot(&unlock.device_id);
        let mut mirror = slot.lock().await;
        self.store.unlock(unlock).await?;
        *mirror = Some(Mirror {
            webview: webview.to_owned(),
            page,
        });
        Ok(())
    }

    /// A lock drops the device's keys even when the area never mirrored them (a run's prompt
    /// unlocked them); a release lets go only of what the area holds.
    pub(super) async fn lock(&self, device_id: &str, mode: LockMode) {
        let slot = self.slot(device_id);
        let mut mirror = slot.lock().await;
        let mirrored = mirror.take().is_some();
        match mode {
            LockMode::Lock => self.store.lock(device_id).await,
            LockMode::Release if mirrored => self.store.release(device_id).await,
            LockMode::Release => {}
        }
    }

    pub(super) async fn keep(&self, device_id: &str, keep: bool) {
        let slot = self.slot(device_id);
        let mirror = slot.lock().await;
        if mirror.is_some() {
            self.store.keep(device_id, keep).await;
        }
    }

    /// Every device locks, mirrored or not; unlocks still running finish first.
    pub(super) async fn lock_all(&self) {
        for (_, slot) in self.slots() {
            slot.lock().await.take();
        }
        self.store.lock_all().await;
    }

    /// A page starts loading in `webview`, so its earlier pages are gone: the area lets go of
    /// what they unlocked. Unlocks of the new page are left alone.
    pub(super) fn page_loaded(
        self: &Arc<Self>,
        webview: &str,
    ) -> impl Future<Output = ()> + Send + use<K> {
        let page = self.pages.fetch_add(1, Ordering::AcqRel) + 1;
        let webview = webview.to_owned();
        let mirrors = self.clone();
        async move {
            mirrors
                .release_where(|mirror| mirror.webview == webview && mirror.page < page)
                .await;
        }
    }

    pub(super) fn close_all(self: &Arc<Self>) -> impl Future<Output = ()> + Send + use<K> {
        let mirrors = self.clone();
        async move { mirrors.release_where(|_| true).await }
    }

    async fn release_where(&self, gone: impl Fn(&Mirror) -> bool) {
        for (device_id, slot) in self.slots() {
            let mut mirror = slot.lock().await;
            if mirror.as_ref().is_some_and(&gone) {
                mirror.take();
                self.store.release(&device_id).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[derive(Default)]
    struct FakeStore {
        events: Mutex<Vec<String>>,
        unlock_delay: Option<Duration>,
    }

    impl FakeStore {
        fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }

        fn push(&self, event: String) {
            self.events.lock().unwrap().push(event);
        }
    }

    impl KeyStore for FakeStore {
        async fn unlock(&self, unlock: VaultUnlock) -> Result<(), String> {
            if let Some(delay) = self.unlock_delay {
                tokio::time::sleep(delay).await;
            }
            self.push(format!("unlock {}", unlock.device_id));
            Ok(())
        }

        async fn lock(&self, device_id: &str) {
            self.push(format!("lock {device_id}"));
        }

        async fn release(&self, device_id: &str) {
            self.push(format!("release {device_id}"));
        }

        async fn keep(&self, device_id: &str, keep: bool) {
            self.push(format!("keep {device_id} {keep}"));
        }

        async fn lock_all(&self) {
            self.push("lock all".to_owned());
        }
    }

    fn vault(device_id: &str) -> VaultUnlock {
        serde_json::from_value(json!({
            "deviceId": device_id,
            "password": "correct horse battery",
            "controllerVault": [1, 2, 3],
            "manifestJws": "header.payload.signature",
            "grantId": "owner",
            "apiOrigin": "https://api.flow-like.test",
            "account": "user-1",
            "keepUnlocked": false,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn a_lock_drops_the_keys_and_a_release_lets_go_only_of_what_the_area_holds() {
        let mirrors = Mirrors::new(FakeStore::default());
        mirrors.unlock("main", vault("a")).await.unwrap();
        mirrors.keep("a", true).await;
        mirrors.lock("a", LockMode::Release).await;
        mirrors.lock("a", LockMode::Release).await;
        mirrors.keep("a", false).await;
        mirrors.lock("a", LockMode::Lock).await;
        mirrors.lock("b", LockMode::Lock).await;
        assert_eq!(
            mirrors.store.events(),
            ["unlock a", "keep a true", "release a", "lock a", "lock b"],
            "a lock also reaches keys a run's prompt unlocked"
        );
    }

    #[test]
    fn lock_modes_follow_the_webview_contract() {
        let mode = |value: &str| serde_json::from_value::<LockMode>(json!(value));
        assert_eq!(mode("lock").unwrap(), LockMode::Lock);
        assert_eq!(mode("release").unwrap(), LockMode::Release);
        assert!(mode("keep").is_err());
    }

    #[tokio::test]
    async fn a_reload_releases_what_earlier_pages_unlocked() {
        let mirrors = Mirrors::new(FakeStore::default());
        mirrors.unlock("main", vault("a")).await.unwrap();
        mirrors.unlock("main", vault("b")).await.unwrap();

        mirrors.page_loaded("widget").await;
        assert_eq!(mirrors.store.events().len(), 2);

        let cleanup = mirrors.page_loaded("main");
        mirrors.unlock("main", vault("c")).await.unwrap();
        cleanup.await;
        let mut events = mirrors.store.events();
        events[3..].sort();
        assert_eq!(
            events,
            ["unlock a", "unlock b", "unlock c", "release a", "release b"]
        );

        mirrors.close_all().await;
        assert_eq!(mirrors.store.events().last().unwrap(), "release c");
    }

    #[tokio::test]
    async fn lock_all_waits_for_the_unlock_in_flight_and_locks_every_device() {
        let mirrors = Mirrors::new(FakeStore {
            unlock_delay: Some(Duration::from_millis(80)),
            ..FakeStore::default()
        });
        let unlocking = {
            let mirrors = mirrors.clone();
            tokio::spawn(async move { mirrors.unlock("main", vault("a")).await })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        mirrors.lock_all().await;
        unlocking.await.unwrap().unwrap();
        mirrors.lock("a", LockMode::Release).await;
        assert_eq!(mirrors.store.events(), ["unlock a", "lock all"]);
    }

    #[tokio::test]
    async fn a_lock_waits_for_the_unlock_in_flight() {
        let mirrors = Mirrors::new(FakeStore {
            unlock_delay: Some(Duration::from_millis(80)),
            ..FakeStore::default()
        });
        let unlocking = {
            let mirrors = mirrors.clone();
            tokio::spawn(async move { mirrors.unlock("main", vault("a")).await })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        mirrors.lock("a", LockMode::Release).await;
        unlocking.await.unwrap().unwrap();
        assert_eq!(mirrors.store.events(), ["unlock a", "release a"]);
    }
}
