use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::{Result, anyhow};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

/// What happens when a new state would exceed a capacity limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eviction {
    /// Drop the least recently used state that is not in use; fine when losing it only degrades.
    LeastRecentlyUsed,
    /// Refuse the new state until one expires; for state whose silent loss would corrupt results,
    /// such as entity ids restarting.
    Never,
}

#[derive(Clone, Copy, Debug)]
pub struct RegistryLimits {
    pub idle_ttl: Duration,
    /// States across the whole process
    pub capacity: usize,
    /// States of one app (live and replay runs counted apart), so one tenant cannot crowd out
    /// the others
    pub app_capacity: usize,
    pub eviction: Eviction,
}

/// Registry key plus the quota it counts against: its app, separately for live and replay runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateKey {
    quota: String,
    key: String,
}

/// Process-wide store for state that must outlive a single run, such as a tracker fed one camera
/// frame per run. Entries expire after `idle_ttl` without access. State is lost on process
/// restart, and a deployment that spreads runs over several processes keeps one independent state
/// per process.
pub struct StateRegistry<T> {
    label: &'static str,
    limits: RegistryLimits,
    entries: Mutex<HashMap<String, Entry<T>>>,
}

struct Entry<T> {
    quota: String,
    state: Arc<Mutex<T>>,
    last_access: Instant,
}

impl<T> Entry<T> {
    fn idle(&self) -> bool {
        Arc::strong_count(&self.state) == 1
    }
}

impl<T> StateRegistry<T> {
    pub fn new(label: &'static str, limits: RegistryLimits) -> Self {
        Self {
            label,
            limits: RegistryLimits {
                capacity: limits.capacity.max(1),
                app_capacity: limits.app_capacity.max(1),
                ..limits
            },
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Runs `f` on the state stored under `key`, creating it with `init` first if needed. A state
    /// left poisoned by a panic is replaced with a fresh one.
    pub fn with_state<R>(
        &self,
        key: &StateKey,
        init: impl Fn() -> T,
        f: impl FnOnce(&mut T) -> R,
    ) -> Result<R> {
        let state = self.entry_at(key, Instant::now(), &init)?;
        let mut guard = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = init();
                state.clear_poison();
                guard
            }
        };
        Ok(f(&mut guard))
    }

    fn entry_at(
        &self,
        key: &StateKey,
        now: Instant,
        init: impl Fn() -> T,
    ) -> Result<Arc<Mutex<T>>> {
        let mut entries = self.lock_entries();
        let idle_ttl = self.limits.idle_ttl;
        entries.retain(|_, entry| {
            !entry.idle() || now.saturating_duration_since(entry.last_access) <= idle_ttl
        });

        if let Some(entry) = entries.get_mut(&key.key) {
            entry.last_access = now;
            return Ok(entry.state.clone());
        }

        let app_entries = entries
            .values()
            .filter(|entry| entry.quota == key.quota)
            .count();
        if app_entries >= self.limits.app_capacity {
            self.make_room(&mut entries, Some(&key.quota), self.limits.app_capacity)?;
        }
        if entries.len() >= self.limits.capacity {
            self.make_room(&mut entries, None, self.limits.capacity)?;
        }

        let state = Arc::new(Mutex::new(init()));
        entries.insert(
            key.key.clone(),
            Entry {
                quota: key.quota.clone(),
                state: state.clone(),
                last_access: now,
            },
        );
        Ok(state)
    }

    fn make_room(
        &self,
        entries: &mut HashMap<String, Entry<T>>,
        quota: Option<&str>,
        limit: usize,
    ) -> Result<()> {
        let scope = if quota.is_some() {
            "for this app"
        } else {
            "in this process"
        };
        let hours = self.limits.idle_ttl.as_secs_f64() / 3600.0;
        let refuse = || {
            anyhow!(
                "{} limit of {limit} states {scope} reached; a state is released after {} without use",
                self.label,
                if hours >= 1.0 {
                    format!("{hours:.0} h")
                } else {
                    format!("{:.0} min", self.limits.idle_ttl.as_secs_f64() / 60.0)
                }
            )
        };
        if self.limits.eviction == Eviction::Never {
            return Err(refuse());
        }
        let evict = entries
            .iter()
            .filter(|(_, entry)| entry.idle() && quota.is_none_or(|quota| entry.quota == quota))
            .min_by_key(|(_, entry)| entry.last_access)
            .map(|(key, _)| key.clone())
            .ok_or_else(refuse)?;
        entries.remove(&evict);
        Ok(())
    }

    fn lock_entries(&self) -> MutexGuard<'_, HashMap<String, Entry<T>>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock_entries().len()
    }
}

/// Key shared by every user of the app, e.g. one association task fed by several devices.
/// Replay (shadow) runs never share state with live runs.
pub fn app_key(context: &ExecutionContext, parts: &[&str]) -> StateKey {
    let (app_id, _, shadow) = run_scope(context);
    StateKey {
        key: encode_key("app", shadow, &[app_id], parts),
        quota: encode_key("quota", shadow, &[app_id], &[]),
    }
}

/// Key private to the executing user within the app.
pub fn user_key(context: &ExecutionContext, parts: &[&str]) -> StateKey {
    let (app_id, sub, shadow) = run_scope(context);
    StateKey {
        key: encode_key("user", shadow, &[app_id, sub], parts),
        quota: encode_key("quota", shadow, &[app_id], &[]),
    }
}

fn run_scope(context: &ExecutionContext) -> (&str, &str, bool) {
    context
        .execution_cache
        .as_ref()
        .map(|cache| (cache.app_id.as_str(), cache.sub.as_str(), cache.shadow))
        .unwrap_or(("", "", false))
}

fn encode_key(scope: &str, shadow: bool, owner: &[&str], parts: &[&str]) -> String {
    let mut key = format!("{}:{scope}", if shadow { "shadow" } else { "live" });
    for part in owner.iter().chain(parts) {
        key.push('|');
        key.push_str(&part.len().to_string());
        key.push(':');
        key.push_str(part);
    }
    key
}

/// Current wall-clock time in Unix milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(capacity: usize, app_capacity: usize, eviction: Eviction) -> RegistryLimits {
        RegistryLimits {
            idle_ttl: Duration::from_secs(60),
            capacity,
            app_capacity,
            eviction,
        }
    }

    fn key(app: &str, name: &str) -> StateKey {
        StateKey {
            quota: app.to_string(),
            key: format!("{app}/{name}"),
        }
    }

    #[test]
    fn keys_do_not_collide_across_part_boundaries() {
        assert_ne!(
            encode_key("user", false, &["a", "b"], &["c|d"]),
            encode_key("user", false, &["a", "b"], &["c", "d"])
        );
        assert_ne!(
            encode_key("app", false, &["app"], &["x"]),
            encode_key("app", true, &["app"], &["x"])
        );
        assert_ne!(
            encode_key("user", false, &["ab", ""], &[]),
            encode_key("user", false, &["a", "b"], &[])
        );
        assert_ne!(
            encode_key("app", false, &["a"], &["x"]),
            encode_key("user", false, &["a"], &["x"])
        );
    }

    #[test]
    fn state_persists_between_calls() {
        let registry = StateRegistry::new("test", limits(4, 4, Eviction::Never));
        for expected in 1..=3 {
            let value = registry
                .with_state(
                    &key("a", "k"),
                    || 0u32,
                    |count| {
                        *count += 1;
                        *count
                    },
                )
                .unwrap();
            assert_eq!(value, expected);
        }
    }

    #[test]
    fn idle_entries_expire() {
        let registry = StateRegistry::new(
            "test",
            RegistryLimits {
                idle_ttl: Duration::from_secs(10),
                ..limits(4, 4, Eviction::Never)
            },
        );
        let start = Instant::now();
        drop(registry.entry_at(&key("a", "old"), start, || 1u32).unwrap());
        drop(
            registry
                .entry_at(&key("a", "fresh"), start + Duration::from_secs(5), || 2u32)
                .unwrap(),
        );
        drop(
            registry
                .entry_at(&key("a", "other"), start + Duration::from_secs(11), || 3u32)
                .unwrap(),
        );
        assert_eq!(registry.len(), 2);
        let old = registry
            .entry_at(&key("a", "old"), start + Duration::from_secs(11), || 9u32)
            .unwrap();
        assert_eq!(*old.lock().unwrap(), 9);
    }

    #[test]
    fn capacity_evicts_least_recently_used_idle_entry() {
        let registry = StateRegistry::new("test", limits(2, 2, Eviction::LeastRecentlyUsed));
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        drop(registry.entry_at(&key("x", "a"), at(0), || 1u32).unwrap());
        drop(registry.entry_at(&key("x", "b"), at(1), || 2u32).unwrap());
        drop(registry.entry_at(&key("x", "a"), at(2), || 0u32).unwrap());
        drop(registry.entry_at(&key("x", "c"), at(3), || 3u32).unwrap());
        assert_eq!(registry.len(), 2);
        let b = registry.entry_at(&key("x", "b"), at(4), || 7u32).unwrap();
        assert_eq!(*b.lock().unwrap(), 7);
    }

    #[test]
    fn app_quota_evicts_within_the_app_only() {
        let registry = StateRegistry::new("test", limits(10, 2, Eviction::LeastRecentlyUsed));
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        drop(
            registry
                .entry_at(&key("other", "old"), at(0), || 5u32)
                .unwrap(),
        );
        drop(
            registry
                .entry_at(&key("busy", "a"), at(1), || 1u32)
                .unwrap(),
        );
        drop(
            registry
                .entry_at(&key("busy", "b"), at(2), || 2u32)
                .unwrap(),
        );
        drop(
            registry
                .entry_at(&key("busy", "c"), at(3), || 3u32)
                .unwrap(),
        );
        assert_eq!(registry.len(), 3);
        let other = registry
            .entry_at(&key("other", "old"), at(4), || 0u32)
            .unwrap();
        assert_eq!(*other.lock().unwrap(), 5);
        let a = registry
            .entry_at(&key("busy", "a"), at(5), || 9u32)
            .unwrap();
        assert_eq!(*a.lock().unwrap(), 9);
    }

    #[test]
    fn never_eviction_refuses_instead_of_dropping_live_state() {
        let registry = StateRegistry::new("Association", limits(10, 1, Eviction::Never));
        let now = Instant::now();
        drop(registry.entry_at(&key("a", "one"), now, || 1u32).unwrap());
        let error = registry
            .entry_at(&key("a", "two"), now, || 2u32)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("Association limit of 1 states for this app"),
            "{error}"
        );
        assert!(registry.entry_at(&key("b", "two"), now, || 2u32).is_ok());
        let later = now + Duration::from_secs(61);
        assert!(registry.entry_at(&key("a", "two"), later, || 2u32).is_ok());
    }

    #[test]
    fn full_registry_of_busy_states_errors() {
        let registry = StateRegistry::new("test", limits(1, 1, Eviction::LeastRecentlyUsed));
        let now = Instant::now();
        let held = registry.entry_at(&key("a", "a"), now, || 1u32).unwrap();
        let error = registry
            .entry_at(&key("a", "b"), now, || 2u32)
            .err()
            .unwrap();
        assert!(error.to_string().contains("limit of 1"));
        drop(held);
        assert!(registry.entry_at(&key("a", "b"), now, || 2u32).is_ok());
    }

    #[test]
    fn poisoned_state_is_reset() {
        let registry = StateRegistry::new("test", limits(2, 2, Eviction::Never));
        registry
            .with_state(&key("a", "k"), || 5u32, |v| *v = 6)
            .unwrap();
        let state = registry
            .entry_at(&key("a", "k"), Instant::now(), || 0u32)
            .unwrap();
        let _ = std::thread::spawn(move || {
            let _guard = state.lock().unwrap();
            panic!("poison");
        })
        .join();
        let value = registry
            .with_state(&key("a", "k"), || 5u32, |v| *v)
            .unwrap();
        assert_eq!(value, 5);
    }
}
