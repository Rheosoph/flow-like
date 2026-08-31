//! Prerun manifest: the board-derived, user-independent facts a client needs
//! before starting a run — runtime-configured variables, OAuth requirements,
//! WASM packages and the static element demand — as a tiny binary artifact
//! stored beside the compiled board so prerun routes never load the board.
//!
//! Kept separate from [`super::CompiledBoard`] on purpose: it has its own
//! format version and lifecycle, a format bump only makes readers recompute
//! the manifest (a microsecond board walk), never the compiled board.
//!
//! Envelope (all little-endian):
//! ```text
//! [0..4)  magic "FLPM"
//! [4..6)  MANIFEST_FORMAT_VERSION u16
//! [6]     codec (1 = lz4 size-prepended block, 0 = none)
//! [7]     reserved (0)
//! [8..)   rkyv archive, encoded per codec
//! ```

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use blake3::Hasher;
use flow_like_storage::Path;
use flow_like_types::{Result, Value, anyhow};
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

use crate::a2ui::{Page, SurfaceComponent};
use crate::flow::{
    board::{Board, ExecutionMode},
    node::{Node, NodePermission},
};

/// Bump on ANY change to the structs in this file or to what `from_board`
/// extracts. A mismatch makes readers recompute from the board.
pub const MANIFEST_FORMAT_VERSION: u16 = 2;

const LEGACY_MANIFEST_FORMAT_VERSION: u16 = 1;

/// Public action ids are deterministic, opaque references to one workflow
/// leaf configured on a versioned Page. The prefix lets a later extractor use
/// a different canonical form without making old ids ambiguous.
pub const PAGE_ACTION_ID_VERSION: u8 = 1;
pub const PAGE_ACTION_ID_PREFIX: &str = "pa1_";
pub const PAGE_ACTION_METADATA_KEY: &str = "pageAction";
pub const PAGE_EXECUTION_REVISION_VERSION: u8 = 2;
pub const PAGE_EXECUTION_REVISION_PREFIX: &str = "per2_";
pub const PAGE_SPECIAL_LOAD_MARKER: &str = "page_special:load";
pub const PAGE_SPECIAL_UNLOAD_MARKER: &str = "page_special:unload";
pub const PAGE_SPECIAL_INTERVAL_MARKER: &str = "page_special:interval";

pub const MANIFEST_MAGIC: [u8; 4] = *b"FLPM";

const HEADER_LEN: usize = 8;
const CODEC_NONE: u8 = 0;
const CODEC_LZ4: u8 = 1;
/// A manifest is 1–3 KB; anything claiming more than this is corrupt.
const MAX_DECOMPRESSED_LEN: usize = 16 * 1024 * 1024;
/// Widget references form a graph rather than a guaranteed tree. Bound the
/// recursive walk so malformed inline definitions cannot exhaust the stack.
const MAX_PAGE_WIDGET_NESTING_DEPTH: usize = 32;

/// A runtime-configured variable that needs a value before execution.
#[derive(
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Default,
    PartialEq,
)]
pub struct PrerunVariable {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub data_type: String,
    pub value_type: String,
    pub secret: bool,
    pub schema: Option<String>,
}

/// OAuth provider requirement collected from the board's nodes.
#[derive(
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Default,
    PartialEq,
)]
pub struct PrerunOAuthRequirement {
    pub provider_id: String,
    pub scopes: Vec<String>,
}

/// The stored handler slot that owns an action. Wildcard and legacy slots are
/// distinct from exact event names because the frontend resolves them with
/// different precedence.
#[derive(
    Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
pub enum PrerunPageActionHandler {
    Exact(String),
    Wildcard,
    Legacy,
}

/// Where the children rendered by a widget instance were resolved from.
///
/// The renderer prefers the Page's instance-qualified reference over an
/// inline definition. Recording that choice keeps nested locators tied to the
/// same component tree the user saw.
#[derive(
    Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
pub enum PrerunPageWidgetDefinitionSource {
    PageRef,
    Inline,
}

/// One widget host in the ancestry of a nested Page action.
///
/// Component ids are only unique within their containing Page or widget
/// definition. The ordered path disambiguates repeated child ids and also
/// binds every rendered widget instance that led to the action.
#[derive(
    Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
pub struct PrerunPageWidgetAncestor {
    pub host_component_id: String,
    pub instance_id: String,
    pub widget_id: String,
    pub definition_source: Option<PrerunPageWidgetDefinitionSource>,
}

#[derive(
    Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
pub struct PrerunPageNestedMicroWidgetLocator {
    pub widget_path: Vec<PrerunPageWidgetAncestor>,
    pub host_component_id: String,
    pub instance_id: String,
    pub package_id: String,
    pub package_version: String,
    pub widget_id: String,
    pub contract_event_name: String,
}

/// Structural location of a Page action. Widget child ids are qualified by
/// their host and instance because a definition can be rendered more than
/// once on the same Page.
#[derive(
    Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
pub enum PrerunPageActionLocator {
    Component {
        component_id: String,
        handler: PrerunPageActionHandler,
        action_index: u32,
    },
    WidgetBinding {
        host_component_id: String,
        instance_id: String,
        widget_id: String,
        widget_action_id: String,
    },
    WidgetChild {
        host_component_id: String,
        instance_id: String,
        child_component_id: String,
        handler: PrerunPageActionHandler,
        action_index: u32,
    },
    MicroWidgetBinding {
        host_component_id: String,
        instance_id: String,
        package_id: String,
        package_version: String,
        widget_id: String,
        contract_event_name: String,
    },
    /// A binding owned by a widget nested inside another widget definition.
    /// Appended to preserve the archive layout of the original v2 variants.
    NestedWidgetBinding {
        widget_path: Vec<PrerunPageWidgetAncestor>,
        widget_action_id: String,
    },
    /// An action owned by a child of a recursively nested widget.
    NestedWidgetChild {
        widget_path: Vec<PrerunPageWidgetAncestor>,
        child_component_id: String,
        handler: PrerunPageActionHandler,
        action_index: u32,
    },
    /// A micro-widget binding rendered inside one or more widget definitions.
    NestedMicroWidgetBinding {
        /// Boxed so appending this variant does not enlarge the archived v2
        /// enum layout used by the original locator variants.
        locator: Box<PrerunPageNestedMicroWidgetLocator>,
    },
}

#[derive(
    Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
pub struct PrerunPageActionEvent {
    pub action_id: String,
    pub node_id: String,
    pub locator: PrerunPageActionLocator,
}

#[derive(
    Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
pub struct PrerunPageEventTarget {
    pub node_id: String,
}

#[derive(
    Archive, RkyvSerialize, RkyvDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
pub struct PrerunPageIntervalEvent {
    pub node_id: String,
    pub interval_seconds: Option<u32>,
}

/// Reserved Page lifecycle hooks. Keeping them out of `action_events` means a
/// user-authored action name can never replace load, unload, or interval.
#[derive(
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
)]
pub struct PrerunPageSpecialEvents {
    pub load: Option<PrerunPageEventTarget>,
    pub unload: Option<PrerunPageEventTarget>,
    pub interval: Option<PrerunPageIntervalEvent>,
}

#[derive(
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
)]
pub struct PrerunPageExecution {
    pub page_id: String,
    pub action_events: Vec<PrerunPageActionEvent>,
    pub special_events: PrerunPageSpecialEvents,
}

impl PrerunPageExecution {
    /// Compile one Event-bound Page without requiring every Page listed by the
    /// board to be loaded.
    pub fn from_page(board: &Board, page: &Page) -> Result<Self> {
        extract_page_execution(board, page)
    }
}

/// Everything prerun needs that depends only on `(app, board, version)`.
#[derive(
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Default,
    PartialEq,
)]
pub struct PrerunManifest {
    pub runtime_variables: Vec<PrerunVariable>,
    pub oauth_requirements: Vec<PrerunOAuthRequirement>,
    pub requires_local_execution: bool,
    /// serde string form of [`ExecutionMode`], see [`PrerunManifest::execution_mode`].
    pub execution_mode: String,
    pub has_wasm_nodes: bool,
    pub wasm_package_ids: Vec<String>,
    /// `(package_id, NodePermission serde strings)`, sorted by package id.
    pub wasm_package_permissions: Vec<(String, Vec<String>)>,
    /// Element selectors every run of this board reads (literal refs on read pins).
    pub element_selectors: Vec<String>,
    /// The board also reads elements it can only name at run time.
    pub element_reads_dynamic: bool,
    /// Page execution bindings, populated only when the caller supplies the
    /// separately stored Pages through [`PrerunManifest::from_board_and_pages`].
    #[serde(default)]
    pub page_events: Vec<PrerunPageExecution>,
    /// blake3 over every field above; clients detect drift by comparing it.
    pub signature: String,
}

/// Exact v1 archive layout. Rkyv archives are positional and do not apply
/// serde defaults, so decoding this type and converting it is what keeps
/// already persisted manifests readable after adding Page execution data.
#[derive(Archive, RkyvSerialize, RkyvDeserialize, Debug, Clone, PartialEq)]
struct LegacyPrerunManifestV1 {
    runtime_variables: Vec<PrerunVariable>,
    oauth_requirements: Vec<PrerunOAuthRequirement>,
    requires_local_execution: bool,
    execution_mode: String,
    has_wasm_nodes: bool,
    wasm_package_ids: Vec<String>,
    wasm_package_permissions: Vec<(String, Vec<String>)>,
    element_selectors: Vec<String>,
    element_reads_dynamic: bool,
    signature: String,
}

impl From<LegacyPrerunManifestV1> for PrerunManifest {
    fn from(value: LegacyPrerunManifestV1) -> Self {
        Self {
            runtime_variables: value.runtime_variables,
            oauth_requirements: value.oauth_requirements,
            requires_local_execution: value.requires_local_execution,
            execution_mode: value.execution_mode,
            has_wasm_nodes: value.has_wasm_nodes,
            wasm_package_ids: value.wasm_package_ids,
            wasm_package_permissions: value.wasm_package_permissions,
            element_selectors: value.element_selectors,
            element_reads_dynamic: value.element_reads_dynamic,
            page_events: Vec::new(),
            signature: value.signature,
        }
    }
}

impl PrerunManifest {
    pub fn from_board(board: &Board) -> Self {
        let mut runtime_variables: Vec<PrerunVariable> = board
            .variables
            .values()
            .filter(|v| v.runtime_configured)
            .map(|v| PrerunVariable {
                id: v.id.clone(),
                name: v.name.clone(),
                description: v.description.clone(),
                data_type: format!("{:?}", v.data_type),
                value_type: format!("{:?}", v.value_type),
                secret: v.secret,
                schema: v.schema.clone(),
            })
            .collect();
        runtime_variables.sort_by(|a, b| a.id.cmp(&b.id));

        let mut nodes = NodeFacts::default();
        for node in board.nodes.values() {
            nodes.visit(node);
        }
        for layer in board.layers.values() {
            for node in layer.nodes.values() {
                nodes.visit(node);
            }
        }

        let demand = crate::a2ui::element_demand(board);

        let mut manifest = Self {
            runtime_variables,
            oauth_requirements: nodes.oauth_requirements(),
            requires_local_execution: nodes.requires_local_execution,
            execution_mode: execution_mode_string(&board.execution_mode),
            has_wasm_nodes: !nodes.wasm_package_ids.is_empty(),
            wasm_package_ids: nodes.sorted_wasm_package_ids(),
            wasm_package_permissions: nodes.wasm_package_permissions(),
            element_selectors: demand.selectors,
            element_reads_dynamic: demand.dynamic,
            page_events: Vec::new(),
            signature: String::new(),
        };
        manifest.signature = manifest.compute_signature();
        manifest
    }

    /// Build a manifest after the caller has loaded the Pages stored beside
    /// this board. `Board` deliberately contains only Page ids, so the plain
    /// synchronous [`Self::from_board`] cannot extract these bindings.
    ///
    /// This constructor is strict: every listed Page must be supplied exactly
    /// once, every Page must belong to this board, and every workflow target
    /// must be an entry node in this exact board.
    pub fn from_board_and_pages(board: &Board, pages: &[Page]) -> Result<Self> {
        let mut by_id = BTreeMap::new();
        for page in pages {
            if by_id.insert(page.id.as_str(), page).is_some() {
                return Err(anyhow!("page '{}' was supplied more than once", page.id));
            }
        }

        if !board.page_ids.is_empty() {
            let listed: BTreeSet<&str> = board.page_ids.iter().map(String::as_str).collect();
            for page_id in by_id.keys() {
                if !listed.contains(page_id) {
                    return Err(anyhow!(
                        "page '{}' is not listed by board '{}'",
                        page_id,
                        board.id
                    ));
                }
            }
            for page_id in &board.page_ids {
                if !by_id.contains_key(page_id.as_str()) {
                    return Err(anyhow!(
                        "board '{}' lists page '{}' but its payload was not supplied",
                        board.id,
                        page_id
                    ));
                }
            }
        }

        let mut page_events = Vec::with_capacity(pages.len());
        for page in by_id.values() {
            if let Some(page_board_id) = page.board_id.as_deref()
                && page_board_id != board.id
            {
                return Err(anyhow!(
                    "page '{}' belongs to board '{}', not '{}'",
                    page.id,
                    page_board_id,
                    board.id
                ));
            }
            page_events.push(extract_page_execution(board, page)?);
        }

        let mut manifest = Self::from_board(board);
        manifest.page_events = page_events;
        manifest.signature = manifest.compute_signature();
        Ok(manifest)
    }

    /// The board's execution mode; `Default` when the stored string is unknown.
    pub fn execution_mode(&self) -> ExecutionMode {
        serde_json::from_value(Value::String(self.execution_mode.clone())).unwrap_or_default()
    }

    /// WASM permissions per package; unknown permission strings are skipped.
    pub fn wasm_permissions(&self) -> HashMap<String, Vec<NodePermission>> {
        self.wasm_package_permissions
            .iter()
            .map(|(package_id, permissions)| {
                let parsed: Vec<NodePermission> = permissions
                    .iter()
                    .filter_map(|p| serde_json::from_value(Value::String(p.clone())).ok())
                    .collect();
                (package_id.clone(), parsed)
            })
            .collect()
    }

    /// Stable hash over every field, ordering collections so neither map
    /// iteration nor node walk order can shift it.
    fn compute_signature(&self) -> String {
        let mut h = Hasher::new();

        h.update(self.execution_mode.as_bytes());
        h.update(&[self.requires_local_execution as u8]);
        h.update(&[self.has_wasm_nodes as u8]);

        let mut vars: Vec<&PrerunVariable> = self.runtime_variables.iter().collect();
        vars.sort_by(|a, b| a.id.cmp(&b.id));
        for v in vars {
            h.update(b"|var|");
            h.update(v.id.as_bytes());
            h.update(b"|");
            h.update(v.name.as_bytes());
            h.update(b"|");
            h.update(v.data_type.as_bytes());
            h.update(b"|");
            h.update(v.value_type.as_bytes());
            h.update(b"|");
            h.update(&[v.secret as u8]);
            h.update(b"|");
            if let Some(d) = &v.description {
                h.update(d.as_bytes());
            }
            h.update(b"|");
            if let Some(s) = &v.schema {
                h.update(s.as_bytes());
            }
        }

        let mut providers: Vec<&PrerunOAuthRequirement> = self.oauth_requirements.iter().collect();
        providers.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
        for p in providers {
            h.update(b"|oauth|");
            h.update(p.provider_id.as_bytes());
            let mut scopes = p.scopes.clone();
            scopes.sort();
            for s in scopes {
                h.update(b"|");
                h.update(s.as_bytes());
            }
        }

        let mut ids = self.wasm_package_ids.clone();
        ids.sort();
        for id in ids {
            h.update(b"|wasm|");
            h.update(id.as_bytes());
        }

        let mut packages: Vec<&(String, Vec<String>)> =
            self.wasm_package_permissions.iter().collect();
        packages.sort_by(|a, b| a.0.cmp(&b.0));
        for (package_id, permissions) in packages {
            h.update(b"|wp|");
            h.update(package_id.as_bytes());
            let mut permissions = permissions.clone();
            permissions.sort();
            for p in permissions {
                h.update(b"|");
                h.update(p.as_bytes());
            }
        }

        let mut selectors = self.element_selectors.clone();
        selectors.sort();
        for selector in selectors {
            h.update(b"|element|");
            hash_string(&mut h, &selector);
        }
        h.update(b"|element-dynamic|");
        h.update(&[self.element_reads_dynamic as u8]);

        let mut pages: Vec<&PrerunPageExecution> = self.page_events.iter().collect();
        pages.sort_by(|a, b| a.page_id.cmp(&b.page_id));
        for page in pages {
            hash_page_execution(&mut h, page);
        }

        h.finalize().to_hex().to_string()
    }
}

/// Derive the opaque public id for one static Page action. The target node is
/// part of the digest, so retargeting an otherwise unchanged UI slot cannot
/// silently preserve an old action id.
pub fn page_action_id(page_id: &str, node_id: &str, locator: &PrerunPageActionLocator) -> String {
    let mut h = Hasher::new();
    h.update(b"flow-like/page-action");
    h.update(&[PAGE_ACTION_ID_VERSION]);
    hash_string(&mut h, page_id);
    hash_string(&mut h, node_id);
    hash_locator(&mut h, locator);
    format!("{PAGE_ACTION_ID_PREFIX}{}", h.finalize().to_hex())
}

/// Bind one Page's static execution authority to its exact board snapshot.
///
/// Board identity, immutable version, and the deterministic semantic board
/// hash are included in addition to prerun requirements. A local project with
/// a stale or different workflow can therefore never match the server Page
/// revision merely because its runtime requirements and Page locators happen
/// to be equal.
pub fn page_execution_revision(board: &Board, execution: &PrerunPageExecution) -> Result<String> {
    let board_manifest = PrerunManifest::from_board(board);
    if !board.page_ids.is_empty()
        && !board
            .page_ids
            .iter()
            .any(|page_id| page_id == &execution.page_id)
    {
        return Err(anyhow!(
            "page '{}' is not listed by board '{}'",
            execution.page_id,
            board.id
        ));
    }

    let mut action_ids = BTreeSet::new();
    for action in &execution.action_events {
        validate_entry_node(
            board,
            &execution.page_id,
            "compiled action",
            &action.node_id,
        )?;
        let expected = page_action_id(&execution.page_id, &action.node_id, &action.locator);
        if action.action_id != expected {
            return Err(anyhow!(
                "page '{}' action '{}' is not canonical",
                execution.page_id,
                action.action_id
            ));
        }
        if !action_ids.insert(action.action_id.as_str()) {
            return Err(anyhow!(
                "page '{}' contains duplicate action id '{}'",
                execution.page_id,
                action.action_id
            ));
        }
    }
    if let Some(load) = &execution.special_events.load {
        validate_entry_node(board, &execution.page_id, "compiled load", &load.node_id)?;
    }
    if let Some(unload) = &execution.special_events.unload {
        validate_entry_node(
            board,
            &execution.page_id,
            "compiled unload",
            &unload.node_id,
        )?;
    }
    if let Some(interval) = &execution.special_events.interval {
        validate_entry_node(
            board,
            &execution.page_id,
            "compiled interval",
            &interval.node_id,
        )?;
        if interval.interval_seconds.is_none_or(|seconds| seconds == 0) {
            return Err(anyhow!(
                "page '{}' compiled interval is missing a positive period",
                execution.page_id
            ));
        }
    }

    let mut h = Hasher::new();
    h.update(b"flow-like/page-execution-revision");
    h.update(&[PAGE_EXECUTION_REVISION_VERSION]);
    hash_string(&mut h, &board.id);
    h.update(&board.version.0.to_le_bytes());
    h.update(&board.version.1.to_le_bytes());
    h.update(&board.version.2.to_le_bytes());
    h.update(&board.content_hash().to_le_bytes());
    hash_string(&mut h, &board_manifest.signature);
    hash_page_execution(&mut h, execution);
    Ok(format!(
        "{PAGE_EXECUTION_REVISION_PREFIX}{}",
        h.finalize().to_hex()
    ))
}

fn hash_string(h: &mut Hasher, value: &str) {
    h.update(&(value.len() as u64).to_le_bytes());
    h.update(value.as_bytes());
}

fn hash_handler(h: &mut Hasher, handler: &PrerunPageActionHandler) {
    match handler {
        PrerunPageActionHandler::Exact(event_name) => {
            h.update(&[0]);
            hash_string(h, event_name);
        }
        PrerunPageActionHandler::Wildcard => {
            h.update(&[1]);
        }
        PrerunPageActionHandler::Legacy => {
            h.update(&[2]);
        }
    }
}

fn hash_widget_path(h: &mut Hasher, widget_path: &[PrerunPageWidgetAncestor]) {
    h.update(&(widget_path.len() as u64).to_le_bytes());
    for ancestor in widget_path {
        hash_string(h, &ancestor.host_component_id);
        hash_string(h, &ancestor.instance_id);
        hash_string(h, &ancestor.widget_id);
        match ancestor.definition_source {
            Some(PrerunPageWidgetDefinitionSource::PageRef) => h.update(&[1]),
            Some(PrerunPageWidgetDefinitionSource::Inline) => h.update(&[2]),
            None => h.update(&[0]),
        };
    }
}

fn hash_locator(h: &mut Hasher, locator: &PrerunPageActionLocator) {
    match locator {
        PrerunPageActionLocator::Component {
            component_id,
            handler,
            action_index,
        } => {
            h.update(&[0]);
            hash_string(h, component_id);
            hash_handler(h, handler);
            h.update(&action_index.to_le_bytes());
        }
        PrerunPageActionLocator::WidgetBinding {
            host_component_id,
            instance_id,
            widget_id,
            widget_action_id,
        } => {
            h.update(&[1]);
            hash_string(h, host_component_id);
            hash_string(h, instance_id);
            hash_string(h, widget_id);
            hash_string(h, widget_action_id);
        }
        PrerunPageActionLocator::WidgetChild {
            host_component_id,
            instance_id,
            child_component_id,
            handler,
            action_index,
        } => {
            h.update(&[2]);
            hash_string(h, host_component_id);
            hash_string(h, instance_id);
            hash_string(h, child_component_id);
            hash_handler(h, handler);
            h.update(&action_index.to_le_bytes());
        }
        PrerunPageActionLocator::MicroWidgetBinding {
            host_component_id,
            instance_id,
            package_id,
            package_version,
            widget_id,
            contract_event_name,
        } => {
            h.update(&[3]);
            hash_string(h, host_component_id);
            hash_string(h, instance_id);
            hash_string(h, package_id);
            hash_string(h, package_version);
            hash_string(h, widget_id);
            hash_string(h, contract_event_name);
        }
        PrerunPageActionLocator::NestedWidgetBinding {
            widget_path,
            widget_action_id,
        } => {
            h.update(&[4]);
            hash_widget_path(h, widget_path);
            hash_string(h, widget_action_id);
        }
        PrerunPageActionLocator::NestedWidgetChild {
            widget_path,
            child_component_id,
            handler,
            action_index,
        } => {
            h.update(&[5]);
            hash_widget_path(h, widget_path);
            hash_string(h, child_component_id);
            hash_handler(h, handler);
            h.update(&action_index.to_le_bytes());
        }
        PrerunPageActionLocator::NestedMicroWidgetBinding { locator } => {
            h.update(&[6]);
            hash_widget_path(h, &locator.widget_path);
            hash_string(h, &locator.host_component_id);
            hash_string(h, &locator.instance_id);
            hash_string(h, &locator.package_id);
            hash_string(h, &locator.package_version);
            hash_string(h, &locator.widget_id);
            hash_string(h, &locator.contract_event_name);
        }
    }
}

fn hash_optional_target(h: &mut Hasher, label: &[u8], target: Option<&PrerunPageEventTarget>) {
    h.update(b"|");
    h.update(label);
    h.update(b"|");
    if let Some(target) = target {
        h.update(&[1]);
        hash_string(h, &target.node_id);
    } else {
        h.update(&[0]);
    }
}

fn hash_page_execution(h: &mut Hasher, page: &PrerunPageExecution) {
    h.update(b"|page|");
    hash_string(h, &page.page_id);
    hash_optional_target(h, b"load", page.special_events.load.as_ref());
    hash_optional_target(h, b"unload", page.special_events.unload.as_ref());
    h.update(b"|interval|");
    if let Some(interval) = &page.special_events.interval {
        h.update(&[1]);
        hash_string(h, &interval.node_id);
        match interval.interval_seconds {
            Some(seconds) => {
                h.update(&[1]);
                h.update(&seconds.to_le_bytes());
            }
            None => {
                h.update(&[0]);
            }
        }
    } else {
        h.update(&[0]);
    }

    let mut actions: Vec<&PrerunPageActionEvent> = page.action_events.iter().collect();
    actions.sort_by(|a, b| {
        a.action_id
            .cmp(&b.action_id)
            .then_with(|| a.node_id.cmp(&b.node_id))
    });
    for action in actions {
        h.update(b"|action|");
        hash_string(h, &action.action_id);
        hash_string(h, &action.node_id);
        hash_locator(h, &action.locator);
    }
}

enum ComponentActionScope<'a> {
    Component {
        component_id: &'a str,
    },
    WidgetChild {
        widget_path: &'a [PrerunPageWidgetAncestor],
        child_component_id: &'a str,
    },
}

impl ComponentActionScope<'_> {
    fn locator(
        &self,
        handler: PrerunPageActionHandler,
        action_index: u32,
    ) -> PrerunPageActionLocator {
        match self {
            Self::Component { component_id } => PrerunPageActionLocator::Component {
                component_id: (*component_id).to_string(),
                handler,
                action_index,
            },
            Self::WidgetChild {
                widget_path,
                child_component_id,
            } => match *widget_path {
                [ancestor] => PrerunPageActionLocator::WidgetChild {
                    host_component_id: ancestor.host_component_id.clone(),
                    instance_id: ancestor.instance_id.clone(),
                    child_component_id: (*child_component_id).to_string(),
                    handler,
                    action_index,
                },
                _ => PrerunPageActionLocator::NestedWidgetChild {
                    widget_path: widget_path.to_vec(),
                    child_component_id: (*child_component_id).to_string(),
                    handler,
                    action_index,
                },
            },
        }
    }
}

/// Extract and validate all static execution bindings from one Page payload.
/// The caller must pass the exact board version that owns the Page.
pub fn extract_page_execution(board: &Board, page: &Page) -> Result<PrerunPageExecution> {
    if let Some(page_board_id) = page.board_id.as_deref()
        && page_board_id != board.id
    {
        return Err(anyhow!(
            "page '{}' belongs to board '{}', not '{}'",
            page.id,
            page_board_id,
            board.id
        ));
    }

    let mut action_events = Vec::new();
    let mut component_ids = BTreeSet::new();
    let mut widget_traversal = PageWidgetTraversal::default();

    for component in &page.components {
        if component.id.is_empty() {
            return Err(anyhow!("page '{}' contains an empty component id", page.id));
        }
        if !component_ids.insert(component.id.as_str()) {
            return Err(anyhow!(
                "page '{}' contains duplicate component id '{}'",
                page.id,
                component.id
            ));
        }

        extract_component_actions(
            board,
            &page.id,
            &component.component,
            ComponentActionScope::Component {
                component_id: &component.id,
            },
            &mut action_events,
        )?;

        match component_type(&component.component) {
            Some("widgetInstance") => extract_widget_bindings(
                board,
                page,
                &component.id,
                &component.component,
                &[],
                &mut widget_traversal,
                &mut action_events,
            )?,
            Some("microWidgetInstance") => extract_micro_widget_bindings(
                board,
                page,
                &component.id,
                &component.component,
                &[],
                &mut widget_traversal,
                &mut action_events,
            )?,
            _ => {}
        }
    }

    action_events.sort_by(|a, b| a.action_id.cmp(&b.action_id));
    for pair in action_events.windows(2) {
        if pair[0].action_id == pair[1].action_id {
            return Err(anyhow!(
                "page '{}' produced duplicate action id '{}'",
                page.id,
                pair[0].action_id
            ));
        }
    }

    let load = optional_special_target(board, page, "load", page.on_load_event_id.as_deref())?;
    let unload =
        optional_special_target(board, page, "unload", page.on_unload_event_id.as_deref())?;
    let interval = match page.on_interval_event_id.as_deref() {
        Some(node_id) if !node_id.is_empty() => {
            validate_entry_node(board, &page.id, "special interval", node_id)?;
            let seconds = page
                .on_interval_seconds
                .filter(|seconds| *seconds > 0)
                .ok_or_else(|| {
                    anyhow!(
                        "page '{}' configures interval node '{}' without a positive interval",
                        page.id,
                        node_id
                    )
                })?;
            Some(PrerunPageIntervalEvent {
                node_id: node_id.to_string(),
                interval_seconds: Some(seconds),
            })
        }
        _ => None,
    };

    Ok(PrerunPageExecution {
        page_id: page.id.clone(),
        action_events,
        special_events: PrerunPageSpecialEvents {
            load,
            unload,
            interval,
        },
    })
}

/// Clone a Page and annotate each trusted workflow action or binding with the
/// opaque id the Event endpoint accepts. Raw node ids remain in place for
/// legacy clients; new clients must route through `pageAction` metadata.
pub fn decorate_page_actions(
    page: &Page,
    execution: &PrerunPageExecution,
    manifest_revision: &str,
) -> Result<Page> {
    if execution.page_id != page.id {
        return Err(anyhow!(
            "page execution data for '{}' cannot decorate page '{}'",
            execution.page_id,
            page.id
        ));
    }

    let mut decorated_value = serde_json::to_value(page)
        .map_err(|error| anyhow!("failed to encode Page for action decoration: {error}"))?;
    let mut metadata_path = Vec::new();
    strip_spoofed_page_action_metadata(&mut decorated_value, &mut metadata_path);
    let mut decorated: Page = serde_json::from_value(decorated_value)
        .map_err(|error| anyhow!("failed to decode Page for action decoration: {error}"))?;
    for action in &execution.action_events {
        if action.action_id != page_action_id(&page.id, &action.node_id, &action.locator) {
            return Err(anyhow!(
                "page '{}' action '{}' does not match its canonical locator",
                page.id,
                action.action_id
            ));
        }

        let metadata = serde_json::json!({
            "actionId": action.action_id,
            "manifestRevision": manifest_revision,
        });

        match &action.locator {
            PrerunPageActionLocator::Component {
                component_id,
                handler,
                action_index,
            } => {
                let component = find_page_component_mut(&mut decorated, component_id)?;
                let slot = action_slot_mut(&mut component.component, handler, *action_index)
                    .ok_or_else(|| {
                        anyhow!(
                            "page '{}' action '{}' no longer resolves to component '{}'",
                            page.id,
                            action.action_id,
                            component_id
                        )
                    })?;
                decorate_workflow_action(slot, action, metadata)?;
            }
            PrerunPageActionLocator::WidgetBinding {
                host_component_id,
                widget_action_id,
                ..
            }
            | PrerunPageActionLocator::MicroWidgetBinding {
                host_component_id,
                contract_event_name: widget_action_id,
                ..
            } => {
                let component = find_page_component_mut(&mut decorated, host_component_id)?;
                let slot = binding_slot_mut(&mut component.component, widget_action_id)
                    .ok_or_else(|| {
                        anyhow!(
                            "page '{}' action '{}' no longer resolves to binding '{}'",
                            page.id,
                            action.action_id,
                            widget_action_id
                        )
                    })?;
                decorate_workflow_binding(slot, action, metadata)?;
            }
            PrerunPageActionLocator::WidgetChild {
                host_component_id,
                instance_id,
                child_component_id,
                handler,
                action_index,
            } => {
                let slot = if let Some(widget) = decorated.widget_refs.get_mut(instance_id) {
                    let child = widget
                        .components
                        .iter_mut()
                        .find(|component| component.id == *child_component_id)
                        .ok_or_else(|| {
                            anyhow!(
                                "page '{}' widget instance '{}' no longer contains child '{}'",
                                page.id,
                                instance_id,
                                child_component_id
                            )
                        })?;
                    action_slot_mut(&mut child.component, handler, *action_index)
                } else {
                    let host = find_page_component_mut(&mut decorated, host_component_id)?;
                    inline_widget_action_slot_mut(
                        &mut host.component,
                        child_component_id,
                        handler,
                        *action_index,
                    )
                }
                .ok_or_else(|| {
                    anyhow!(
                        "page '{}' action '{}' no longer resolves inside widget '{}'",
                        page.id,
                        action.action_id,
                        instance_id
                    )
                })?;
                decorate_workflow_action(slot, action, metadata)?;
            }
            PrerunPageActionLocator::NestedWidgetBinding { .. }
            | PrerunPageActionLocator::NestedWidgetChild { .. }
            | PrerunPageActionLocator::NestedMicroWidgetBinding { .. } => {
                decorate_nested_page_action(&mut decorated, action, metadata)?;
            }
        }
    }

    Ok(decorated)
}

fn strip_spoofed_page_action_metadata(value: &mut Value, path: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if is_page_action_slot(path) || is_page_binding_slot(path) {
                map.remove(PAGE_ACTION_METADATA_KEY);
                map.remove("page_action");
            }

            for (key, child) in map.iter_mut() {
                if matches!(key.as_str(), "literalJson" | "literal_json")
                    && let Value::String(raw) = child
                {
                    strip_spoofed_embedded_page_json(raw, path);
                    continue;
                }
                path.push(key.clone());
                strip_spoofed_page_action_metadata(child, path);
                path.pop();
            }
        }
        Value::Array(items) => {
            if is_page_json_byte_blob(path)
                && let Some(bytes) = json_byte_array(items)
                && let Ok(mut parsed) = serde_json::from_slice::<Value>(&bytes)
            {
                let before = parsed.clone();
                path.push("$jsonBytes".to_string());
                strip_spoofed_page_action_metadata(&mut parsed, path);
                path.pop();
                if parsed != before
                    && let Ok(encoded) = serde_json::to_vec(&parsed)
                {
                    *items = encoded
                        .into_iter()
                        .map(|byte| Value::Number(byte.into()))
                        .collect();
                    return;
                }
            }

            for (index, child) in items.iter_mut().enumerate() {
                path.push(index.to_string());
                strip_spoofed_page_action_metadata(child, path);
                path.pop();
            }
        }
        _ => {}
    }
}

fn strip_spoofed_embedded_page_json(raw: &mut String, path: &mut Vec<String>) {
    let trimmed = raw.trim_start();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return;
    }
    let Ok(mut parsed) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let before = parsed.clone();
    path.push("$literalJson".to_string());
    strip_spoofed_page_action_metadata(&mut parsed, path);
    path.pop();
    if parsed != before
        && let Ok(encoded) = serde_json::to_string(&parsed)
    {
        *raw = encoded;
    }
}

#[derive(Clone)]
enum PageValuePathSegment {
    Key(String),
    Index(usize),
}

fn decorate_nested_page_action(
    page: &mut Page,
    action: &PrerunPageActionEvent,
    metadata: Value,
) -> Result<()> {
    let mut value = serde_json::to_value(&*page)
        .map_err(|error| anyhow!("failed to encode Page for nested action decoration: {error}"))?;

    match &action.locator {
        PrerunPageActionLocator::NestedWidgetBinding {
            widget_path,
            widget_action_id,
        } => {
            let component_path = resolve_widget_host_path(&value, &page.id, widget_path)?;
            let component = value_at_page_path_mut(&mut value, &component_path)
                .expect("a path resolved against the same Page value");
            let slot = binding_slot_mut(component, widget_action_id).ok_or_else(|| {
                anyhow!(
                    "page '{}' action '{}' no longer resolves to nested binding '{}'",
                    page.id,
                    action.action_id,
                    widget_action_id
                )
            })?;
            decorate_workflow_binding(slot, action, metadata)?;
        }
        PrerunPageActionLocator::NestedWidgetChild {
            widget_path,
            child_component_id,
            handler,
            action_index,
        } => {
            let component_path =
                resolve_widget_child_path(&value, &page.id, widget_path, child_component_id)?;
            let component = value_at_page_path_mut(&mut value, &component_path)
                .expect("a path resolved against the same Page value");
            let slot = action_slot_mut(component, handler, *action_index).ok_or_else(|| {
                anyhow!(
                    "page '{}' action '{}' no longer resolves to nested component '{}'",
                    page.id,
                    action.action_id,
                    child_component_id
                )
            })?;
            decorate_workflow_action(slot, action, metadata)?;
        }
        PrerunPageActionLocator::NestedMicroWidgetBinding { locator } => {
            let component_path = resolve_widget_child_path(
                &value,
                &page.id,
                &locator.widget_path,
                &locator.host_component_id,
            )?;
            let component = value_at_page_path_mut(&mut value, &component_path)
                .expect("a path resolved against the same Page value");
            validate_micro_widget_locator(component, &page.id, locator)?;
            let slot = binding_slot_mut(component, &locator.contract_event_name).ok_or_else(|| {
                anyhow!(
                    "page '{}' action '{}' no longer resolves to nested micro-widget binding '{}'",
                    page.id,
                    action.action_id,
                    locator.contract_event_name
                )
            })?;
            decorate_workflow_binding(slot, action, metadata)?;
        }
        _ => unreachable!("only nested locators use nested decoration"),
    }

    *page = serde_json::from_value(value)
        .map_err(|error| anyhow!("failed to decode decorated nested Page: {error}"))?;
    Ok(())
}

fn resolve_widget_host_path(
    page: &Value,
    page_id: &str,
    widget_path: &[PrerunPageWidgetAncestor],
) -> Result<Vec<PageValuePathSegment>> {
    let Some(first) = widget_path.first() else {
        return Err(anyhow!(
            "page '{}' contains an empty nested widget path",
            page_id
        ));
    };
    let top_components = vec![PageValuePathSegment::Key("components".to_string())];
    let mut host_path =
        surface_component_value_path(page, &top_components, &first.host_component_id).ok_or_else(
            || {
                anyhow!(
                    "page '{}' no longer contains widget host '{}'",
                    page_id,
                    first.host_component_id
                )
            },
        )?;
    validate_widget_ancestor(
        value_at_page_path(page, &host_path).expect("resolved Page component path"),
        page_id,
        first,
    )?;

    for (parent, ancestor) in widget_path.iter().zip(widget_path.iter().skip(1)) {
        let child_components_path = widget_children_value_path(page, &host_path, parent, page_id)?;
        host_path =
            surface_component_value_path(page, &child_components_path, &ancestor.host_component_id)
                .ok_or_else(|| {
                    anyhow!(
                        "page '{}' widget instance '{}' no longer contains nested host '{}'",
                        page_id,
                        parent.instance_id,
                        ancestor.host_component_id
                    )
                })?;
        validate_widget_ancestor(
            value_at_page_path(page, &host_path).expect("resolved nested component path"),
            page_id,
            ancestor,
        )?;
    }

    Ok(host_path)
}

fn resolve_widget_child_path(
    page: &Value,
    page_id: &str,
    widget_path: &[PrerunPageWidgetAncestor],
    child_component_id: &str,
) -> Result<Vec<PageValuePathSegment>> {
    let host_path = resolve_widget_host_path(page, page_id, widget_path)?;
    let owner = widget_path
        .last()
        .expect("resolve_widget_host_path rejects empty paths");
    let children_path = widget_children_value_path(page, &host_path, owner, page_id)?;
    surface_component_value_path(page, &children_path, child_component_id).ok_or_else(|| {
        anyhow!(
            "page '{}' widget instance '{}' no longer contains child '{}'",
            page_id,
            owner.instance_id,
            child_component_id
        )
    })
}

fn widget_children_value_path(
    page: &Value,
    host_path: &[PageValuePathSegment],
    ancestor: &PrerunPageWidgetAncestor,
    page_id: &str,
) -> Result<Vec<PageValuePathSegment>> {
    match ancestor.definition_source {
        Some(PrerunPageWidgetDefinitionSource::PageRef) => {
            let path = vec![
                PageValuePathSegment::Key("widgetRefs".to_string()),
                PageValuePathSegment::Key(ancestor.instance_id.clone()),
                PageValuePathSegment::Key("components".to_string()),
            ];
            if value_at_page_path(page, &path).is_none() {
                return Err(anyhow!(
                    "page '{}' no longer contains widget reference '{}'",
                    page_id,
                    ancestor.instance_id
                ));
            }
            Ok(path)
        }
        Some(PrerunPageWidgetDefinitionSource::Inline) => {
            if page
                .get("widgetRefs")
                .and_then(Value::as_object)
                .is_some_and(|refs| refs.contains_key(&ancestor.instance_id))
            {
                return Err(anyhow!(
                    "page '{}' widget instance '{}' changed from inline to referenced",
                    page_id,
                    ancestor.instance_id
                ));
            }
            let mut path = host_path.to_vec();
            path.extend([
                PageValuePathSegment::Key("inlineWidgetDef".to_string()),
                PageValuePathSegment::Key("components".to_string()),
            ]);
            if value_at_page_path(page, &path).is_none() {
                return Err(anyhow!(
                    "page '{}' widget instance '{}' no longer has an inline definition",
                    page_id,
                    ancestor.instance_id
                ));
            }
            Ok(path)
        }
        None => Err(anyhow!(
            "page '{}' widget instance '{}' has no recorded child definition",
            page_id,
            ancestor.instance_id
        )),
    }
}

fn surface_component_value_path(
    page: &Value,
    components_path: &[PageValuePathSegment],
    component_id: &str,
) -> Option<Vec<PageValuePathSegment>> {
    let components = value_at_page_path(page, components_path)?.as_array()?;
    let index = components.iter().position(|component| {
        component
            .as_object()
            .and_then(|component| component.get("id"))
            .and_then(Value::as_str)
            == Some(component_id)
    })?;
    let mut path = components_path.to_vec();
    path.push(PageValuePathSegment::Index(index));
    path.push(PageValuePathSegment::Key("component".to_string()));
    Some(path)
}

fn value_at_page_path<'a>(
    mut value: &'a Value,
    path: &[PageValuePathSegment],
) -> Option<&'a Value> {
    for segment in path {
        value = match segment {
            PageValuePathSegment::Key(key) => value.as_object()?.get(key)?,
            PageValuePathSegment::Index(index) => value.as_array()?.get(*index)?,
        };
    }
    Some(value)
}

fn value_at_page_path_mut<'a>(
    mut value: &'a mut Value,
    path: &[PageValuePathSegment],
) -> Option<&'a mut Value> {
    for segment in path {
        value = match segment {
            PageValuePathSegment::Key(key) => value.as_object_mut()?.get_mut(key)?,
            PageValuePathSegment::Index(index) => value.as_array_mut()?.get_mut(*index)?,
        };
    }
    Some(value)
}

fn validate_widget_ancestor(
    component: &Value,
    page_id: &str,
    ancestor: &PrerunPageWidgetAncestor,
) -> Result<()> {
    let component = component.as_object().ok_or_else(|| {
        anyhow!(
            "page '{}' widget host '{}' is no longer an object",
            page_id,
            ancestor.host_component_id
        )
    })?;
    if component.get("type").and_then(Value::as_str) != Some("widgetInstance")
        || component.get("instanceId").and_then(Value::as_str) != Some(&ancestor.instance_id)
        || component.get("widgetId").and_then(Value::as_str) != Some(&ancestor.widget_id)
    {
        return Err(anyhow!(
            "page '{}' widget host '{}' changed identity after extraction",
            page_id,
            ancestor.host_component_id
        ));
    }
    Ok(())
}

fn validate_micro_widget_locator(
    component: &Value,
    page_id: &str,
    locator: &PrerunPageNestedMicroWidgetLocator,
) -> Result<()> {
    let component = component.as_object().ok_or_else(|| {
        anyhow!(
            "page '{}' micro-widget host '{}' is no longer an object",
            page_id,
            locator.host_component_id
        )
    })?;
    let identity_matches = component.get("type").and_then(Value::as_str)
        == Some("microWidgetInstance")
        && component.get("instanceId").and_then(Value::as_str) == Some(&locator.instance_id)
        && component.get("packageId").and_then(Value::as_str) == Some(&locator.package_id)
        && component.get("packageVersion").and_then(Value::as_str)
            == Some(&locator.package_version)
        && component.get("widgetId").and_then(Value::as_str) == Some(&locator.widget_id);
    if !identity_matches {
        return Err(anyhow!(
            "page '{}' micro-widget host '{}' changed identity after extraction",
            page_id,
            locator.host_component_id
        ));
    }
    Ok(())
}

/// Remove raw workflow routing from a projected runtime Page.
///
/// Callers without direct Board execution authority receive only opaque
/// `pageAction` selectors. The Event resolver remains the sole component that
/// can turn those selectors into board entry nodes. Reserved lifecycle
/// markers retain the frontend's configured/not-configured signal without
/// exposing their node ids.
pub fn redact_page_execution_routes(page: &Page) -> Result<Page> {
    let mut value = serde_json::to_value(page)
        .map_err(|error| anyhow!("failed to encode Page for route redaction: {error}"))?;
    let Some(root) = value.as_object_mut() else {
        return Err(anyhow!("encoded Page is not an object"));
    };

    for (field, marker) in [
        ("onLoadEventId", PAGE_SPECIAL_LOAD_MARKER),
        ("onUnloadEventId", PAGE_SPECIAL_UNLOAD_MARKER),
        ("onIntervalEventId", PAGE_SPECIAL_INTERVAL_MARKER),
    ] {
        if root
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|target| !target.is_empty())
        {
            root.insert(field.to_string(), Value::String(marker.to_string()));
        }
    }

    let mut path = Vec::new();
    redact_page_value(&mut value, &mut path);
    serde_json::from_value(value)
        .map_err(|error| anyhow!("failed to decode redacted Page: {error}"))
}

fn redact_page_value(value: &mut Value, path: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if is_page_action_slot(path) && is_workflow_action_object(map) {
                if let Some(context) = map.get_mut("context").and_then(Value::as_object_mut) {
                    remove_page_routing_keys(context);
                }
            } else if is_page_binding_slot(path) {
                if let Some(workflow) = page_workflow_binding_mut(map) {
                    remove_page_routing_keys(workflow);
                }
            }

            for (key, child) in map.iter_mut() {
                if matches!(key.as_str(), "literalJson" | "literal_json")
                    && let Value::String(raw) = child
                {
                    redact_embedded_page_json(raw, path);
                    continue;
                }
                path.push(key.clone());
                redact_page_value(child, path);
                path.pop();
            }
        }
        Value::Array(items) => {
            if is_page_json_byte_blob(path)
                && let Some(bytes) = json_byte_array(items)
                && let Ok(mut parsed) = serde_json::from_slice::<Value>(&bytes)
            {
                let before = parsed.clone();
                path.push("$jsonBytes".to_string());
                redact_page_value(&mut parsed, path);
                path.pop();
                if parsed != before
                    && let Ok(encoded) = serde_json::to_vec(&parsed)
                {
                    *items = encoded
                        .into_iter()
                        .map(|byte| Value::Number(byte.into()))
                        .collect();
                    return;
                }
            }

            for (index, child) in items.iter_mut().enumerate() {
                path.push(index.to_string());
                redact_page_value(child, path);
                path.pop();
            }
        }
        _ => {}
    }
}

fn redact_embedded_page_json(raw: &mut String, path: &mut Vec<String>) {
    let trimmed = raw.trim_start();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return;
    }
    let Ok(mut parsed) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let before = parsed.clone();
    path.push("$literalJson".to_string());
    redact_page_value(&mut parsed, path);
    path.pop();
    if parsed != before
        && let Ok(encoded) = serde_json::to_string(&parsed)
    {
        *raw = encoded;
    }
}

fn is_page_action_slot(path: &[String]) -> bool {
    let Some(index) = path.last() else {
        return false;
    };
    if index.parse::<usize>().is_err() {
        return false;
    }
    matches!(
        path.get(path.len().saturating_sub(2)).map(String::as_str),
        Some("actions")
    ) || matches!(
        path.get(path.len().saturating_sub(3)).map(String::as_str),
        Some("eventHandlers" | "event_handlers")
    )
}

fn is_page_binding_slot(path: &[String]) -> bool {
    matches!(
        path.get(path.len().saturating_sub(2)).map(String::as_str),
        Some("actionBindings")
    )
}

fn is_workflow_action_object(map: &serde_json::Map<String, Value>) -> bool {
    map.get("name").and_then(Value::as_str) == Some("workflow_event")
        && map.get("context").is_some_and(Value::is_object)
}

fn page_workflow_binding_mut(
    map: &mut serde_json::Map<String, Value>,
) -> Option<&mut serde_json::Map<String, Value>> {
    for key in ["workflow", "workflowEvent", "workflow_event"] {
        if map.get(key).is_some_and(Value::is_object) {
            return map.get_mut(key).and_then(Value::as_object_mut);
        }
    }
    None
}

fn remove_page_routing_keys(map: &mut serde_json::Map<String, Value>) {
    for key in [
        "nodeId",
        "node_id",
        "flowId",
        "flow_id",
        "eventId",
        "event_id",
        "appId",
        "app_id",
        "boardId",
        "board_id",
        "boardVersion",
        "board_version",
    ] {
        map.remove(key);
    }
}

fn is_page_json_byte_blob(path: &[String]) -> bool {
    matches!(
        path.last().map(String::as_str),
        Some("defaultValue" | "default_value")
    ) || matches!(
        path.get(path.len().saturating_sub(2)).map(String::as_str),
        Some(
            "exposedPropValues"
                | "exposed_prop_values"
                | "customizationValues"
                | "customization_values"
        )
    )
}

fn json_byte_array(items: &[Value]) -> Option<Vec<u8>> {
    items
        .iter()
        .map(|value| u8::try_from(value.as_u64()?).ok())
        .collect()
}

fn find_page_component_mut<'a>(
    page: &'a mut Page,
    component_id: &str,
) -> Result<&'a mut SurfaceComponent> {
    page.components
        .iter_mut()
        .find(|component| component.id == component_id)
        .ok_or_else(|| {
            anyhow!(
                "page '{}' no longer contains component '{}'",
                page.id,
                component_id
            )
        })
}

fn action_slot_mut<'a>(
    component: &'a mut Value,
    handler: &PrerunPageActionHandler,
    action_index: u32,
) -> Option<&'a mut Value> {
    let component = component.as_object_mut()?;
    let actions = match handler {
        PrerunPageActionHandler::Legacy => component.get_mut("actions")?.as_array_mut()?,
        PrerunPageActionHandler::Exact(event_name) => {
            let key = if component.contains_key("eventHandlers") {
                "eventHandlers"
            } else {
                "event_handlers"
            };
            component
                .get_mut(key)?
                .as_object_mut()?
                .get_mut(event_name)?
                .as_array_mut()?
        }
        PrerunPageActionHandler::Wildcard => {
            let key = if component.contains_key("eventHandlers") {
                "eventHandlers"
            } else {
                "event_handlers"
            };
            component
                .get_mut(key)?
                .as_object_mut()?
                .get_mut("*")?
                .as_array_mut()?
        }
    };
    actions.get_mut(action_index as usize)
}

fn binding_slot_mut<'a>(component: &'a mut Value, binding_id: &str) -> Option<&'a mut Value> {
    component
        .as_object_mut()?
        .get_mut("actionBindings")?
        .as_object_mut()?
        .get_mut(binding_id)
}

fn inline_widget_action_slot_mut<'a>(
    host: &'a mut Value,
    child_component_id: &str,
    handler: &PrerunPageActionHandler,
    action_index: u32,
) -> Option<&'a mut Value> {
    let children = host
        .as_object_mut()?
        .get_mut("inlineWidgetDef")?
        .as_object_mut()?
        .get_mut("components")?
        .as_array_mut()?;
    let child = children.iter_mut().find(|child| {
        child
            .as_object()
            .and_then(|child| child.get("id"))
            .and_then(Value::as_str)
            == Some(child_component_id)
    })?;
    let component = child.as_object_mut()?.get_mut("component")?;
    action_slot_mut(component, handler, action_index)
}

fn decorate_workflow_action(
    slot: &mut Value,
    action: &PrerunPageActionEvent,
    metadata: Value,
) -> Result<()> {
    if workflow_action_node_id(slot) != Some(action.node_id.as_str()) {
        return Err(anyhow!(
            "page action '{}' target changed after extraction",
            action.action_id
        ));
    }
    let slot = slot
        .as_object_mut()
        .expect("workflow action extraction only accepts objects");
    slot.insert(PAGE_ACTION_METADATA_KEY.to_string(), metadata);
    Ok(())
}

fn decorate_workflow_binding(
    slot: &mut Value,
    action: &PrerunPageActionEvent,
    metadata: Value,
) -> Result<()> {
    if workflow_binding_node_id(slot) != Some(action.node_id.as_str()) {
        return Err(anyhow!(
            "page action '{}' binding target changed after extraction",
            action.action_id
        ));
    }
    let slot = slot
        .as_object_mut()
        .expect("workflow binding extraction only accepts objects");
    slot.insert(PAGE_ACTION_METADATA_KEY.to_string(), metadata);
    Ok(())
}

fn optional_special_target(
    board: &Board,
    page: &Page,
    kind: &str,
    node_id: Option<&str>,
) -> Result<Option<PrerunPageEventTarget>> {
    let Some(node_id) = node_id.filter(|node_id| !node_id.is_empty()) else {
        return Ok(None);
    };
    validate_entry_node(board, &page.id, &format!("special {kind}"), node_id)?;
    Ok(Some(PrerunPageEventTarget {
        node_id: node_id.to_string(),
    }))
}

fn extract_component_actions(
    board: &Board,
    page_id: &str,
    component: &Value,
    scope: ComponentActionScope<'_>,
    out: &mut Vec<PrerunPageActionEvent>,
) -> Result<()> {
    let Some(component) = component.as_object() else {
        return Ok(());
    };

    if let Some(action) = component
        .get("actions")
        .and_then(Value::as_array)
        .and_then(|actions| actions.first())
        && let Some(node_id) = workflow_action_node_id(action)
    {
        push_action(
            board,
            page_id,
            node_id,
            scope.locator(PrerunPageActionHandler::Legacy, 0),
            out,
        )?;
    }

    let handlers = component
        .get("eventHandlers")
        .or_else(|| component.get("event_handlers"))
        .and_then(Value::as_object);
    if let Some(handlers) = handlers {
        for (event_name, actions) in handlers {
            let Some(actions) = actions.as_array() else {
                continue;
            };
            let handler = if event_name == "*" {
                PrerunPageActionHandler::Wildcard
            } else {
                PrerunPageActionHandler::Exact(event_name.clone())
            };
            for (index, action) in actions.iter().enumerate() {
                let Some(node_id) = workflow_action_node_id(action) else {
                    continue;
                };
                let action_index = u32::try_from(index).map_err(|_| {
                    anyhow!(
                        "page '{}' handler '{}' contains too many actions",
                        page_id,
                        event_name
                    )
                })?;
                push_action(
                    board,
                    page_id,
                    node_id,
                    scope.locator(handler.clone(), action_index),
                    out,
                )?;
            }
        }
    }

    Ok(())
}

#[derive(Default)]
struct PageWidgetTraversal {
    instance_ids: BTreeSet<String>,
    active_page_refs: BTreeSet<String>,
}

fn extract_widget_bindings(
    board: &Board,
    page: &Page,
    host_component_id: &str,
    host_component: &Value,
    parent_widget_path: &[PrerunPageWidgetAncestor],
    traversal: &mut PageWidgetTraversal,
    out: &mut Vec<PrerunPageActionEvent>,
) -> Result<()> {
    if parent_widget_path.len() >= MAX_PAGE_WIDGET_NESTING_DEPTH {
        return Err(anyhow!(
            "page '{}' widget nesting exceeds the maximum depth of {} at component '{}'",
            page.id,
            MAX_PAGE_WIDGET_NESTING_DEPTH,
            host_component_id
        ));
    }

    let component = host_component
        .as_object()
        .expect("widget component type was read from an object");
    let instance_id = required_string(component, "instanceId", &page.id, host_component_id)?;
    let widget_id = required_string(component, "widgetId", &page.id, host_component_id)?;
    let definition_source = if page.widget_refs.contains_key(instance_id) {
        Some(PrerunPageWidgetDefinitionSource::PageRef)
    } else if inline_widget_children(component).is_some() {
        Some(PrerunPageWidgetDefinitionSource::Inline)
    } else {
        None
    };
    validate_widget_exposed_prop_paths(
        page,
        host_component_id,
        component,
        instance_id,
        definition_source.as_ref(),
    )?;
    validate_static_runtime_child_updates(page, host_component_id, component)?;

    if matches!(
        definition_source,
        Some(PrerunPageWidgetDefinitionSource::PageRef)
    ) && traversal.active_page_refs.contains(instance_id)
    {
        return Err(anyhow!(
            "page '{}' widget reference cycle reaches instance '{}' at component '{}'",
            page.id,
            instance_id,
            host_component_id
        ));
    }
    if !traversal.instance_ids.insert(instance_id.to_string()) {
        return Err(anyhow!(
            "page '{}' contains duplicate widget instance id '{}'",
            page.id,
            instance_id
        ));
    }

    let mut widget_path = parent_widget_path.to_vec();
    widget_path.push(PrerunPageWidgetAncestor {
        host_component_id: host_component_id.to_string(),
        instance_id: instance_id.to_string(),
        widget_id: widget_id.to_string(),
        definition_source: definition_source.clone(),
    });

    if let Some(bindings) = component.get("actionBindings").and_then(Value::as_object) {
        for (widget_action_id, binding) in bindings {
            if handler_shadows_binding(component, widget_action_id) {
                continue;
            }
            let Some(node_id) = workflow_binding_node_id(binding) else {
                continue;
            };
            push_action(
                board,
                &page.id,
                node_id,
                if let [ancestor] = widget_path.as_slice() {
                    PrerunPageActionLocator::WidgetBinding {
                        host_component_id: ancestor.host_component_id.clone(),
                        instance_id: ancestor.instance_id.clone(),
                        widget_id: ancestor.widget_id.clone(),
                        widget_action_id: widget_action_id.clone(),
                    }
                } else {
                    PrerunPageActionLocator::NestedWidgetBinding {
                        widget_path: widget_path.clone(),
                        widget_action_id: widget_action_id.clone(),
                    }
                },
                out,
            )?;
        }
    }

    match definition_source {
        Some(PrerunPageWidgetDefinitionSource::PageRef) => {
            let widget = page
                .widget_refs
                .get(instance_id)
                .expect("PageRef source was selected from this map");
            traversal.active_page_refs.insert(instance_id.to_string());
            let result = extract_widget_children(
                board,
                page,
                instance_id,
                &widget.components,
                &widget_path,
                traversal,
                out,
            );
            traversal.active_page_refs.remove(instance_id);
            result?;
        }
        Some(PrerunPageWidgetDefinitionSource::Inline) => {
            let children = inline_widget_children(component)
                .expect("Inline source was selected from this component");
            extract_inline_widget_children(
                board,
                page,
                instance_id,
                children,
                &widget_path,
                traversal,
                out,
            )?;
        }
        None => {}
    }

    Ok(())
}

fn extract_widget_children(
    board: &Board,
    page: &Page,
    instance_id: &str,
    children: &[SurfaceComponent],
    widget_path: &[PrerunPageWidgetAncestor],
    traversal: &mut PageWidgetTraversal,
    out: &mut Vec<PrerunPageActionEvent>,
) -> Result<()> {
    let mut ids = BTreeSet::new();
    for child in children {
        if child.id.is_empty() {
            return Err(anyhow!(
                "page '{}' widget instance '{}' contains an empty child id",
                page.id,
                instance_id
            ));
        }
        if !ids.insert(child.id.as_str()) {
            return Err(anyhow!(
                "page '{}' widget instance '{}' contains duplicate child id '{}'",
                page.id,
                instance_id,
                child.id
            ));
        }
        extract_nested_component(
            board,
            page,
            &child.id,
            &child.component,
            widget_path,
            traversal,
            out,
        )?;
    }
    Ok(())
}

fn extract_inline_widget_children(
    board: &Board,
    page: &Page,
    instance_id: &str,
    children: &[Value],
    widget_path: &[PrerunPageWidgetAncestor],
    traversal: &mut PageWidgetTraversal,
    out: &mut Vec<PrerunPageActionEvent>,
) -> Result<()> {
    let mut ids = BTreeSet::new();
    for child in children {
        let Some(child) = child.as_object() else {
            continue;
        };
        let Some(child_id) = child.get("id").and_then(Value::as_str) else {
            continue;
        };
        if child_id.is_empty() {
            return Err(anyhow!(
                "page '{}' widget instance '{}' contains an empty child id",
                page.id,
                instance_id
            ));
        }
        if !ids.insert(child_id) {
            return Err(anyhow!(
                "page '{}' widget instance '{}' contains duplicate child id '{}'",
                page.id,
                instance_id,
                child_id
            ));
        }
        let Some(component) = child.get("component") else {
            continue;
        };
        extract_nested_component(
            board,
            page,
            child_id,
            component,
            widget_path,
            traversal,
            out,
        )?;
    }
    Ok(())
}

fn extract_nested_component(
    board: &Board,
    page: &Page,
    component_id: &str,
    component: &Value,
    widget_path: &[PrerunPageWidgetAncestor],
    traversal: &mut PageWidgetTraversal,
    out: &mut Vec<PrerunPageActionEvent>,
) -> Result<()> {
    extract_component_actions(
        board,
        &page.id,
        component,
        ComponentActionScope::WidgetChild {
            widget_path,
            child_component_id: component_id,
        },
        out,
    )?;

    match component_type(component) {
        Some("widgetInstance") => extract_widget_bindings(
            board,
            page,
            component_id,
            component,
            widget_path,
            traversal,
            out,
        ),
        Some("microWidgetInstance") => extract_micro_widget_bindings(
            board,
            page,
            component_id,
            component,
            widget_path,
            traversal,
            out,
        ),
        _ => Ok(()),
    }
}

fn extract_micro_widget_bindings(
    board: &Board,
    page: &Page,
    host_component_id: &str,
    host_component: &Value,
    widget_path: &[PrerunPageWidgetAncestor],
    traversal: &mut PageWidgetTraversal,
    out: &mut Vec<PrerunPageActionEvent>,
) -> Result<()> {
    let component = host_component
        .as_object()
        .expect("micro widget component type was read from an object");
    let instance_id = required_string(component, "instanceId", &page.id, host_component_id)?;
    if !traversal.instance_ids.insert(instance_id.to_string()) {
        return Err(anyhow!(
            "page '{}' contains duplicate widget instance id '{}'",
            page.id,
            instance_id
        ));
    }
    let package_id = required_string(component, "packageId", &page.id, host_component_id)?;
    let package_version =
        required_string(component, "packageVersion", &page.id, host_component_id)?;
    let widget_id = required_string(component, "widgetId", &page.id, host_component_id)?;
    let declared_events = component
        .get("contract")
        .and_then(Value::as_object)
        .and_then(|contract| contract.get("events"))
        .and_then(Value::as_object);

    if let Some(bindings) = component.get("actionBindings").and_then(Value::as_object) {
        for (event_name, binding) in bindings {
            if declared_events.is_none_or(|events| !events.contains_key(event_name))
                || handler_shadows_binding(component, event_name)
            {
                continue;
            }
            let Some(node_id) = workflow_binding_node_id(binding) else {
                continue;
            };
            push_action(
                board,
                &page.id,
                node_id,
                if widget_path.is_empty() {
                    PrerunPageActionLocator::MicroWidgetBinding {
                        host_component_id: host_component_id.to_string(),
                        instance_id: instance_id.to_string(),
                        package_id: package_id.to_string(),
                        package_version: package_version.to_string(),
                        widget_id: widget_id.to_string(),
                        contract_event_name: event_name.clone(),
                    }
                } else {
                    PrerunPageActionLocator::NestedMicroWidgetBinding {
                        locator: Box::new(PrerunPageNestedMicroWidgetLocator {
                            widget_path: widget_path.to_vec(),
                            host_component_id: host_component_id.to_string(),
                            instance_id: instance_id.to_string(),
                            package_id: package_id.to_string(),
                            package_version: package_version.to_string(),
                            widget_id: widget_id.to_string(),
                            contract_event_name: event_name.clone(),
                        }),
                    }
                },
                out,
            )?;
        }
    }

    Ok(())
}

fn inline_widget_children(component: &serde_json::Map<String, Value>) -> Option<&[Value]> {
    component
        .get("inlineWidgetDef")?
        .as_object()?
        .get("components")?
        .as_array()
        .map(Vec::as_slice)
}

fn validate_widget_exposed_prop_paths(
    page: &Page,
    host_component_id: &str,
    component: &serde_json::Map<String, Value>,
    instance_id: &str,
    definition_source: Option<&PrerunPageWidgetDefinitionSource>,
) -> Result<()> {
    match definition_source {
        Some(PrerunPageWidgetDefinitionSource::PageRef) => {
            let widget = page
                .widget_refs
                .get(instance_id)
                .expect("PageRef source was selected from this map");
            for exposed_prop in &widget.exposed_props {
                reject_executable_exposed_prop_path(
                    &page.id,
                    host_component_id,
                    &exposed_prop.id,
                    &exposed_prop.property_path,
                )?;
            }
        }
        Some(PrerunPageWidgetDefinitionSource::Inline) => {
            let exposed_props = component
                .get("inlineWidgetDef")
                .and_then(Value::as_object)
                .and_then(|definition| {
                    definition
                        .get("exposedProps")
                        .or_else(|| definition.get("exposed_props"))
                })
                .and_then(Value::as_array);
            if let Some(exposed_props) = exposed_props {
                for exposed_prop in exposed_props {
                    let Some(exposed_prop) = exposed_prop.as_object() else {
                        continue;
                    };
                    let prop_id = exposed_prop
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("<unknown>");
                    let Some(property_path) = exposed_prop
                        .get("propertyPath")
                        .or_else(|| exposed_prop.get("property_path"))
                        .and_then(Value::as_str)
                    else {
                        continue;
                    };
                    reject_executable_exposed_prop_path(
                        &page.id,
                        host_component_id,
                        prop_id,
                        property_path,
                    )?;
                }
            }
        }
        None => {}
    }
    Ok(())
}

fn reject_executable_exposed_prop_path(
    page_id: &str,
    host_component_id: &str,
    prop_id: &str,
    property_path: &str,
) -> Result<()> {
    if property_path.split('.').any(|segment| {
        matches!(
            segment.trim(),
            "actions" | "eventHandlers" | "event_handlers" | "actionBindings"
        )
    }) {
        return Err(anyhow!(
            "page '{}' widget host '{}' exposes executable property path '{}' through prop '{}'",
            page_id,
            host_component_id,
            property_path,
            prop_id
        ));
    }
    Ok(())
}

fn validate_static_runtime_child_updates(
    page: &Page,
    host_component_id: &str,
    component: &serde_json::Map<String, Value>,
) -> Result<()> {
    let Some(updates) = component
        .get("runtimeChildUpdates")
        .or_else(|| component.get("runtime_child_updates"))
        .and_then(Value::as_object)
    else {
        return Ok(());
    };

    for operations in updates.values().filter_map(Value::as_array) {
        for operation in operations {
            let Some(operation) = operation.as_object() else {
                continue;
            };
            let operation_type = operation.get("type").and_then(Value::as_str);
            let introduces_executable_fields =
                matches!(operation_type, Some("setAction" | "setEventActions"))
                    || match operation_type {
                        Some("createComponent") => operation
                            .get("component")
                            .is_some_and(value_contains_executable_component_field),
                        Some("setProps") => operation
                            .get("props")
                            .is_some_and(value_contains_executable_component_field),
                        Some(_) | None => operation
                            .keys()
                            .any(|key| is_executable_component_field(key)),
                    };

            if introduces_executable_fields {
                return Err(anyhow!(
                    "page '{}' widget host '{}' contains persisted runtime child update '{}' that can replace executable actions",
                    page.id,
                    host_component_id,
                    operation_type.unwrap_or("setProps")
                ));
            }
        }
    }
    Ok(())
}

fn value_contains_executable_component_field(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, child)| {
            is_executable_component_field(key) || value_contains_executable_component_field(child)
        }),
        Value::Array(items) => items.iter().any(value_contains_executable_component_field),
        _ => false,
    }
}

fn is_executable_component_field(field: &str) -> bool {
    matches!(
        field,
        "actions" | "eventHandlers" | "event_handlers" | "actionBindings"
    )
}

fn required_string<'a>(
    component: &'a serde_json::Map<String, Value>,
    field: &str,
    page_id: &str,
    component_id: &str,
) -> Result<&'a str> {
    component
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "page '{}' component '{}' is missing '{}'",
                page_id,
                component_id,
                field
            )
        })
}

fn component_type(component: &Value) -> Option<&str> {
    component.as_object()?.get("type")?.as_str()
}

fn handler_shadows_binding(component: &serde_json::Map<String, Value>, event_name: &str) -> bool {
    component
        .get("eventHandlers")
        .or_else(|| component.get("event_handlers"))
        .and_then(Value::as_object)
        .is_some_and(|handlers| handlers.contains_key(event_name) || handlers.contains_key("*"))
}

fn workflow_action_node_id(action: &Value) -> Option<&str> {
    let action = action.as_object()?;
    if action.get("name")?.as_str()? != "workflow_event" {
        return None;
    }
    let context = action.get("context")?.as_object()?;
    context
        .get("nodeId")
        .or_else(|| context.get("node_id"))
        .and_then(bound_string)
        .filter(|node_id| !node_id.is_empty())
}

fn workflow_binding_node_id(binding: &Value) -> Option<&str> {
    let binding = binding.as_object()?;
    let workflow = binding
        .get("workflow")
        .or_else(|| binding.get("workflowEvent"))
        .or_else(|| binding.get("workflow_event"))?
        .as_object()?;
    workflow
        .get("flowId")
        .or_else(|| workflow.get("eventId"))
        .or_else(|| workflow.get("nodeId"))
        .or_else(|| workflow.get("flow_id"))
        .or_else(|| workflow.get("event_id"))
        .and_then(bound_string)
        .filter(|node_id| !node_id.is_empty())
}

fn bound_string(value: &Value) -> Option<&str> {
    match value {
        Value::String(value) => Some(value.as_str()),
        Value::Object(value) => value.get("literalString").and_then(Value::as_str),
        _ => None,
    }
}

fn push_action(
    board: &Board,
    page_id: &str,
    node_id: &str,
    locator: PrerunPageActionLocator,
    out: &mut Vec<PrerunPageActionEvent>,
) -> Result<()> {
    validate_entry_node(board, page_id, "action", node_id)?;
    out.push(PrerunPageActionEvent {
        action_id: page_action_id(page_id, node_id, &locator),
        node_id: node_id.to_string(),
        locator,
    });
    Ok(())
}

fn validate_entry_node(board: &Board, page_id: &str, source: &str, node_id: &str) -> Result<()> {
    let node = board.nodes.get(node_id).or_else(|| {
        board
            .layers
            .values()
            .find_map(|layer| layer.nodes.get(node_id))
    });
    let Some(node) = node else {
        return Err(anyhow!(
            "page '{}' {} targets missing node '{}' on board '{}'",
            page_id,
            source,
            node_id,
            board.id
        ));
    };
    if node.start != Some(true) {
        return Err(anyhow!(
            "page '{}' {} targets node '{}' which is not an entry node",
            page_id,
            source,
            node_id
        ));
    }
    Ok(())
}

/// Per-node facts accumulated over the board walk. Maps are ordered so the
/// manifest is byte-identical for identical boards.
#[derive(Default)]
struct NodeFacts {
    oauth_scopes: BTreeMap<String, Vec<String>>,
    requires_local_execution: bool,
    wasm_package_ids: Vec<String>,
    wasm_permissions: BTreeMap<String, Vec<NodePermission>>,
}

impl NodeFacts {
    fn visit(&mut self, node: &Node) {
        if let Some(wasm) = &node.wasm {
            if !self.wasm_package_ids.contains(&wasm.package_id) {
                self.wasm_package_ids.push(wasm.package_id.clone());
            }
            if !wasm.permissions.is_empty() {
                let entry = self
                    .wasm_permissions
                    .entry(wasm.package_id.clone())
                    .or_default();
                for perm in &wasm.permissions {
                    if !entry.contains(perm) {
                        entry.push(*perm);
                    }
                }
            }
        }
        if node.only_offline {
            self.requires_local_execution = true;
        }
        if let Some(providers) = &node.oauth_providers {
            for provider_id in providers {
                self.oauth_scopes.entry(provider_id.clone()).or_default();
            }
        }
        // required_oauth_scopes only contributes scopes for providers already
        // registered via oauth_providers — it's informational, not a trigger.
        if let Some(required_scopes) = &node.required_oauth_scopes {
            for (provider_id, scopes) in required_scopes {
                if let Some(entry) = self.oauth_scopes.get_mut(provider_id) {
                    for scope in scopes {
                        if !entry.contains(scope) {
                            entry.push(scope.clone());
                        }
                    }
                }
            }
        }
    }

    fn oauth_requirements(&self) -> Vec<PrerunOAuthRequirement> {
        self.oauth_scopes
            .iter()
            .map(|(provider_id, scopes)| {
                let mut scopes = scopes.clone();
                scopes.sort();
                PrerunOAuthRequirement {
                    provider_id: provider_id.clone(),
                    scopes,
                }
            })
            .collect()
    }

    fn sorted_wasm_package_ids(&self) -> Vec<String> {
        let mut ids = self.wasm_package_ids.clone();
        ids.sort();
        ids
    }

    fn wasm_package_permissions(&self) -> Vec<(String, Vec<String>)> {
        self.wasm_permissions
            .iter()
            .map(|(package_id, permissions)| {
                let strings = permissions.iter().map(permission_string).collect();
                (package_id.clone(), strings)
            })
            .collect()
    }
}

fn execution_mode_string(mode: &ExecutionMode) -> String {
    serde_json::to_value(mode)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{mode:?}"))
}

fn permission_string(permission: &NodePermission) -> String {
    serde_json::to_value(permission)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{permission:?}"))
}

/// Prerun manifest of an immutable board version, beside its compiled artifact.
pub fn manifest_path(board_dir: &Path, board_id: &str, version: (u32, u32, u32)) -> Path {
    super::version_artifact_dir(board_dir, board_id)
        .child(format!("{}_{}_{}.prerun", version.0, version.1, version.2))
}

/// Prerun manifest of a floating draft, beside its compiled artifact and keyed
/// by the same `.board` etag.
pub fn draft_manifest_path(app_id: &str, board_id: &str, e_tag: &str) -> Path {
    super::draft_artifact_dir(app_id, board_id)
        .child(format!("{}.prerun", super::draft_artifact_stem(e_tag)))
}

pub fn encode_manifest(manifest: &PrerunManifest) -> Result<Vec<u8>> {
    let archive = rkyv::to_bytes::<rkyv::rancor::Error>(manifest)
        .map_err(|e| anyhow!("failed to serialize prerun manifest: {e}"))?;
    let compressed = lz4_flex::compress_prepend_size(&archive);

    let mut out = Vec::with_capacity(HEADER_LEN + compressed.len());
    out.extend_from_slice(&MANIFEST_MAGIC);
    out.extend_from_slice(&MANIFEST_FORMAT_VERSION.to_le_bytes());
    out.push(CODEC_LZ4);
    out.push(0);
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// Fails on wrong magic, a format version this build does not understand, an
/// unknown codec or a truncated payload — callers recompute from the board.
pub fn decode_manifest(bytes: &[u8]) -> Result<PrerunManifest> {
    if bytes.len() < HEADER_LEN {
        return Err(anyhow!(
            "prerun manifest too short: {} bytes, header needs {HEADER_LEN}",
            bytes.len()
        ));
    }
    if bytes[0..4] != MANIFEST_MAGIC {
        return Err(anyhow!("prerun manifest has wrong magic"));
    }
    let format_version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if format_version != MANIFEST_FORMAT_VERSION && format_version != LEGACY_MANIFEST_FORMAT_VERSION
    {
        return Err(anyhow!(
            "prerun manifest format v{format_version}, this build reads v{LEGACY_MANIFEST_FORMAT_VERSION} and v{MANIFEST_FORMAT_VERSION}"
        ));
    }

    let payload = &bytes[HEADER_LEN..];
    let raw: Cow<[u8]> = match bytes[6] {
        CODEC_NONE => Cow::Borrowed(payload),
        CODEC_LZ4 => {
            if payload.len() < 4 {
                return Err(anyhow!("prerun manifest payload truncated"));
            }
            let len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
            if len > MAX_DECOMPRESSED_LEN {
                return Err(anyhow!(
                    "prerun manifest claims an implausible {len}-byte payload"
                ));
            }
            Cow::Owned(
                lz4_flex::decompress_size_prepended(payload)
                    .map_err(|e| anyhow!("failed to decompress prerun manifest: {e}"))?,
            )
        }
        codec => return Err(anyhow!("prerun manifest uses unknown codec {codec}")),
    };

    let mut aligned = rkyv::util::AlignedVec::<16>::new();
    aligned.extend_from_slice(&raw);
    if format_version == LEGACY_MANIFEST_FORMAT_VERSION {
        let archived =
            rkyv::access::<rkyv::Archived<LegacyPrerunManifestV1>, rkyv::rancor::Error>(&aligned)
                .map_err(|e| anyhow!("legacy prerun manifest failed validation: {e}"))?;
        let legacy = rkyv::deserialize::<LegacyPrerunManifestV1, rkyv::rancor::Error>(archived)
            .map_err(|e| anyhow!("failed to deserialize legacy prerun manifest: {e}"))?;
        return Ok(legacy.into());
    }

    let archived = rkyv::access::<rkyv::Archived<PrerunManifest>, rkyv::rancor::Error>(&aligned)
        .map_err(|e| anyhow!("prerun manifest failed validation: {e}"))?;
    rkyv::deserialize::<PrerunManifest, rkyv::rancor::Error>(archived)
        .map_err(|e| anyhow!("failed to deserialize prerun manifest: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{
        board::{Layer, LayerType},
        node::NodeWasm,
        pin::ValueType,
        variable::{Variable, VariableType},
    };
    use flow_like_types::json::json;

    #[derive(Archive, RkyvSerialize, RkyvDeserialize)]
    enum LegacyPrerunPageActionLocatorV2 {
        Component {
            component_id: String,
            handler: PrerunPageActionHandler,
            action_index: u32,
        },
        WidgetBinding {
            host_component_id: String,
            instance_id: String,
            widget_id: String,
            widget_action_id: String,
        },
        WidgetChild {
            host_component_id: String,
            instance_id: String,
            child_component_id: String,
            handler: PrerunPageActionHandler,
            action_index: u32,
        },
        MicroWidgetBinding {
            host_component_id: String,
            instance_id: String,
            package_id: String,
            package_version: String,
            widget_id: String,
            contract_event_name: String,
        },
    }

    fn add_entry(board: &mut Board, id: &str) {
        let mut node = Node::new("events_simple", id, "", "Events");
        node.id = id.to_string();
        node.set_start(true);
        board.nodes.insert(node.id.clone(), node);
    }

    fn sample_manifest() -> PrerunManifest {
        let mut manifest = PrerunManifest {
            runtime_variables: vec![PrerunVariable {
                id: "var-1".into(),
                name: "api_key".into(),
                description: Some("Key".into()),
                data_type: "String".into(),
                value_type: "Normal".into(),
                secret: true,
                schema: None,
            }],
            oauth_requirements: vec![PrerunOAuthRequirement {
                provider_id: "jira".into(),
                scopes: vec!["read:issue".into()],
            }],
            requires_local_execution: true,
            execution_mode: "Local".into(),
            has_wasm_nodes: true,
            wasm_package_ids: vec!["pkg-a".into()],
            wasm_package_permissions: vec![(
                "pkg-a".into(),
                vec!["network:http".into(), "storage:read".into()],
            )],
            element_selectors: vec!["page-1/btn-1".into(), "type:switch".into()],
            element_reads_dynamic: false,
            page_events: Vec::new(),
            signature: String::new(),
        };
        manifest.signature = manifest.compute_signature();
        manifest
    }

    fn sample_board(element_id: &str) -> Board {
        let mut board = Board::new_detached(Some("prerun-board".into()), Path::default());
        board.execution_mode = ExecutionMode::Local;

        let mut key = Variable::new("api_key", VariableType::String, ValueType::Normal);
        key.id = "var-1".into();
        key.runtime_configured = true;
        key.secret = true;
        key.description = Some("Key".into());
        board.variables.insert(key.id.clone(), key);

        let mut plain = Variable::new("plain", VariableType::Integer, ValueType::Array);
        plain.id = "var-0".into();
        board.variables.insert(plain.id.clone(), plain);

        let mut offline = Node::new("rpa_click", "Click", "", "RPA");
        offline.id = "offline".into();
        offline.only_offline = true;
        board.nodes.insert(offline.id.clone(), offline);

        let mut oauth = Node::new("jira_issue", "Jira Issue", "", "Jira");
        oauth.id = "oauth".into();
        oauth.oauth_providers = Some(vec!["jira".into()]);
        oauth.required_oauth_scopes = Some(HashMap::from([
            (
                "jira".to_string(),
                vec!["write:issue".to_string(), "read:issue".to_string()],
            ),
            ("github".to_string(), vec!["repo".to_string()]),
        ]));
        board.nodes.insert(oauth.id.clone(), oauth);

        let mut wasm = Node::new("pkg_node", "Package Node", "", "Packages");
        wasm.id = "wasm".into();
        wasm.wasm = Some(NodeWasm {
            package_id: "pkg-a".into(),
            permissions: vec![
                NodePermission::NetworkHttp,
                NodePermission::StorageRead,
                NodePermission::NetworkHttp,
            ],
        });
        let mut layer = Layer::new("layer-1".into(), "Layer".into(), LayerType::Function);
        layer.nodes.insert(wasm.id.clone(), wasm);
        board.layers.insert(layer.id.clone(), layer);

        let mut reader = Node::new(
            "a2ui_get_button_label",
            "Get Button Label",
            "",
            "UI/Elements/Button",
        );
        reader.id = "reader".into();
        reader
            .add_input_pin("element_ref", "Button", "", VariableType::Struct)
            .set_default_value(Some(json!(element_id)));
        board.nodes.insert(reader.id.clone(), reader);

        board
    }

    #[test]
    fn round_trip_preserves_manifest() {
        let manifest = sample_manifest();
        let bytes = encode_manifest(&manifest).unwrap();
        assert_eq!(&bytes[0..4], &MANIFEST_MAGIC);
        assert_eq!(
            u16::from_le_bytes([bytes[4], bytes[5]]),
            MANIFEST_FORMAT_VERSION
        );
        assert_eq!(bytes[6], CODEC_LZ4);
        assert_eq!(decode_manifest(&bytes).unwrap(), manifest);
    }

    #[test]
    fn decodes_uncompressed_payload() {
        let manifest = sample_manifest();
        let archive = rkyv::to_bytes::<rkyv::rancor::Error>(&manifest).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MANIFEST_MAGIC);
        bytes.extend_from_slice(&MANIFEST_FORMAT_VERSION.to_le_bytes());
        bytes.push(CODEC_NONE);
        bytes.push(0);
        bytes.extend_from_slice(&archive);
        assert_eq!(decode_manifest(&bytes).unwrap(), manifest);
    }

    #[test]
    fn rejects_wrong_magic() {
        let mut bytes = encode_manifest(&sample_manifest()).unwrap();
        bytes[0..4].copy_from_slice(b"FLCB");
        let err = decode_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("wrong magic"), "{err}");
    }

    #[test]
    fn rejects_other_format_version() {
        let mut bytes = encode_manifest(&sample_manifest()).unwrap();
        bytes[4..6].copy_from_slice(&(MANIFEST_FORMAT_VERSION + 1).to_le_bytes());
        let err = decode_manifest(&bytes).unwrap_err().to_string();
        assert!(
            err.contains(&format!("format v{}", MANIFEST_FORMAT_VERSION + 1)),
            "{err}"
        );
    }

    #[test]
    fn rejects_unknown_codec() {
        let mut bytes = encode_manifest(&sample_manifest()).unwrap();
        bytes[6] = 7;
        let err = decode_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("unknown codec 7"), "{err}");
    }

    #[test]
    fn rejects_truncated_input() {
        let bytes = encode_manifest(&sample_manifest()).unwrap();

        let err = decode_manifest(&bytes[..5]).unwrap_err().to_string();
        assert!(err.contains("too short"), "{err}");

        let err = decode_manifest(&bytes[..HEADER_LEN + 2])
            .unwrap_err()
            .to_string();
        assert!(err.contains("truncated"), "{err}");

        let err = decode_manifest(&bytes[..bytes.len() - 5])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("decompress") || err.contains("validation"),
            "{err}"
        );
    }

    #[test]
    fn rejects_implausible_length_prefix() {
        let mut bytes = encode_manifest(&sample_manifest()).unwrap();
        bytes[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = decode_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("implausible"), "{err}");
    }

    #[test]
    fn from_board_extracts_every_field() {
        let manifest = PrerunManifest::from_board(&sample_board("page-1/btn-1"));

        assert_eq!(
            manifest.runtime_variables,
            vec![PrerunVariable {
                id: "var-1".into(),
                name: "api_key".into(),
                description: Some("Key".into()),
                data_type: "String".into(),
                value_type: "Normal".into(),
                secret: true,
                schema: None,
            }]
        );
        assert_eq!(
            manifest.oauth_requirements,
            vec![PrerunOAuthRequirement {
                provider_id: "jira".into(),
                scopes: vec!["read:issue".into(), "write:issue".into()],
            }],
            "scopes only attach to providers registered via oauth_providers"
        );
        assert!(manifest.requires_local_execution);
        assert_eq!(manifest.execution_mode, "Local");
        assert_eq!(manifest.execution_mode(), ExecutionMode::Local);
        assert!(manifest.has_wasm_nodes);
        assert_eq!(manifest.wasm_package_ids, vec!["pkg-a".to_string()]);
        assert_eq!(
            manifest.wasm_package_permissions,
            vec![(
                "pkg-a".to_string(),
                vec!["network:http".to_string(), "storage:read".to_string()]
            )]
        );
        assert_eq!(
            manifest.wasm_permissions(),
            HashMap::from([(
                "pkg-a".to_string(),
                vec![NodePermission::NetworkHttp, NodePermission::StorageRead]
            )])
        );
        assert!(
            manifest
                .element_selectors
                .iter()
                .any(|s| s.contains("page-1/btn-1")),
            "literal element_ref becomes a selector: {:?}",
            manifest.element_selectors
        );
        assert!(!manifest.element_reads_dynamic);
        assert_eq!(manifest.signature.len(), 64);
        assert_eq!(manifest.signature, manifest.compute_signature());
    }

    #[test]
    fn from_board_is_deterministic() {
        let a = PrerunManifest::from_board(&sample_board("page-1/btn-1"));
        let b = PrerunManifest::from_board(&sample_board("page-1/btn-1"));
        assert_eq!(a, b);

        let c = PrerunManifest::from_board(&sample_board("page-1/btn-2"));
        assert_ne!(a.element_selectors, c.element_selectors);
        assert_ne!(
            a.signature, c.signature,
            "element demand is part of the v2 drift signature"
        );
    }

    #[test]
    fn signature_covers_element_and_page_fields() {
        let base = sample_manifest();

        let mut selector = base.clone();
        selector.element_selectors.push("glob:feed-row-*".into());
        assert_ne!(selector.compute_signature(), base.signature);

        let mut dynamic = base.clone();
        dynamic.element_reads_dynamic = true;
        assert_ne!(dynamic.compute_signature(), base.signature);

        let mut page = base.clone();
        page.page_events.push(PrerunPageExecution {
            page_id: "page-1".into(),
            action_events: Vec::new(),
            special_events: PrerunPageSpecialEvents::default(),
        });
        assert_ne!(page.compute_signature(), base.signature);

        let mut reordered = base.clone();
        reordered.oauth_requirements[0].scopes = vec!["read:issue".into()];
        reordered.wasm_package_permissions[0].1.reverse();
        assert_eq!(
            reordered.compute_signature(),
            base.signature,
            "permission order does not shift the hash"
        );
    }

    fn page_board() -> Board {
        let mut board = Board::new_detached(Some("page-board".into()), Path::default());
        for id in [
            "load",
            "unload",
            "interval",
            "legacy",
            "exact-1",
            "exact-2",
            "wildcard",
            "widget-submit",
            "widget-child",
            "micro-point",
        ] {
            add_entry(&mut board, id);
        }
        board.page_ids.push("page-1".into());
        board
    }

    fn configured_page() -> Page {
        let mut page = Page::new("page-1", "Page", "/");
        page.board_id = Some("page-board".into());
        page.on_load_event_id = Some("load".into());
        page.on_unload_event_id = Some("unload".into());
        page.on_interval_event_id = Some("interval".into());
        page.on_interval_seconds = Some(30);
        page.components.push(SurfaceComponent::new(
            "button",
            json!({
                "type": "button",
                "actions": [
                    { "name": "workflow_event", "context": { "nodeId": "legacy" } },
                    { "name": "workflow_event", "context": { "nodeId": "ignored-legacy" } }
                ],
                "eventHandlers": {
                    "click": [
                        { "name": "navigate_page", "context": { "route": "/next" } },
                        { "name": "workflow_event", "context": { "nodeId": "exact-1" } },
                        { "name": "workflow_event", "context": { "nodeId": "exact-2" } }
                    ],
                    "*": [
                        { "name": "workflow_event", "context": { "nodeId": "wildcard" } }
                    ]
                }
            }),
        ));

        let mut widget = crate::a2ui::Widget::new("widget-1", "Widget", "child");
        widget.components.push(SurfaceComponent::new(
            "child",
            json!({
                "type": "button",
                "eventHandlers": {
                    "click": [
                        { "name": "workflow_event", "context": { "nodeId": "widget-child" } }
                    ]
                }
            }),
        ));
        page.widget_refs.insert("instance-1".into(), widget);
        page.components.push(SurfaceComponent::new(
            "widget-host",
            json!({
                "type": "widgetInstance",
                "instanceId": "instance-1",
                "widgetId": "widget-1",
                "actionBindings": {
                    "submit": {
                        "workflow": { "flowId": "widget-submit", "inputMappings": {} }
                    },
                    "shadowed": {
                        "workflow": { "flowId": "missing-shadowed-node", "inputMappings": {} }
                    }
                },
                "eventHandlers": {
                    "shadowed": []
                }
            }),
        ));

        page.components.push(SurfaceComponent::new(
            "micro-host",
            json!({
                "type": "microWidgetInstance",
                "instanceId": "micro-1",
                "packageId": "pkg.example",
                "packageVersion": "1.2.3",
                "widgetId": "chart",
                "contract": {
                    "events": {
                        "pointSelected": { "payloadSchema": { "type": "object" } },
                        "shadowed": {}
                    }
                },
                "actionBindings": {
                    "pointSelected": {
                        "workflow": { "flowId": "micro-point", "inputMappings": {} }
                    },
                    "shadowed": {
                        "workflow": { "flowId": "missing-micro-shadowed", "inputMappings": {} }
                    },
                    "notDeclared": {
                        "workflow": { "flowId": "missing-undeclared", "inputMappings": {} }
                    }
                },
                "eventHandlers": {
                    "shadowed": []
                }
            }),
        ));
        page
    }

    fn nested_widget_page() -> Page {
        let mut page = Page::new("page-1", "Nested Page", "/nested");
        page.board_id = Some("page-board".into());

        let mut outer = crate::a2ui::Widget::new("outer-widget", "Outer", "inner-host");
        outer.components.push(SurfaceComponent::new(
            "inner-host",
            json!({
                "type": "widgetInstance",
                "instanceId": "inner-instance",
                "widgetId": "inner-widget",
                "actionBindings": {
                    "submit": {
                        "workflow": { "flowId": "nested-submit", "inputMappings": {} },
                        "pageAction": { "actionId": "forged-binding" },
                        "page_action": { "actionId": "forged-binding-alias" }
                    }
                }
            }),
        ));

        let mut inner = crate::a2ui::Widget::new("inner-widget", "Inner", "nested-button");
        inner.components.push(SurfaceComponent::new(
            "nested-button",
            json!({
                "type": "button",
                "eventHandlers": {
                    "click": [
                        {
                            "name": "workflow_event",
                            "context": { "nodeId": "nested-first" },
                            "pageAction": { "actionId": "forged-action" },
                            "page_action": { "actionId": "forged-action-alias" }
                        },
                        {
                            "name": "workflow_event",
                            "context": { "nodeId": "nested-second" }
                        }
                    ]
                }
            }),
        ));
        inner.components.push(SurfaceComponent::new(
            "nested-micro",
            json!({
                "type": "microWidgetInstance",
                "instanceId": "nested-micro-instance",
                "packageId": "pkg.nested",
                "packageVersion": "2.0.0",
                "widgetId": "nested-chart",
                "contract": {
                    "events": {
                        "selected": { "payloadSchema": { "type": "object" } }
                    }
                },
                "actionBindings": {
                    "selected": {
                        "workflow": { "flowId": "nested-micro", "inputMappings": {} }
                    }
                }
            }),
        ));

        page.widget_refs.insert("outer-instance".into(), outer);
        page.widget_refs.insert("inner-instance".into(), inner);
        page.components.push(SurfaceComponent::new(
            "outer-host",
            json!({
                "type": "widgetInstance",
                "instanceId": "outer-instance",
                "widgetId": "outer-widget"
            }),
        ));
        page
    }

    #[test]
    fn canonical_action_ids_are_deterministic_and_unambiguous() {
        let first = PrerunPageActionLocator::Component {
            component_id: "element:one".into(),
            handler: PrerunPageActionHandler::Exact("click:primary".into()),
            action_index: 0,
        };
        let second = PrerunPageActionLocator::Component {
            component_id: "element".into(),
            handler: PrerunPageActionHandler::Exact("one:click:primary".into()),
            action_index: 0,
        };

        let id = page_action_id("page", "node", &first);
        assert!(id.starts_with(PAGE_ACTION_ID_PREFIX));
        assert_eq!(id, page_action_id("page", "node", &first));
        assert_ne!(id, page_action_id("page", "node", &second));
        assert_ne!(id, page_action_id("page", "another-node", &first));
    }

    #[test]
    fn appended_nested_locators_preserve_original_v2_archive_layout() {
        assert_eq!(
            std::mem::size_of::<ArchivedLegacyPrerunPageActionLocatorV2>(),
            std::mem::size_of::<ArchivedPrerunPageActionLocator>()
        );
        assert_eq!(
            std::mem::align_of::<ArchivedLegacyPrerunPageActionLocatorV2>(),
            std::mem::align_of::<ArchivedPrerunPageActionLocator>()
        );

        let cases = [
            (
                LegacyPrerunPageActionLocatorV2::Component {
                    component_id: "button".into(),
                    handler: PrerunPageActionHandler::Exact("click".into()),
                    action_index: 2,
                },
                PrerunPageActionLocator::Component {
                    component_id: "button".into(),
                    handler: PrerunPageActionHandler::Exact("click".into()),
                    action_index: 2,
                },
            ),
            (
                LegacyPrerunPageActionLocatorV2::WidgetBinding {
                    host_component_id: "widget".into(),
                    instance_id: "instance".into(),
                    widget_id: "definition".into(),
                    widget_action_id: "submit".into(),
                },
                PrerunPageActionLocator::WidgetBinding {
                    host_component_id: "widget".into(),
                    instance_id: "instance".into(),
                    widget_id: "definition".into(),
                    widget_action_id: "submit".into(),
                },
            ),
            (
                LegacyPrerunPageActionLocatorV2::WidgetChild {
                    host_component_id: "widget".into(),
                    instance_id: "instance".into(),
                    child_component_id: "button".into(),
                    handler: PrerunPageActionHandler::Wildcard,
                    action_index: 1,
                },
                PrerunPageActionLocator::WidgetChild {
                    host_component_id: "widget".into(),
                    instance_id: "instance".into(),
                    child_component_id: "button".into(),
                    handler: PrerunPageActionHandler::Wildcard,
                    action_index: 1,
                },
            ),
            (
                LegacyPrerunPageActionLocatorV2::MicroWidgetBinding {
                    host_component_id: "micro".into(),
                    instance_id: "instance".into(),
                    package_id: "package".into(),
                    package_version: "1.0.0".into(),
                    widget_id: "chart".into(),
                    contract_event_name: "selected".into(),
                },
                PrerunPageActionLocator::MicroWidgetBinding {
                    host_component_id: "micro".into(),
                    instance_id: "instance".into(),
                    package_id: "package".into(),
                    package_version: "1.0.0".into(),
                    widget_id: "chart".into(),
                    contract_event_name: "selected".into(),
                },
            ),
        ];

        for (legacy, expected) in cases {
            let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&legacy).unwrap();
            let archived =
                rkyv::access::<ArchivedPrerunPageActionLocator, rkyv::rancor::Error>(&bytes)
                    .unwrap();
            let decoded =
                rkyv::deserialize::<PrerunPageActionLocator, rkyv::rancor::Error>(archived)
                    .unwrap();
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn page_execution_revisions_are_canonical_and_authority_sensitive() {
        let board = page_board();
        let page = configured_page();
        let execution = PrerunPageExecution::from_page(&board, &page).unwrap();
        let revision = page_execution_revision(&board, &execution).unwrap();
        assert!(revision.starts_with(PAGE_EXECUTION_REVISION_PREFIX));

        let mut reordered = execution.clone();
        reordered.action_events.reverse();
        assert_eq!(
            page_execution_revision(&board, &reordered).unwrap(),
            revision,
            "stored action order does not change the canonical revision"
        );

        let mut changed_action = execution.clone();
        let action = &mut changed_action.action_events[0];
        action.node_id = "load".into();
        action.action_id =
            page_action_id(&changed_action.page_id, &action.node_id, &action.locator);
        assert_ne!(
            page_execution_revision(&board, &changed_action).unwrap(),
            revision
        );

        let mut changed_special = execution.clone();
        changed_special.special_events.load = Some(PrerunPageEventTarget {
            node_id: "unload".into(),
        });
        assert_ne!(
            page_execution_revision(&board, &changed_special).unwrap(),
            revision
        );

        let mut changed_board = board.clone();
        changed_board.name.push_str(" changed");
        assert_ne!(
            page_execution_revision(&changed_board, &execution).unwrap(),
            revision
        );

        let mut changed_version = board.clone();
        changed_version.version = (9, 8, 7);
        assert_ne!(
            page_execution_revision(&changed_version, &execution).unwrap(),
            revision
        );
    }

    #[test]
    fn page_execution_revision_rejects_noncanonical_or_foreign_inputs() {
        let board = page_board();
        let page = configured_page();
        let execution = PrerunPageExecution::from_page(&board, &page).unwrap();

        let mut noncanonical = execution.clone();
        noncanonical.action_events[0].node_id = "load".into();
        assert!(page_execution_revision(&board, &noncanonical).is_err());

        let mut foreign = execution.clone();
        foreign.page_id = "foreign-page".into();
        assert!(page_execution_revision(&board, &foreign).is_err());
    }

    #[test]
    fn extracts_ordered_component_widget_and_micro_widget_actions() {
        let board = page_board();
        let page = configured_page();
        let execution = PrerunPageExecution::from_page(&board, &page).unwrap();

        assert_eq!(execution.page_id, "page-1");
        assert_eq!(execution.action_events.len(), 7);
        assert_eq!(
            execution.special_events.load.as_ref().unwrap().node_id,
            "load"
        );
        assert_eq!(
            execution.special_events.unload.as_ref().unwrap().node_id,
            "unload"
        );
        assert_eq!(
            execution
                .special_events
                .interval
                .as_ref()
                .unwrap()
                .interval_seconds,
            Some(30)
        );

        let exact: Vec<_> = execution
            .action_events
            .iter()
            .filter_map(|action| match &action.locator {
                PrerunPageActionLocator::Component {
                    component_id,
                    handler: PrerunPageActionHandler::Exact(event),
                    action_index,
                } if component_id == "button" && event == "click" => {
                    Some((action.node_id.as_str(), *action_index))
                }
                _ => None,
            })
            .collect();
        assert!(exact.contains(&("exact-1", 1)));
        assert!(exact.contains(&("exact-2", 2)));
        assert!(execution.action_events.iter().any(|action| matches!(
            &action.locator,
            PrerunPageActionLocator::Component {
                handler: PrerunPageActionHandler::Legacy,
                ..
            }
        ) && action.node_id == "legacy"));
        assert!(execution.action_events.iter().any(|action| matches!(
            &action.locator,
            PrerunPageActionLocator::WidgetBinding { widget_action_id, .. }
                if widget_action_id == "submit"
        ) && action.node_id
            == "widget-submit"));
        assert!(execution.action_events.iter().any(|action| matches!(
            &action.locator,
            PrerunPageActionLocator::WidgetChild { child_component_id, .. }
                if child_component_id == "child"
        ) && action.node_id == "widget-child"));
        assert!(execution.action_events.iter().any(|action| matches!(
            &action.locator,
            PrerunPageActionLocator::MicroWidgetBinding {
                contract_event_name,
                ..
            } if contract_event_name == "pointSelected"
        ) && action.node_id == "micro-point"));
        assert!(
            !execution
                .action_events
                .iter()
                .any(|action| action.node_id.contains("shadowed")
                    || action.node_id.contains("undeclared"))
        );
    }

    #[test]
    fn decorates_every_extracted_leaf_without_changing_legacy_routing() {
        let board = page_board();
        let mut page = configured_page();
        page.components[0].component["eventHandlers"]["click"][1]["pageAction"] =
            json!({ "actionId": "forged" });
        page.components[0].component["eventHandlers"]["click"][1]["page_action"] =
            json!({ "actionId": "forged-alias" });
        let execution = PrerunPageExecution::from_page(&board, &page).unwrap();
        let decorated = decorate_page_actions(&page, &execution, "revision-1").unwrap();

        assert_eq!(
            page.components[0].component["eventHandlers"]["click"][1][PAGE_ACTION_METADATA_KEY]["actionId"],
            "forged"
        );
        let direct = &decorated.components[0].component["eventHandlers"]["click"][1];
        assert_eq!(direct["context"]["nodeId"], "exact-1");
        assert_eq!(
            direct[PAGE_ACTION_METADATA_KEY]["manifestRevision"],
            "revision-1"
        );
        assert!(
            direct[PAGE_ACTION_METADATA_KEY]["actionId"]
                .as_str()
                .unwrap()
                .starts_with(PAGE_ACTION_ID_PREFIX)
        );
        assert_ne!(direct[PAGE_ACTION_METADATA_KEY]["actionId"], "forged");
        assert!(direct.get("page_action").is_none());

        let widget = decorated
            .components
            .iter()
            .find(|component| component.id == "widget-host")
            .unwrap();
        assert_eq!(
            widget.component["actionBindings"]["submit"][PAGE_ACTION_METADATA_KEY]["manifestRevision"],
            "revision-1"
        );
        let child = &decorated.widget_refs["instance-1"].components[0].component;
        assert_eq!(
            child["eventHandlers"]["click"][0][PAGE_ACTION_METADATA_KEY]["manifestRevision"],
            "revision-1"
        );
        let micro = decorated
            .components
            .iter()
            .find(|component| component.id == "micro-host")
            .unwrap();
        assert_eq!(
            micro.component["actionBindings"]["pointSelected"][PAGE_ACTION_METADATA_KEY]["manifestRevision"],
            "revision-1"
        );
    }

    #[test]
    fn redacts_runtime_routes_but_preserves_opaque_actions_and_lifecycle_shape() {
        let board = page_board();
        let page = configured_page();
        let execution = PrerunPageExecution::from_page(&board, &page).unwrap();
        let decorated = decorate_page_actions(&page, &execution, "revision-1").unwrap();
        let redacted = redact_page_execution_routes(&decorated).unwrap();

        let action = &redacted.components[0].component["eventHandlers"]["click"][1];
        assert!(action["context"].get("nodeId").is_none());
        assert!(action[PAGE_ACTION_METADATA_KEY]["actionId"].is_string());
        assert_eq!(
            redacted.on_load_event_id.as_deref(),
            Some(PAGE_SPECIAL_LOAD_MARKER)
        );
        assert_eq!(
            redacted.on_unload_event_id.as_deref(),
            Some(PAGE_SPECIAL_UNLOAD_MARKER)
        );
        assert_eq!(
            redacted.on_interval_event_id.as_deref(),
            Some(PAGE_SPECIAL_INTERVAL_MARKER)
        );

        let widget = redacted
            .components
            .iter()
            .find(|component| component.id == "widget-host")
            .unwrap();
        let binding = &widget.component["actionBindings"]["submit"];
        assert!(binding["workflow"].get("flowId").is_none());
        assert!(binding[PAGE_ACTION_METADATA_KEY]["actionId"].is_string());

        let mut empty_lifecycle = Page::new("empty", "Empty", "/empty");
        empty_lifecycle.on_load_event_id = Some(String::new());
        let empty_lifecycle = redact_page_execution_routes(&empty_lifecycle).unwrap();
        assert_eq!(empty_lifecycle.on_load_event_id.as_deref(), Some(""));
    }

    #[test]
    fn recursively_extracts_decorates_and_redacts_nested_widget_actions() {
        let mut board = page_board();
        for id in [
            "nested-submit",
            "nested-first",
            "nested-second",
            "nested-micro",
        ] {
            add_entry(&mut board, id);
        }
        let page = nested_widget_page();

        let execution = PrerunPageExecution::from_page(&board, &page).unwrap();
        assert_eq!(execution.action_events.len(), 4);
        assert!(execution.action_events.iter().any(|action| matches!(
            &action.locator,
            PrerunPageActionLocator::NestedWidgetBinding {
                widget_path,
                widget_action_id,
            } if widget_path.len() == 2 && widget_action_id == "submit"
        ) && action.node_id
            == "nested-submit"));

        let nested_children: Vec<_> = execution
            .action_events
            .iter()
            .filter_map(|action| match &action.locator {
                PrerunPageActionLocator::NestedWidgetChild {
                    widget_path,
                    child_component_id,
                    handler: PrerunPageActionHandler::Exact(event),
                    action_index,
                } if widget_path.len() == 2
                    && child_component_id == "nested-button"
                    && event == "click" =>
                {
                    Some((action.node_id.as_str(), *action_index))
                }
                _ => None,
            })
            .collect();
        assert!(nested_children.contains(&("nested-first", 0)));
        assert!(nested_children.contains(&("nested-second", 1)));
        assert!(execution.action_events.iter().any(|action| matches!(
            &action.locator,
            PrerunPageActionLocator::NestedMicroWidgetBinding { locator }
                if locator.widget_path.len() == 2
                    && locator.host_component_id == "nested-micro"
                    && locator.contract_event_name == "selected"
        ) && action.node_id == "nested-micro"));

        let decorated = decorate_page_actions(&page, &execution, "nested-revision").unwrap();
        let nested_host = &decorated.widget_refs["outer-instance"].components[0].component;
        let nested_binding = &nested_host["actionBindings"]["submit"];
        assert_eq!(
            nested_binding[PAGE_ACTION_METADATA_KEY]["manifestRevision"],
            "nested-revision"
        );
        assert!(nested_binding.get("page_action").is_none());

        let inner = &decorated.widget_refs["inner-instance"];
        let button = &inner
            .components
            .iter()
            .find(|component| component.id == "nested-button")
            .unwrap()
            .component;
        let first = &button["eventHandlers"]["click"][0];
        let second = &button["eventHandlers"]["click"][1];
        assert_eq!(
            first[PAGE_ACTION_METADATA_KEY]["manifestRevision"],
            "nested-revision"
        );
        assert!(first.get("page_action").is_none());
        assert_ne!(
            first[PAGE_ACTION_METADATA_KEY]["actionId"],
            second[PAGE_ACTION_METADATA_KEY]["actionId"]
        );
        let micro = &inner
            .components
            .iter()
            .find(|component| component.id == "nested-micro")
            .unwrap()
            .component;
        assert!(
            micro["actionBindings"]["selected"][PAGE_ACTION_METADATA_KEY]["actionId"].is_string()
        );

        let redacted = redact_page_execution_routes(&decorated).unwrap();
        let redacted_inner = &redacted.widget_refs["inner-instance"];
        let redacted_button = &redacted_inner
            .components
            .iter()
            .find(|component| component.id == "nested-button")
            .unwrap()
            .component;
        assert!(
            redacted_button["eventHandlers"]["click"][0]["context"]
                .get("nodeId")
                .is_none()
        );
        assert!(
            redacted_button["eventHandlers"]["click"][0][PAGE_ACTION_METADATA_KEY]["actionId"]
                .is_string()
        );
        let redacted_micro = &redacted_inner
            .components
            .iter()
            .find(|component| component.id == "nested-micro")
            .unwrap()
            .component;
        assert!(
            redacted_micro["actionBindings"]["selected"]["workflow"]
                .get("flowId")
                .is_none()
        );
    }

    #[test]
    fn recursively_decorates_inline_widget_definitions() {
        let mut board = page_board();
        add_entry(&mut board, "inline-nested-action");
        let mut page = Page::new("page-1", "Inline", "/inline");
        page.board_id = Some("page-board".into());
        page.components.push(SurfaceComponent::new(
            "outer-inline-host",
            json!({
                "type": "widgetInstance",
                "instanceId": "outer-inline-instance",
                "widgetId": "outer-inline-widget",
                "inlineWidgetDef": {
                    "components": [{
                        "id": "inner-inline-host",
                        "component": {
                            "type": "widgetInstance",
                            "instanceId": "inner-inline-instance",
                            "widgetId": "inner-inline-widget",
                            "inlineWidgetDef": {
                                "components": [{
                                    "id": "inline-button",
                                    "component": {
                                        "type": "button",
                                        "eventHandlers": {
                                            "click": [{
                                                "name": "workflow_event",
                                                "context": { "nodeId": "inline-nested-action" }
                                            }]
                                        }
                                    }
                                }]
                            }
                        }
                    }]
                }
            }),
        ));

        let execution = extract_page_execution(&board, &page).unwrap();
        assert!(matches!(
            &execution.action_events[0].locator,
            PrerunPageActionLocator::NestedWidgetChild { widget_path, .. }
                if widget_path.len() == 2
                    && widget_path.iter().all(|ancestor| matches!(
                        ancestor.definition_source,
                        Some(PrerunPageWidgetDefinitionSource::Inline)
                    ))
        ));

        let decorated = decorate_page_actions(&page, &execution, "inline-revision").unwrap();
        let action = &decorated.components[0].component["inlineWidgetDef"]["components"][0]["component"]
            ["inlineWidgetDef"]["components"][0]["component"]["eventHandlers"]["click"][0];
        assert_eq!(
            action[PAGE_ACTION_METADATA_KEY]["manifestRevision"],
            "inline-revision"
        );

        let redacted = redact_page_execution_routes(&decorated).unwrap();
        let action = &redacted.components[0].component["inlineWidgetDef"]["components"][0]["component"]
            ["inlineWidgetDef"]["components"][0]["component"]["eventHandlers"]["click"][0];
        assert!(action["context"].get("nodeId").is_none());
        assert!(action[PAGE_ACTION_METADATA_KEY]["actionId"].is_string());
    }

    #[test]
    fn rejects_executable_exposed_props_for_referenced_and_inline_widgets() {
        let board = page_board();

        let mut referenced = Page::new("page-1", "Referenced", "/referenced");
        referenced.board_id = Some("page-board".into());
        let mut widget = crate::a2ui::Widget::new("referenced-widget", "Referenced", "child");
        widget.exposed_props.push(crate::a2ui::ExposedProp::new(
            "handler-override",
            "Handler override",
            "child",
            "settings.eventHandlers.click",
            crate::a2ui::ExposedPropType::Json,
        ));
        referenced
            .widget_refs
            .insert("referenced-instance".into(), widget);
        referenced.components.push(SurfaceComponent::new(
            "referenced-host",
            json!({
                "type": "widgetInstance",
                "instanceId": "referenced-instance",
                "widgetId": "referenced-widget"
            }),
        ));
        let error = extract_page_execution(&board, &referenced)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("exposes executable property path"),
            "{error}"
        );

        let mut inline = Page::new("page-1", "Inline", "/inline-exposed");
        inline.board_id = Some("page-board".into());
        inline.components.push(SurfaceComponent::new(
            "inline-host",
            json!({
                "type": "widgetInstance",
                "instanceId": "inline-instance",
                "widgetId": "inline-widget",
                "inlineWidgetDef": {
                    "components": [],
                    "exposedProps": [{
                        "id": "binding-override",
                        "targetComponentId": "child",
                        "propertyPath": "actionBindings.submit",
                        "propType": "Json"
                    }]
                }
            }),
        ));
        let error = extract_page_execution(&board, &inline)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("exposes executable property path"),
            "{error}"
        );
    }

    #[test]
    fn rejects_persisted_runtime_updates_that_replace_widget_actions() {
        let board = page_board();
        let mut page = Page::new("page-1", "Runtime updates", "/runtime-updates");
        page.board_id = Some("page-board".into());
        page.components.push(SurfaceComponent::new(
            "runtime-host",
            json!({
                "type": "widgetInstance",
                "instanceId": "runtime-instance",
                "widgetId": "runtime-widget",
                "inlineWidgetDef": {
                    "components": [{
                        "id": "button",
                        "component": { "type": "button" }
                    }]
                },
                "runtimeChildUpdates": {
                    "button": [{
                        "type": "setEventActions",
                        "eventName": "click",
                        "actions": [{
                            "name": "workflow_event",
                            "context": { "nodeId": "exact-1" }
                        }]
                    }]
                }
            }),
        ));

        let error = extract_page_execution(&board, &page)
            .unwrap_err()
            .to_string();
        assert!(error.contains("persisted runtime child update"), "{error}");
    }

    #[test]
    fn rejects_widget_reference_cycles_excessive_depth_and_nested_duplicates() {
        let board = page_board();

        let mut cyclic = Page::new("page-1", "Cyclic", "/cyclic");
        cyclic.board_id = Some("page-board".into());
        let mut cycle_widget = crate::a2ui::Widget::new("cycle-widget", "Cycle", "again");
        cycle_widget.components.push(SurfaceComponent::new(
            "again",
            json!({
                "type": "widgetInstance",
                "instanceId": "cycle-instance",
                "widgetId": "cycle-widget"
            }),
        ));
        cyclic
            .widget_refs
            .insert("cycle-instance".into(), cycle_widget);
        cyclic.components.push(SurfaceComponent::new(
            "cycle-host",
            json!({
                "type": "widgetInstance",
                "instanceId": "cycle-instance",
                "widgetId": "cycle-widget"
            }),
        ));
        let error = extract_page_execution(&board, &cyclic)
            .unwrap_err()
            .to_string();
        assert!(error.contains("widget reference cycle"), "{error}");

        let mut nested = json!({ "type": "text", "text": "leaf" });
        for depth in (0..=MAX_PAGE_WIDGET_NESTING_DEPTH).rev() {
            nested = json!({
                "type": "widgetInstance",
                "instanceId": format!("inline-{depth}"),
                "widgetId": format!("inline-widget-{depth}"),
                "inlineWidgetDef": {
                    "components": [{
                        "id": format!("child-{depth}"),
                        "component": nested
                    }]
                }
            });
        }
        let mut too_deep = Page::new("page-1", "Deep", "/deep");
        too_deep.board_id = Some("page-board".into());
        too_deep
            .components
            .push(SurfaceComponent::new("deep-host", nested));
        let error = extract_page_execution(&board, &too_deep)
            .unwrap_err()
            .to_string();
        assert!(error.contains("maximum depth"), "{error}");

        let mut duplicate = Page::new("page-1", "Duplicate", "/duplicate");
        duplicate.board_id = Some("page-board".into());
        let mut widget = crate::a2ui::Widget::new("duplicate-widget", "Duplicate", "micro-a");
        for child_id in ["micro-a", "micro-b"] {
            widget.components.push(SurfaceComponent::new(
                child_id,
                json!({
                    "type": "microWidgetInstance",
                    "instanceId": "same-nested-instance",
                    "packageId": "pkg.duplicate",
                    "packageVersion": "1.0.0",
                    "widgetId": "duplicate-micro",
                    "contract": { "events": {} },
                    "actionBindings": {}
                }),
            ));
        }
        duplicate
            .widget_refs
            .insert("duplicate-instance".into(), widget);
        duplicate.components.push(SurfaceComponent::new(
            "duplicate-host",
            json!({
                "type": "widgetInstance",
                "instanceId": "duplicate-instance",
                "widgetId": "duplicate-widget"
            }),
        ));
        let error = extract_page_execution(&board, &duplicate)
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicate widget instance id"), "{error}");
    }

    #[test]
    fn redacts_routes_nested_in_literal_json() {
        let mut page = Page::new("page-1", "Page", "/");
        page.components.push(SurfaceComponent::new(
            "json",
            json!({
                "type": "text",
                "value": {
                    "literalJson": "{\"actions\":[{\"name\":\"workflow_event\",\"context\":{\"nodeId\":\"secret-node\",\"input\":\"kept\"},\"pageAction\":{\"actionId\":\"pa1_test\"}}]}"
                }
            }),
        ));

        let redacted = redact_page_execution_routes(&page).unwrap();
        let raw = redacted.components[0].component["value"]["literalJson"]
            .as_str()
            .unwrap();
        let inner: Value = serde_json::from_str(raw).unwrap();
        assert!(inner["actions"][0]["context"].get("nodeId").is_none());
        assert_eq!(inner["actions"][0]["context"]["input"], "kept");
        assert_eq!(inner["actions"][0]["pageAction"]["actionId"], "pa1_test");
    }

    #[test]
    fn whole_board_page_manifest_is_strict_and_signed() {
        let board = page_board();
        let page = configured_page();
        let manifest = PrerunManifest::from_board_and_pages(&board, &[page]).unwrap();
        assert_eq!(manifest.page_events.len(), 1);
        assert_eq!(manifest.signature, manifest.compute_signature());

        let missing = PrerunManifest::from_board_and_pages(&board, &[])
            .unwrap_err()
            .to_string();
        assert!(missing.contains("payload was not supplied"), "{missing}");
    }

    #[test]
    fn rejects_non_entry_targets_and_duplicate_widget_instances() {
        let mut board = page_board();
        let mut internal = Node::new("noop", "Internal", "", "Other");
        internal.id = "internal".into();
        board.nodes.insert(internal.id.clone(), internal);
        let mut page = configured_page();
        page.components[0].component["eventHandlers"]["click"][1]["context"]["nodeId"] =
            json!("internal");
        let error = extract_page_execution(&board, &page)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not an entry node"), "{error}");

        let mut duplicate = configured_page();
        duplicate.components.push(SurfaceComponent::new(
            "another-widget-host",
            json!({
                "type": "widgetInstance",
                "instanceId": "instance-1",
                "widgetId": "widget-1",
                "actionBindings": {}
            }),
        ));
        let error = extract_page_execution(&board, &duplicate)
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicate widget instance id"), "{error}");

        let mut foreign = configured_page();
        foreign.board_id = Some("another-board".into());
        let error = extract_page_execution(&board, &foreign)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("belongs to board 'another-board'"),
            "{error}"
        );
    }

    #[test]
    fn decodes_v1_archive_with_empty_page_defaults() {
        let current = sample_manifest();
        let legacy = LegacyPrerunManifestV1 {
            runtime_variables: current.runtime_variables.clone(),
            oauth_requirements: current.oauth_requirements.clone(),
            requires_local_execution: current.requires_local_execution,
            execution_mode: current.execution_mode.clone(),
            has_wasm_nodes: current.has_wasm_nodes,
            wasm_package_ids: current.wasm_package_ids.clone(),
            wasm_package_permissions: current.wasm_package_permissions.clone(),
            element_selectors: current.element_selectors.clone(),
            element_reads_dynamic: current.element_reads_dynamic,
            signature: current.signature.clone(),
        };
        let archive = rkyv::to_bytes::<rkyv::rancor::Error>(&legacy).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MANIFEST_MAGIC);
        bytes.extend_from_slice(&LEGACY_MANIFEST_FORMAT_VERSION.to_le_bytes());
        bytes.push(CODEC_NONE);
        bytes.push(0);
        bytes.extend_from_slice(&archive);

        let decoded = decode_manifest(&bytes).unwrap();
        assert!(decoded.page_events.is_empty());
        assert_eq!(decoded.runtime_variables, current.runtime_variables);
        assert_eq!(decoded.signature, current.signature);
    }

    #[test]
    fn serde_defaults_page_events_for_old_documents() {
        let manifest = sample_manifest();
        let mut value = serde_json::to_value(&manifest).unwrap();
        value.as_object_mut().unwrap().remove("page_events");
        let decoded: PrerunManifest = serde_json::from_value(value).unwrap();
        assert!(decoded.page_events.is_empty());
    }

    #[test]
    fn unknown_strings_parse_leniently() {
        let mut manifest = sample_manifest();
        manifest.execution_mode = "Sideways".into();
        manifest.wasm_package_permissions[0]
            .1
            .push("network:carrier-pigeon".into());
        assert_eq!(manifest.execution_mode(), ExecutionMode::default());
        assert_eq!(
            manifest.wasm_permissions()["pkg-a"],
            vec![NodePermission::NetworkHttp, NodePermission::StorageRead]
        );
    }

    #[test]
    fn manifest_paths_sit_beside_artifacts() {
        let board_dir = Path::from("apps/app-1/meta");
        let artifact = super::super::artifact_path(&board_dir, "board-1", (1, 2, 3)).to_string();
        let manifest = manifest_path(&board_dir, "board-1", (1, 2, 3)).to_string();
        assert_eq!(manifest, "apps/app-1/meta/compiled/board-1/1_2_3.prerun");
        assert_eq!(
            manifest.trim_end_matches(".prerun"),
            artifact.trim_end_matches(".flcb")
        );

        let draft_artifact =
            super::super::draft_artifact_path("app-1", "board-1", "etag").to_string();
        let draft_manifest = draft_manifest_path("app-1", "board-1", "etag").to_string();
        assert!(draft_manifest.ends_with(".prerun"));
        assert_eq!(
            draft_manifest.trim_end_matches(".prerun"),
            draft_artifact.trim_end_matches(".flcb")
        );
    }
}
