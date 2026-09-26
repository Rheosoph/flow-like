use super::{HostState, read_token};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

mod seal;
pub(super) use seal::Sealer;

const TTL: Duration = Duration::from_secs(300);
const MAX_GRANTS: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Scope {
    pub app_id: String,
    pub event_id: String,
    pub page_id: String,
    pub board_id: String,
    pub board_version: [u32; 3],
    pub manifest_revision: String,
}

#[derive(Clone)]
pub(crate) struct Admission {
    pub(super) scope: Scope,
    pub(super) action_id: String,
    pub(super) capability: String,
}
impl Admission {
    pub(super) async fn validate(
        &self,
        host: &HostState,
        target: &str,
        fingerprint: blake3::Hash,
    ) -> Result<()> {
        ensure!(
            resolve(
                host,
                &self.scope,
                &self.action_id,
                &self.capability,
                fingerprint
            )
            .await?
                == target,
            "Page action target changed"
        );
        Ok(())
    }
}

pub(super) struct RunGrant {
    id: String,
    scope: Scope,
    fingerprint: blake3::Hash,
    expires: Instant,
    cancel: CancellationToken,
    // A completed render can be clicked after its stream closes. Failed and
    // cancelled producers never leave working callbacks behind.
    status: AtomicU8,
}
impl RunGrant {
    fn live(&self) -> bool {
        self.expires > Instant::now()
            && match self.status.load(Ordering::Acquire) {
                0 => !self.cancel.is_cancelled(),
                1 => true,
                _ => false,
            }
    }
}

pub(super) struct Producer {
    pub run: Arc<RunGrant>,
}
impl Producer {
    pub fn complete(&self) {
        if !self.run.cancel.is_cancelled() {
            self.run.status.store(1, Ordering::Release);
        }
    }
}
impl Drop for Producer {
    fn drop(&mut self) {
        let _ = self
            .run
            .status
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire);
    }
}

struct Entry {
    action_id: String,
    target: String,
    run: Arc<RunGrant>,
}

pub(super) struct Registry {
    pub incarnation: String,
    entries: Mutex<HashMap<String, Entry>>,
}
impl Registry {
    pub fn new(incarnation: Option<String>) -> Self {
        Self {
            incarnation: incarnation.unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string()),
            entries: Mutex::default(),
        }
    }

    pub fn begin(
        self: &Arc<Self>,
        id: String,
        scope: Scope,
        allowed: HashSet<String>,
        fingerprint: blake3::Hash,
        cancel: CancellationToken,
    ) -> (Producer, Sealer) {
        let run = Arc::new(RunGrant {
            id,
            scope,
            fingerprint,
            expires: Instant::now() + TTL,
            cancel,
            status: AtomicU8::new(0),
        });
        (
            Producer { run: run.clone() },
            Sealer::new(self.clone(), run, allowed),
        )
    }

    fn issue(&self, run: &Arc<RunGrant>, target: &str) -> Result<(String, String)> {
        ensure!(run.live(), "Page action producer is no longer active");
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("Page action registry unavailable"))?;
        entries.retain(|_, entry| entry.run.live());
        if let Some((token, entry)) = entries
            .iter()
            .find(|(_, entry)| Arc::ptr_eq(&entry.run, run) && entry.target == target)
        {
            return Ok((entry.action_id.clone(), token.clone()));
        }
        ensure!(
            entries.len() < MAX_GRANTS,
            "Page action capability limit reached"
        );
        let mut bytes = [0; 32];
        rand_core::OsRng.fill_bytes(&mut bytes);
        let token = format!(
            "sac1.{}.{}",
            self.incarnation,
            URL_SAFE_NO_PAD.encode(bytes)
        );
        let action_id = format!("da1_{}", uuid::Uuid::new_v4().simple());
        entries.insert(
            token.clone(),
            Entry {
                action_id: action_id.clone(),
                target: target.into(),
                run: run.clone(),
            },
        );
        Ok((action_id, token))
    }

    pub fn resolve_local(
        &self,
        host: &HostState,
        scope: &Scope,
        action_id: &str,
        token: &str,
        fingerprint: &str,
    ) -> Result<String> {
        ensure!(!host.cancel.is_cancelled(), "Service stopped");
        ensure!(
            owner(token)? == self.incarnation,
            "Page capability belongs to another worker"
        );
        ensure!(
            blake3::hash(&read_token(&host.secret)?).to_hex().as_str() == fingerprint,
            "Service access was rotated"
        );
        let entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("Page action registry unavailable"))?;
        let entry = entries
            .get(token)
            .context("Page action capability is unavailable")?;
        ensure!(
            entry.run.live()
                && &entry.run.scope == scope
                && entry.action_id == action_id
                && entry.run.fingerprint.to_hex().as_str() == fingerprint
                && !entry.run.id.is_empty(),
            "Page action capability scope differs or expired"
        );
        Ok(entry.target.clone())
    }
}

fn owner(token: &str) -> Result<&str> {
    let mut parts = token.split('.');
    ensure!(
        parts.next() == Some("sac1"),
        "Invalid Page action capability"
    );
    let owner = parts.next().context("Invalid Page action capability")?;
    let secret = parts.next().context("Invalid Page action capability")?;
    ensure!(
        owner.len() == 32
            && owner
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && secret.len() == 43
            && secret
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            && parts.next().is_none(),
        "Invalid Page action capability"
    );
    Ok(owner)
}

pub(super) async fn resolve(
    host: &HostState,
    scope: &Scope,
    action_id: &str,
    token: &str,
    fingerprint: blake3::Hash,
) -> Result<String> {
    let owner = owner(token)?;
    let fingerprint = fingerprint.to_hex().to_string();
    if owner == host.actions.incarnation {
        return host
            .actions
            .resolve_local(host, scope, action_id, token, &fingerprint);
    }
    #[cfg(unix)]
    if let Some(route) = &host.reply_route {
        return route
            .resolve_action(owner, scope, action_id, token, &fingerprint)
            .await;
    }
    anyhow::bail!("Page action producer is unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scope() -> Scope {
        Scope {
            app_id: "project".into(),
            event_id: "event".into(),
            page_id: "page".into(),
            board_id: "board".into(),
            board_version: [1, 0, 0],
            manifest_revision: "revision".into(),
        }
    }

    #[test]
    fn sealer_mints_only_reachable_pinned_targets_and_strips_untrusted_routes() {
        let registry = Arc::new(Registry::new(None));
        let (_producer, sealer) = registry.begin(
            "run".into(),
            scope(),
            ["entry".into()].into_iter().collect(),
            blake3::hash(b"service"),
            CancellationToken::new(),
        );
        let action = |context| json!({"name":"workflow_event", "context":context, "pageAction":{"actionId":"forged","capabilityJwt":"forged"}});
        let mut payload = json!({"type":"beginRendering", "components":[{"id":"button", "component":{"type":"button", "eventHandlers":{"click":[
            action(json!({"nodeId":"entry"})), action(json!({"nodeId":"unlisted"})),
            action(json!({"nodeId":"entry","appId":"other"})), action(json!({"nodeId":"entry","boardId":"other"}))
        ]},"actionBindings":{"unused":{"workflow":{"nodeId":"entry"}}}}}]});
        let report = sealer.seal_payload("a2ui", &mut payload);
        assert_eq!(report.sealed, 1);
        let component = &payload["components"][0]["component"];
        assert!(component.get("actionBindings").is_none());
        let actions = component["eventHandlers"]["click"].as_array().unwrap();
        assert!(
            actions[0]["pageAction"]["capabilityJwt"]
                .as_str()
                .unwrap()
                .starts_with("sac1.")
        );
        for action in actions {
            assert_eq!(action["context"], json!({}));
        }
        for action in &actions[1..] {
            assert!(action.get("pageAction").is_none());
        }
        assert_eq!(registry.entries.lock().unwrap().len(), 1);

        let mut unknown = json!({"type":"upsertElement","value":{"type":"unknown","eventHandlers":{"click":[action(json!({"nodeId":"entry"}))]},"pageAction":{"actionId":"forged"}}});
        sealer.seal_payload("a2ui", &mut unknown);
        assert!(unknown["value"].get("eventHandlers").is_none());
        assert!(unknown["value"].get("pageAction").is_none());
        let mut data = json!({"type":"dataModelUpdate","data":{"actions":[action(json!({"nodeId":"entry"}))]}});
        let unchanged = data.clone();
        assert_eq!(sealer.seal_payload("a2ui", &mut data).sealed, 0);
        assert_eq!(data, unchanged);
    }

    #[test]
    fn producer_outcome_and_deadline_bound_capability_lifetime() {
        let registry = Arc::new(Registry::new(None));
        let cancel = CancellationToken::new();
        let (failed, _) = registry.begin(
            "failed".into(),
            scope(),
            HashSet::new(),
            blake3::hash(b"service"),
            cancel.clone(),
        );
        let run = failed.run.clone();
        assert!(run.live());
        drop(failed);
        assert!(!run.live());
        assert!(registry.issue(&run, "entry").is_err());
        let (completed, _) = registry.begin(
            "complete".into(),
            scope(),
            HashSet::new(),
            blake3::hash(b"service"),
            cancel.clone(),
        );
        let run = completed.run.clone();
        completed.complete();
        drop(completed);
        cancel.cancel();
        assert!(run.live());
        let expired = Arc::new(RunGrant {
            id: "expired".into(),
            scope: scope(),
            fingerprint: blake3::hash(b"service"),
            expires: Instant::now(),
            cancel: CancellationToken::new(),
            status: AtomicU8::new(1),
        });
        assert!(!expired.live());
        assert!(registry.issue(&expired, "entry").is_err());
    }
}
