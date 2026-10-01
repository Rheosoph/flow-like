//! Every tool schema FlowPilot can advertise must be valid on typed-schema providers: OpenAI's
//! Responses API forces strict mode through rig and rejects typeless nodes, rig's Gemini
//! conversion turns a typeless node into `"type": ""`, and Anthropic rejects composition keywords
//! at the root of a tool's input schema.

use std::collections::BTreeSet;
use std::sync::Arc;

use rig::{completion::ToolDefinition, tool::Tool, tools::ThinkTool};
use serde_json::{Map, Value};

use super::ir_tools::{
    BeginFlowIrDraftTool, BoundBeginFlowIrDraftTool, CommitFlowIrDraftTool,
    FlowIrAcceptanceBinding, FlowIrDraftStore, PlanFlowIrTool, UpdateFlowIrDraftTool,
    UpsertFlowIrModuleTool, ValidateFlowIrDraftTool,
};
use super::platform::{PlatformSurface, PlatformToolBridge};
use super::provider::CatalogProvider;
use super::tool_spec::{
    PlatformToolSpec, cross_board_source_tool_specs, data_studio_database_tool_spec,
    data_studio_specialist_tool_specs, data_studio_tool_specs, global_assistant_tool_specs,
    home_specialist_tool_specs, home_specialist_tool_specs_for_access, interact_app_page_tool_spec,
    public_web_tool_specs, runtime_execution_tool_specs, scoped_call_app_chat_spec,
    scout_specialist_tool_specs, scout_tool_specs, workflow_context_tool_specs,
    workspace_research_tool_specs,
};
use super::tools::{
    CatalogTool, CheckFlowScriptTool, CommitFlowScriptTool, DatabaseContextTool, ExecuteEventTool,
    ExecuteNodeTool, ExtendTimeBudgetTool, FilterCategoryTool, FindConnectableNodesTool,
    GetCurrentFlowScriptTool, GetDeclarationsTool, GetNodeDetailsTool, GetUnconfiguredNodesTool,
    ListBoardNodesTool, ModelFacingEmitCommandsTool, PatchFlowScriptTool, PlanBoardScopeTool,
    QueryExecutionLogsTool, QueryLogsTool, ReadSymbolTool, RunBoardTestsTool, SearchByPinTool,
    SearchTemplatesTool, SearchWorkspaceTool, StorageContextTool, UiInspectContextTool,
    WriteFlowScriptTool,
};
use super::types::NodeMetadata;
use super::{CopilotToolSurface, GraphContext, TestFlowScriptTool, prepare_context};
use crate::flow::board::Board;
use crate::state::{FlowLikeConfig, FlowLikeState};
use crate::utils::http::HTTPClient;

/// Keywords that let a node omit `type` because their variants or target are checked instead.
const TYPE_SUBSTITUTES: [&str; 5] = ["anyOf", "oneOf", "allOf", "enum", "$ref"];
const NAMED_CHILDREN: [&str; 5] = [
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
];
const LISTED_CHILDREN: [&str; 5] = ["items", "anyOf", "oneOf", "allOf", "prefixItems"];
/// Single-schema keywords. A boolean there only allows or forbids extra entries.
const NESTED_CHILDREN: [&str; 10] = [
    "additionalProperties",
    "unevaluatedProperties",
    "additionalItems",
    "unevaluatedItems",
    "not",
    "if",
    "then",
    "else",
    "contains",
    "propertyNames",
];
/// Anthropic rejects root `oneOf`/`anyOf`/`allOf`, and strict OpenAI requires a plain object root.
const ROOT_FORBIDDEN: [&str; 5] = ["oneOf", "anyOf", "allOf", "not", "enum"];

fn check_schema_node(node: &Value, path: &str, violations: &mut Vec<String>) {
    let Some(schema) = node.as_object() else {
        violations.push(format!("{path}: boolean schema `{node}` carries no type"));
        return;
    };
    violations.extend(type_violations(schema, path));
    for (child_path, child) in child_schemas(schema, path) {
        check_schema_node(child, &child_path, violations);
    }
}

fn type_violations(schema: &Map<String, Value>, path: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let types = match schema.get("type") {
        None => {
            if !TYPE_SUBSTITUTES.iter().any(|key| schema.contains_key(*key)) {
                violations.push(format!("{path}: schema has no `type`"));
            }
            Vec::new()
        }
        Some(Value::String(name)) => vec![name.as_str()],
        Some(Value::Array(names)) if !names.is_empty() && names.iter().all(Value::is_string) => {
            names.iter().filter_map(Value::as_str).collect()
        }
        Some(other) => {
            violations.push(format!(
                "{path}: `type` must be a string or a non-empty array of strings, got {other}"
            ));
            Vec::new()
        }
    };
    if types.contains(&"array") && !schema.contains_key("items") {
        violations.push(format!("{path}: array schema has no `items`"));
    }
    violations
}

fn child_schemas<'a>(schema: &'a Map<String, Value>, path: &str) -> Vec<(String, &'a Value)> {
    let mut children = named_child_schemas(schema, path);
    children.extend(listed_child_schemas(schema, path));
    children.extend(NESTED_CHILDREN.into_iter().filter_map(|key| {
        let child = schema.get(key).filter(|child| child.is_object())?;
        Some((format!("{path}/{key}"), child))
    }));
    children
}

fn named_child_schemas<'a>(schema: &'a Map<String, Value>, path: &str) -> Vec<(String, &'a Value)> {
    NAMED_CHILDREN
        .into_iter()
        .filter_map(|key| Some((key, schema.get(key)?.as_object()?)))
        .flat_map(|(key, named)| {
            named
                .iter()
                .map(move |(name, child)| (format!("{path}/{key}/{name}"), child))
        })
        .collect()
}

fn listed_child_schemas<'a>(
    schema: &'a Map<String, Value>,
    path: &str,
) -> Vec<(String, &'a Value)> {
    let mut children = Vec::new();
    for key in LISTED_CHILDREN {
        match schema.get(key) {
            Some(Value::Array(listed)) => children.extend(
                listed
                    .iter()
                    .enumerate()
                    .map(|(index, child)| (format!("{path}/{key}/{index}"), child)),
            ),
            Some(child) if key == "items" => children.push((format!("{path}/items"), child)),
            _ => {}
        }
    }
    children
}

fn root_violations(parameters: &Value, root: &str) -> Vec<String> {
    let mut violations = Vec::new();
    if parameters.get("type").and_then(Value::as_str) != Some("object") {
        violations.push(format!(
            "{root}: tool parameters must be a `type: object` schema"
        ));
    }
    violations.extend(
        ROOT_FORBIDDEN
            .into_iter()
            .filter(|key| parameters.get(*key).is_some())
            .map(|key| {
                format!("{root}: top-level `{key}` is rejected by Anthropic and strict OpenAI")
            }),
    );
    violations
}

fn definition_violations(definition: &ToolDefinition) -> Vec<String> {
    let root = format!("{}:#", definition.name);
    let mut violations = Vec::new();
    check_schema_node(&definition.parameters, &root, &mut violations);
    violations.extend(root_violations(&definition.parameters, &root));
    violations
}

fn spec_definitions(specs: Vec<PlatformToolSpec>) -> Vec<ToolDefinition> {
    specs
        .iter()
        .map(PlatformToolSpec::to_tool_definition)
        .collect()
}

trait IntoSpecs {
    fn into_specs(self) -> Vec<PlatformToolSpec>;
}

impl IntoSpecs for Vec<PlatformToolSpec> {
    fn into_specs(self) -> Vec<PlatformToolSpec> {
        self
    }
}

impl IntoSpecs for PlatformToolSpec {
    fn into_specs(self) -> Vec<PlatformToolSpec> {
        vec![self]
    }
}

macro_rules! spec_sets {
    ($($builder:ident($($arg:expr),*)),+ $(,)?) => {
        vec![$((
            stringify!($builder),
            spec_definitions(IntoSpecs::into_specs($builder($($arg),*))),
        )),+]
    };
}

/// Every public spec builder in `tool_spec.rs`, keyed by builder name.
fn spec_builder_definitions() -> Vec<(&'static str, Vec<ToolDefinition>)> {
    spec_sets![
        global_assistant_tool_specs(true),
        public_web_tool_specs(),
        scout_tool_specs(),
        scout_specialist_tool_specs(),
        workspace_research_tool_specs(),
        data_studio_tool_specs(),
        data_studio_database_tool_spec(),
        data_studio_specialist_tool_specs(),
        home_specialist_tool_specs(),
        home_specialist_tool_specs_for_access(true),
        workflow_context_tool_specs(),
        runtime_execution_tool_specs(),
        scoped_call_app_chat_spec(),
        interact_app_page_tool_spec(),
        cross_board_source_tool_specs(),
    ]
}

const PLATFORM_SURFACES: [PlatformSurface; 6] = [
    PlatformSurface::Orchestrator,
    PlatformSurface::Scout,
    PlatformSurface::DataStudio,
    PlatformSurface::Home,
    PlatformSurface::HomeReadOnly,
    PlatformSurface::OntologyQuery,
];

/// Exhaustive, so a new surface fails to compile until it is named here and in `PLATFORM_SURFACES`.
fn surface_label(surface: PlatformSurface) -> &'static str {
    match surface {
        PlatformSurface::Orchestrator => "orchestrator",
        PlatformSurface::Scout => "scout",
        PlatformSurface::DataStudio => "data_studio",
        PlatformSurface::Home => "home",
        PlatformSurface::HomeReadOnly => "home_read_only",
        PlatformSurface::OntologyQuery => "ontology_query",
    }
}

fn surface_definitions() -> Vec<(String, Vec<ToolDefinition>)> {
    PLATFORM_SURFACES
        .into_iter()
        .flat_map(|surface| {
            [false, true].map(|memory_enabled| {
                (
                    format!("{}/memory={memory_enabled}", surface_label(surface)),
                    spec_definitions(surface.tool_specs(memory_enabled)),
                )
            })
        })
        .collect()
}

struct NoCatalog;

#[async_trait::async_trait]
impl CatalogProvider for NoCatalog {
    async fn search(&self, _query: &str) -> Vec<NodeMetadata> {
        Vec::new()
    }
    async fn search_by_pin_type(&self, _pin_type: &str, _is_input: bool) -> Vec<NodeMetadata> {
        Vec::new()
    }
    async fn filter_by_category(&self, _category_prefix: &str) -> Vec<NodeMetadata> {
        Vec::new()
    }
    async fn get_node_metadata(&self, _node_type: &str) -> Option<NodeMetadata> {
        None
    }
    async fn get_all_nodes(&self) -> Vec<String> {
        Vec::new()
    }
}

struct NoBridge;

#[async_trait::async_trait]
impl PlatformToolBridge for NoBridge {
    async fn call(&self, _tool_name: &str, _arguments: Value) -> String {
        String::new()
    }
}

macro_rules! definitions {
    ($($tool:ident $({ $($field:tt)* })?),+ $(,)?) => {
        vec![$((
            stringify!($tool),
            $tool $({ $($field)* })?.definition(String::new()).await,
        )),+]
    };
}

/// Board tools the desktop SDK adapters advertise although `FlowCopilot::chat` never attaches them.
const SDK_ONLY_BOARD_TOOLS: [&str; 2] = ["ExtendTimeBudgetTool", "BeginFlowIrDraftTool"];
/// Board tools `FlowCopilot::chat` attaches only when `CopilotToolSurface` exposes their name.
const HIDDEN_BOARD_TOOLS: [(&str, &str); 1] = [(
    "ModelFacingEmitCommandsTool",
    ModelFacingEmitCommandsTool::NAME,
)];

/// Dependencies of every rig tool the board copilot can attach, including the currently disabled
/// typed Flow IR lifecycle and the tools only the SDK adapters advertise.
struct BoardCopilotTools {
    board: Arc<Board>,
    provider: Arc<dyn CatalogProvider>,
    bridge: Arc<dyn PlatformToolBridge>,
    graph_context: Arc<GraphContext>,
    store: Arc<FlowIrDraftStore>,
    acceptance_binding: FlowIrAcceptanceBinding,
}

type NamedDefinitions = Vec<(&'static str, ToolDefinition)>;

impl BoardCopilotTools {
    fn new() -> Self {
        let board = Arc::new(Board::new_detached(
            Some("tool-schema-guard".to_string()),
            flow_like_storage::Path::default(),
        ));
        let store = Arc::new(FlowIrDraftStore::new());
        let acceptance_binding = store.bind_request_acceptance_contract(&board.id, "guard schemas");
        Self {
            graph_context: Arc::new(prepare_context(&board, &[]).expect("empty board context")),
            board,
            provider: Arc::new(NoCatalog),
            bridge: Arc::new(NoBridge),
            store,
            acceptance_binding,
        }
    }

    /// Tool type name and definition for every covered board tool.
    async fn definitions(&self) -> NamedDefinitions {
        let mut definitions = self.inspection_definitions().await;
        definitions.extend(self.flowscript_definitions().await);
        definitions.extend(self.typed_flow_ir_definitions().await);
        definitions.extend(self.runtime_bridge_definitions().await);
        definitions
    }

    async fn inspection_definitions(&self) -> NamedDefinitions {
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let graph_context = &self.graph_context;
        let provider = &self.provider;
        definitions![
            ThinkTool,
            ExtendTimeBudgetTool,
            PlanBoardScopeTool,
            GetCurrentFlowScriptTool {
                board: self.board.clone()
            },
            GetDeclarationsTool {
                provider: provider.clone()
            },
            GetNodeDetailsTool {
                graph_context: graph_context.clone()
            },
            ListBoardNodesTool {
                graph_context: graph_context.clone()
            },
            GetUnconfiguredNodesTool {
                graph_context: graph_context.clone()
            },
            FindConnectableNodesTool {
                provider: provider.clone(),
                graph_context: graph_context.clone(),
            },
            CatalogTool {
                provider: provider.clone()
            },
            SearchByPinTool {
                provider: provider.clone()
            },
            FilterCategoryTool {
                provider: provider.clone()
            },
            SearchTemplatesTool {
                templates: Vec::new(),
                current_template_id: None,
            },
            QueryLogsTool {
                state,
                run_context: None,
            },
        ]
    }

    async fn flowscript_definitions(&self) -> NamedDefinitions {
        let (board, provider, store) = (&self.board, &self.provider, &self.store);
        let binding = &self.acceptance_binding;
        definitions![
            WriteFlowScriptTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
                acceptance_binding: binding.clone(),
            },
            PatchFlowScriptTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
                acceptance_binding: binding.clone(),
            },
            CheckFlowScriptTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
                acceptance_binding: binding.clone(),
            },
            TestFlowScriptTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
                acceptance_binding: binding.clone(),
            },
            CommitFlowScriptTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
                acceptance_binding: binding.clone(),
            },
        ]
    }

    async fn typed_flow_ir_definitions(&self) -> NamedDefinitions {
        let (board, provider, store) = (&self.board, &self.provider, &self.store);
        definitions![
            PlanFlowIrTool {
                provider: provider.clone()
            },
            BoundBeginFlowIrDraftTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
                acceptance_binding: self.acceptance_binding.clone(),
            },
            BeginFlowIrDraftTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
            },
            UpdateFlowIrDraftTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
            },
            UpsertFlowIrModuleTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
            },
            ValidateFlowIrDraftTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
            },
            CommitFlowIrDraftTool {
                board: board.clone(),
                provider: provider.clone(),
                store: store.clone(),
            },
        ]
    }

    async fn runtime_bridge_definitions(&self) -> NamedDefinitions {
        let bridge = &self.bridge;
        definitions![
            DatabaseContextTool {
                bridge: bridge.clone()
            },
            StorageContextTool {
                bridge: bridge.clone()
            },
            UiInspectContextTool {
                bridge: bridge.clone()
            },
            SearchWorkspaceTool {
                bridge: bridge.clone()
            },
            ReadSymbolTool {
                bridge: bridge.clone()
            },
            ExecuteEventTool {
                bridge: bridge.clone()
            },
            ExecuteNodeTool {
                bridge: bridge.clone()
            },
            QueryExecutionLogsTool {
                bridge: bridge.clone()
            },
            RunBoardTestsTool {
                bridge: bridge.clone()
            },
        ]
    }
}

/// Public builders in `tool_spec.rs` that return one spec or a spec list.
fn declared_spec_builders(source: &str) -> BTreeSet<&str> {
    source
        .lines()
        .filter_map(|line| line.strip_prefix("pub fn "))
        .filter(|signature| {
            signature.ends_with("-> Vec<PlatformToolSpec> {")
                || signature.ends_with("-> PlatformToolSpec {")
        })
        .filter_map(|signature| signature.split_once('(').map(|(name, _)| name))
        .collect()
}

/// Rig tool types that `FlowCopilot::chat` in `mod.rs` attaches with `.tool(...)`.
fn attached_board_tools(source: &str) -> BTreeSet<&str> {
    source
        .split(".tool(")
        .skip(1)
        .filter_map(|rest| {
            let rest = rest.trim_start();
            let end = rest
                .find(|character: char| {
                    !(character.is_alphanumeric() || character == '_' || character == ':')
                })
                .unwrap_or(rest.len());
            rest[..end].rsplit("::").next()
        })
        .collect()
}

fn sorted_difference<'a>(left: &BTreeSet<&'a str>, right: &BTreeSet<&'a str>) -> Vec<&'a str> {
    left.difference(right).copied().collect()
}

#[tokio::test]
async fn every_advertised_tool_schema_is_portable() {
    let mut sets = surface_definitions();
    sets.extend(
        spec_builder_definitions()
            .into_iter()
            .map(|(builder, definitions)| (builder.to_string(), definitions)),
    );
    let board_tools = BoardCopilotTools::new().definitions().await;
    sets.push((
        "board_copilot".to_string(),
        board_tools
            .into_iter()
            .map(|(_, definition)| definition)
            .collect(),
    ));

    let violations = sets
        .iter()
        .flat_map(|(set, definitions)| {
            definitions.iter().flat_map(move |definition| {
                definition_violations(definition)
                    .into_iter()
                    .map(move |violation| format!("[{set}] {violation}"))
            })
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "tool schemas that typed-schema providers reject:\n{}",
        violations.join("\n")
    );
}

#[tokio::test]
async fn schema_guard_covers_every_production_tool_source() {
    let declared = declared_spec_builders(include_str!("tool_spec.rs"));
    let covered = spec_builder_definitions()
        .into_iter()
        .map(|(builder, _)| builder)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        declared, covered,
        "spec builders in tool_spec.rs and spec_builder_definitions() differ"
    );

    for (_, name) in HIDDEN_BOARD_TOOLS {
        for read_only in [false, true] {
            assert!(
                !CopilotToolSurface::for_mode(read_only).exposes(name),
                "{name} reaches the model (read_only={read_only}); add it to BoardCopilotTools"
            );
        }
    }
    let attached = attached_board_tools(include_str!("mod.rs"))
        .into_iter()
        .filter(|tool| !HIDDEN_BOARD_TOOLS.iter().any(|(hidden, _)| hidden == tool))
        .collect::<BTreeSet<_>>();
    let covered = BoardCopilotTools::new()
        .definitions()
        .await
        .into_iter()
        .map(|(tool, _)| tool)
        .filter(|tool| !SDK_ONLY_BOARD_TOOLS.contains(tool))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        (
            sorted_difference(&attached, &covered),
            sorted_difference(&covered, &attached)
        ),
        (Vec::new(), Vec::new()),
        "(attached in FlowCopilot::chat but unchecked, checked but never attached)"
    );
}

#[test]
fn schema_guard_reports_typeless_and_itemless_nodes() {
    let mut violations = Vec::new();
    check_schema_node(
        &serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "description": "anything" },
                "list": { "type": ["array", "null"] },
                "any": true,
                "scalar": { "anyOf": [{ "type": "string" }, { "description": "untyped" }] },
                "choice": { "enum": ["a", "b"] },
                "map": { "type": "object", "additionalProperties": { "description": "untyped" } },
                "closed": { "type": "object", "additionalProperties": false },
                "tuple": { "type": "array", "items": { "type": "string" }, "prefixItems": [{ "type": "string" }, {}] },
                "rows": { "type": "array", "items": { "type": "array", "items": {} } },
                "negated": { "type": "string", "not": { "description": "untyped" } }
            },
            "$defs": { "Shared": { "properties": {} } }
        }),
        "tool:#",
        &mut violations,
    );
    assert_eq!(
        violations,
        vec![
            "tool:#/properties/value: schema has no `type`",
            "tool:#/properties/list: array schema has no `items`",
            "tool:#/properties/any: boolean schema `true` carries no type",
            "tool:#/properties/scalar/anyOf/1: schema has no `type`",
            "tool:#/properties/map/additionalProperties: schema has no `type`",
            "tool:#/properties/tuple/prefixItems/1: schema has no `type`",
            "tool:#/properties/rows/items/items: schema has no `type`",
            "tool:#/properties/negated/not: schema has no `type`",
            "tool:#/$defs/Shared: schema has no `type`",
        ]
    );
}

#[test]
fn schema_guard_rejects_root_composition() {
    let definition = ToolDefinition {
        name: "tool".to_string(),
        description: String::new(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": { "source": { "type": "string" } },
            "oneOf": [
                { "type": "object", "required": ["source"] },
                { "type": "object", "required": ["edits"] }
            ]
        }),
    };
    assert_eq!(
        definition_violations(&definition),
        vec!["tool:#: top-level `oneOf` is rejected by Anthropic and strict OpenAI"]
    );
}
