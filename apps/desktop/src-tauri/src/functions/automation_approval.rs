use super::TauriFunctionError;
use super::permissions::{AutomationCapability, ensure_capabilities};
use crate::state::{TauriFlowLikeState, TauriSettingsState};
use flow_like::flow::board::Board;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

static GRANT_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static ONCE: LazyLock<Mutex<BTreeMap<String, Vec<Instant>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
const ONCE_LIFETIME: Duration = Duration::from_secs(300);

#[derive(Debug, Serialize)]
pub struct AutomationRequirements {
    pub required: Vec<AutomationCapability>,
    pub revision: String,
    pub identity: String,
    pub approved: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    Once,
    Board,
    Event,
}

fn canonical(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(fields) => {
            let sorted: BTreeMap<_, _> = fields
                .into_iter()
                .map(|(key, value)| (key, canonical(value)))
                .collect();
            serde_json::to_value(sorted).expect("JSON values serialize")
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonical).collect())
        }
        other => other,
    }
}

fn revision(board: &Board) -> anyhow::Result<String> {
    let content = canonical(serde_json::json!({
        "id": board.id, "nodes": board.nodes, "layers": board.layers,
        "variables": board.variables, "version": board.version,
        "refs": board.refs,
        "capabilities": board_capabilities(board),
    }));
    Ok(blake3::hash(&serde_json::to_vec(&content)?)
        .to_hex()
        .to_string())
}

#[cfg(desktop)]
fn boolean_input(node: &flow_like::flow::node::Node, name: &str) -> Option<bool> {
    let mut values = node
        .pins
        .values()
        .filter(|pin| pin.name == name)
        .map(|pin| {
            if !pin.depends_on.is_empty() {
                return None;
            }
            Some(
                pin.default_value
                    .as_ref()
                    .and_then(|bytes| serde_json::from_slice::<bool>(bytes).ok())
                    .unwrap_or(false),
            )
        });
    let first = values.next().unwrap_or(Some(false));
    if values.all(|value| value == first) {
        first
    } else {
        None
    }
}

pub fn board_capabilities(board: &Board) -> Vec<AutomationCapability> {
    #[cfg(desktop)]
    {
        use flow_like_catalog::automation_capabilities::required_capabilities;
        let mut capabilities = BTreeSet::new();
        for node in board
            .nodes
            .values()
            .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        {
            let mut required = required_capabilities(&node.name);
            if node.name.starts_with("computer_mouse_")
                || node.name == "computer_natural_mouse_move"
            {
                let template = boolean_input(node, "use_template_matching");
                if template == Some(false) {
                    required
                        .retain(|capability| *capability != AutomationCapability::ScreenCapture);
                }
                if template != Some(true) && boolean_input(node, "use_fingerprint") != Some(false) {
                    required.push(AutomationCapability::Accessibility);
                }
            }
            if node.name == "computer_focus_window"
                && boolean_input(node, "launch_if_not_found") != Some(false)
            {
                required.push(AutomationCapability::ApplicationLaunch);
            }
            capabilities.extend(required);
        }
        capabilities.into_iter().collect()
    }
    #[cfg(mobile)]
    {
        let _ = board;
        vec![]
    }
}

fn profile_identity(profile: &flow_like::profile::Profile) -> anyhow::Result<String> {
    Ok(
        blake3::hash(&serde_json::to_vec(&(&profile.id, &profile.hub))?)
            .to_hex()
            .to_string(),
    )
}

async fn identity(handler: &AppHandle) -> anyhow::Result<String> {
    let profile = TauriSettingsState::current_profile(handler).await?;
    profile_identity(&profile.hub_profile)
}

fn key(
    identity: &str,
    app_id: &str,
    board_id: &str,
    revision: &str,
    event_id: Option<&str>,
) -> String {
    blake3::hash(
        &serde_json::to_vec(&(identity, app_id, board_id, revision, event_id))
            .expect("strings serialize"),
    )
    .to_hex()
    .to_string()
}

fn grant_path(handler: &AppHandle) -> anyhow::Result<std::path::PathBuf> {
    Ok(handler
        .path()
        .app_data_dir()?
        .join("automation-grants.json"))
}

fn read_grants(handler: &AppHandle) -> anyhow::Result<BTreeSet<String>> {
    match std::fs::read(grant_path(handler)?) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(error) => Err(error.into()),
    }
}

fn persist_grant(handler: &AppHandle, grant: String) -> anyhow::Result<()> {
    let _guard = GRANT_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Automation grant lock failed"))?;
    let mut grants = read_grants(handler)?;
    grants.insert(grant);
    let path = grant_path(handler)?;
    std::fs::create_dir_all(
        path.parent()
            .ok_or_else(|| anyhow::anyhow!("Missing grant directory"))?,
    )?;
    let temporary = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec(&grants)?)?;
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn take_once(
    grants: &mut BTreeMap<String, Vec<Instant>>,
    grant: &str,
    now: Instant,
    consume: bool,
) -> bool {
    grants.retain(|_, expirations| {
        expirations.retain(|expiry| *expiry > now);
        !expirations.is_empty()
    });
    let Some(expirations) = grants.get_mut(grant) else {
        return false;
    };
    if consume {
        expirations.pop();
    }
    true
}

async fn has_approval(
    handler: &AppHandle,
    app_id: &str,
    board: &Board,
    event_id: Option<&str>,
    consume: Option<bool>,
    identity: &str,
) -> anyhow::Result<bool> {
    let revision = revision(board)?;
    let board_key = key(&identity, app_id, &board.id, &revision, None);
    let event_key = key(&identity, app_id, &board.id, &revision, event_id);
    {
        let _guard = GRANT_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("Automation grant lock failed"))?;
        let grants = read_grants(handler)?;
        if grants.contains(&board_key) || grants.contains(&event_key) {
            return Ok(true);
        }
    }
    let Some(consume) = consume else {
        return Ok(false);
    };
    let mut once = ONCE
        .lock()
        .map_err(|_| anyhow::anyhow!("Automation grant lock failed"))?;
    Ok(take_once(&mut once, &event_key, Instant::now(), consume))
}

pub async fn ensure_automation_approved(
    handler: &AppHandle,
    app_id: &str,
    board: &Board,
    event_id: Option<&str>,
    consume: bool,
    profile: &flow_like::profile::Profile,
) -> anyhow::Result<()> {
    let required = board_capabilities(board);
    if required.is_empty() {
        return Ok(());
    }
    ensure_capabilities(required)?;
    let identity = profile_identity(profile)?;
    if identity != self::identity(handler).await? {
        anyhow::bail!(
            "The active profile changed before automation could start. Run the workflow again."
        );
    }
    if !has_approval(
        handler,
        app_id,
        board,
        event_id,
        consume.then_some(true),
        &identity,
    )
    .await?
    {
        anyhow::bail!(
            "Automation approval is required for this workflow revision. Open it on this desktop and approve its capabilities before running it."
        );
    }
    Ok(())
}

async fn resolve(
    handler: &AppHandle,
    app_id: &str,
    board_id: &str,
    version: Option<(u32, u32, u32)>,
) -> anyhow::Result<std::sync::Arc<flow_like::flow::compiled::CompiledRunTemplate>> {
    let state = TauriFlowLikeState::construct(handler).await?;
    super::flow::run::resolve_run_template(&state, app_id, board_id, version).await
}

#[tauri::command(async)]
pub async fn get_rpa_requirements(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    version: Option<(u32, u32, u32)>,
    event_id: Option<String>,
) -> Result<AutomationRequirements, TauriFunctionError> {
    let identity = identity(&handler).await?;
    let template = resolve(&handler, &app_id, &board_id, version).await?;
    if identity != self::identity(&handler).await? {
        return Err(TauriFunctionError::new(
            "The active profile changed. Review the workflow again.",
        ));
    }
    let required = board_capabilities(&template.board);
    Ok(AutomationRequirements {
        approved: required.is_empty()
            || has_approval(
                &handler,
                &app_id,
                &template.board,
                event_id.as_deref(),
                None,
                &identity,
            )
            .await?,
        required,
        revision: revision(&template.board)?,
        identity,
    })
}

#[tauri::command(async)]
pub async fn grant_rpa_automation(
    handler: AppHandle,
    window: tauri::WebviewWindow,
    app_id: String,
    board_id: String,
    version: Option<(u32, u32, u32)>,
    event_id: Option<String>,
    expected_revision: String,
    expected_identity: String,
    scope: ApprovalScope,
) -> Result<(), TauriFunctionError> {
    if window.label() != "main" {
        return Err(TauriFunctionError::new(
            "Automation can only be approved in the main desktop window",
        ));
    }
    if identity(&handler).await? != expected_identity {
        return Err(TauriFunctionError::new(
            "The active profile changed while approval was open. Review it again.",
        ));
    }
    let template = resolve(&handler, &app_id, &board_id, version).await?;
    let actual_revision = revision(&template.board)?;
    if actual_revision != expected_revision {
        return Err(TauriFunctionError::new(
            "The workflow changed while approval was open. Review it again.",
        ));
    }
    if matches!(scope, ApprovalScope::Event) && event_id.is_none() {
        return Err(TauriFunctionError::new("Event approval requires an event"));
    }
    let identity = identity(&handler).await?;
    if identity != expected_identity {
        return Err(TauriFunctionError::new(
            "The active profile changed while approval was open. Review it again.",
        ));
    }
    let event = if matches!(scope, ApprovalScope::Board) {
        None
    } else {
        event_id.as_deref()
    };
    let grant = key(&identity, &app_id, &board_id, &actual_revision, event);
    if matches!(scope, ApprovalScope::Once) {
        ONCE.lock()
            .map_err(|_| TauriFunctionError::new("Automation grant lock failed"))?
            .entry(grant)
            .or_default()
            .push(Instant::now() + ONCE_LIFETIME);
    } else {
        persist_grant(&handler, grant)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_identity_includes_the_hub() {
        let mut profile = flow_like::profile::Profile::default();
        let original = profile_identity(&profile).unwrap();
        profile.hub = "https://another-hub.example".into();
        assert_ne!(original, profile_identity(&profile).unwrap());
    }

    #[cfg(desktop)]
    #[test]
    fn capabilities_include_nested_nodes_and_optional_template_capture() {
        use flow_like::flow::{
            board::{Layer, LayerType},
            node::Node,
            variable::VariableType,
        };
        let mut board = Board::new_detached(Some("board".into()), Default::default());
        let mut node = Node::new("computer_mouse_click", "Click", "Click", "Automation");
        node.add_input_pin(
            "use_template_matching",
            "Template",
            "Use template",
            VariableType::Boolean,
        )
        .set_default_value(Some(serde_json::json!(false)));
        board.nodes.insert(node.id.clone(), node.clone());
        assert_eq!(
            board_capabilities(&board),
            vec![AutomationCapability::InputControl]
        );

        let template = node
            .pins
            .values_mut()
            .find(|pin| pin.name == "use_template_matching")
            .unwrap();
        template.depends_on.insert("dynamic-value".into());
        board.nodes.insert(node.id.clone(), node);
        let mut layer = Layer::new("layer".into(), "Layer".into(), LayerType::Function);
        let screenshot = Node::new("computer_screenshot", "Screenshot", "Capture", "Automation");
        layer.nodes.insert(screenshot.id.clone(), screenshot);
        board.layers.insert(layer.id.clone(), layer);
        assert_eq!(
            board_capabilities(&board),
            vec![
                AutomationCapability::InputControl,
                AutomationCapability::ScreenCapture
            ]
        );
    }

    #[cfg(desktop)]
    #[test]
    fn optional_accessibility_and_application_launch_are_approved() {
        use flow_like::flow::{node::Node, variable::VariableType};
        let mut board = Board::new_detached(Some("board".into()), Default::default());
        let mut click = Node::new("computer_mouse_click", "Click", "Click", "Automation");
        click
            .add_input_pin(
                "use_fingerprint",
                "Fingerprint",
                "Match target",
                VariableType::Boolean,
            )
            .set_default_value(Some(serde_json::json!(true)));
        board.nodes.insert(click.id.clone(), click);
        assert!(board_capabilities(&board).contains(&AutomationCapability::Accessibility));

        let mut focus = Node::new("computer_focus_window", "Focus", "Focus", "Automation");
        focus
            .add_input_pin(
                "launch_if_not_found",
                "Launch",
                "Launch",
                VariableType::Boolean,
            )
            .depends_on
            .insert("dynamic".into());
        board.nodes.insert(focus.id.clone(), focus);
        assert!(board_capabilities(&board).contains(&AutomationCapability::ApplicationLaunch));
    }

    #[test]
    fn revisions_change_when_executable_node_inputs_change() {
        use flow_like::flow::{node::Node, variable::VariableType};
        let mut board = Board::new_detached(Some("board".into()), Default::default());
        let mut node = Node::new("computer_key_type", "Type", "Type", "Automation");
        node.add_input_pin("text", "Text", "Text", VariableType::String)
            .set_default_value(Some(serde_json::json!("original")));
        board.nodes.insert(node.id.clone(), node);
        let original = revision(&board).unwrap();
        let input = board
            .nodes
            .values_mut()
            .next()
            .unwrap()
            .pins
            .values_mut()
            .next()
            .unwrap();
        input.set_default_value(Some(serde_json::json!("changed")));
        assert_ne!(original, revision(&board).unwrap());
    }

    #[test]
    fn revisions_change_when_referenced_schemas_change() {
        use flow_like::flow::{node::Node, variable::VariableType};
        let mut board = Board::new_detached(Some("board".into()), Default::default());
        let mut node = Node::new("browser_execute_plan", "Plan", "Execute plan", "Automation");
        node.add_input_pin("plan", "Plan", "", VariableType::Struct)
            .schema = Some("plan-schema".into());
        board.nodes.insert(node.id.clone(), node);
        board.refs.insert(
            "plan-schema".into(),
            r#"{"type":"object","properties":{"action":{"enum":["click"]}}}"#.into(),
        );
        let original = revision(&board).unwrap();
        board.refs.insert(
            "plan-schema".into(),
            r#"{"type":"object","properties":{"action":{"enum":["click","navigate"]}}}"#.into(),
        );
        assert_ne!(original, revision(&board).unwrap());
    }

    #[test]
    fn grants_are_bound_to_identity_app_revision_and_event() {
        let original = key("profile", "app", "board", "revision", Some("event"));
        for other in [
            key("other", "app", "board", "revision", Some("event")),
            key("profile", "other", "board", "revision", Some("event")),
            key("profile", "app", "board", "changed", Some("event")),
            key("profile", "app", "board", "revision", None),
        ] {
            assert_ne!(original, other);
        }
    }
    #[test]
    fn once_grants_expire_and_are_consumed_atomically() {
        let now = Instant::now();
        let mut grants = BTreeMap::from([("run".into(), vec![now + ONCE_LIFETIME])]);
        assert!(take_once(&mut grants, "run", now, false));
        assert!(take_once(&mut grants, "run", now, true));
        assert!(!take_once(&mut grants, "run", now, true));
        grants.insert("run".into(), vec![now]);
        assert!(!take_once(&mut grants, "run", now, false));
    }
}
