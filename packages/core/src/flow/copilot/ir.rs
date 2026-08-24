//! Typed, provider-agnostic input surface for FlowPilot workflow generation.
//!
//! The human-facing FlowScript language remains the editable/round-trippable representation of a
//! board. `FlowIrProgram` is deliberately smaller: a model chooses exact catalog node and pin
//! names, gives every produced value a stable step id, and makes ambiguous execution
//! continuations explicit. The compiler below produces the regular `flow_like_ast::BoardAst`, so
//! there is still one reconciliation and application pipeline.

use std::collections::{BTreeMap, HashMap, HashSet};

use flow_like_ast::model::{
    Arg, Block, BoardAst, BranchArm, Call, Container, DEFAULT_FUNCTION_CACHE_NAMESPACE,
    DEFAULT_FUNCTION_CACHE_TTL_SECONDS, EventBlock, Expr, FnDecl, FunctionCache,
    FunctionCacheScope, InterfaceDecl, Literal, ObjectField, Param, Stmt, TypeRef, VarDecl,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::provider::metadata_to_signature;
use super::search::{score_catalog_metadata, tokenize_query_text};
use super::types::{NodeMetadata, PinMetadata};
use crate::flow::ast::{
    MAX_NODES_PER_LAYER, RenderOptions, dynamic_placeholder_config_pin, render,
    synthesize_dynamic_input_pin_from_template,
};

pub const FLOW_IR_VERSION: &str = "flowpilot.ir/v1";
pub const MAX_FLOW_IR_MODULES: usize = 64;
pub const MAX_FLOW_IR_TOTAL_STEPS: usize = 4_096;
pub const MAX_FLOW_IR_NESTING_DEPTH: usize = 32;
pub const MAX_FLOW_IR_VALUES: usize = 16_384;
pub const MAX_FLOW_IR_SERIALIZED_BYTES: usize = 1_048_576;
pub const MAX_FLOW_IR_CAPABILITY_REQUIREMENTS: usize = 128;
pub const MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION: usize = 16;

fn default_ir_version() -> String {
    FLOW_IR_VERSION.to_string()
}

fn default_true() -> bool {
    true
}

/// Complete typed workflow input. Modules are independently replaceable in a draft session.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrProgram {
    #[serde(default = "default_ir_version")]
    pub version: String,
    #[serde(default)]
    pub interfaces: Vec<FlowIrInterface>,
    #[serde(default)]
    pub variables: Vec<FlowIrVariable>,
    #[serde(default)]
    pub modules: Vec<FlowIrModule>,
}

impl Default for FlowIrProgram {
    fn default() -> Self {
        Self {
            version: default_ir_version(),
            interfaces: Vec::new(),
            variables: Vec::new(),
            modules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrInterface {
    pub name: String,
    #[serde(default)]
    pub fields: Vec<FlowIrInterfaceField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrInterfaceField {
    pub name: String,
    #[serde(rename = "type")]
    pub value_type: FlowIrType,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub default: Option<FlowIrLiteral>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrVariable {
    pub name: String,
    #[serde(rename = "type")]
    pub value_type: FlowIrType,
    #[serde(default)]
    pub default: Option<FlowIrLiteral>,
    #[serde(default)]
    pub exposed: bool,
    #[serde(default)]
    pub secret: bool,
    #[serde(default = "default_true")]
    pub editable: bool,
    #[serde(default)]
    pub runtime_configured: bool,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub anchor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrParam {
    pub name: String,
    #[serde(rename = "type")]
    pub value_type: FlowIrType,
}

/// Result-cache settings for a typed FlowPilot function module.
///
/// The cache key includes the function layer and all function inputs. A hit skips the complete
/// function body, including side effects, so this is only safe for input-determined functions.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrFunctionCache {
    /// Namespace used to group entries for targeted invalidation. Defaults to `global`.
    #[serde(default = "default_flow_ir_function_cache_namespace")]
    pub namespace: String,
    /// Entry lifetime in seconds. Omission defaults to 300; zero (and legacy `null`) is permanent.
    #[serde(default = "default_flow_ir_function_cache_ttl_seconds")]
    pub ttl_seconds: Option<u64>,
    /// Whether cached results are shared by the app or isolated to the triggering user.
    #[serde(default)]
    pub scope: FlowIrFunctionCacheScope,
}

fn default_flow_ir_function_cache_namespace() -> String {
    DEFAULT_FUNCTION_CACHE_NAMESPACE.to_string()
}

fn default_flow_ir_function_cache_ttl_seconds() -> Option<u64> {
    Some(DEFAULT_FUNCTION_CACHE_TTL_SECONDS)
}

impl Default for FlowIrFunctionCache {
    fn default() -> Self {
        Self {
            namespace: default_flow_ir_function_cache_namespace(),
            ttl_seconds: default_flow_ir_function_cache_ttl_seconds(),
            scope: FlowIrFunctionCacheScope::App,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FlowIrFunctionCacheScope {
    #[default]
    App,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FlowIrModule {
    Function {
        name: String,
        #[serde(default)]
        params: Vec<FlowIrParam>,
        #[serde(default)]
        returns: Vec<FlowIrParam>,
        /// Optional result-cache configuration. Omit it to leave the function uncached. A cache
        /// hit skips the entire body and all side effects.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<FlowIrFunctionCache>,
        #[serde(default)]
        steps: Vec<FlowIrStep>,
        #[serde(default)]
        anchor: Option<String>,
    },
    Event {
        /// FlowScript event header, normally the declaration display name (`eventsSimple`).
        name: String,
        /// Exact live catalog node type (`events_simple`).
        node_type: String,
        #[serde(default)]
        params: Vec<FlowIrParam>,
        #[serde(default)]
        steps: Vec<FlowIrStep>,
        #[serde(default)]
        anchor: Option<String>,
    },
}

impl FlowIrModule {
    pub fn name(&self) -> &str {
        match self {
            Self::Function { name, .. } | Self::Event { name, .. } => name,
        }
    }

    pub fn steps(&self) -> &[FlowIrStep] {
        match self {
            Self::Function { steps, .. } | Self::Event { steps, .. } => steps,
        }
    }

    pub fn executable_step_count(&self) -> usize {
        count_steps(self.steps())
    }
}

impl FlowIrProgram {
    pub fn executable_step_count(&self) -> usize {
        self.modules
            .iter()
            .map(FlowIrModule::executable_step_count)
            .sum()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrExecutionArm {
    /// Exact execution output pin on the node.
    pub pin: String,
    #[serde(default)]
    pub steps: Vec<FlowIrStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FlowIrStep {
    /// Invoke one exact catalog declaration. `id` is the only way later values reference outputs.
    Node {
        id: String,
        node_type: String,
        #[serde(default)]
        args: Vec<FlowIrArg>,
        /// Required when this node has multiple execution outputs and later steps should continue
        /// through one of them. The value is the exact execution output pin name.
        #[serde(default)]
        continue_from: Option<String>,
        /// Outcome-specific bodies for nodes with multiple execution outputs (for example,
        /// separate HTTP success and error paths).
        #[serde(default)]
        exec_arms: Vec<FlowIrExecutionArm>,
        #[serde(default)]
        anchor: Option<String>,
    },
    /// Invoke a function declared in this same program.
    #[serde(alias = "call")]
    CallFunction {
        id: String,
        function: String,
        #[serde(default)]
        args: Vec<FlowIrArg>,
        #[serde(default)]
        anchor: Option<String>,
    },
    /// Boolean control flow. The compiler selects the catalog's canonical Branch declaration.
    If {
        id: String,
        condition: FlowIrValue,
        #[serde(default, alias = "then")]
        then_steps: Vec<FlowIrStep>,
        #[serde(default, alias = "else")]
        else_steps: Vec<FlowIrStep>,
        #[serde(default)]
        anchor: Option<String>,
    },
    /// Sequential or parallel for-each. `item` and optional `index` become typed aliases inside
    /// the loop body; there is no mutable loop-variable syntax for the model to synthesize.
    ForEach {
        id: String,
        array: FlowIrValue,
        item: String,
        #[serde(default)]
        index: Option<String>,
        #[serde(default)]
        parallel: bool,
        #[serde(default)]
        steps: Vec<FlowIrStep>,
        #[serde(default)]
        anchor: Option<String>,
    },
    Assign {
        target: String,
        value: FlowIrValue,
    },
    Return {
        #[serde(default)]
        values: Vec<FlowIrValue>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrArg {
    pub pin: String,
    /// Zero-based occurrence for declarations that unfortunately contain duplicate pin names.
    #[serde(default)]
    pub occurrence: usize,
    pub value: FlowIrValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FlowIrValue {
    Literal {
        value: FlowIrLiteral,
    },
    /// Function parameter, top-level variable, or loop alias.
    #[serde(alias = "param")]
    Ref {
        name: String,
    },
    /// Exact named data output from an earlier step in this lexical scope.
    Output {
        step: String,
        pin: String,
        /// Duplicate data output names cannot be represented safely by FlowScript today. Only
        /// occurrence zero is accepted until the AST carries stable pin identity.
        #[serde(default)]
        occurrence: usize,
    },
    List {
        items: Vec<FlowIrValue>,
    },
    Object {
        fields: Vec<FlowIrObjectField>,
    },
    /// Function/Event references for a synthetic `tools` or `fnRefs` node argument. These are
    /// lowered to bare FlowScript refs and materialize as SetNodeFunctionRefs, never as data pins.
    FunctionRefs {
        functions: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowIrObjectField {
    pub key: String,
    pub value: FlowIrValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum FlowIrLiteral {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
    Json(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq, Hash)]
pub struct FlowIrType {
    #[serde(alias = "kind")]
    pub data_type: FlowIrDataType,
    #[serde(default)]
    pub container: FlowIrContainer,
    /// Optional nominal interface name for `struct` values.
    #[serde(default)]
    pub interface: Option<String>,
}

impl<'de> Deserialize<'de> for FlowIrType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DetailedType {
            #[serde(alias = "kind")]
            data_type: FlowIrDataType,
            #[serde(default)]
            container: FlowIrContainer,
            #[serde(default)]
            interface: Option<String>,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum TypeInput {
            Shorthand(FlowIrDataType),
            Detailed(DetailedType),
        }

        Ok(match TypeInput::deserialize(deserializer)? {
            TypeInput::Shorthand(data_type) => Self::scalar(data_type),
            TypeInput::Detailed(value) => Self {
                data_type: value.data_type,
                container: value.container,
                interface: value.interface,
            },
        })
    }
}

impl FlowIrType {
    pub const fn scalar(data_type: FlowIrDataType) -> Self {
        Self {
            data_type,
            container: FlowIrContainer::Normal,
            interface: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FlowIrDataType {
    String,
    #[serde(alias = "int")]
    Integer,
    Float,
    #[serde(alias = "bool")]
    Boolean,
    Struct,
    Generic,
    Date,
    Path,
    Bytes,
    /// Internal fail-closed representation for a catalog type this IR version does not know.
    #[serde(skip)]
    #[schemars(skip)]
    Unsupported,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FlowIrContainer {
    #[default]
    Normal,
    Array,
    Map,
    Set,
}

/// Stable, JSON-pointer-addressable compiler diagnostic. This is also useful before FlowScript
/// exists, so repair never has to scrape prose to discover a declaration or pin.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct FlowIrDiagnostic {
    pub code: String,
    pub phase: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caused_by: Vec<String>,
}

impl FlowIrDiagnostic {
    fn new(
        code: impl Into<String>,
        path: impl Into<String>,
        scope: Option<&str>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            phase: "compile".to_string(),
            path: path.into(),
            scope: scope.map(str::to_string),
            message: message.into(),
            expected: None,
            actual: None,
            declaration: None,
            pin: None,
            fix: None,
            caused_by: Vec::new(),
        }
    }
}

fn validate_authored_type(
    value_type: &FlowIrType,
    path: &str,
    scope: Option<&str>,
    interface_names: &HashSet<String>,
    diagnostics: &mut Vec<FlowIrDiagnostic>,
) {
    if value_type.data_type == FlowIrDataType::Unsupported {
        diagnostics.push(FlowIrDiagnostic::new(
            "IR_DATA_TYPE_UNSUPPORTED",
            format!("{path}/data_type"),
            scope,
            "unsupported is an internal fail-closed type and cannot be authored",
        ));
    }
    if let Some(interface) = value_type.interface.as_deref() {
        if value_type.data_type != FlowIrDataType::Struct {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_INTERFACE_TYPE_INVALID",
                format!("{path}/interface"),
                scope,
                "a nominal interface may only qualify a struct type",
            ));
        } else if !flow_like_ast::is_valid_identifier(interface)
            || !interface_names.contains(&normalize_symbol(interface))
        {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_INTERFACE_REFERENCE_MISSING",
                format!("{path}/interface"),
                scope,
                format!("interface {interface:?} is not declared in this program"),
            );
            diagnostic.actual = Some(interface.to_string());
            diagnostics.push(diagnostic);
        }
    }
}

pub(super) fn validate_ir_resource_limits(program: &FlowIrProgram) -> Vec<FlowIrDiagnostic> {
    let mut diagnostics = Vec::new();
    if program.modules.len() > MAX_FLOW_IR_MODULES {
        diagnostics.push(FlowIrDiagnostic::new(
            "IR_MODULE_LIMIT_EXCEEDED",
            "/modules",
            None,
            format!(
                "typed program contains {} modules; the limit is {MAX_FLOW_IR_MODULES}",
                program.modules.len()
            ),
        ));
    }
    if serde_json::to_vec(program)
        .map(|encoded| encoded.len() > MAX_FLOW_IR_SERIALIZED_BYTES)
        .unwrap_or(true)
    {
        diagnostics.push(FlowIrDiagnostic::new(
            "IR_SIZE_LIMIT_EXCEEDED",
            "/",
            None,
            format!(
                "typed program exceeds the {} byte serialized limit",
                MAX_FLOW_IR_SERIALIZED_BYTES
            ),
        ));
    }

    let mut step_stack = program
        .modules
        .iter()
        .enumerate()
        .flat_map(|(module_index, module)| {
            module
                .steps()
                .iter()
                .enumerate()
                .map(move |(step_index, step)| {
                    (
                        step,
                        1_usize,
                        format!("/modules/{module_index}/steps/{step_index}"),
                    )
                })
        })
        .collect::<Vec<_>>();
    let mut value_stack = Vec::<(&FlowIrValue, usize, String)>::new();
    let mut step_count = 0_usize;
    let mut value_count = 0_usize;
    let mut depth_reported = false;
    while let Some((step, depth, path)) = step_stack.pop() {
        step_count = step_count.saturating_add(1);
        if depth > MAX_FLOW_IR_NESTING_DEPTH && !depth_reported {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_NESTING_LIMIT_EXCEEDED",
                &path,
                None,
                format!("typed step nesting exceeds the limit of {MAX_FLOW_IR_NESTING_DEPTH}"),
            ));
            depth_reported = true;
        }
        match step {
            FlowIrStep::Node {
                args, exec_arms, ..
            } => {
                for (argument_index, argument) in args.iter().enumerate() {
                    value_stack.push((
                        &argument.value,
                        1,
                        format!("{path}/args/{argument_index}/value"),
                    ));
                }
                for (arm_index, arm) in exec_arms.iter().enumerate() {
                    for (step_index, step) in arm.steps.iter().enumerate() {
                        step_stack.push((
                            step,
                            depth + 1,
                            format!("{path}/exec_arms/{arm_index}/steps/{step_index}"),
                        ));
                    }
                }
            }
            FlowIrStep::CallFunction { args, .. } => {
                for (argument_index, argument) in args.iter().enumerate() {
                    value_stack.push((
                        &argument.value,
                        1,
                        format!("{path}/args/{argument_index}/value"),
                    ));
                }
            }
            FlowIrStep::If {
                condition,
                then_steps,
                else_steps,
                ..
            } => {
                value_stack.push((condition, 1, format!("{path}/condition")));
                for (step_index, step) in then_steps.iter().enumerate() {
                    step_stack.push((step, depth + 1, format!("{path}/then_steps/{step_index}")));
                }
                for (step_index, step) in else_steps.iter().enumerate() {
                    step_stack.push((step, depth + 1, format!("{path}/else_steps/{step_index}")));
                }
            }
            FlowIrStep::ForEach { array, steps, .. } => {
                value_stack.push((array, 1, format!("{path}/array")));
                for (step_index, step) in steps.iter().enumerate() {
                    step_stack.push((step, depth + 1, format!("{path}/steps/{step_index}")));
                }
            }
            FlowIrStep::Assign { value, .. } => {
                value_stack.push((value, 1, format!("{path}/value")));
            }
            FlowIrStep::Return { values } => {
                for (value_index, value) in values.iter().enumerate() {
                    value_stack.push((value, 1, format!("{path}/values/{value_index}")));
                }
            }
        }
        if step_count > MAX_FLOW_IR_TOTAL_STEPS {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_TOTAL_STEP_LIMIT_EXCEEDED",
                "/modules",
                None,
                format!("typed program exceeds the {MAX_FLOW_IR_TOTAL_STEPS} step limit"),
            ));
            break;
        }
    }

    let mut value_depth_reported = false;
    while let Some((value, depth, path)) = value_stack.pop() {
        value_count = value_count.saturating_add(1);
        if depth > MAX_FLOW_IR_NESTING_DEPTH && !value_depth_reported {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_VALUE_NESTING_LIMIT_EXCEEDED",
                &path,
                None,
                format!("typed value nesting exceeds the limit of {MAX_FLOW_IR_NESTING_DEPTH}"),
            ));
            value_depth_reported = true;
        }
        match value {
            FlowIrValue::List { items } => {
                for (index, item) in items.iter().enumerate() {
                    value_stack.push((item, depth + 1, format!("{path}/items/{index}")));
                }
            }
            FlowIrValue::Object { fields } => {
                for (index, field) in fields.iter().enumerate() {
                    value_stack.push((
                        &field.value,
                        depth + 1,
                        format!("{path}/fields/{index}/value"),
                    ));
                }
            }
            FlowIrValue::Literal { .. }
            | FlowIrValue::Ref { .. }
            | FlowIrValue::Output { .. }
            | FlowIrValue::FunctionRefs { .. } => {}
        }
        if value_count > MAX_FLOW_IR_VALUES {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_VALUE_LIMIT_EXCEEDED",
                "/modules",
                None,
                format!("typed program exceeds the {MAX_FLOW_IR_VALUES} value limit"),
            ));
            break;
        }
    }
    diagnostics
}

fn validate_unique_ir_anchors(program: &FlowIrProgram, diagnostics: &mut Vec<FlowIrDiagnostic>) {
    fn register(
        anchor: &Option<String>,
        path: String,
        scope: Option<&str>,
        seen: &mut HashMap<String, String>,
        diagnostics: &mut Vec<FlowIrDiagnostic>,
    ) {
        let Some(anchor) = anchor.as_deref().map(str::trim).filter(|id| !id.is_empty()) else {
            return;
        };
        if let Some(first_path) = seen.insert(anchor.to_string(), path.clone()) {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_DUPLICATE_ANCHOR",
                path,
                scope,
                format!("anchor {anchor:?} is used by more than one typed entity"),
            );
            diagnostic.expected = Some("one entity per stable board anchor".to_string());
            diagnostic.actual = Some(format!("already used at {first_path}"));
            diagnostic.fix = Some(
                "keep the anchor only on the one retained entity it identifies; leave new entities unanchored"
                    .to_string(),
            );
            diagnostics.push(diagnostic);
        }
    }

    fn visit_steps(
        steps: &[FlowIrStep],
        path: &str,
        scope: &str,
        seen: &mut HashMap<String, String>,
        diagnostics: &mut Vec<FlowIrDiagnostic>,
    ) {
        for (index, step) in steps.iter().enumerate() {
            let step_path = format!("{path}/{index}");
            match step {
                FlowIrStep::Node {
                    anchor, exec_arms, ..
                } => {
                    register(
                        anchor,
                        format!("{step_path}/anchor"),
                        Some(scope),
                        seen,
                        diagnostics,
                    );
                    for (arm_index, arm) in exec_arms.iter().enumerate() {
                        visit_steps(
                            &arm.steps,
                            &format!("{step_path}/exec_arms/{arm_index}/steps"),
                            scope,
                            seen,
                            diagnostics,
                        );
                    }
                }
                FlowIrStep::CallFunction { anchor, .. } => register(
                    anchor,
                    format!("{step_path}/anchor"),
                    Some(scope),
                    seen,
                    diagnostics,
                ),
                FlowIrStep::If {
                    anchor,
                    then_steps,
                    else_steps,
                    ..
                } => {
                    register(
                        anchor,
                        format!("{step_path}/anchor"),
                        Some(scope),
                        seen,
                        diagnostics,
                    );
                    visit_steps(
                        then_steps,
                        &format!("{step_path}/then_steps"),
                        scope,
                        seen,
                        diagnostics,
                    );
                    visit_steps(
                        else_steps,
                        &format!("{step_path}/else_steps"),
                        scope,
                        seen,
                        diagnostics,
                    );
                }
                FlowIrStep::ForEach { anchor, steps, .. } => {
                    register(
                        anchor,
                        format!("{step_path}/anchor"),
                        Some(scope),
                        seen,
                        diagnostics,
                    );
                    visit_steps(
                        steps,
                        &format!("{step_path}/steps"),
                        scope,
                        seen,
                        diagnostics,
                    );
                }
                FlowIrStep::Assign { .. } | FlowIrStep::Return { .. } => {}
            }
        }
    }

    let mut seen = HashMap::new();
    for (index, variable) in program.variables.iter().enumerate() {
        register(
            &variable.anchor,
            format!("/variables/{index}/anchor"),
            Some(&variable.name),
            &mut seen,
            diagnostics,
        );
    }
    for (index, module) in program.modules.iter().enumerate() {
        let anchor = match module {
            FlowIrModule::Function { anchor, .. } | FlowIrModule::Event { anchor, .. } => anchor,
        };
        register(
            anchor,
            format!("/modules/{index}/anchor"),
            Some(module.name()),
            &mut seen,
            diagnostics,
        );
        visit_steps(
            module.steps(),
            &format!("/modules/{index}/steps"),
            module.name(),
            &mut seen,
            diagnostics,
        );
    }
}

fn validate_module_step_identifiers(
    steps: &[FlowIrStep],
    path: &str,
    scope: &str,
    reserved_symbols: &HashSet<String>,
    diagnostics: &mut Vec<FlowIrDiagnostic>,
) {
    let mut seen = reserved_symbols.clone();
    let mut stack = steps
        .iter()
        .enumerate()
        .map(|(index, step)| (step, format!("{path}/{index}")))
        .collect::<Vec<_>>();
    while let Some((step, step_path)) = stack.pop() {
        let id = match step {
            FlowIrStep::Node { id, exec_arms, .. } => {
                for (arm_index, arm) in exec_arms.iter().enumerate() {
                    for (index, child) in arm.steps.iter().enumerate() {
                        stack.push((
                            child,
                            format!("{step_path}/exec_arms/{arm_index}/steps/{index}"),
                        ));
                    }
                }
                Some(id)
            }
            FlowIrStep::CallFunction { id, .. } => Some(id),
            FlowIrStep::If {
                id,
                then_steps,
                else_steps,
                ..
            } => {
                for (index, child) in then_steps.iter().enumerate() {
                    stack.push((child, format!("{step_path}/then_steps/{index}")));
                }
                for (index, child) in else_steps.iter().enumerate() {
                    stack.push((child, format!("{step_path}/else_steps/{index}")));
                }
                Some(id)
            }
            FlowIrStep::ForEach { id, steps, .. } => {
                for (index, child) in steps.iter().enumerate() {
                    stack.push((child, format!("{step_path}/steps/{index}")));
                }
                Some(id)
            }
            FlowIrStep::Assign { .. } | FlowIrStep::Return { .. } => None,
        };
        if let Some(id) = id
            && (!flow_like_ast::is_valid_identifier(id) || !seen.insert(normalize_symbol(id)))
        {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_STEP_ID_INVALID",
                format!("{step_path}/id"),
                Some(scope),
                "step ids must be valid and unique throughout the complete module",
            ));
        }
        if let FlowIrStep::ForEach { item, index, .. } = step {
            for (name, field) in [(Some(item), "item"), (index.as_ref(), "index")]
                .into_iter()
                .filter_map(|(name, field)| name.map(|name| (name, field)))
            {
                if !flow_like_ast::is_valid_identifier(name) || !seen.insert(normalize_symbol(name))
                {
                    diagnostics.push(FlowIrDiagnostic::new(
                        "IR_LOOP_ALIAS_SHADOWS_SYMBOL",
                        format!("{step_path}/{field}"),
                        Some(scope),
                        "loop aliases must be valid and cannot shadow parameters, variables, step ids, or another alias",
                    ));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowIrCompileResult {
    #[serde(skip)]
    pub ast: Option<BoardAst>,
    pub flowscript: String,
    pub diagnostics: Vec<FlowIrDiagnostic>,
    pub module_node_counts: BTreeMap<String, usize>,
}

impl FlowIrCompileResult {
    pub fn is_valid(&self) -> bool {
        self.ast.is_some() && self.diagnostics.is_empty()
    }
}

#[derive(Debug, Clone)]
struct FunctionSignature {
    name: String,
    params: Vec<FlowIrParam>,
    returns: Vec<FlowIrParam>,
}

#[derive(Debug, Clone)]
struct ValueSource {
    value_type: FlowIrType,
    expression: Expr,
}

#[derive(Debug, Clone)]
struct ModuleContext<'a> {
    scope: &'a str,
    /// Event returns materialize a terminal result node. Function returns only wire boundary
    /// values and therefore do not behave like an early control-flow exit.
    event_returns_terminate: bool,
    catalog: &'a HashMap<String, &'a NodeMetadata>,
    functions: &'a HashMap<String, FunctionSignature>,
    reference_targets: &'a HashMap<String, String>,
    symbols: HashMap<String, ValueSource>,
    step_outputs: HashMap<String, HashMap<String, ValueSource>>,
    ambiguous_step_outputs: HashSet<(String, String)>,
    diagnostics: Vec<FlowIrDiagnostic>,
}

/// Compile a constrained program into the canonical BoardAst and rendered FlowScript.
pub fn compile_flow_ir(program: &FlowIrProgram, catalog: &[NodeMetadata]) -> FlowIrCompileResult {
    let mut diagnostics = Vec::new();
    if program.version != FLOW_IR_VERSION {
        let mut diagnostic = FlowIrDiagnostic::new(
            "IR_VERSION_UNSUPPORTED",
            "/version",
            None,
            format!(
                "unsupported FlowPilot IR version {:?}; expected {:?}",
                program.version, FLOW_IR_VERSION
            ),
        );
        diagnostic.expected = Some(FLOW_IR_VERSION.to_string());
        diagnostic.actual = Some(program.version.clone());
        diagnostics.push(diagnostic);
    }
    let resource_diagnostics = validate_ir_resource_limits(program);
    let resource_limits_exceeded = !resource_diagnostics.is_empty();
    diagnostics.extend(resource_diagnostics);
    if resource_limits_exceeded {
        return FlowIrCompileResult {
            ast: None,
            flowscript: String::new(),
            diagnostics,
            module_node_counts: BTreeMap::new(),
        };
    }
    validate_unique_ir_anchors(program, &mut diagnostics);

    let mut interface_names = HashSet::new();
    for (interface_index, interface) in program.interfaces.iter().enumerate() {
        let normalized = normalize_symbol(&interface.name);
        if !flow_like_ast::is_valid_identifier(&interface.name)
            || !interface_names.insert(normalized)
        {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_INTERFACE_NAME_INVALID",
                format!("/interfaces/{interface_index}/name"),
                Some(&interface.name),
                "interface names must be valid, unique FlowScript identifiers",
            ));
        }
    }
    for (interface_index, interface) in program.interfaces.iter().enumerate() {
        let mut field_names = HashSet::new();
        for (field_index, field) in interface.fields.iter().enumerate() {
            if field.name.trim().is_empty() || !field_names.insert(normalize_symbol(&field.name)) {
                diagnostics.push(FlowIrDiagnostic::new(
                    "IR_INTERFACE_FIELD_NAME_INVALID",
                    format!("/interfaces/{interface_index}/fields/{field_index}/name"),
                    Some(&interface.name),
                    "interface field names must be non-empty and unique",
                ));
            }
            validate_authored_type(
                &field.value_type,
                &format!("/interfaces/{interface_index}/fields/{field_index}/type"),
                Some(&interface.name),
                &interface_names,
                &mut diagnostics,
            );
            if field.value_type.container == FlowIrContainer::Set {
                diagnostics.push(FlowIrDiagnostic::new(
                    "IR_INTERFACE_SET_UNSUPPORTED",
                    format!("/interfaces/{interface_index}/fields/{field_index}/type/container"),
                    Some(&interface.name),
                    "FlowScript interfaces cannot represent set fields without changing their shape",
                ));
            }
            if let Some(default) = &field.default {
                let actual = literal_type(default);
                if !types_compatible(&actual, &field.value_type) {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_INTERFACE_DEFAULT_TYPE",
                        format!("/interfaces/{interface_index}/fields/{field_index}/default"),
                        Some(&interface.name),
                        format!("default for field {:?} has the wrong type", field.name),
                    );
                    diagnostic.expected = Some(type_label(&field.value_type));
                    diagnostic.actual = Some(type_label(&actual));
                    diagnostics.push(diagnostic);
                }
            }
        }
    }

    let catalog_by_name = catalog
        .iter()
        .map(|metadata| (normalize_symbol(&metadata.name), metadata))
        .collect::<HashMap<_, _>>();
    let mut functions = HashMap::new();
    let mut module_names = HashSet::new();
    for (index, module) in program.modules.iter().enumerate() {
        let name = module.name().trim();
        if !flow_like_ast::is_valid_identifier(name) || !module_names.insert(normalize_symbol(name))
        {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_MODULE_NAME_INVALID",
                format!("/modules/{index}/name"),
                Some(name),
                "module names must be valid, unique FlowScript identifiers",
            ));
        }
        let (params, returns) = match module {
            FlowIrModule::Function {
                params, returns, ..
            } => (params.as_slice(), returns.as_slice()),
            FlowIrModule::Event { params, .. } => (params.as_slice(), &[][..]),
        };
        let mut param_names = HashSet::new();
        for (param_index, param) in params.iter().enumerate() {
            if !flow_like_ast::is_valid_identifier(&param.name)
                || !param_names.insert(normalize_symbol(&param.name))
            {
                diagnostics.push(FlowIrDiagnostic::new(
                    "IR_PARAMETER_NAME_INVALID",
                    format!("/modules/{index}/params/{param_index}/name"),
                    Some(name),
                    "parameter names must be valid, unique FlowScript identifiers",
                ));
            }
            validate_authored_type(
                &param.value_type,
                &format!("/modules/{index}/params/{param_index}/type"),
                Some(name),
                &interface_names,
                &mut diagnostics,
            );
        }
        let mut return_names = HashSet::new();
        for (return_index, return_param) in returns.iter().enumerate() {
            if !flow_like_ast::is_valid_identifier(&return_param.name)
                || !return_names.insert(normalize_symbol(&return_param.name))
            {
                diagnostics.push(FlowIrDiagnostic::new(
                    "IR_RETURN_NAME_INVALID",
                    format!("/modules/{index}/returns/{return_index}/name"),
                    Some(name),
                    "return names must be valid, unique FlowScript identifiers",
                ));
            }
            validate_authored_type(
                &return_param.value_type,
                &format!("/modules/{index}/returns/{return_index}/type"),
                Some(name),
                &interface_names,
                &mut diagnostics,
            );
        }
        if let FlowIrModule::Function {
            params, returns, ..
        } = module
        {
            functions.insert(
                normalize_symbol(name),
                FunctionSignature {
                    name: name.to_string(),
                    params: params.clone(),
                    returns: returns.clone(),
                },
            );
        }
    }

    let mut global_symbols = HashMap::new();
    let mut variable_names = HashSet::new();
    for (index, variable) in program.variables.iter().enumerate() {
        if !flow_like_ast::is_valid_identifier(&variable.name)
            || !variable_names.insert(normalize_symbol(&variable.name))
        {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_VARIABLE_NAME_INVALID",
                format!("/variables/{index}/name"),
                None,
                "variable names must be valid, unique FlowScript identifiers",
            ));
        }
        validate_authored_type(
            &variable.value_type,
            &format!("/variables/{index}/type"),
            None,
            &interface_names,
            &mut diagnostics,
        );
        if let Some(default) = &variable.default {
            let actual = literal_type(default);
            if !types_compatible(&actual, &variable.value_type) {
                let mut diagnostic = FlowIrDiagnostic::new(
                    "IR_VARIABLE_DEFAULT_TYPE",
                    format!("/variables/{index}/default"),
                    None,
                    format!(
                        "default for variable {:?} has the wrong type",
                        variable.name
                    ),
                );
                diagnostic.expected = Some(type_label(&variable.value_type));
                diagnostic.actual = Some(type_label(&actual));
                diagnostics.push(diagnostic);
            }
        }
        global_symbols.insert(
            normalize_symbol(&variable.name),
            ValueSource {
                value_type: variable.value_type.clone(),
                expression: Expr::Ref(variable.name.clone()),
            },
        );
    }

    let interfaces = program
        .interfaces
        .iter()
        .map(interface_to_ast)
        .collect::<Vec<_>>();
    let variables = program
        .variables
        .iter()
        .map(variable_to_ast)
        .collect::<Vec<_>>();
    let mut ast = BoardAst {
        board_id: String::new(),
        uses: Vec::new(),
        interfaces,
        variables,
        functions: Vec::new(),
        events: Vec::new(),
        modules: Vec::new(),
    };
    let mut module_node_counts = BTreeMap::new();
    let mut root_node_count = 0_usize;
    let reference_targets = program
        .modules
        .iter()
        .map(|module| (normalize_symbol(module.name()), module.name().to_string()))
        .collect::<HashMap<_, _>>();

    for (module_index, module) in program.modules.iter().enumerate() {
        let scope = module.name();
        let params = match module {
            FlowIrModule::Function { params, .. } | FlowIrModule::Event { params, .. } => params,
        };
        let mut reserved_symbols = global_symbols.keys().cloned().collect::<HashSet<_>>();
        reserved_symbols.extend(params.iter().map(|param| normalize_symbol(&param.name)));
        validate_module_step_identifiers(
            module.steps(),
            &format!("/modules/{module_index}/steps"),
            scope,
            &reserved_symbols,
            &mut diagnostics,
        );
        validate_return_placement(
            module,
            &format!("/modules/{module_index}/steps"),
            &mut diagnostics,
        );
        let body_count = count_materialized_steps(
            module.steps(),
            &variable_names,
            matches!(module, FlowIrModule::Event { .. }),
        );
        let count = if matches!(module, FlowIrModule::Event { .. }) {
            // Every Event entry and all Event bodies materialize on the shared root layer.
            let event_count = body_count.saturating_add(1);
            root_node_count = root_node_count.saturating_add(event_count);
            event_count
        } else {
            body_count
        };
        module_node_counts.insert(scope.to_string(), count);
        if matches!(module, FlowIrModule::Function { .. }) && count > MAX_NODES_PER_LAYER {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_NODE_BUDGET_EXCEEDED",
                format!("/modules/{module_index}/steps"),
                Some(scope),
                format!(
                    "module {scope:?} requires {count} nodes; the per-layer limit is {MAX_NODES_PER_LAYER}"
                ),
            );
            diagnostic.expected = Some(format!("<= {MAX_NODES_PER_LAYER} nodes"));
            diagnostic.actual = Some(format!("{count} nodes"));
            diagnostic.fix = Some(
                "move one responsibility into a separate function module and call it here"
                    .to_string(),
            );
            diagnostics.push(diagnostic);
        }
        validate_unreachable_steps(
            module.steps(),
            &format!("/modules/{module_index}/steps"),
            scope,
            &catalog_by_name,
            matches!(module, FlowIrModule::Event { .. }),
            &mut diagnostics,
        );
        if let FlowIrModule::Function { returns, steps, .. } = module
            && !returns.is_empty()
            && !steps_always_return(steps, &catalog_by_name)
        {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_RETURN_MISSING",
                format!("/modules/{module_index}/steps"),
                Some(scope),
                format!("function {scope:?} does not return on every reachable path"),
            );
            diagnostic.expected = Some(format!("{} return value(s) on every path", returns.len()));
            diagnostic.fix = Some(
                "add a return to the missing branch or an unconditional final return".to_string(),
            );
            diagnostics.push(diagnostic);
        }

        let mut symbols = global_symbols.clone();
        let params = match module {
            FlowIrModule::Function { params, .. } | FlowIrModule::Event { params, .. } => params,
        };
        for param in params {
            symbols.insert(
                normalize_symbol(&param.name),
                ValueSource {
                    value_type: param.value_type.clone(),
                    expression: Expr::Ref(param.name.clone()),
                },
            );
        }
        let mut context = ModuleContext {
            scope,
            event_returns_terminate: matches!(module, FlowIrModule::Event { .. }),
            catalog: &catalog_by_name,
            functions: &functions,
            reference_targets: &reference_targets,
            symbols,
            step_outputs: HashMap::new(),
            ambiguous_step_outputs: HashSet::new(),
            diagnostics: Vec::new(),
        };
        let returns = match module {
            FlowIrModule::Function { returns, .. } => returns.as_slice(),
            FlowIrModule::Event { .. } => &[],
        };
        let block = compile_steps(
            module.steps(),
            returns,
            &format!("/modules/{module_index}/steps"),
            &mut context,
            false,
        );
        diagnostics.extend(context.diagnostics);

        match module {
            FlowIrModule::Function {
                name,
                params,
                returns,
                cache,
                anchor,
                ..
            } => ast.functions.push(FnDecl {
                name: name.clone(),
                params: params.iter().map(param_to_ast).collect(),
                returns: returns.iter().map(param_to_ast).collect(),
                body: block,
                cache: cache.as_ref().map(|cache| FunctionCache {
                    namespace: cache.namespace.clone(),
                    ttl_seconds: cache.ttl_seconds,
                    scope: match cache.scope {
                        FlowIrFunctionCacheScope::App => FunctionCacheScope::App,
                        FlowIrFunctionCacheScope::User => FunctionCacheScope::User,
                    },
                }),
                anchor: anchor.clone(),
            }),
            FlowIrModule::Event {
                name,
                node_type,
                params,
                anchor,
                ..
            } => {
                let resolved_event = resolve_catalog_node(node_type, &catalog_by_name);
                if resolved_event.is_none() {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_EVENT_DECLARATION_MISSING",
                        format!("/modules/{module_index}/node_type"),
                        Some(scope),
                        format!("event node type {node_type:?} is not in the live catalog"),
                    );
                    diagnostic.declaration = Some(node_type.clone());
                    diagnostic.fix = Some(
                        "select an exact event node_type returned by the capability planner"
                            .to_string(),
                    );
                    diagnostics.push(diagnostic);
                }
                if let Some(metadata) = resolved_event {
                    let has_exec_input = metadata
                        .inputs
                        .iter()
                        .any(|pin| pin.data_type.eq_ignore_ascii_case("Execution"));
                    let has_exec_output = metadata
                        .outputs
                        .iter()
                        .any(|pin| pin.data_type.eq_ignore_ascii_case("Execution"));
                    if has_exec_input || !has_exec_output {
                        let mut diagnostic = FlowIrDiagnostic::new(
                            "IR_EVENT_ENTRY_INVALID",
                            format!("/modules/{module_index}/node_type"),
                            Some(scope),
                            format!("catalog node {:?} cannot be an Event entry", metadata.name),
                        );
                        diagnostic.expected = Some(
                            "no Execution input and at least one Execution output".to_string(),
                        );
                        diagnostic.actual = Some(format!(
                            "execution_input={has_exec_input}, execution_output={has_exec_output}"
                        ));
                        diagnostics.push(diagnostic);
                    }
                    for (param_index, param) in params.iter().enumerate() {
                        let matching = metadata.outputs.iter().find(|pin| {
                            !pin.data_type.eq_ignore_ascii_case("Execution")
                                && pin.name.eq_ignore_ascii_case(&param.name)
                        });
                        match matching {
                            Some(pin) if !types_compatible(&pin_type(pin), &param.value_type) => {
                                let mut diagnostic = FlowIrDiagnostic::new(
                                    "IR_EVENT_PARAM_TYPE",
                                    format!("/modules/{module_index}/params/{param_index}/type"),
                                    Some(scope),
                                    format!(
                                        "Event parameter {:?} does not match catalog output {:?}",
                                        param.name, pin.name
                                    ),
                                );
                                diagnostic.expected = Some(type_label(&pin_type(pin)));
                                diagnostic.actual = Some(type_label(&param.value_type));
                                diagnostic.pin = Some(pin.name.clone());
                                diagnostics.push(diagnostic);
                            }
                            Some(_) => {}
                            None if metadata.name == "events_generic" => {
                                // Generic entries intentionally materialize authored custom data
                                // output pins during reconciliation.
                            }
                            None => {
                                let mut diagnostic = FlowIrDiagnostic::new(
                                    "IR_EVENT_PARAM_PIN_MISSING",
                                    format!("/modules/{module_index}/params/{param_index}/name"),
                                    Some(scope),
                                    format!(
                                        "Event {:?} has no catalog data output named {:?}",
                                        metadata.name, param.name
                                    ),
                                );
                                diagnostic.pin = Some(param.name.clone());
                                diagnostic.fix = Some(
                                    "use an exact catalog output, or use events_generic when the entry must declare a custom parameter"
                                        .to_string(),
                                );
                                diagnostics.push(diagnostic);
                            }
                        }
                    }
                }
                ast.events.push(EventBlock {
                    name: name.clone(),
                    event_name: None,
                    // Reconciliation treats a typed Event node_type as an authoritative catalog
                    // identity. Canonicalize an accepted camel/display spelling here just as node
                    // calls already do, so compile-time resolution and materialization cannot
                    // disagree about the same declaration.
                    node_type: resolved_event
                        .map(|metadata| metadata.name.clone())
                        .unwrap_or_else(|| node_type.clone()),
                    params: params.iter().map(param_to_ast).collect(),
                    body: block,
                    anchor: anchor.clone(),
                });
            }
        }
    }

    if root_node_count > 0 {
        module_node_counts.insert("$root".to_string(), root_node_count);
    }
    if root_node_count > MAX_NODES_PER_LAYER {
        let mut diagnostic = FlowIrDiagnostic::new(
            "IR_NODE_BUDGET_EXCEEDED",
            "/modules",
            Some("$root"),
            format!(
                "all Event entries and bodies require {root_node_count} root-layer nodes; the limit is {MAX_NODES_PER_LAYER}"
            ),
        );
        diagnostic.expected = Some(format!("<= {MAX_NODES_PER_LAYER} root nodes"));
        diagnostic.actual = Some(format!("{root_node_count} root nodes"));
        diagnostic.fix = Some(
            "move Event body responsibilities into function modules and keep each Event as a thin entry"
                .to_string(),
        );
        diagnostics.push(diagnostic);
    }

    let flowscript = render(
        &ast,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    if diagnostics.is_empty()
        && let Err(error) = flow_like_ast::parse(&flowscript)
    {
        diagnostics.push(FlowIrDiagnostic {
            code: "IR_RENDER_INVARIANT".to_string(),
            phase: "render".to_string(),
            path: "/".to_string(),
            scope: None,
            message: format!("typed IR rendered invalid FlowScript: {error}"),
            expected: Some("parser-valid FlowScript".to_string()),
            actual: Some(error.to_string()),
            declaration: None,
            pin: None,
            fix: Some(
                "report this as a FlowPilot compiler bug; do not repair the JSON".to_string(),
            ),
            caused_by: Vec::new(),
        });
    }

    FlowIrCompileResult {
        ast: diagnostics.is_empty().then_some(ast),
        flowscript,
        diagnostics,
        module_node_counts,
    }
}

fn compile_steps(
    steps: &[FlowIrStep],
    returns: &[FlowIrParam],
    path: &str,
    context: &mut ModuleContext<'_>,
    outer_continuation: bool,
) -> Block {
    compile_steps_with_offset(steps, returns, path, context, outer_continuation, 0)
}

fn compile_steps_with_offset(
    steps: &[FlowIrStep],
    returns: &[FlowIrParam],
    path: &str,
    context: &mut ModuleContext<'_>,
    outer_continuation: bool,
    index_offset: usize,
) -> Block {
    let mut statements = Vec::new();
    let mut index = 0;
    while index < steps.len() {
        let step_path = format!("{path}/{}", index + index_offset);
        match &steps[index] {
            FlowIrStep::Node {
                id,
                node_type,
                args,
                continue_from,
                exec_arms,
                anchor,
            } => {
                let Some(metadata) = resolve_catalog_node(node_type, context.catalog) else {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_DECLARATION_MISSING",
                        format!("{step_path}/node_type"),
                        Some(context.scope),
                        format!("node type {node_type:?} is not in the live catalog"),
                    );
                    diagnostic.declaration = Some(node_type.clone());
                    diagnostic.fix = Some(
                        "use an exact node_type returned by plan_flow_ir/get_declarations"
                            .to_string(),
                    );
                    context.diagnostics.push(diagnostic);
                    index += 1;
                    continue;
                };
                let has_exec_input = metadata
                    .inputs
                    .iter()
                    .any(|pin| pin.data_type.eq_ignore_ascii_case("Execution"));
                let has_exec_output = metadata
                    .outputs
                    .iter()
                    .any(|pin| pin.data_type.eq_ignore_ascii_case("Execution"));
                if has_exec_output && !has_exec_input {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_ENTRY_NODE_AS_STEP",
                        format!("{step_path}/node_type"),
                        Some(context.scope),
                        format!(
                            "node {node_type:?} produces execution but cannot accept an incoming execution cursor"
                        ),
                    );
                    diagnostic.declaration =
                        Some(metadata_to_signature(metadata).render_declaration());
                    diagnostic.fix = Some(
                        "author this declaration as an Event module entry, or choose a catalog node with an Execution input"
                            .to_string(),
                    );
                    context.diagnostics.push(diagnostic);
                }
                let call = compile_catalog_call(id, metadata, args, &step_path, context);
                let exec_outputs = metadata
                    .outputs
                    .iter()
                    .filter(|pin| pin.data_type.eq_ignore_ascii_case("Execution"))
                    .collect::<Vec<_>>();
                let has_local_tail = index + 1 < steps.len();
                let has_continuation = has_local_tail || outer_continuation;
                // Always lower a multi-execution declaration as an explicit branch. Treating a
                // tail-position call as a plain `let` leaves reconciliation to guess a default
                // execution output, which is exactly the ambiguity this IR is meant to remove.
                if exec_outputs.len() > 1 {
                    let continuation = continue_from.as_deref();
                    if has_continuation && continuation.is_none() {
                        let mut diagnostic = FlowIrDiagnostic::new(
                            "IR_EXEC_CONTINUATION_REQUIRED",
                            format!("{step_path}/continue_from"),
                            Some(context.scope),
                            format!(
                                "node {node_type:?} has multiple execution outputs and a following continuation"
                            ),
                        );
                        diagnostic.expected = Some(
                            exec_outputs
                                .iter()
                                .map(|pin| pin.name.as_str())
                                .collect::<Vec<_>>()
                                .join(" | "),
                        );
                        diagnostic.fix = Some(
                            "set continue_from to the exact outcome that may reach following steps and use exec_arms for outcome-specific work"
                                .to_string(),
                        );
                        context.diagnostics.push(diagnostic);
                    }
                    if let Some(continuation) = continuation
                        && !exec_outputs
                            .iter()
                            .any(|pin| pin.name.eq_ignore_ascii_case(continuation))
                    {
                        let mut diagnostic = FlowIrDiagnostic::new(
                            "IR_EXEC_CONTINUATION_UNKNOWN",
                            format!("{step_path}/continue_from"),
                            Some(context.scope),
                            format!("{continuation:?} is not an execution output on {node_type:?}"),
                        );
                        diagnostic.pin = Some(continuation.to_string());
                        diagnostic.expected = Some(
                            exec_outputs
                                .iter()
                                .map(|pin| pin.name.as_str())
                                .collect::<Vec<_>>()
                                .join(" | "),
                        );
                        context.diagnostics.push(diagnostic);
                    }

                    let mut seen_arms = HashSet::new();
                    let mut compiled_arms = HashMap::<String, Block>::new();
                    let mut compiled_arm_contexts = HashMap::new();
                    for (arm_index, arm) in exec_arms.iter().enumerate() {
                        let arm_key = normalize_symbol(&arm.pin);
                        let arm_path = format!("{step_path}/exec_arms/{arm_index}");
                        if !seen_arms.insert(arm_key.clone()) {
                            let mut diagnostic = FlowIrDiagnostic::new(
                                "IR_EXEC_ARM_DUPLICATE",
                                format!("{arm_path}/pin"),
                                Some(context.scope),
                                format!("execution arm {:?} is declared more than once", arm.pin),
                            );
                            diagnostic.pin = Some(arm.pin.clone());
                            context.diagnostics.push(diagnostic);
                            continue;
                        }
                        if !exec_outputs
                            .iter()
                            .any(|pin| pin.name.eq_ignore_ascii_case(&arm.pin))
                        {
                            let mut diagnostic = FlowIrDiagnostic::new(
                                "IR_EXEC_ARM_UNKNOWN",
                                format!("{arm_path}/pin"),
                                Some(context.scope),
                                format!(
                                    "{:?} is not an execution output on {node_type:?}",
                                    arm.pin
                                ),
                            );
                            diagnostic.pin = Some(arm.pin.clone());
                            diagnostic.expected = Some(
                                exec_outputs
                                    .iter()
                                    .map(|pin| pin.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" | "),
                            );
                            context.diagnostics.push(diagnostic);
                            continue;
                        }
                        let mut arm_context = context.clone();
                        arm_context.diagnostics.clear();
                        let arm_reaches_continuation = has_continuation
                            && continuation.is_some_and(|pin| pin.eq_ignore_ascii_case(&arm.pin));
                        let body = compile_steps(
                            &arm.steps,
                            returns,
                            &format!("{arm_path}/steps"),
                            &mut arm_context,
                            arm_reaches_continuation,
                        );
                        context.diagnostics.append(&mut arm_context.diagnostics);
                        compiled_arm_contexts.insert(arm_key.clone(), arm_context);
                        compiled_arms.insert(arm_key, body);
                    }

                    if outer_continuation && let Some(continuation) = continuation {
                        for pin in exec_outputs
                            .iter()
                            .filter(|pin| !pin.name.eq_ignore_ascii_case(continuation))
                        {
                            let terminates = exec_arms
                                .iter()
                                .find(|arm| arm.pin.eq_ignore_ascii_case(&pin.name))
                                .is_some_and(|arm| {
                                    steps_prevent_fallthrough(
                                        &arm.steps,
                                        context.catalog,
                                        context.event_returns_terminate,
                                    )
                                });
                            if !terminates {
                                let mut diagnostic = FlowIrDiagnostic::new(
                                    "IR_EXEC_OUTCOME_MUST_TERMINATE",
                                    format!("{step_path}/exec_arms"),
                                    Some(context.scope),
                                    format!(
                                        "outcome {:?} would otherwise rejoin an outer continuation despite continue_from {:?}",
                                        pin.name, continuation
                                    ),
                                );
                                diagnostic.pin = Some(pin.name.clone());
                                diagnostic.fix = Some(if context.event_returns_terminate {
                                    "add an explicit outcome arm that returns from the Event or ends in a terminal execution node"
                                        .to_string()
                                } else {
                                    "move the outer continuation into the selected outcome arm, or end every other outcome in a catalog node with an execution input and no execution output"
                                        .to_string()
                                });
                                context.diagnostics.push(diagnostic);
                            }
                        }
                    }

                    if has_local_tail
                        && let Some(continuation) = continuation
                        && exec_outputs
                            .iter()
                            .any(|pin| pin.name.eq_ignore_ascii_case(continuation))
                    {
                        let continuation_key = normalize_symbol(continuation);
                        // Data outputs authored inside the selected execution arm are in scope
                        // for the following tail because that tail is lowered into this arm.
                        let mut tail_context = compiled_arm_contexts
                            .remove(&continuation_key)
                            .unwrap_or_else(|| context.clone());
                        tail_context.diagnostics.clear();
                        let tail = compile_steps_with_offset(
                            &steps[index + 1..],
                            returns,
                            path,
                            &mut tail_context,
                            outer_continuation,
                            index + 1 + index_offset,
                        );
                        context.diagnostics.append(&mut tail_context.diagnostics);
                        compiled_arms
                            .entry(continuation_key)
                            .or_default()
                            .stmts
                            .extend(tail.stmts);
                    }
                    let arms = exec_outputs
                        .iter()
                        .map(|pin| BranchArm {
                            label: pin.name.clone(),
                            body: compiled_arms
                                .remove(&normalize_symbol(&pin.name))
                                .unwrap_or_default(),
                        })
                        .collect();
                    statements.push(Stmt::Branch {
                        bind: Some(id.clone()),
                        call,
                        condition: None,
                        arms,
                        anchor: anchor.clone(),
                    });
                    if has_local_tail {
                        break;
                    }
                    index += 1;
                    continue;
                }
                if let Some(continuation) = continue_from
                    && exec_outputs.len() <= 1
                {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_EXEC_CONTINUATION_REDUNDANT",
                        format!("{step_path}/continue_from"),
                        Some(context.scope),
                        "continue_from is only valid for a node with multiple execution outputs",
                    );
                    diagnostic.pin = Some(continuation.clone());
                    context.diagnostics.push(diagnostic);
                }
                if !exec_arms.is_empty() {
                    context.diagnostics.push(FlowIrDiagnostic::new(
                        "IR_EXEC_ARMS_REDUNDANT",
                        format!("{step_path}/exec_arms"),
                        Some(context.scope),
                        "exec_arms is only valid for a node with multiple execution outputs",
                    ));
                }
                statements.push(Stmt::Let {
                    name: id.clone(),
                    call,
                    anchor: anchor.clone(),
                });
            }
            FlowIrStep::CallFunction {
                id,
                function,
                args,
                anchor,
            } => {
                let key = normalize_symbol(function);
                let Some(signature) = context.functions.get(&key).cloned() else {
                    context.diagnostics.push(FlowIrDiagnostic::new(
                        "IR_FUNCTION_MISSING",
                        format!("{step_path}/function"),
                        Some(context.scope),
                        format!("function {function:?} is not declared in this program"),
                    ));
                    index += 1;
                    continue;
                };
                let call =
                    compile_function_call(id, function, args, &signature, &step_path, context);
                statements.push(Stmt::Let {
                    name: id.clone(),
                    call,
                    anchor: anchor.clone(),
                });
            }
            FlowIrStep::If {
                id,
                condition,
                then_steps,
                else_steps,
                anchor,
            } => {
                let condition_source =
                    compile_value(condition, &format!("{step_path}/condition"), context);
                if !types_compatible(
                    &condition_source.value_type,
                    &FlowIrType::scalar(FlowIrDataType::Boolean),
                ) {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_BRANCH_CONDITION_TYPE",
                        format!("{step_path}/condition"),
                        Some(context.scope),
                        "if condition must be a boolean value",
                    );
                    diagnostic.expected = Some("boolean".to_string());
                    diagnostic.actual = Some(type_label(&condition_source.value_type));
                    context.diagnostics.push(diagnostic);
                }
                let Some(branch_metadata) = resolve_builtin(
                    context.catalog,
                    &["control_branch", "controlBranch", "branch"],
                    "condition",
                ) else {
                    context.diagnostics.push(FlowIrDiagnostic::new(
                        "IR_BRANCH_DECLARATION_MISSING",
                        step_path.clone(),
                        Some(context.scope),
                        "the live catalog has no canonical boolean Branch declaration",
                    ));
                    index += 1;
                    continue;
                };
                register_step_outputs(id, branch_metadata, &step_path, context);
                let condition_pin = branch_metadata
                    .inputs
                    .iter()
                    .find(|pin| {
                        !pin.data_type.eq_ignore_ascii_case("Execution")
                            && (pin.name.eq_ignore_ascii_case("condition")
                                || pin.data_type.eq_ignore_ascii_case("Boolean"))
                    })
                    .map(|pin| pin.name.clone())
                    .unwrap_or_else(|| "condition".to_string());
                let call = Call {
                    node_type: branch_metadata.name.clone(),
                    display: flow_like_ast::to_camel_case(&branch_metadata.name),
                    path: Vec::new(),
                    receiver: None,
                    positional: Vec::new(),
                    args: vec![Arg {
                        name: condition_pin,
                        value: condition_source.expression.clone(),
                    }],
                    anchor: anchor.clone(),
                };
                let mut then_context = context.clone();
                then_context.diagnostics.clear();
                let then_body = compile_steps(
                    then_steps,
                    returns,
                    &format!("{step_path}/then_steps"),
                    &mut then_context,
                    index + 1 < steps.len() || outer_continuation,
                );
                let mut else_context = context.clone();
                else_context.diagnostics.clear();
                let else_body = compile_steps(
                    else_steps,
                    returns,
                    &format!("{step_path}/else_steps"),
                    &mut else_context,
                    index + 1 < steps.len() || outer_continuation,
                );
                context.diagnostics.extend(then_context.diagnostics);
                context.diagnostics.extend(else_context.diagnostics);
                statements.push(Stmt::Branch {
                    bind: Some(id.clone()),
                    call,
                    condition: Some(condition_source.expression),
                    arms: vec![
                        BranchArm {
                            label: "true".to_string(),
                            body: then_body,
                        },
                        BranchArm {
                            label: "false".to_string(),
                            body: else_body,
                        },
                    ],
                    anchor: anchor.clone(),
                });
            }
            FlowIrStep::ForEach {
                id,
                array,
                item,
                index: index_alias,
                parallel,
                steps: body_steps,
                anchor,
            } => {
                let array_source = compile_value(array, &format!("{step_path}/array"), context);
                if array_source.value_type.container != FlowIrContainer::Array {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_LOOP_ARRAY_TYPE",
                        format!("{step_path}/array"),
                        Some(context.scope),
                        "for_each input must be an array",
                    );
                    diagnostic.expected = Some("array".to_string());
                    diagnostic.actual = Some(type_label(&array_source.value_type));
                    context.diagnostics.push(diagnostic);
                }
                let candidates = if *parallel {
                    [
                        "control_par_for_each",
                        "controlParForEach",
                        "forEachParallel",
                    ]
                } else {
                    ["control_for_each", "controlForEach", "forEach"]
                };
                let Some(loop_metadata) = resolve_builtin(context.catalog, &candidates, "array")
                else {
                    context.diagnostics.push(FlowIrDiagnostic::new(
                        "IR_LOOP_DECLARATION_MISSING",
                        step_path.clone(),
                        Some(context.scope),
                        "the live catalog has no matching for-each declaration",
                    ));
                    index += 1;
                    continue;
                };
                register_step_outputs(id, loop_metadata, &step_path, context);
                let array_pin = loop_metadata
                    .inputs
                    .iter()
                    .find(|pin| {
                        !pin.data_type.eq_ignore_ascii_case("Execution")
                            && (pin.name.eq_ignore_ascii_case("array")
                                || pin.value_type.eq_ignore_ascii_case("Array"))
                    })
                    .map(|pin| pin.name.clone())
                    .unwrap_or_else(|| "array".to_string());
                let mut body_context = context.clone();
                body_context.diagnostics.clear();
                if !flow_like_ast::is_valid_identifier(item)
                    || body_context.symbols.contains_key(&normalize_symbol(item))
                {
                    context.diagnostics.push(FlowIrDiagnostic::new(
                        "IR_LOOP_ITEM_NAME_INVALID",
                        format!("{step_path}/item"),
                        Some(context.scope),
                        "loop item must be a valid identifier that does not shadow a parameter or variable",
                    ));
                }
                if let Some(index_alias) = index_alias
                    && (!flow_like_ast::is_valid_identifier(index_alias)
                        || normalize_symbol(index_alias) == normalize_symbol(item)
                        || body_context
                            .symbols
                            .contains_key(&normalize_symbol(index_alias)))
                {
                    context.diagnostics.push(FlowIrDiagnostic::new(
                        "IR_LOOP_INDEX_NAME_INVALID",
                        format!("{step_path}/index"),
                        Some(context.scope),
                        "loop index must be a valid, distinct identifier that does not shadow a parameter or variable",
                    ));
                }
                let element_type = FlowIrType {
                    data_type: array_source.value_type.data_type,
                    container: FlowIrContainer::Normal,
                    interface: array_source.value_type.interface.clone(),
                };
                body_context.symbols.insert(
                    normalize_symbol(item),
                    ValueSource {
                        value_type: element_type,
                        expression: Expr::Field {
                            base: Box::new(Expr::Ref(id.clone())),
                            pin: "value".to_string(),
                        },
                    },
                );
                if let Some(index_alias) = index_alias {
                    body_context.symbols.insert(
                        normalize_symbol(index_alias),
                        ValueSource {
                            value_type: FlowIrType::scalar(FlowIrDataType::Integer),
                            expression: Expr::Field {
                                base: Box::new(Expr::Ref(id.clone())),
                                pin: "index".to_string(),
                            },
                        },
                    );
                }
                let body = compile_steps(
                    body_steps,
                    returns,
                    &format!("{step_path}/steps"),
                    &mut body_context,
                    true,
                );
                context.diagnostics.extend(body_context.diagnostics);
                statements.push(Stmt::Loop {
                    keyword: if *parallel {
                        "forEachParallel".to_string()
                    } else {
                        "forEach".to_string()
                    },
                    bind: Some(id.clone()),
                    iterable: None,
                    element: None,
                    index: None,
                    call: Call {
                        node_type: loop_metadata.name.clone(),
                        display: flow_like_ast::to_camel_case(&loop_metadata.name),
                        path: Vec::new(),
                        receiver: None,
                        positional: Vec::new(),
                        args: vec![Arg {
                            name: array_pin,
                            value: array_source.expression,
                        }],
                        anchor: anchor.clone(),
                    },
                    body,
                    anchor: anchor.clone(),
                });
            }
            FlowIrStep::Assign { target, value } => {
                let source = compile_value(value, &format!("{step_path}/value"), context);
                if let Some(expected) = context.symbols.get(&normalize_symbol(target)) {
                    if !authored_value_compatible(&source.value_type, &expected.value_type, value) {
                        let mut diagnostic = FlowIrDiagnostic::new(
                            "IR_ASSIGN_TYPE",
                            format!("{step_path}/value"),
                            Some(context.scope),
                            format!("assignment to {target:?} has the wrong type"),
                        );
                        diagnostic.expected = Some(type_label(&expected.value_type));
                        diagnostic.actual = Some(type_label(&source.value_type));
                        context.diagnostics.push(diagnostic);
                    }
                } else {
                    context.diagnostics.push(FlowIrDiagnostic::new(
                        "IR_ASSIGN_TARGET_MISSING",
                        format!("{step_path}/target"),
                        Some(context.scope),
                        format!("assignment target {target:?} is not a parameter or variable"),
                    ));
                }
                statements.push(Stmt::Assign {
                    target: target.clone(),
                    value: source.expression,
                    anchor: None,
                });
            }
            FlowIrStep::Return { values } => {
                let invalid_arity = if context.event_returns_terminate {
                    values.len() > 1
                } else {
                    values.len() != returns.len()
                };
                if invalid_arity {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_RETURN_ARITY",
                        format!("{step_path}/values"),
                        Some(context.scope),
                        if context.event_returns_terminate {
                            "event return supports at most one generic result value"
                        } else {
                            "return value count does not match the function declaration"
                        },
                    );
                    diagnostic.expected = Some(if context.event_returns_terminate {
                        "0 or 1".to_string()
                    } else {
                        returns.len().to_string()
                    });
                    diagnostic.actual = Some(values.len().to_string());
                    context.diagnostics.push(diagnostic);
                }
                let mut expressions = Vec::new();
                for (value_index, value) in values.iter().enumerate() {
                    let source =
                        compile_value(value, &format!("{step_path}/values/{value_index}"), context);
                    if let Some(expected) = returns.get(value_index)
                        && !authored_value_compatible(
                            &source.value_type,
                            &expected.value_type,
                            value,
                        )
                    {
                        let mut diagnostic = FlowIrDiagnostic::new(
                            "IR_RETURN_TYPE",
                            format!("{step_path}/values/{value_index}"),
                            Some(context.scope),
                            format!("return value {} has the wrong type", value_index + 1),
                        );
                        diagnostic.expected = Some(type_label(&expected.value_type));
                        diagnostic.actual = Some(type_label(&source.value_type));
                        context.diagnostics.push(diagnostic);
                    }
                    expressions.push(source.expression);
                }
                statements.push(Stmt::Return {
                    values: expressions,
                    anchor: None,
                });
            }
        }
        index += 1;
    }
    Block { stmts: statements }
}

fn compile_catalog_call(
    id: &str,
    metadata: &NodeMetadata,
    args: &[FlowIrArg],
    path: &str,
    context: &mut ModuleContext<'_>,
) -> Call {
    let mut compiled_args = Vec::<(usize, Arg)>::new();
    let dynamic_template = dynamic_placeholder_config_pin(&metadata.name).and_then(|config_pin| {
        args.iter().find_map(|argument| {
            (argument.pin.eq_ignore_ascii_case(config_pin)
                && matches!(argument.value, FlowIrValue::Literal { .. }))
            .then_some(match &argument.value {
                FlowIrValue::Literal {
                    value: FlowIrLiteral::String(template),
                } => Some(template.as_str()),
                _ => None,
            })
            .flatten()
        })
    });
    let mut supplied = HashSet::new();
    let mut authored_occurrences = HashMap::<String, HashSet<usize>>::new();
    for argument in args {
        authored_occurrences
            .entry(normalize_symbol(&argument.pin))
            .or_default()
            .insert(argument.occurrence);
    }
    for (pin, occurrences) in &authored_occurrences {
        if let Some(maximum) = occurrences.iter().copied().max()
            && (0..=maximum).any(|occurrence| !occurrences.contains(&occurrence))
        {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_INPUT_OCCURRENCE_SPARSE",
                format!("{path}/args"),
                Some(context.scope),
                "duplicate input pin occurrences must be contiguous starting at zero",
            );
            diagnostic.pin = Some(pin.clone());
            context.diagnostics.push(diagnostic);
        }
    }
    let mut seen_arguments = HashSet::new();
    for (argument_index, argument) in args.iter().enumerate() {
        let argument_path = format!("{path}/args/{argument_index}");
        let argument_key = (normalize_symbol(&argument.pin), argument.occurrence);
        if !seen_arguments.insert(argument_key) {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_INPUT_OCCURRENCE_DUPLICATE",
                format!("{argument_path}/occurrence"),
                Some(context.scope),
                format!(
                    "input {:?} occurrence {} is supplied more than once",
                    argument.pin, argument.occurrence
                ),
            );
            diagnostic.pin = Some(argument.pin.clone());
            context.diagnostics.push(diagnostic);
            continue;
        }
        if let FlowIrValue::FunctionRefs { functions } = &argument.value {
            let normalized_pin = normalize_symbol(&argument.pin);
            let synthetic_name = match normalized_pin.as_str() {
                "tools" => Some("tools"),
                "fnrefs" => Some("fnRefs"),
                _ => None,
            };
            let tools_capable = normalized_pin != "tools"
                || normalize_symbol(&metadata.name).contains("registerfunctiontool")
                || metadata.capability_tags.iter().any(|tag| {
                    let tag = normalize_symbol(tag);
                    tag.contains("functiontool") || tag.contains("functionref")
                });
            if argument.occurrence != 0 || synthetic_name.is_none() || !tools_capable {
                let mut diagnostic = FlowIrDiagnostic::new(
                    "IR_FUNCTION_REFS_CONTEXT_INVALID",
                    format!("{argument_path}/pin"),
                    Some(context.scope),
                    "function_refs values are only valid at occurrence zero on a supported synthetic tools/fnRefs argument",
                );
                diagnostic.pin = Some(argument.pin.clone());
                context.diagnostics.push(diagnostic);
                continue;
            }
            if functions.is_empty() || functions.len() > MAX_FLOW_IR_MODULES {
                context.diagnostics.push(FlowIrDiagnostic::new(
                    "IR_FUNCTION_REFS_COUNT_INVALID",
                    format!("{argument_path}/value/functions"),
                    Some(context.scope),
                    format!(
                        "function_refs must contain 1..={MAX_FLOW_IR_MODULES} retained module targets"
                    ),
                ));
            }
            let mut canonical_targets = Vec::new();
            let mut seen_targets = HashSet::new();
            for (target_index, target) in functions.iter().enumerate() {
                let key = normalize_symbol(target);
                let Some(canonical) = context.reference_targets.get(&key) else {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_FUNCTION_REF_TARGET_MISSING",
                        format!("{argument_path}/value/functions/{target_index}"),
                        Some(context.scope),
                        format!("function/tool target {target:?} is not a retained module"),
                    );
                    diagnostic.fix = Some(
                        "add the complete target module to the same draft and expected_modules before registering it"
                            .to_string(),
                    );
                    context.diagnostics.push(diagnostic);
                    continue;
                };
                if seen_targets.insert(key) {
                    canonical_targets.push(Expr::Ref(canonical.clone()));
                }
            }
            compiled_args.push((
                usize::MAX,
                Arg {
                    name: synthetic_name
                        .expect("validated synthetic argument")
                        .to_string(),
                    value: Expr::Array(canonical_targets),
                },
            ));
            continue;
        }
        let matching = metadata
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, pin)| pin.name.eq_ignore_ascii_case(&argument.pin))
            .collect::<Vec<_>>();
        let synthesized_pin;
        let (catalog_index, pin) = match matching.get(argument.occurrence).copied() {
            Some(found) => found,
            None => {
                let predicted = (argument.occurrence == 0)
                    .then(|| {
                        dynamic_template.and_then(|template| {
                            synthesize_dynamic_input_pin_from_template(
                                metadata,
                                template,
                                &argument.pin,
                            )
                        })
                    })
                    .flatten();
                let Some(predicted) = predicted else {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_INPUT_PIN_MISSING",
                        format!("{argument_path}/pin"),
                        Some(context.scope),
                        format!(
                            "node {:?} has no input pin {:?} at occurrence {}",
                            metadata.name, argument.pin, argument.occurrence
                        ),
                    );
                    diagnostic.declaration =
                        Some(metadata_to_signature(metadata).render_declaration());
                    diagnostic.pin = Some(argument.pin.clone());
                    diagnostic.fix = Some(
                        "use one exact static input or a placeholder declared by this call's literal template config"
                            .to_string(),
                    );
                    context.diagnostics.push(diagnostic);
                    continue;
                };
                synthesized_pin = predicted;
                (usize::MAX - 1, &synthesized_pin)
            }
        };
        if pin.data_type.eq_ignore_ascii_case("Execution") {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_EXEC_PIN_AS_ARGUMENT",
                format!("{argument_path}/pin"),
                Some(context.scope),
                "execution pins are wired by step order/branches, not supplied as data arguments",
            );
            diagnostic.pin = Some(pin.name.clone());
            context.diagnostics.push(diagnostic);
            continue;
        }
        let value = compile_value(&argument.value, &format!("{argument_path}/value"), context);
        let expected = pin_type(pin);
        if expected.data_type == FlowIrDataType::Unsupported {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_CATALOG_TYPE_UNSUPPORTED",
                format!("{argument_path}/pin"),
                Some(context.scope),
                format!(
                    "catalog pin {:?}.{:?} uses unsupported data type {:?}",
                    metadata.name, pin.name, pin.data_type
                ),
            );
            diagnostic.declaration = Some(metadata_to_signature(metadata).render_declaration());
            diagnostic.pin = Some(pin.name.clone());
            context.diagnostics.push(diagnostic);
        }
        if !authored_value_compatible(&value.value_type, &expected, &argument.value) {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_INPUT_TYPE",
                format!("{argument_path}/value"),
                Some(context.scope),
                format!(
                    "value for {:?}.{:?} has the wrong type",
                    metadata.name, pin.name
                ),
            );
            diagnostic.expected = Some(type_label(&expected));
            diagnostic.actual = Some(type_label(&value.value_type));
            diagnostic.declaration = Some(metadata_to_signature(metadata).render_declaration());
            diagnostic.pin = Some(pin.name.clone());
            diagnostic.fix = Some(
                "insert an explicit catalog conversion node and reference its typed output"
                    .to_string(),
            );
            context.diagnostics.push(diagnostic);
        }
        supplied.insert((normalize_symbol(&pin.name), argument.occurrence));
        compiled_args.push((
            catalog_index,
            Arg {
                name: pin.name.clone(),
                value: value.expression,
            },
        ));
    }

    for required in &metadata.required_inputs {
        let required_key = normalize_symbol(required);
        if !supplied.iter().any(|(name, _)| name == &required_key) {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_REQUIRED_INPUT_MISSING",
                format!("{path}/args"),
                Some(context.scope),
                format!(
                    "required input {required:?} is missing on {:?}",
                    metadata.name
                ),
            );
            diagnostic.declaration = Some(metadata_to_signature(metadata).render_declaration());
            diagnostic.pin = Some(required.clone());
            context.diagnostics.push(diagnostic);
        }
    }

    register_step_outputs(id, metadata, path, context);
    compiled_args.sort_by_key(|(catalog_index, _)| *catalog_index);
    Call {
        node_type: metadata.name.clone(),
        display: flow_like_ast::to_camel_case(&metadata.name),
        path: Vec::new(),
        receiver: None,
        positional: Vec::new(),
        args: compiled_args
            .into_iter()
            .map(|(_, argument)| argument)
            .collect(),
        anchor: None,
    }
}

fn compile_function_call(
    id: &str,
    function: &str,
    args: &[FlowIrArg],
    signature: &FunctionSignature,
    path: &str,
    context: &mut ModuleContext<'_>,
) -> Call {
    let mut compiled_args = Vec::<(usize, Arg)>::new();
    let mut supplied = HashSet::new();
    let mut seen_arguments = HashSet::new();
    for (argument_index, argument) in args.iter().enumerate() {
        if argument.occurrence != 0 {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_FUNCTION_INPUT_OCCURRENCE_INVALID",
                format!("{path}/args/{argument_index}/occurrence"),
                Some(context.scope),
                "function parameters have unique names; occurrence must be zero",
            );
            diagnostic.pin = Some(argument.pin.clone());
            context.diagnostics.push(diagnostic);
            continue;
        }
        if !seen_arguments.insert(normalize_symbol(&argument.pin)) {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_FUNCTION_INPUT_DUPLICATE",
                format!("{path}/args/{argument_index}/pin"),
                Some(context.scope),
                format!(
                    "function input {:?} is supplied more than once",
                    argument.pin
                ),
            );
            diagnostic.pin = Some(argument.pin.clone());
            context.diagnostics.push(diagnostic);
            continue;
        }
        let Some((param_index, param)) = signature
            .params
            .iter()
            .enumerate()
            .find(|(_, param)| param.name.eq_ignore_ascii_case(&argument.pin))
        else {
            context.diagnostics.push(FlowIrDiagnostic::new(
                "IR_FUNCTION_INPUT_MISSING",
                format!("{path}/args/{argument_index}/pin"),
                Some(context.scope),
                format!("function {function:?} has no parameter {:?}", argument.pin),
            ));
            continue;
        };
        let source = compile_value(
            &argument.value,
            &format!("{path}/args/{argument_index}/value"),
            context,
        );
        if !authored_value_compatible(&source.value_type, &param.value_type, &argument.value) {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_FUNCTION_INPUT_TYPE",
                format!("{path}/args/{argument_index}/value"),
                Some(context.scope),
                format!(
                    "argument {:?} for {function:?} has the wrong type",
                    param.name
                ),
            );
            diagnostic.expected = Some(type_label(&param.value_type));
            diagnostic.actual = Some(type_label(&source.value_type));
            context.diagnostics.push(diagnostic);
        }
        supplied.insert(normalize_symbol(&param.name));
        compiled_args.push((
            param_index,
            Arg {
                name: param.name.clone(),
                value: source.expression,
            },
        ));
    }
    for param in &signature.params {
        if !supplied.contains(&normalize_symbol(&param.name)) {
            context.diagnostics.push(FlowIrDiagnostic::new(
                "IR_FUNCTION_REQUIRED_INPUT_MISSING",
                format!("{path}/args"),
                Some(context.scope),
                format!("function input {:?} is missing", param.name),
            ));
        }
    }
    context.step_outputs.insert(
        normalize_symbol(id),
        signature
            .returns
            .iter()
            .map(|param| {
                (
                    normalize_symbol(&param.name),
                    ValueSource {
                        value_type: param.value_type.clone(),
                        expression: Expr::Field {
                            base: Box::new(Expr::Ref(id.to_string())),
                            pin: param.name.clone(),
                        },
                    },
                )
            })
            .collect(),
    );
    compiled_args.sort_by_key(|(param_index, _)| *param_index);
    Call {
        node_type: signature.name.clone(),
        display: signature.name.clone(),
        path: Vec::new(),
        receiver: None,
        positional: Vec::new(),
        args: compiled_args
            .into_iter()
            .map(|(_, argument)| argument)
            .collect(),
        anchor: None,
    }
}

fn compile_value(value: &FlowIrValue, path: &str, context: &mut ModuleContext<'_>) -> ValueSource {
    match value {
        FlowIrValue::Literal { value } => ValueSource {
            value_type: literal_type(value),
            expression: Expr::Literal(literal_to_ast(value)),
        },
        FlowIrValue::Ref { name } => context
            .symbols
            .get(&normalize_symbol(name))
            .cloned()
            .unwrap_or_else(|| {
                context.diagnostics.push(FlowIrDiagnostic::new(
                    "IR_REFERENCE_MISSING",
                    path,
                    Some(context.scope),
                    format!("reference {name:?} is not a parameter, variable, or loop alias"),
                ));
                unknown_source(name)
            }),
        FlowIrValue::Output {
            step,
            pin,
            occurrence,
        } => {
            let key = (normalize_symbol(step), normalize_symbol(pin));
            if *occurrence != 0 || context.ambiguous_step_outputs.contains(&key) {
                let mut diagnostic = FlowIrDiagnostic::new(
                    "IR_OUTPUT_OCCURRENCE_UNSUPPORTED",
                    path,
                    Some(context.scope),
                    format!(
                        "output {step:?}.{pin:?} is ambiguous; duplicate output pin identity is not representable safely yet"
                    ),
                );
                diagnostic.pin = Some(pin.clone());
                diagnostic.fix = Some(
                    "use a declaration with unique output names or insert a catalog adapter"
                        .to_string(),
                );
                context.diagnostics.push(diagnostic);
                return unknown_source(step);
            }
            context
                .step_outputs
                .get(&key.0)
                .and_then(|outputs| outputs.get(&key.1))
                .cloned()
                .unwrap_or_else(|| {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_OUTPUT_REFERENCE_MISSING",
                        path,
                        Some(context.scope),
                        format!("step {step:?} has no previously declared data output {pin:?}"),
                    );
                    diagnostic.pin = Some(pin.clone());
                    diagnostic.fix = Some(
                    "reference an earlier step id and an exact data output pin from its declaration"
                        .to_string(),
                );
                    context.diagnostics.push(diagnostic);
                    unknown_source(step)
                })
        }
        FlowIrValue::FunctionRefs { .. } => {
            context.diagnostics.push(FlowIrDiagnostic::new(
                "IR_FUNCTION_REFS_CONTEXT_INVALID",
                path,
                Some(context.scope),
                "function_refs is only valid as the complete value of a synthetic tools/fnRefs node argument",
            ));
            unknown_source("functionRefs")
        }
        FlowIrValue::List { items } => {
            if let Some(dynamic_path) = first_dynamic_composite_path(value, path) {
                let mut diagnostic = FlowIrDiagnostic::new(
                    "IR_DYNAMIC_COMPOSITE_UNSUPPORTED",
                    dynamic_path,
                    Some(context.scope),
                    "dynamic list/object leaves cannot yet be materialized by reconciliation",
                );
                diagnostic.fix = Some(
                    "use a JSON literal when every leaf is constant, or build the value with explicit catalog array/struct construction nodes"
                        .to_string(),
                );
                context.diagnostics.push(diagnostic);
                return unknown_source("dynamicComposite");
            }
            let mut expressions = Vec::new();
            let mut item_type: Option<FlowIrType> = None;
            for (index, item) in items.iter().enumerate() {
                let source = compile_value(item, &format!("{path}/items/{index}"), context);
                if let Some(expected) = item_type.as_ref()
                    && !types_compatible(&source.value_type, expected)
                    && !types_compatible(expected, &source.value_type)
                {
                    let mut diagnostic = FlowIrDiagnostic::new(
                        "IR_LIST_ITEM_TYPE",
                        format!("{path}/items/{index}"),
                        Some(context.scope),
                        "list items must have compatible element types",
                    );
                    diagnostic.expected = Some(type_label(expected));
                    diagnostic.actual = Some(type_label(&source.value_type));
                    context.diagnostics.push(diagnostic);
                    item_type = Some(FlowIrType::scalar(FlowIrDataType::Generic));
                } else if item_type
                    .as_ref()
                    .is_some_and(|current| current.data_type == FlowIrDataType::Integer)
                    && source.value_type.data_type == FlowIrDataType::Float
                {
                    item_type = Some(FlowIrType::scalar(FlowIrDataType::Float));
                } else if item_type.is_none() {
                    item_type = Some(source.value_type.clone());
                }
                expressions.push(source.expression);
            }
            let mut value_type =
                item_type.unwrap_or_else(|| FlowIrType::scalar(FlowIrDataType::Generic));
            value_type.container = FlowIrContainer::Array;
            ValueSource {
                value_type,
                expression: Expr::Array(expressions),
            }
        }
        FlowIrValue::Object { fields } => {
            if let Some(dynamic_path) = first_dynamic_composite_path(value, path) {
                let mut diagnostic = FlowIrDiagnostic::new(
                    "IR_DYNAMIC_COMPOSITE_UNSUPPORTED",
                    dynamic_path,
                    Some(context.scope),
                    "dynamic list/object leaves cannot yet be materialized by reconciliation",
                );
                diagnostic.fix = Some(
                    "use a JSON literal when every leaf is constant, or build the value with explicit catalog array/struct construction nodes"
                        .to_string(),
                );
                context.diagnostics.push(diagnostic);
                return unknown_source("dynamicComposite");
            }
            ValueSource {
                value_type: FlowIrType::scalar(FlowIrDataType::Struct),
                expression: Expr::Object(
                    fields
                        .iter()
                        .enumerate()
                        .map(|(index, field)| ObjectField {
                            key: field.key.clone(),
                            value: compile_value(
                                &field.value,
                                &format!("{path}/fields/{index}/value"),
                                context,
                            )
                            .expression,
                        })
                        .collect(),
                ),
            }
        }
    }
}

fn first_dynamic_composite_path(value: &FlowIrValue, path: &str) -> Option<String> {
    match value {
        FlowIrValue::Literal { .. } => None,
        FlowIrValue::Ref { .. } | FlowIrValue::Output { .. } | FlowIrValue::FunctionRefs { .. } => {
            Some(path.to_string())
        }
        FlowIrValue::List { items } => items.iter().enumerate().find_map(|(index, item)| {
            first_dynamic_composite_path(item, &format!("{path}/items/{index}"))
        }),
        FlowIrValue::Object { fields } => fields.iter().enumerate().find_map(|(index, field)| {
            first_dynamic_composite_path(&field.value, &format!("{path}/fields/{index}/value"))
        }),
    }
}

fn register_step_outputs(
    id: &str,
    metadata: &NodeMetadata,
    _path: &str,
    context: &mut ModuleContext<'_>,
) {
    let key = normalize_symbol(id);
    if !flow_like_ast::is_valid_identifier(id) || context.step_outputs.contains_key(&key) {
        return;
    }
    let mut outputs = HashMap::new();
    let mut output_names = HashSet::new();
    for pin in metadata
        .outputs
        .iter()
        .filter(|pin| !pin.data_type.eq_ignore_ascii_case("Execution"))
    {
        let output_key = normalize_symbol(&pin.name);
        if !output_names.insert(output_key.clone()) {
            context
                .ambiguous_step_outputs
                .insert((key.clone(), output_key.clone()));
        }
        outputs.entry(output_key).or_insert_with(|| ValueSource {
            value_type: pin_type(pin),
            expression: Expr::Field {
                base: Box::new(Expr::Ref(id.to_string())),
                pin: pin.name.clone(),
            },
        });
    }
    context.step_outputs.insert(key, outputs);
}

fn resolve_catalog_node<'a>(
    requested: &str,
    catalog: &'a HashMap<String, &'a NodeMetadata>,
) -> Option<&'a NodeMetadata> {
    catalog
        .get(&normalize_symbol(requested))
        .copied()
        .or_else(|| {
            catalog.values().copied().find(|metadata| {
                normalize_symbol(&flow_like_ast::to_camel_case(&metadata.name))
                    == normalize_symbol(requested)
            })
        })
}

fn resolve_builtin<'a>(
    catalog: &'a HashMap<String, &'a NodeMetadata>,
    names: &[&str],
    required_input: &str,
) -> Option<&'a NodeMetadata> {
    names
        .iter()
        .find_map(|name| resolve_catalog_node(name, catalog))
        .or_else(|| {
            catalog.values().copied().find(|metadata| {
                metadata
                    .inputs
                    .iter()
                    .any(|pin| pin.name.eq_ignore_ascii_case(required_input))
                    && names.iter().any(|name| {
                        normalize_symbol(&metadata.name).contains(&normalize_symbol(name))
                    })
            })
        })
}

fn normalize_symbol(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn count_steps(steps: &[FlowIrStep]) -> usize {
    steps
        .iter()
        .map(|step| match step {
            FlowIrStep::Node { exec_arms, .. } => {
                1 + exec_arms
                    .iter()
                    .map(|arm| count_steps(&arm.steps))
                    .sum::<usize>()
            }
            FlowIrStep::CallFunction { .. } => 1,
            FlowIrStep::If {
                then_steps,
                else_steps,
                ..
            } => 1 + count_steps(then_steps) + count_steps(else_steps),
            FlowIrStep::ForEach { steps, .. } => 1 + count_steps(steps),
            FlowIrStep::Assign { .. } | FlowIrStep::Return { .. } => 1,
        })
        .sum()
}

fn count_materialized_steps(
    steps: &[FlowIrStep],
    global_variables: &HashSet<String>,
    event: bool,
) -> usize {
    steps
        .iter()
        .map(|step| match step {
            FlowIrStep::Node { exec_arms, .. } => {
                1 + exec_arms
                    .iter()
                    .map(|arm| count_materialized_steps(&arm.steps, global_variables, event))
                    .sum::<usize>()
            }
            FlowIrStep::CallFunction { .. } => 1,
            FlowIrStep::If {
                then_steps,
                else_steps,
                ..
            } => {
                1 + count_materialized_steps(then_steps, global_variables, event)
                    + count_materialized_steps(else_steps, global_variables, event)
            }
            FlowIrStep::ForEach { steps, .. } => {
                1 + count_materialized_steps(steps, global_variables, event)
            }
            FlowIrStep::Assign { target, .. } => {
                usize::from(global_variables.contains(&normalize_symbol(target)))
            }
            FlowIrStep::Return { .. } => usize::from(event),
        })
        .sum()
}

fn steps_always_return(steps: &[FlowIrStep], catalog: &HashMap<String, &NodeMetadata>) -> bool {
    let Some((step, remaining)) = steps.split_first() else {
        return false;
    };
    match step {
        FlowIrStep::Return { .. } => true,
        FlowIrStep::If {
            then_steps,
            else_steps,
            ..
        } => {
            let remaining_returns = steps_always_return(remaining, catalog);
            (remaining_returns || steps_always_return(then_steps, catalog))
                && (remaining_returns || steps_always_return(else_steps, catalog))
        }
        FlowIrStep::Node {
            node_type,
            continue_from,
            exec_arms,
            ..
        } => {
            let Some(metadata) = resolve_catalog_node(node_type, catalog) else {
                return false;
            };
            let exec_outputs = metadata
                .outputs
                .iter()
                .filter(|pin| pin.data_type.eq_ignore_ascii_case("Execution"))
                .collect::<Vec<_>>();
            if exec_outputs.len() <= 1 {
                return steps_always_return(remaining, catalog);
            }
            let remaining_returns = steps_always_return(remaining, catalog);
            exec_outputs.iter().all(|pin| {
                let arm_returns = exec_arms
                    .iter()
                    .find(|arm| arm.pin.eq_ignore_ascii_case(&pin.name))
                    .is_some_and(|arm| steps_always_return(&arm.steps, catalog));
                arm_returns
                    || (continue_from
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(&pin.name))
                        && remaining_returns)
            })
        }
        FlowIrStep::CallFunction { .. }
        | FlowIrStep::ForEach { .. }
        | FlowIrStep::Assign { .. } => steps_always_return(remaining, catalog),
    }
}

/// Whether a block consumes its incoming execution cursor without exposing a cursor that the
/// enclosing FlowScript branch can fan into a later statement. This intentionally differs from
/// `steps_always_return`: a function `return` wires boundary data but reconciliation does not
/// model it as an early execution exit, while an Event return materializes a terminal result node.
fn steps_prevent_fallthrough(
    steps: &[FlowIrStep],
    catalog: &HashMap<String, &NodeMetadata>,
    event_returns_terminate: bool,
) -> bool {
    let Some((step, remaining)) = steps.split_first() else {
        return false;
    };
    let remaining_terminates =
        || steps_prevent_fallthrough(remaining, catalog, event_returns_terminate);
    match step {
        FlowIrStep::Return { .. } => event_returns_terminate || remaining_terminates(),
        FlowIrStep::If {
            then_steps,
            else_steps,
            ..
        } => {
            remaining_terminates()
                || (steps_prevent_fallthrough(then_steps, catalog, event_returns_terminate)
                    && steps_prevent_fallthrough(else_steps, catalog, event_returns_terminate))
        }
        FlowIrStep::Node {
            node_type,
            continue_from,
            exec_arms,
            ..
        } => {
            let Some(metadata) = resolve_catalog_node(node_type, catalog) else {
                return false;
            };
            let accepts_execution = metadata
                .inputs
                .iter()
                .any(|pin| pin.data_type.eq_ignore_ascii_case("Execution"));
            let exec_outputs = metadata
                .outputs
                .iter()
                .filter(|pin| pin.data_type.eq_ignore_ascii_case("Execution"))
                .collect::<Vec<_>>();
            if accepts_execution && exec_outputs.is_empty() {
                return true;
            }
            if exec_outputs.len() <= 1 {
                return remaining_terminates();
            }
            let remaining_terminates = remaining_terminates();
            exec_outputs.iter().all(|pin| {
                exec_arms
                    .iter()
                    .find(|arm| arm.pin.eq_ignore_ascii_case(&pin.name))
                    .is_some_and(|arm| {
                        steps_prevent_fallthrough(&arm.steps, catalog, event_returns_terminate)
                    })
                    || (continue_from
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(&pin.name))
                        && remaining_terminates)
            })
        }
        FlowIrStep::CallFunction { .. }
        | FlowIrStep::ForEach { .. }
        | FlowIrStep::Assign { .. } => remaining_terminates(),
    }
}

/// Reconciliation currently wires function return values to layer boundary pins but does not
/// materialize an execution-terminating return node. Until that representation exists, accepting
/// an early/nested function return would make later side effects run and could wire competing
/// values. Keep the supported subset explicit: one unconditional final top-level return.
fn validate_return_placement(
    module: &FlowIrModule,
    path: &str,
    diagnostics: &mut Vec<FlowIrDiagnostic>,
) {
    let FlowIrModule::Function { steps, .. } = module else {
        return;
    };
    for (index, step) in steps.iter().enumerate() {
        let step_path = format!("{path}/{index}");
        if matches!(step, FlowIrStep::Return { .. }) && index + 1 != steps.len() {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_FUNCTION_RETURN_POSITION",
                step_path.clone(),
                Some(module.name()),
                "function return must be the final unconditional top-level step",
            ));
        }
        reject_nested_function_returns(step, &step_path, module.name(), diagnostics);
    }
}

fn reject_nested_function_returns(
    step: &FlowIrStep,
    path: &str,
    scope: &str,
    diagnostics: &mut Vec<FlowIrDiagnostic>,
) {
    let mut children = Vec::<(&FlowIrStep, String)>::new();
    match step {
        FlowIrStep::Node { exec_arms, .. } => {
            for (arm_index, arm) in exec_arms.iter().enumerate() {
                children.extend(arm.steps.iter().enumerate().map(|(index, child)| {
                    (child, format!("{path}/exec_arms/{arm_index}/steps/{index}"))
                }));
            }
        }
        FlowIrStep::If {
            then_steps,
            else_steps,
            ..
        } => {
            children.extend(
                then_steps
                    .iter()
                    .enumerate()
                    .map(|(index, child)| (child, format!("{path}/then_steps/{index}"))),
            );
            children.extend(
                else_steps
                    .iter()
                    .enumerate()
                    .map(|(index, child)| (child, format!("{path}/else_steps/{index}"))),
            );
        }
        FlowIrStep::ForEach { steps, .. } => {
            children.extend(
                steps
                    .iter()
                    .enumerate()
                    .map(|(index, child)| (child, format!("{path}/steps/{index}"))),
            );
        }
        FlowIrStep::CallFunction { .. } | FlowIrStep::Assign { .. } | FlowIrStep::Return { .. } => {
        }
    }
    for (child, child_path) in children {
        if matches!(child, FlowIrStep::Return { .. }) {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_FUNCTION_RETURN_POSITION",
                child_path.clone(),
                Some(scope),
                "nested function returns are unsupported because reconciliation cannot terminate that execution branch",
            ));
        }
        reject_nested_function_returns(child, &child_path, scope, diagnostics);
    }
}

fn validate_unreachable_steps(
    steps: &[FlowIrStep],
    path: &str,
    scope: &str,
    catalog: &HashMap<String, &NodeMetadata>,
    event_returns_terminate: bool,
    diagnostics: &mut Vec<FlowIrDiagnostic>,
) {
    let mut terminated = false;
    for (index, step) in steps.iter().enumerate() {
        let step_path = format!("{path}/{index}");
        if terminated {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_UNREACHABLE_STEP",
                step_path,
                Some(scope),
                "step is unreachable because every preceding path already returned",
            ));
            continue;
        }
        match step {
            FlowIrStep::Node { exec_arms, .. } => {
                for (arm_index, arm) in exec_arms.iter().enumerate() {
                    validate_unreachable_steps(
                        &arm.steps,
                        &format!("{step_path}/exec_arms/{arm_index}/steps"),
                        scope,
                        catalog,
                        event_returns_terminate,
                        diagnostics,
                    );
                }
            }
            FlowIrStep::If {
                then_steps,
                else_steps,
                ..
            } => {
                validate_unreachable_steps(
                    then_steps,
                    &format!("{step_path}/then_steps"),
                    scope,
                    catalog,
                    event_returns_terminate,
                    diagnostics,
                );
                validate_unreachable_steps(
                    else_steps,
                    &format!("{step_path}/else_steps"),
                    scope,
                    catalog,
                    event_returns_terminate,
                    diagnostics,
                );
            }
            FlowIrStep::ForEach { steps, .. } => validate_unreachable_steps(
                steps,
                &format!("{step_path}/steps"),
                scope,
                catalog,
                event_returns_terminate,
                diagnostics,
            ),
            FlowIrStep::CallFunction { .. }
            | FlowIrStep::Assign { .. }
            | FlowIrStep::Return { .. } => {}
        }
        terminated =
            steps_prevent_fallthrough(std::slice::from_ref(step), catalog, event_returns_terminate);
    }
}

fn pin_type(pin: &PinMetadata) -> FlowIrType {
    FlowIrType {
        data_type: data_type_from_label(&pin.data_type),
        container: container_from_label(&pin.value_type),
        interface: None,
    }
}

fn data_type_from_label(label: &str) -> FlowIrDataType {
    match label.to_ascii_lowercase().as_str() {
        "string" => FlowIrDataType::String,
        "integer" | "int" => FlowIrDataType::Integer,
        "float" | "double" | "number" => FlowIrDataType::Float,
        "boolean" | "bool" => FlowIrDataType::Boolean,
        "struct" | "object" => FlowIrDataType::Struct,
        "generic" | "any" => FlowIrDataType::Generic,
        "date" => FlowIrDataType::Date,
        "path" | "pathbuf" => FlowIrDataType::Path,
        "byte" | "bytes" => FlowIrDataType::Bytes,
        _ => FlowIrDataType::Unsupported,
    }
}

fn container_from_label(label: &str) -> FlowIrContainer {
    match label.to_ascii_lowercase().as_str() {
        "array" => FlowIrContainer::Array,
        "hashmap" | "map" => FlowIrContainer::Map,
        "hashset" | "set" => FlowIrContainer::Set,
        _ => FlowIrContainer::Normal,
    }
}

fn types_compatible(actual: &FlowIrType, expected: &FlowIrType) -> bool {
    actual.container == expected.container
        && actual.data_type != FlowIrDataType::Unsupported
        && expected.data_type != FlowIrDataType::Unsupported
        && (actual.data_type == expected.data_type
            || expected.data_type == FlowIrDataType::Generic
            || (actual.data_type == FlowIrDataType::Integer
                && expected.data_type == FlowIrDataType::Float))
        && (expected.data_type != FlowIrDataType::Struct
            || expected.interface.is_none()
            || actual.interface == expected.interface)
}

fn authored_value_compatible(
    actual: &FlowIrType,
    expected: &FlowIrType,
    authored: &FlowIrValue,
) -> bool {
    types_compatible(actual, expected)
        || (matches!(authored, FlowIrValue::List { items } if items.is_empty())
            && actual.container == FlowIrContainer::Array
            && expected.container == FlowIrContainer::Array)
}

fn type_label(value_type: &FlowIrType) -> String {
    let base = match value_type.data_type {
        FlowIrDataType::String => "string".to_string(),
        FlowIrDataType::Integer => "integer".to_string(),
        FlowIrDataType::Float => "float".to_string(),
        FlowIrDataType::Boolean => "boolean".to_string(),
        FlowIrDataType::Struct => value_type
            .interface
            .clone()
            .unwrap_or_else(|| "struct".to_string()),
        FlowIrDataType::Generic => "generic".to_string(),
        FlowIrDataType::Date => "date".to_string(),
        FlowIrDataType::Path => "path".to_string(),
        FlowIrDataType::Bytes => "bytes".to_string(),
        FlowIrDataType::Unsupported => "unsupported".to_string(),
    };
    match value_type.container {
        FlowIrContainer::Normal => base,
        FlowIrContainer::Array => format!("{base}[]"),
        FlowIrContainer::Map => format!("map<{base}>"),
        FlowIrContainer::Set => format!("set<{base}>"),
    }
}

fn literal_type(literal: &FlowIrLiteral) -> FlowIrType {
    FlowIrType::scalar(match literal {
        FlowIrLiteral::String(_) => FlowIrDataType::String,
        FlowIrLiteral::Integer(_) => FlowIrDataType::Integer,
        FlowIrLiteral::Float(_) => FlowIrDataType::Float,
        FlowIrLiteral::Boolean(_) => FlowIrDataType::Boolean,
        FlowIrLiteral::Null => FlowIrDataType::Generic,
        FlowIrLiteral::Json(value) if value.is_array() => {
            return FlowIrType {
                data_type: FlowIrDataType::Generic,
                container: FlowIrContainer::Array,
                interface: None,
            };
        }
        FlowIrLiteral::Json(_) => FlowIrDataType::Struct,
    })
}

fn literal_to_ast(literal: &FlowIrLiteral) -> Literal {
    match literal {
        FlowIrLiteral::String(value) => Literal::String(value.clone()),
        FlowIrLiteral::Integer(value) => Literal::Int(*value),
        FlowIrLiteral::Float(value) => Literal::Float(*value),
        FlowIrLiteral::Boolean(value) => Literal::Bool(*value),
        FlowIrLiteral::Null => Literal::Null,
        FlowIrLiteral::Json(value) => Literal::Json(value.to_string()),
    }
}

fn unknown_source(name: &str) -> ValueSource {
    ValueSource {
        value_type: FlowIrType::scalar(FlowIrDataType::Generic),
        expression: Expr::Ref(name.to_string()),
    }
}

fn param_to_ast(param: &FlowIrParam) -> Param {
    Param {
        name: param.name.clone(),
        ty: type_to_ast(&param.value_type),
    }
}

fn type_to_ast(value_type: &FlowIrType) -> TypeRef {
    TypeRef::new(
        match value_type.data_type {
            FlowIrDataType::String => "string",
            FlowIrDataType::Integer => "int",
            FlowIrDataType::Float => "float",
            FlowIrDataType::Boolean => "bool",
            FlowIrDataType::Struct => value_type.interface.as_deref().unwrap_or("Struct"),
            FlowIrDataType::Generic => "any",
            FlowIrDataType::Date => "Date",
            FlowIrDataType::Path => "Path",
            FlowIrDataType::Bytes => "bytes",
            FlowIrDataType::Unsupported => "any",
        },
        match value_type.container {
            FlowIrContainer::Normal => Container::Normal,
            FlowIrContainer::Array => Container::Array,
            FlowIrContainer::Map => Container::Map,
            FlowIrContainer::Set => Container::Set,
        },
    )
}

fn variable_to_ast(variable: &FlowIrVariable) -> VarDecl {
    VarDecl {
        name: variable.name.clone(),
        ty: type_to_ast(&variable.value_type),
        default: variable.default.as_ref().map(literal_to_ast),
        exposed: variable.exposed,
        secret: variable.secret,
        editable: variable.editable,
        runtime_configured: variable.runtime_configured,
        category: variable.category.clone(),
        description: variable.description.clone(),
        schema: None,
        anchor: variable.anchor.clone(),
    }
}

fn interface_to_ast(interface: &FlowIrInterface) -> InterfaceDecl {
    use flow_like_ast::model::{InterfaceField, InterfaceType};
    InterfaceDecl {
        name: interface.name.clone(),
        fields: interface
            .fields
            .iter()
            .map(|field| {
                let mut element_type = field.value_type.clone();
                element_type.container = FlowIrContainer::Normal;
                // `type_label` is the diagnostic/IR spelling (for example `date`), while
                // FlowScript's built-in temporal type is case-sensitive and spelled `Date`.
                let label = if element_type.data_type == FlowIrDataType::Date {
                    "Date".to_string()
                } else {
                    type_label(&element_type)
                };
                let element = InterfaceType::Named(if label == "struct" {
                    "Struct".to_string()
                } else {
                    label
                });
                InterfaceField {
                    name: field.name.clone(),
                    ty: match field.value_type.container {
                        FlowIrContainer::Normal => element,
                        FlowIrContainer::Array => InterfaceType::Array(Box::new(element)),
                        FlowIrContainer::Map => InterfaceType::Map(Box::new(element)),
                        // Rejected by compile validation because FlowScript interface syntax has
                        // no set field form. Keep rendering total for diagnostic responses.
                        FlowIrContainer::Set => InterfaceType::Array(Box::new(element)),
                    },
                    optional: field.optional,
                    default: field.default.as_ref().map(literal_to_ast),
                }
            })
            .collect(),
        schema: None,
    }
}

// -------------------------------------------------------------------------------------------------
// Capability feasibility planning

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowCapabilityPlanRequest {
    #[serde(default)]
    pub requirements: Vec<FlowCapabilityRequirement>,
    #[serde(default)]
    pub modules: Vec<FlowModuleEstimate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowCapabilityRequirement {
    pub id: String,
    /// Focused semantic description of one catalog operation. Protocol/service, operation, and
    /// algorithm/type words are treated as fail-closed anchors, not merely ranking terms.
    pub intent: String,
    #[serde(default = "default_true")]
    pub required: bool,
    /// Exact live-catalog node selected after semantic discovery. Every required capability must
    /// select one before the plan can be feasible. Omit it on a discovery call, then copy one of
    /// that resolution's returned candidate `node_type` values and resubmit the complete request.
    #[serde(default)]
    pub exact_node_type: Option<String>,
    #[serde(default)]
    pub inputs: Vec<FlowPinRequirement>,
    #[serde(default)]
    pub outputs: Vec<FlowPinRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowPinRequirement {
    /// Acceptable exact/case-insensitive pin names, ordered by preference.
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub data_type: Option<FlowIrDataType>,
    #[serde(default)]
    pub container: Option<FlowIrContainer>,
    #[serde(default)]
    pub execution: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowModuleEstimate {
    pub name: String,
    /// Functions own a separate layer; every Event shares the root layer.
    pub kind: FlowModuleKind,
    /// All materialized body nodes, including variable setters and Event result-return nodes.
    /// Event entry nodes are added automatically by the planner.
    pub estimated_nodes: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowModuleKind {
    Function,
    Event,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlowCapabilityPlan {
    pub feasible: bool,
    pub requirements: Vec<FlowCapabilityResolution>,
    pub module_budget_violations: Vec<FlowIrDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlowCapabilityResolution {
    pub id: String,
    pub intent: String,
    pub required: bool,
    pub supported: bool,
    /// The live catalog contains compatible candidates, but this required capability has not yet
    /// selected one via `exact_node_type`. Copy one returned candidate and resubmit the full plan.
    #[serde(default)]
    pub selection_required: bool,
    pub candidates: Vec<FlowCapabilityCandidate>,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlowCapabilityCandidate {
    pub node_type: String,
    pub display: String,
    pub score: i32,
    pub declaration: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapabilitySemanticAnchor {
    // Protocols/services.
    Imap,
    Smtp,
    Slack,
    Email,
    Http,
    Webhook,
    // Operations whose distinction matters within a protocol/service family.
    Connect,
    Disconnect,
    Send,
    Fetch,
    SearchOrList,
    MarkProcessed,
    Match,
    Capture,
    // Algorithms/types.
    Hash,
    Sha256,
    Regex,
}

impl CapabilitySemanticAnchor {
    fn label(self) -> &'static str {
        match self {
            Self::Imap => "protocol:imap",
            Self::Smtp => "protocol:smtp",
            Self::Slack => "service:slack",
            Self::Email => "service:email",
            Self::Http => "protocol:http",
            Self::Webhook => "service:webhook",
            Self::Connect => "operation:connect",
            Self::Disconnect => "operation:disconnect",
            Self::Send => "operation:send",
            Self::Fetch => "operation:fetch",
            Self::SearchOrList => "operation:search_or_list",
            Self::MarkProcessed => "operation:mark_processed",
            Self::Match => "operation:match",
            Self::Capture => "operation:capture",
            Self::Hash => "algorithm:hash",
            Self::Sha256 => "algorithm:sha256",
            Self::Regex => "type:regex",
        }
    }

    fn matches(self, document: &SemanticDocument) -> bool {
        match self {
            Self::Imap => document.has_any(&["imap"]),
            Self::Smtp => document.has_any(&["smtp"]),
            Self::Slack => document.has_any(&["slack"]),
            Self::Email => document.has_any(&["email", "mail"]),
            Self::Http => document.has_any(&["http", "https"]),
            Self::Webhook => document.has_any(&["webhook"]),
            // Deliberately do not accept the generic word `connection`: send/fetch nodes often
            // describe their connection input, and disconnect nodes close a connection. The
            // operation itself must be named.
            Self::Connect => document.has_any(&[
                "connect",
                "connecting",
                "establish",
                "login",
                "authenticate",
            ]),
            Self::Disconnect => document.has_any(&[
                "disconnect",
                "disconnects",
                "close",
                "closes",
                "logout",
                "terminate",
            ]),
            Self::Send => document.has_any(&[
                "send", "sends", "sending", "deliver", "delivers", "transmit", "outbound",
            ]),
            Self::Fetch => document.has_any(&[
                "fetch",
                "fetches",
                "read",
                "reads",
                "retrieve",
                "retrieves",
                "download",
            ]),
            Self::SearchOrList => {
                document.has_any(&["search", "searches", "list", "lists", "query", "scan"])
            }
            Self::MarkProcessed => {
                document.has_any(&["mark", "marked", "seen", "flag", "processed"])
            }
            Self::Match => document.has_any(&["match", "matches", "matching", "find", "test"]),
            Self::Capture => {
                document.has_any(&["capture", "captures", "capturing", "group", "groups"])
            }
            Self::Hash => {
                let specific_algorithm =
                    document.has_any(&["digest", "checksum", "sha", "sha256", "blake3", "md5"]);
                specific_algorithm
                    || (document.has_any(&["hash"])
                        && !document.has_any(&["map", "hashmap"])
                        && !document.has_phrase("hash map"))
            }
            Self::Sha256 => {
                document.has_any(&["sha256"])
                    || document.has_phrase("sha 256")
                    || document.has_phrase("sha-256")
            }
            Self::Regex => {
                document.has_any(&["regex", "regexp"]) || document.has_phrase("regular expression")
            }
        }
    }
}

struct SemanticDocument {
    normalized: String,
    tokens: HashSet<String>,
}

impl SemanticDocument {
    fn new(text: String) -> Self {
        Self {
            tokens: tokenize_query_text(&text).into_iter().collect(),
            normalized: text.to_ascii_lowercase(),
        }
    }

    fn has_any(&self, terms: &[&str]) -> bool {
        terms.iter().any(|term| self.tokens.contains(*term))
    }

    fn has_phrase(&self, phrase: &str) -> bool {
        self.normalized.contains(phrase)
    }
}

fn push_semantic_anchor(
    anchors: &mut Vec<CapabilitySemanticAnchor>,
    anchor: CapabilitySemanticAnchor,
) {
    if !anchors.contains(&anchor) {
        anchors.push(anchor);
    }
}

fn intent_semantic_anchors(intent: &str) -> Vec<CapabilitySemanticAnchor> {
    let document = SemanticDocument::new(intent.to_string());
    let mut anchors = Vec::new();

    if document.has_any(&["imap"]) {
        push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Imap);
    }
    if document.has_any(&["smtp"]) {
        push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Smtp);
    }
    if document.has_any(&["slack"]) {
        push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Slack);
    }
    if document.has_any(&["email", "mail"]) {
        push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Email);
    }
    if document.has_any(&["http", "https"]) {
        push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Http);
    }
    if document.has_any(&["webhook"]) {
        push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Webhook);
    }

    let protocol_or_service_anchored = anchors.iter().any(|anchor| {
        matches!(
            anchor,
            CapabilitySemanticAnchor::Imap
                | CapabilitySemanticAnchor::Smtp
                | CapabilitySemanticAnchor::Slack
                | CapabilitySemanticAnchor::Email
                | CapabilitySemanticAnchor::Http
                | CapabilitySemanticAnchor::Webhook
        )
    });
    if protocol_or_service_anchored {
        if document.has_any(&[
            "connect",
            "connecting",
            "establish",
            "login",
            "authenticate",
        ]) {
            push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Connect);
        }
        if document.has_any(&[
            "disconnect",
            "disconnecting",
            "close",
            "closing",
            "logout",
            "terminate",
        ]) {
            push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Disconnect);
        }
        if document.has_any(&[
            "send", "sending", "deliver", "transmit", "outbound", "reply",
        ]) {
            push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Send);
        }
        if document.has_any(&["fetch", "read", "retrieve", "download"]) {
            push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Fetch);
        }
        if document.has_any(&["search", "list", "query", "scan"]) {
            push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::SearchOrList);
        }
        if document.has_any(&["mark", "seen", "flag", "processed"]) {
            push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::MarkProcessed);
        }
    }

    let regex_intent = document.has_any(&["regex", "regexp"])
        || document.has_phrase("regular expression")
        || (document.has_any(&["match", "matches", "matching"])
            && document.has_any(&["capture", "captures", "group", "groups"]));
    if regex_intent {
        push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Regex);
        if document.has_any(&["match", "matches", "matching", "test", "find"]) {
            push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Match);
        }
        if document.has_any(&["capture", "captures", "capturing", "group", "groups"]) {
            push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Capture);
        }
    }

    let hash_intent = document.has_any(&[
        "hash", "hashing", "digest", "checksum", "sha", "sha256", "blake3", "md5",
    ]);
    if hash_intent {
        push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Hash);
    }
    if document.has_any(&["sha256"])
        || document.has_phrase("sha 256")
        || document.has_phrase("sha-256")
    {
        push_semantic_anchor(&mut anchors, CapabilitySemanticAnchor::Sha256);
    }

    anchors
}

fn catalog_semantic_document(metadata: &NodeMetadata) -> SemanticDocument {
    SemanticDocument::new(format!(
        "{} {} {} {} {}",
        metadata.name,
        metadata.friendly_name,
        metadata.description,
        metadata.category.as_deref().unwrap_or_default(),
        metadata.capability_tags.join(" ")
    ))
}

fn missing_semantic_anchors(
    metadata: &NodeMetadata,
    anchors: &[CapabilitySemanticAnchor],
) -> Vec<&'static str> {
    let document = catalog_semantic_document(metadata);
    anchors
        .iter()
        .copied()
        .filter(|anchor| !anchor.matches(&document))
        .map(CapabilitySemanticAnchor::label)
        .collect()
}

/// Resolve every requested capability against the same live metadata reconciliation will use.
pub fn plan_flow_capabilities(
    request: &FlowCapabilityPlanRequest,
    catalog: &[NodeMetadata],
) -> FlowCapabilityPlan {
    let mut module_budget_violations = Vec::new();
    if request.requirements.len() > MAX_FLOW_IR_CAPABILITY_REQUIREMENTS {
        let mut diagnostic = FlowIrDiagnostic::new(
            "IR_CAPABILITY_REQUIREMENT_LIMIT_EXCEEDED",
            "/requirements",
            None,
            format!(
                "capability plan contains {} requirements; the limit is {MAX_FLOW_IR_CAPABILITY_REQUIREMENTS}",
                request.requirements.len()
            ),
        );
        diagnostic.expected = Some(format!("<= {MAX_FLOW_IR_CAPABILITY_REQUIREMENTS}"));
        diagnostic.actual = Some(request.requirements.len().to_string());
        module_budget_violations.push(diagnostic);
    }
    if request.modules.len() > MAX_FLOW_IR_MODULES {
        let mut diagnostic = FlowIrDiagnostic::new(
            "IR_CAPABILITY_MODULE_LIMIT_EXCEEDED",
            "/modules",
            None,
            format!(
                "capability plan estimates {} modules; the limit is {MAX_FLOW_IR_MODULES}",
                request.modules.len()
            ),
        );
        diagnostic.expected = Some(format!("<= {MAX_FLOW_IR_MODULES}"));
        diagnostic.actual = Some(request.modules.len().to_string());
        module_budget_violations.push(diagnostic);
    }

    let mut resolutions = Vec::new();
    for (requirement_index, requirement) in request
        .requirements
        .iter()
        .take(MAX_FLOW_IR_CAPABILITY_REQUIREMENTS)
        .enumerate()
    {
        let oversized_direction = [
            ("inputs", requirement.inputs.len()),
            ("outputs", requirement.outputs.len()),
        ]
        .into_iter()
        .find(|(_, count)| *count > MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION);
        if let Some((direction, count)) = oversized_direction {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_CAPABILITY_PIN_REQUIREMENT_LIMIT_EXCEEDED",
                format!("/requirements/{requirement_index}/{direction}"),
                None,
                format!(
                    "capability {:?} contains {count} {direction} requirements; the per-direction limit is {MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION}",
                    requirement.id
                ),
            );
            diagnostic.expected = Some(format!("<= {MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION}"));
            diagnostic.actual = Some(count.to_string());
            module_budget_violations.push(diagnostic);
            resolutions.push(FlowCapabilityResolution {
                id: requirement.id.clone(),
                intent: requirement.intent.clone(),
                required: requirement.required,
                supported: false,
                selection_required: false,
                candidates: Vec::new(),
                missing: vec![format!(
                    "{direction} pin contract exceeds the planner safety limit"
                )],
            });
            continue;
        }
        let semantic_anchors = intent_semantic_anchors(&requirement.intent);
        let exact_node_type = requirement
            .exact_node_type
            .as_deref()
            .filter(|node_type| !node_type.trim().is_empty());
        let mut candidates = catalog
            .iter()
            .filter_map(|metadata| {
                if let Some(exact) = exact_node_type
                    && normalize_symbol(exact) != normalize_symbol(&metadata.name)
                    && normalize_symbol(exact)
                        != normalize_symbol(&flow_like_ast::to_camel_case(&metadata.name))
                {
                    return None;
                }
                // Ranking is deliberately secondary. A high lexical score caused generic
                // `*_from_string` wrappers to masquerade as regex matching and a selector string
                // conversion to outrank SHA-256. High-signal protocol/service, operation, and
                // algorithm/type anchors must all agree before this declaration is even scored.
                if !missing_semantic_anchors(metadata, &semantic_anchors).is_empty() {
                    return None;
                }
                let input_match =
                    match_pin_requirements_distinct(&requirement.inputs, &metadata.inputs)
                        .is_some();
                let output_match =
                    match_pin_requirements_distinct(&requirement.outputs, &metadata.outputs)
                        .is_some();
                if !(input_match && output_match) {
                    return None;
                }
                let semantic_score = score_catalog_metadata(metadata, &requirement.intent);
                // `exact_node_type` is a declaration constraint, not a semantic override. Anchor
                // families enforce high-signal intents above; unanchored/general intents still
                // need positive live-metadata evidence before the exact match receives MAX score.
                if exact_node_type.is_some()
                    && !requirement.intent.trim().is_empty()
                    && semantic_anchors.is_empty()
                    && semantic_score <= 0
                {
                    return None;
                }
                let score = if exact_node_type.is_some() {
                    i32::MAX
                } else {
                    {
                        if semantic_anchors.is_empty() {
                            semantic_score
                        } else {
                            // Passing every discriminative anchor is itself semantic evidence,
                            // even when the search scorer lacks the synonym (for example
                            // `digest` intent versus an exact `sha256` catalog name).
                            semantic_score.max(1)
                        }
                    }
                };
                (score > 0 || requirement.intent.trim().is_empty()).then_some((score, metadata))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.name.cmp(&right.1.name))
        });
        let candidates = candidates
            .into_iter()
            .take(5)
            .map(|(score, metadata)| FlowCapabilityCandidate {
                node_type: metadata.name.clone(),
                display: flow_like_ast::to_camel_case(&metadata.name),
                score,
                declaration: metadata_to_signature(metadata).render_declaration(),
            })
            .collect::<Vec<_>>();
        let supported = !candidates.is_empty();
        let selection_required = requirement.required && supported && exact_node_type.is_none();
        let mut missing = Vec::new();
        if selection_required {
            missing.push(
                "selection_required: copy one candidate.node_type into exact_node_type and resubmit the complete capability plan"
                    .to_string(),
            );
        } else if !supported {
            if let Some(exact) = exact_node_type {
                missing.push(format!("exact node type {exact:?}"));
                if let Some(metadata) = catalog.iter().find(|metadata| {
                    normalize_symbol(exact) == normalize_symbol(&metadata.name)
                        || normalize_symbol(exact)
                            == normalize_symbol(&flow_like_ast::to_camel_case(&metadata.name))
                }) {
                    let missing_anchors = missing_semantic_anchors(metadata, &semantic_anchors);
                    if !missing_anchors.is_empty() {
                        missing.push(format!(
                            "semantic anchors not satisfied: {}",
                            missing_anchors.join(", ")
                        ));
                    }
                }
            }
            missing.extend(
                requirement
                    .inputs
                    .iter()
                    .map(|pin| format!("input {}", pin_requirement_label(pin))),
            );
            missing.extend(
                requirement
                    .outputs
                    .iter()
                    .map(|pin| format!("output {}", pin_requirement_label(pin))),
            );
            if missing.is_empty() {
                missing.push(format!(
                    "catalog capability matching {:?}",
                    requirement.intent
                ));
            }
        }
        resolutions.push(FlowCapabilityResolution {
            id: requirement.id.clone(),
            intent: requirement.intent.clone(),
            required: requirement.required,
            supported,
            selection_required,
            candidates,
            missing,
        });
    }

    let mut estimated_by_scope = BTreeMap::<String, usize>::new();
    let mut estimated_module_names = HashSet::new();
    for (module_index, module) in request.modules.iter().take(MAX_FLOW_IR_MODULES).enumerate() {
        if !estimated_module_names.insert(normalize_symbol(&module.name)) {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_MODULE_ESTIMATE_DUPLICATE",
                format!("/modules/{module_index}/name"),
                Some(&module.name),
                format!("module {:?} is estimated more than once", module.name),
            );
            diagnostic.fix = Some(
                "keep exactly one Function/Event estimate for each expected module".to_string(),
            );
            module_budget_violations.push(diagnostic);
            continue;
        }
        let scope = match module.kind {
            FlowModuleKind::Function => module.name.clone(),
            FlowModuleKind::Event => "$root".to_string(),
        };
        let count = module
            .estimated_nodes
            .saturating_add(usize::from(module.kind == FlowModuleKind::Event));
        let total = estimated_by_scope.entry(scope).or_default();
        *total = total.saturating_add(count);
    }
    for (scope, estimated_nodes) in estimated_by_scope {
        if estimated_nodes <= MAX_NODES_PER_LAYER {
            continue;
        }
        let mut diagnostic = FlowIrDiagnostic::new(
            "IR_ESTIMATED_NODE_BUDGET_EXCEEDED",
            "/modules",
            Some(&scope),
            format!(
                "scope {scope:?} is estimated at {estimated_nodes} nodes; the limit is {MAX_NODES_PER_LAYER}"
            ),
        );
        diagnostic.expected = Some(format!("<= {MAX_NODES_PER_LAYER} nodes"));
        diagnostic.actual = Some(format!("{estimated_nodes} nodes"));
        diagnostic.fix =
            Some("split responsibilities into function layers before generation".to_string());
        module_budget_violations.push(diagnostic);
    }
    let feasible = resolutions.iter().all(|resolution| {
        !resolution.required || (resolution.supported && !resolution.selection_required)
    }) && module_budget_violations.is_empty();
    FlowCapabilityPlan {
        feasible,
        requirements: resolutions,
        module_budget_violations,
    }
}

/// Verify that every required planned capability is present in the authored program, including
/// the required pin contract. Planning proves availability; this gate proves implementation.
pub fn validate_flow_capability_usage(
    program: &FlowIrProgram,
    request: &FlowCapabilityPlanRequest,
    plan: &FlowCapabilityPlan,
    catalog: &[NodeMetadata],
) -> Vec<FlowIrDiagnostic> {
    let catalog_by_name = catalog
        .iter()
        .map(|metadata| (normalize_symbol(&metadata.name), metadata))
        .collect::<HashMap<_, _>>();
    let execution = IrExecutionAnalysis::new(program, &catalog_by_name);
    let reachable_functions = reachable_flow_ir_function_names(program);
    let mut diagnostics = Vec::new();

    for (requirement_index, requirement) in request.requirements.iter().enumerate() {
        if !requirement.required {
            continue;
        }
        let Some(resolution) = plan.requirements.get(requirement_index) else {
            diagnostics.push(FlowIrDiagnostic::new(
                "IR_CAPABILITY_PLAN_MISMATCH",
                format!("/capability_plan/requirements/{requirement_index}"),
                None,
                "capability plan does not correspond to its request",
            ));
            continue;
        };
        if !resolution.supported {
            continue;
        }
        let candidates = resolution
            .candidates
            .iter()
            .map(|candidate| normalize_symbol(&candidate.node_type))
            .collect::<HashSet<_>>();
        let implemented = program.modules.iter().any(|module| {
            if matches!(module, FlowIrModule::Function { .. })
                && !reachable_functions.contains(&normalize_symbol(module.name()))
            {
                return false;
            }
            let output_references = collect_module_output_references(module);
            match module {
                FlowIrModule::Event {
                    node_type,
                    params,
                    steps,
                    ..
                } if candidates.contains(&normalize_symbol(node_type)) => {
                    resolve_catalog_node(node_type, &catalog_by_name).is_some_and(|metadata| {
                        event_implements_requirement(
                            requirement,
                            metadata,
                            params,
                            steps,
                            &execution,
                        )
                    })
                }
                _ => {
                    let (event_scope, boundary_continuation) = match module {
                        FlowIrModule::Function { name, .. } => {
                            (false, execution.function_is_impure(name))
                        }
                        FlowIrModule::Event { .. } => (true, false),
                    };
                    steps_implement_requirement(
                        module.steps(),
                        requirement,
                        &candidates,
                        &catalog_by_name,
                        &output_references,
                        &execution,
                        event_scope,
                        boundary_continuation,
                        boundary_continuation,
                    )
                }
            }
        });
        if !implemented {
            let mut diagnostic = FlowIrDiagnostic::new(
                "IR_REQUIRED_CAPABILITY_UNUSED",
                format!("/capability_plan/requirements/{requirement_index}"),
                None,
                format!(
                    "required capability {:?} is available but not implemented with its required pin contract",
                    requirement.id
                ),
            );
            diagnostic.expected = Some(
                resolution
                    .candidates
                    .iter()
                    .map(|candidate| candidate.node_type.as_str())
                    .collect::<Vec<_>>()
                    .join(" | "),
            );
            diagnostic.fix = Some(
                "add one planned declaration and wire every required input/output pin".to_string(),
            );
            diagnostics.push(diagnostic);
        }
    }
    diagnostics
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReachableFlowIrOccurrence {
    CatalogNode(String),
    /// Compiler-lowered control flow has real runtime semantics even though it is not authored as
    /// a catalog node in typed IR.
    BuiltInAction(&'static str),
}

/// Runtime semantic occurrences reachable from an Event root. Function layers are included only
/// when a reachable `call_function` step references them; merely declaring a helper cannot prove
/// that a required capability is implemented.
pub(crate) fn reachable_flow_ir_occurrences(
    program: &FlowIrProgram,
) -> Vec<ReachableFlowIrOccurrence> {
    fn collect_steps(
        steps: &[FlowIrStep],
        functions: &HashMap<String, &FlowIrModule>,
        visiting: &mut HashSet<String>,
        occurrences: &mut Vec<ReachableFlowIrOccurrence>,
    ) {
        for step in steps {
            match step {
                FlowIrStep::Node {
                    node_type,
                    exec_arms,
                    ..
                } => {
                    occurrences.push(ReachableFlowIrOccurrence::CatalogNode(normalize_symbol(
                        node_type,
                    )));
                    for arm in exec_arms {
                        collect_steps(&arm.steps, functions, visiting, occurrences);
                    }
                }
                FlowIrStep::CallFunction { function, .. } => {
                    let key = normalize_symbol(function);
                    if visiting.insert(key.clone()) {
                        if let Some(module) = functions.get(&key) {
                            collect_steps(module.steps(), functions, visiting, occurrences);
                        }
                        visiting.remove(&key);
                    }
                }
                FlowIrStep::If {
                    then_steps,
                    else_steps,
                    ..
                } => {
                    occurrences.push(ReachableFlowIrOccurrence::BuiltInAction("branch"));
                    collect_steps(then_steps, functions, visiting, occurrences);
                    collect_steps(else_steps, functions, visiting, occurrences);
                }
                FlowIrStep::ForEach { steps, .. } => {
                    occurrences.push(ReachableFlowIrOccurrence::BuiltInAction("iterate"));
                    collect_steps(steps, functions, visiting, occurrences);
                }
                FlowIrStep::Return { .. } => break,
                FlowIrStep::Assign { .. } => {}
            }
        }
    }

    let functions = program
        .modules
        .iter()
        .filter(|module| matches!(module, FlowIrModule::Function { .. }))
        .map(|module| (normalize_symbol(module.name()), module))
        .collect::<HashMap<_, _>>();
    let mut occurrences = Vec::new();
    for module in &program.modules {
        let FlowIrModule::Event {
            node_type, steps, ..
        } = module
        else {
            continue;
        };
        occurrences.push(ReachableFlowIrOccurrence::CatalogNode(normalize_symbol(
            node_type,
        )));
        collect_steps(steps, &functions, &mut HashSet::new(), &mut occurrences);
    }
    occurrences
}

fn reachable_flow_ir_function_names(program: &FlowIrProgram) -> HashSet<String> {
    fn collect_steps(
        steps: &[FlowIrStep],
        functions: &HashMap<String, &FlowIrModule>,
        reachable: &mut HashSet<String>,
    ) {
        for step in steps {
            match step {
                FlowIrStep::CallFunction { function, .. } => {
                    let key = normalize_symbol(function);
                    if reachable.insert(key.clone())
                        && let Some(module) = functions.get(&key)
                    {
                        collect_steps(module.steps(), functions, reachable);
                    }
                }
                FlowIrStep::Node { exec_arms, .. } => {
                    for arm in exec_arms {
                        collect_steps(&arm.steps, functions, reachable);
                    }
                }
                FlowIrStep::If {
                    then_steps,
                    else_steps,
                    ..
                } => {
                    collect_steps(then_steps, functions, reachable);
                    collect_steps(else_steps, functions, reachable);
                }
                FlowIrStep::ForEach { steps, .. } => {
                    collect_steps(steps, functions, reachable);
                }
                FlowIrStep::Return { .. } => break,
                FlowIrStep::Assign { .. } => {}
            }
        }
    }

    let functions = program
        .modules
        .iter()
        .filter(|module| matches!(module, FlowIrModule::Function { .. }))
        .map(|module| (normalize_symbol(module.name()), module))
        .collect::<HashMap<_, _>>();
    let mut reachable = HashSet::new();
    for module in &program.modules {
        if matches!(module, FlowIrModule::Event { .. }) {
            collect_steps(module.steps(), &functions, &mut reachable);
        }
    }
    reachable
}

#[allow(clippy::too_many_arguments)]
fn steps_implement_requirement(
    steps: &[FlowIrStep],
    requirement: &FlowCapabilityRequirement,
    candidates: &HashSet<String>,
    catalog: &HashMap<String, &NodeMetadata>,
    output_references: &HashSet<(String, String, usize)>,
    execution: &IrExecutionAnalysis<'_>,
    event_scope: bool,
    outer_continuation: bool,
    function_boundary_continuation: bool,
) -> bool {
    steps.iter().enumerate().any(|(index, step)| match step {
        FlowIrStep::Node {
            id,
            node_type,
            args,
            continue_from,
            exec_arms,
            ..
        } => {
            let tail_has_consumer =
                execution.steps_have_execution_consumer(&steps[index + 1..], event_scope);
            let has_continuation = tail_has_consumer || outer_continuation;
            let reaches_function_boundary = function_boundary_continuation;
            let used_exec_arms = exec_arms
                .iter()
                .filter(|arm| {
                    execution.steps_have_execution_consumer(&arm.steps, event_scope)
                        || (has_continuation
                            && continue_from
                                .as_deref()
                                .is_some_and(|pin| pin.eq_ignore_ascii_case(&arm.pin))
                            && !steps_prevent_fallthrough(&arm.steps, catalog, event_scope))
                })
                .map(|arm| normalize_symbol(&arm.pin))
                .collect::<HashSet<_>>();
            let terminal_exec_arms = exec_arms
                .iter()
                .filter(|arm| steps_prevent_fallthrough(&arm.steps, catalog, event_scope))
                .map(|arm| normalize_symbol(&arm.pin))
                .collect::<HashSet<_>>();
            let this_node = candidates.contains(&normalize_symbol(node_type))
                && resolve_catalog_node(node_type, catalog).is_some_and(|metadata| {
                    node_implements_requirement(
                        requirement,
                        metadata,
                        id,
                        args,
                        continue_from.as_deref(),
                        &used_exec_arms,
                        &terminal_exec_arms,
                        output_references,
                        has_continuation,
                        reaches_function_boundary,
                    )
                });
            this_node
                || exec_arms.iter().any(|arm| {
                    let arm_reaches_continuation = has_continuation
                        && continue_from
                            .as_deref()
                            .is_some_and(|pin| pin.eq_ignore_ascii_case(&arm.pin))
                        || (reaches_function_boundary
                            && !terminal_exec_arms.contains(&normalize_symbol(&arm.pin)));
                    steps_implement_requirement(
                        &arm.steps,
                        requirement,
                        candidates,
                        catalog,
                        output_references,
                        execution,
                        event_scope,
                        arm_reaches_continuation,
                        reaches_function_boundary,
                    )
                })
        }
        FlowIrStep::If {
            then_steps,
            else_steps,
            ..
        } => {
            let tail_has_consumer =
                execution.steps_have_execution_consumer(&steps[index + 1..], event_scope);
            let has_continuation = tail_has_consumer || outer_continuation;
            let reaches_function_boundary = function_boundary_continuation;
            steps_implement_requirement(
                then_steps,
                requirement,
                candidates,
                catalog,
                output_references,
                execution,
                event_scope,
                has_continuation,
                reaches_function_boundary,
            ) || steps_implement_requirement(
                else_steps,
                requirement,
                candidates,
                catalog,
                output_references,
                execution,
                event_scope,
                has_continuation,
                reaches_function_boundary,
            )
        }
        FlowIrStep::ForEach { steps, .. } => {
            // The loop body has an actual back-edge to the loop controller.
            steps_implement_requirement(
                steps,
                requirement,
                candidates,
                catalog,
                output_references,
                execution,
                event_scope,
                true,
                false,
            )
        }
        FlowIrStep::CallFunction { .. } | FlowIrStep::Assign { .. } | FlowIrStep::Return { .. } => {
            false
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn node_implements_requirement(
    requirement: &FlowCapabilityRequirement,
    metadata: &NodeMetadata,
    id: &str,
    args: &[FlowIrArg],
    continue_from: Option<&str>,
    used_exec_arms: &HashSet<String>,
    terminal_exec_arms: &HashSet<String>,
    output_references: &HashSet<(String, String, usize)>,
    has_continuation: bool,
    function_boundary_continuation: bool,
) -> bool {
    let inputs_satisfied = pin_requirements_used_distinct(
        &requirement.inputs,
        &metadata.inputs,
        &|pin_index, required, pin| {
            if required.execution {
                return true;
            }
            let occurrence = pin_occurrence(&metadata.inputs, pin_index);
            args.iter().any(|argument| {
                normalize_symbol(&argument.pin) == normalize_symbol(&pin.name)
                    && argument.occurrence == occurrence
            })
        },
    );
    let execution_output_count = metadata
        .outputs
        .iter()
        .filter(|pin| pin.data_type.eq_ignore_ascii_case("Execution"))
        .count();
    let outputs_satisfied = pin_requirements_used_distinct(
        &requirement.outputs,
        &metadata.outputs,
        &|pin_index, required, pin| {
            if required.execution {
                (execution_output_count <= 1 && has_continuation)
                    || (has_continuation
                        && continue_from.is_some_and(|name| name.eq_ignore_ascii_case(&pin.name)))
                    || used_exec_arms.contains(&normalize_symbol(&pin.name))
                    || (function_boundary_continuation
                        && !terminal_exec_arms.contains(&normalize_symbol(&pin.name)))
            } else {
                output_references.contains(&(
                    normalize_symbol(id),
                    normalize_symbol(&pin.name),
                    pin_occurrence(&metadata.outputs, pin_index),
                ))
            }
        },
    );
    inputs_satisfied && outputs_satisfied
}

fn event_implements_requirement(
    requirement: &FlowCapabilityRequirement,
    metadata: &NodeMetadata,
    params: &[FlowIrParam],
    steps: &[FlowIrStep],
    execution: &IrExecutionAnalysis<'_>,
) -> bool {
    let inputs_satisfied =
        pin_requirements_used_distinct(&requirement.inputs, &metadata.inputs, &|_, required, _| {
            required.execution
        });
    let outputs_satisfied = pin_requirements_used_distinct(
        &requirement.outputs,
        &metadata.outputs,
        &|_, required, pin| {
            if required.execution {
                execution.steps_have_execution_consumer(steps, true)
            } else {
                params
                    .iter()
                    .any(|param| param.name.eq_ignore_ascii_case(&pin.name))
            }
        },
    );
    inputs_satisfied && outputs_satisfied
}

struct IrExecutionAnalysis<'a> {
    program: &'a FlowIrProgram,
    catalog: &'a HashMap<String, &'a NodeMetadata>,
    globals: HashSet<String>,
}

impl<'a> IrExecutionAnalysis<'a> {
    fn new(program: &'a FlowIrProgram, catalog: &'a HashMap<String, &'a NodeMetadata>) -> Self {
        Self {
            program,
            catalog,
            globals: program
                .variables
                .iter()
                .map(|variable| normalize_symbol(&variable.name))
                .collect(),
        }
    }

    fn function_is_impure(&self, name: &str) -> bool {
        self.function_is_impure_inner(name, &mut HashSet::new())
    }

    fn function_is_impure_inner(&self, name: &str, seen: &mut HashSet<String>) -> bool {
        let key = normalize_symbol(name);
        if !seen.insert(key.clone()) {
            return false;
        }
        let impure = self
            .program
            .modules
            .iter()
            .find(|module| {
                matches!(module, FlowIrModule::Function { .. })
                    && normalize_symbol(module.name()) == key
            })
            .is_some_and(|module| {
                self.steps_have_execution_consumer_inner(module.steps(), false, true, seen)
            });
        seen.remove(&key);
        impure
    }

    fn steps_have_execution_consumer(&self, steps: &[FlowIrStep], event_scope: bool) -> bool {
        self.steps_have_execution_consumer_inner(steps, event_scope, false, &mut HashSet::new())
    }

    fn steps_have_execution_consumer_inner(
        &self,
        steps: &[FlowIrStep],
        event_scope: bool,
        include_multi_output_branches: bool,
        seen: &mut HashSet<String>,
    ) -> bool {
        steps.iter().any(|step| match step {
            FlowIrStep::Node { node_type, .. } => resolve_catalog_node(node_type, self.catalog)
                .is_some_and(|metadata| {
                    metadata
                        .inputs
                        .iter()
                        .any(|pin| pin.data_type.eq_ignore_ascii_case("Execution"))
                        || (include_multi_output_branches
                            && metadata
                                .outputs
                                .iter()
                                .filter(|pin| pin.data_type.eq_ignore_ascii_case("Execution"))
                                .count()
                                > 1)
                }),
            // Both constructs materialize an execution controller even when their bodies are
            // otherwise pure.
            FlowIrStep::If { .. } | FlowIrStep::ForEach { .. } => true,
            // Event returns lower to a terminal result node. Function returns only wire data to
            // the function boundary and do not themselves consume an execution cursor.
            FlowIrStep::Return { .. } => event_scope,
            FlowIrStep::Assign { target, .. } => self.globals.contains(&normalize_symbol(target)),
            FlowIrStep::CallFunction { function, .. } => {
                self.function_is_impure_inner(function, seen)
            }
        })
    }
}

fn pin_requirements_used_distinct<F>(
    requirements: &[FlowPinRequirement],
    pins: &[PinMetadata],
    is_used: &F,
) -> bool
where
    F: Fn(usize, &FlowPinRequirement, &PinMetadata) -> bool,
{
    fn augment<F>(
        requirement_index: usize,
        requirements: &[FlowPinRequirement],
        pins: &[PinMetadata],
        seen_pins: &mut [bool],
        pin_owners: &mut [Option<usize>],
        is_used: &F,
    ) -> bool
    where
        F: Fn(usize, &FlowPinRequirement, &PinMetadata) -> bool,
    {
        for (pin_index, pin) in pins.iter().enumerate() {
            if seen_pins[pin_index]
                || !pin_requirement_matches(
                    &requirements[requirement_index],
                    std::slice::from_ref(pin),
                )
                || !is_used(pin_index, &requirements[requirement_index], pin)
            {
                continue;
            }
            seen_pins[pin_index] = true;
            let previous_owner = pin_owners[pin_index];
            if previous_owner.is_none()
                || augment(
                    previous_owner.expect("checked above"),
                    requirements,
                    pins,
                    seen_pins,
                    pin_owners,
                    is_used,
                )
            {
                pin_owners[pin_index] = Some(requirement_index);
                return true;
            }
        }
        false
    }

    let mut pin_owners = vec![None; pins.len()];
    for requirement_index in 0..requirements.len() {
        let mut seen_pins = vec![false; pins.len()];
        if !augment(
            requirement_index,
            requirements,
            pins,
            &mut seen_pins,
            &mut pin_owners,
            is_used,
        ) {
            return false;
        }
    }
    true
}

fn pin_occurrence(pins: &[PinMetadata], pin_index: usize) -> usize {
    pins[..pin_index]
        .iter()
        .filter(|candidate| candidate.name.eq_ignore_ascii_case(&pins[pin_index].name))
        .count()
}

fn collect_module_output_references(module: &FlowIrModule) -> HashSet<(String, String, usize)> {
    fn collect_value(value: &FlowIrValue, output: &mut HashSet<(String, String, usize)>) {
        match value {
            FlowIrValue::Output {
                step,
                pin,
                occurrence,
            } => {
                output.insert((normalize_symbol(step), normalize_symbol(pin), *occurrence));
            }
            FlowIrValue::List { items } => {
                for item in items {
                    collect_value(item, output);
                }
            }
            FlowIrValue::Object { fields } => {
                for field in fields {
                    collect_value(&field.value, output);
                }
            }
            FlowIrValue::Literal { .. }
            | FlowIrValue::Ref { .. }
            | FlowIrValue::FunctionRefs { .. } => {}
        }
    }
    fn collect_steps(steps: &[FlowIrStep], output: &mut HashSet<(String, String, usize)>) {
        for step in steps {
            match step {
                FlowIrStep::Node {
                    args, exec_arms, ..
                } => {
                    for argument in args {
                        collect_value(&argument.value, output);
                    }
                    for arm in exec_arms {
                        collect_steps(&arm.steps, output);
                    }
                }
                FlowIrStep::CallFunction { args, .. } => {
                    for argument in args {
                        collect_value(&argument.value, output);
                    }
                }
                FlowIrStep::If {
                    condition,
                    then_steps,
                    else_steps,
                    ..
                } => {
                    collect_value(condition, output);
                    collect_steps(then_steps, output);
                    collect_steps(else_steps, output);
                }
                FlowIrStep::ForEach { array, steps, .. } => {
                    collect_value(array, output);
                    collect_steps(steps, output);
                }
                FlowIrStep::Assign { value, .. } => collect_value(value, output),
                FlowIrStep::Return { values } => {
                    for value in values {
                        collect_value(value, output);
                    }
                }
            }
        }
    }

    let mut output = HashSet::new();
    collect_steps(module.steps(), &mut output);
    output
}

fn pin_requirement_matches(requirement: &FlowPinRequirement, pins: &[PinMetadata]) -> bool {
    pins.iter().any(|pin| {
        let name_matches = requirement.names.is_empty()
            || requirement
                .names
                .iter()
                .any(|name| normalize_symbol(name) == normalize_symbol(&pin.name));
        let execution_matches = if requirement.execution {
            pin.data_type.eq_ignore_ascii_case("Execution")
        } else {
            !pin.data_type.eq_ignore_ascii_case("Execution")
                && data_type_from_label(&pin.data_type) != FlowIrDataType::Unsupported
        };
        let type_matches = requirement
            .data_type
            .is_none_or(|data_type| data_type_from_label(&pin.data_type) == data_type);
        let container_matches = requirement
            .container
            .is_none_or(|container| container_from_label(&pin.value_type) == container);
        name_matches && execution_matches && type_matches && container_matches
    })
}

fn match_pin_requirements_distinct(
    requirements: &[FlowPinRequirement],
    pins: &[PinMetadata],
) -> Option<Vec<usize>> {
    fn augment(
        requirement_index: usize,
        requirements: &[FlowPinRequirement],
        pins: &[PinMetadata],
        seen_pins: &mut [bool],
        pin_owners: &mut [Option<usize>],
        assignments: &mut [Option<usize>],
    ) -> bool {
        for pin_index in 0..pins.len() {
            if seen_pins[pin_index]
                || !pin_requirement_matches(
                    &requirements[requirement_index],
                    std::slice::from_ref(&pins[pin_index]),
                )
            {
                continue;
            }
            seen_pins[pin_index] = true;
            let previous_owner = pin_owners[pin_index];
            if previous_owner.is_none()
                || augment(
                    previous_owner.expect("checked above"),
                    requirements,
                    pins,
                    seen_pins,
                    pin_owners,
                    assignments,
                )
            {
                pin_owners[pin_index] = Some(requirement_index);
                assignments[requirement_index] = Some(pin_index);
                return true;
            }
        }
        false
    }

    let mut pin_owners = vec![None; pins.len()];
    let mut assignments = vec![None; requirements.len()];
    for requirement_index in 0..requirements.len() {
        let mut seen_pins = vec![false; pins.len()];
        if !augment(
            requirement_index,
            requirements,
            pins,
            &mut seen_pins,
            &mut pin_owners,
            &mut assignments,
        ) {
            return None;
        }
    }
    assignments.into_iter().collect()
}

fn pin_requirement_label(requirement: &FlowPinRequirement) -> String {
    let names = if requirement.names.is_empty() {
        "<any-name>".to_string()
    } else {
        requirement.names.join("|")
    };
    let kind = if requirement.execution {
        "execution".to_string()
    } else {
        requirement
            .data_type
            .map(|value| format!("{value:?}").to_ascii_lowercase())
            .unwrap_or_else(|| "data".to_string())
    };
    format!("{names}:{kind}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(name: &str, data_type: &str, value_type: &str) -> PinMetadata {
        PinMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            data_type: data_type.to_string(),
            value_type: value_type.to_string(),
            default_value: None,
            schema: None,
            is_generic: data_type == "Generic",
            valid_values: None,
            enforce_schema: false,
        }
    }

    fn node(name: &str, inputs: Vec<PinMetadata>, outputs: Vec<PinMetadata>) -> NodeMetadata {
        NodeMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: name.to_string(),
            inputs,
            outputs,
            category: None,
            required_inputs: Vec::new(),
            companion_nodes: Vec::new(),
            capability_tags: Vec::new(),
            namespace: None,
            alias: None,
            receiver: None,
        }
    }

    fn string(value: &str) -> FlowIrValue {
        FlowIrValue::Literal {
            value: FlowIrLiteral::String(value.to_string()),
        }
    }

    #[test]
    fn common_model_aliases_deserialize_but_serialize_canonically() {
        let shorthand: FlowIrType = serde_json::from_value(serde_json::json!("string")).unwrap();
        assert_eq!(shorthand, FlowIrType::scalar(FlowIrDataType::String));
        assert_eq!(
            serde_json::to_value(&shorthand).unwrap(),
            serde_json::json!({
                "data_type": "string",
                "container": "normal",
                "interface": null
            })
        );

        let shorthand_bool: FlowIrType = serde_json::from_value(serde_json::json!("bool")).unwrap();
        assert_eq!(shorthand_bool, FlowIrType::scalar(FlowIrDataType::Boolean));

        let bool_type: FlowIrType = serde_json::from_value(serde_json::json!({
            "kind": "bool"
        }))
        .unwrap();
        assert_eq!(bool_type.data_type, FlowIrDataType::Boolean);
        assert_eq!(
            serde_json::to_value(&bool_type).unwrap()["data_type"],
            "boolean"
        );
        assert!(
            serde_json::to_value(&bool_type)
                .unwrap()
                .get("kind")
                .is_none()
        );

        let int_type: FlowIrType = serde_json::from_value(serde_json::json!({
            "data_type": "int"
        }))
        .unwrap();
        assert_eq!(int_type.data_type, FlowIrDataType::Integer);
        assert_eq!(
            serde_json::to_value(&int_type).unwrap()["data_type"],
            "integer"
        );

        let param: FlowIrValue = serde_json::from_value(serde_json::json!({
            "kind": "param",
            "name": "ticket"
        }))
        .unwrap();
        assert!(matches!(param, FlowIrValue::Ref { ref name } if name == "ticket"));
        assert_eq!(serde_json::to_value(&param).unwrap()["kind"], "ref");

        let call: FlowIrStep = serde_json::from_value(serde_json::json!({
            "kind": "call",
            "id": "invoke",
            "function": "helper",
            "args": []
        }))
        .unwrap();
        assert!(matches!(call, FlowIrStep::CallFunction { .. }));
        assert_eq!(
            serde_json::to_value(&call).unwrap()["kind"],
            "call_function"
        );

        let branch: FlowIrStep = serde_json::from_value(serde_json::json!({
            "kind": "if",
            "id": "branch",
            "condition": {
                "kind": "literal",
                "value": { "type": "boolean", "value": true }
            },
            "then": [],
            "else": []
        }))
        .unwrap();
        let serialized = serde_json::to_value(&branch).unwrap();
        assert!(serialized.get("then_steps").is_some());
        assert!(serialized.get("else_steps").is_some());
        assert!(serialized.get("then").is_none());
    }

    #[test]
    fn typed_ir_compiles_exact_nodes_and_outputs_to_parseable_flowscript() {
        let catalog = vec![
            node(
                "events_simple",
                Vec::new(),
                vec![pin("exec_out", "Execution", "Normal")],
            ),
            node(
                "string_format",
                vec![pin("format_string", "String", "Normal")],
                vec![pin("string", "String", "Normal")],
            ),
        ];
        let program = FlowIrProgram {
            modules: vec![FlowIrModule::Event {
                name: "eventsSimple".to_string(),
                node_type: "eventsSimple".to_string(),
                params: Vec::new(),
                steps: vec![FlowIrStep::Node {
                    id: "message".to_string(),
                    node_type: "string_format".to_string(),
                    args: vec![FlowIrArg {
                        pin: "format_string".to_string(),
                        occurrence: 0,
                        value: string("hello"),
                    }],
                    continue_from: None,
                    exec_arms: Vec::new(),
                    anchor: None,
                }],
                anchor: None,
            }],
            ..Default::default()
        };
        let compiled = compile_flow_ir(&program, &catalog);
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        assert!(flow_like_ast::parse(&compiled.flowscript).is_ok());
        assert!(compiled.flowscript.contains("stringFormat"));
        assert_eq!(
            compiled.ast.as_ref().unwrap().events[0].node_type,
            "events_simple"
        );
    }

    #[test]
    fn typed_ir_function_cache_compiles_to_flowscript_without_losing_settings() {
        let module = FlowIrModule::Function {
            name: "calculatePricing".to_string(),
            params: Vec::new(),
            returns: Vec::new(),
            cache: Some(FlowIrFunctionCache {
                namespace: "pricing".to_string(),
                ttl_seconds: Some(3_600),
                scope: FlowIrFunctionCacheScope::User,
            }),
            steps: Vec::new(),
            anchor: None,
        };
        let serialized = serde_json::to_value(&module).expect("serialize function module");
        assert_eq!(serialized["cache"]["namespace"], "pricing");
        assert_eq!(serialized["cache"]["ttl_seconds"], 3_600);
        assert_eq!(serialized["cache"]["scope"], "user");

        let compiled = compile_flow_ir(
            &FlowIrProgram {
                modules: vec![module],
                ..Default::default()
            },
            &[],
        );
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        let cache = compiled.ast.as_ref().unwrap().functions[0]
            .cache
            .as_ref()
            .expect("compiled function cache");
        assert_eq!(cache.namespace, "pricing");
        assert_eq!(cache.ttl_seconds, Some(3_600));
        assert_eq!(cache.scope, FunctionCacheScope::User);
        assert!(
            compiled
                .flowscript
                .contains(r#"@cache({ namespace: "pricing", ttlSeconds: 3600, scope: "user" })"#)
        );
        assert!(flow_like_ast::parse(&compiled.flowscript).is_ok());
    }

    #[test]
    fn typed_ir_function_cache_defaults_and_explicit_permanent_ttl_are_unambiguous() {
        let default_module: FlowIrModule = serde_json::from_value(serde_json::json!({
            "kind": "function",
            "name": "defaultCached",
            "cache": {}
        }))
        .expect("empty cache object uses the typed IR defaults");
        let FlowIrModule::Function {
            cache: Some(default_cache),
            ..
        } = &default_module
        else {
            panic!("expected a cached function")
        };
        assert_eq!(default_cache, &FlowIrFunctionCache::default());
        assert_eq!(default_cache.namespace, "global");
        assert_eq!(default_cache.ttl_seconds, Some(300));
        assert_eq!(default_cache.scope, FlowIrFunctionCacheScope::App);

        let permanent_module: FlowIrModule = serde_json::from_value(serde_json::json!({
            "kind": "function",
            "name": "permanentCached",
            "cache": { "ttl_seconds": 0 }
        }))
        .expect("zero is the explicit permanent-cache lifetime");
        let compiled = compile_flow_ir(
            &FlowIrProgram {
                modules: vec![permanent_module],
                ..Default::default()
            },
            &[],
        );
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        let cache = compiled.ast.as_ref().unwrap().functions[0]
            .cache
            .as_ref()
            .expect("compiled cache metadata");
        assert_eq!(cache.namespace, "global");
        assert_eq!(cache.ttl_seconds, Some(0));
        assert_eq!(cache.scope, FunctionCacheScope::App);

        let legacy_permanent_module: FlowIrModule = serde_json::from_value(serde_json::json!({
            "kind": "function",
            "name": "legacyPermanentCached",
            "cache": { "ttl_seconds": null }
        }))
        .expect("legacy null cache lifetime remains accepted as permanent");
        let compiled = compile_flow_ir(
            &FlowIrProgram {
                modules: vec![legacy_permanent_module],
                ..Default::default()
            },
            &[],
        );
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        assert!(compiled.flowscript.contains("@cache({ ttlSeconds: 0 })"));
    }

    #[test]
    fn typed_ir_rejects_generic_to_string_without_conversion() {
        let catalog = vec![
            node(
                "struct_get",
                vec![
                    pin("struct", "Struct", "Normal"),
                    pin("field", "String", "Normal"),
                ],
                vec![pin("value", "Generic", "Normal")],
            ),
            node(
                "string_contains",
                vec![
                    pin("string", "String", "Normal"),
                    pin("pattern", "String", "Normal"),
                ],
                vec![pin("contains", "Boolean", "Normal")],
            ),
        ];
        let program = FlowIrProgram {
            variables: vec![FlowIrVariable {
                name: "mail".to_string(),
                value_type: FlowIrType::scalar(FlowIrDataType::Struct),
                default: None,
                exposed: false,
                secret: false,
                editable: true,
                runtime_configured: false,
                category: None,
                description: None,
                anchor: None,
            }],
            modules: vec![FlowIrModule::Function {
                name: "classify".to_string(),
                params: Vec::new(),
                returns: Vec::new(),
                cache: None,
                steps: vec![
                    FlowIrStep::Node {
                        id: "sender".to_string(),
                        node_type: "struct_get".to_string(),
                        args: vec![
                            FlowIrArg {
                                pin: "struct".to_string(),
                                occurrence: 0,
                                value: FlowIrValue::Ref {
                                    name: "mail".to_string(),
                                },
                            },
                            FlowIrArg {
                                pin: "field".to_string(),
                                occurrence: 0,
                                value: string("from"),
                            },
                        ],
                        continue_from: None,
                        exec_arms: Vec::new(),
                        anchor: None,
                    },
                    FlowIrStep::Node {
                        id: "check".to_string(),
                        node_type: "string_contains".to_string(),
                        args: vec![
                            FlowIrArg {
                                pin: "string".to_string(),
                                occurrence: 0,
                                value: FlowIrValue::Output {
                                    step: "sender".to_string(),
                                    pin: "value".to_string(),
                                    occurrence: 0,
                                },
                            },
                            FlowIrArg {
                                pin: "pattern".to_string(),
                                occurrence: 0,
                                value: string("@example.com"),
                            },
                        ],
                        continue_from: None,
                        exec_arms: Vec::new(),
                        anchor: None,
                    },
                ],
                anchor: None,
            }],
            ..Default::default()
        };
        let compiled = compile_flow_ir(&program, &catalog);
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "IR_INPUT_TYPE")
        );
    }

    #[test]
    fn catalog_path_date_and_bytes_types_fail_closed() {
        let catalog = vec![node(
            "typed_sink",
            vec![
                pin("path", "PathBuf", "Normal"),
                pin("date", "Date", "Normal"),
                pin("payload", "Byte", "Normal"),
            ],
            Vec::new(),
        )];
        let program = FlowIrProgram {
            modules: vec![FlowIrModule::Function {
                name: "writeValues".to_string(),
                params: Vec::new(),
                returns: Vec::new(),
                cache: None,
                steps: vec![FlowIrStep::Node {
                    id: "sink".to_string(),
                    node_type: "typed_sink".to_string(),
                    args: ["path", "date", "payload"]
                        .into_iter()
                        .map(|name| FlowIrArg {
                            pin: name.to_string(),
                            occurrence: 0,
                            value: string("not typed"),
                        })
                        .collect(),
                    continue_from: None,
                    exec_arms: Vec::new(),
                    anchor: None,
                }],
                anchor: None,
            }],
            ..Default::default()
        };
        let compiled = compile_flow_ir(&program, &catalog);
        assert_eq!(
            compiled
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "IR_INPUT_TYPE")
                .count(),
            3
        );
        assert!(
            !compiled
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "IR_CATALOG_TYPE_UNSUPPORTED" })
        );
    }

    #[test]
    fn unknown_catalog_types_are_not_treated_as_generic() {
        let catalog = vec![node(
            "future_sink",
            vec![pin("value", "FutureNominalType", "Normal")],
            Vec::new(),
        )];
        let program = FlowIrProgram {
            modules: vec![FlowIrModule::Function {
                name: "futureValue".to_string(),
                params: Vec::new(),
                returns: Vec::new(),
                cache: None,
                steps: vec![FlowIrStep::Node {
                    id: "sink".to_string(),
                    node_type: "future_sink".to_string(),
                    args: vec![FlowIrArg {
                        pin: "value".to_string(),
                        occurrence: 0,
                        value: string("unsafe coercion"),
                    }],
                    continue_from: None,
                    exec_arms: Vec::new(),
                    anchor: None,
                }],
                anchor: None,
            }],
            ..Default::default()
        };
        let compiled = compile_flow_ir(&program, &catalog);
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "IR_CATALOG_TYPE_UNSUPPORTED" })
        );
    }

    #[test]
    fn duplicate_input_occurrences_render_in_catalog_order_and_reject_sparse_inputs() {
        let catalog = vec![node(
            "duplicate_inputs",
            vec![
                pin("value", "String", "Normal"),
                pin("value", "String", "Normal"),
            ],
            Vec::new(),
        )];
        let module = |args| FlowIrModule::Function {
            name: "duplicates".to_string(),
            params: Vec::new(),
            returns: Vec::new(),
            cache: None,
            steps: vec![FlowIrStep::Node {
                id: "pair".to_string(),
                node_type: "duplicate_inputs".to_string(),
                args,
                continue_from: None,
                exec_arms: Vec::new(),
                anchor: None,
            }],
            anchor: None,
        };
        let ordered = compile_flow_ir(
            &FlowIrProgram {
                modules: vec![module(vec![
                    FlowIrArg {
                        pin: "value".to_string(),
                        occurrence: 1,
                        value: string("second"),
                    },
                    FlowIrArg {
                        pin: "value".to_string(),
                        occurrence: 0,
                        value: string("first"),
                    },
                ])],
                ..Default::default()
            },
            &catalog,
        );
        assert!(ordered.diagnostics.is_empty(), "{:?}", ordered.diagnostics);
        assert!(
            ordered.flowscript.find("first").unwrap() < ordered.flowscript.find("second").unwrap()
        );

        let sparse = compile_flow_ir(
            &FlowIrProgram {
                modules: vec![module(vec![FlowIrArg {
                    pin: "value".to_string(),
                    occurrence: 1,
                    value: string("second"),
                }])],
                ..Default::default()
            },
            &catalog,
        );
        assert!(
            sparse
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "IR_INPUT_OCCURRENCE_SPARSE" })
        );
    }

    #[test]
    fn multi_execution_outputs_support_explicit_outcome_bodies() {
        let catalog = vec![
            node(
                "events_simple",
                Vec::new(),
                vec![pin("exec_out", "Execution", "Normal")],
            ),
            node(
                "http_request",
                vec![
                    pin("exec_in", "Execution", "Normal"),
                    pin("url", "String", "Normal"),
                ],
                vec![
                    pin("success", "Execution", "Normal"),
                    pin("error", "Execution", "Normal"),
                ],
            ),
            node(
                "log_message",
                vec![pin("message", "String", "Normal")],
                Vec::new(),
            ),
        ];
        let log_step = |id: &str, message: &str| FlowIrStep::Node {
            id: id.to_string(),
            node_type: "log_message".to_string(),
            args: vec![FlowIrArg {
                pin: "message".to_string(),
                occurrence: 0,
                value: string(message),
            }],
            continue_from: None,
            exec_arms: Vec::new(),
            anchor: None,
        };
        let program = FlowIrProgram {
            modules: vec![FlowIrModule::Event {
                name: "request".to_string(),
                node_type: "events_simple".to_string(),
                params: Vec::new(),
                steps: vec![FlowIrStep::Node {
                    id: "request".to_string(),
                    node_type: "http_request".to_string(),
                    args: vec![FlowIrArg {
                        pin: "url".to_string(),
                        occurrence: 0,
                        value: string("https://example.com"),
                    }],
                    continue_from: None,
                    exec_arms: vec![
                        FlowIrExecutionArm {
                            pin: "success".to_string(),
                            steps: vec![log_step("successLog", "ok")],
                        },
                        FlowIrExecutionArm {
                            pin: "error".to_string(),
                            steps: vec![log_step("errorLog", "failed")],
                        },
                    ],
                    anchor: None,
                }],
                anchor: None,
            }],
            ..Default::default()
        };
        let compiled = compile_flow_ir(&program, &catalog);
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        assert!(flow_like_ast::parse(&compiled.flowscript).is_ok());
        assert!(compiled.flowscript.contains("success: {"));
        assert!(compiled.flowscript.contains("error: {"));
        assert_eq!(compiled.module_node_counts["request"], 4);
    }

    #[test]
    fn nested_multi_execution_outcomes_cannot_rejoin_an_outer_continuation() {
        let catalog = vec![
            node(
                "control_branch",
                vec![pin("condition", "Boolean", "Normal")],
                vec![
                    pin("true", "Execution", "Normal"),
                    pin("false", "Execution", "Normal"),
                ],
            ),
            node(
                "http_request",
                vec![
                    pin("exec_in", "Execution", "Normal"),
                    pin("url", "String", "Normal"),
                ],
                vec![
                    pin("success", "Execution", "Normal"),
                    pin("error", "Execution", "Normal"),
                ],
            ),
            node(
                "log_message",
                vec![pin("message", "String", "Normal")],
                Vec::new(),
            ),
            node(
                "stop_execution",
                vec![pin("exec_in", "Execution", "Normal")],
                Vec::new(),
            ),
        ];
        let request = |exec_arms| FlowIrStep::Node {
            id: "request".to_string(),
            node_type: "http_request".to_string(),
            args: vec![FlowIrArg {
                pin: "url".to_string(),
                occurrence: 0,
                value: string("https://example.com"),
            }],
            continue_from: Some("success".to_string()),
            exec_arms,
            anchor: None,
        };
        let program = |request| FlowIrProgram {
            modules: vec![FlowIrModule::Function {
                name: "nestedRequest".to_string(),
                params: Vec::new(),
                returns: Vec::new(),
                cache: None,
                steps: vec![
                    FlowIrStep::If {
                        id: "enabled".to_string(),
                        condition: FlowIrValue::Literal {
                            value: FlowIrLiteral::Boolean(true),
                        },
                        then_steps: vec![request],
                        else_steps: Vec::new(),
                        anchor: None,
                    },
                    FlowIrStep::Node {
                        id: "after".to_string(),
                        node_type: "log_message".to_string(),
                        args: vec![FlowIrArg {
                            pin: "message".to_string(),
                            occurrence: 0,
                            value: string("success only"),
                        }],
                        continue_from: None,
                        exec_arms: Vec::new(),
                        anchor: None,
                    },
                ],
                anchor: None,
            }],
            ..Default::default()
        };

        let unsafe_program = program(request(Vec::new()));
        let compiled = compile_flow_ir(&unsafe_program, &catalog);
        assert!(compiled.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "IR_EXEC_OUTCOME_MUST_TERMINATE"
                && diagnostic.pin.as_deref() == Some("error")
        }));

        let early_return_program = program(request(vec![FlowIrExecutionArm {
            pin: "error".to_string(),
            steps: vec![FlowIrStep::Return { values: Vec::new() }],
        }]));
        let compiled = compile_flow_ir(&early_return_program, &catalog);
        assert!(compiled.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "IR_EXEC_OUTCOME_MUST_TERMINATE"
                && diagnostic.pin.as_deref() == Some("error")
        }));

        let safe_program = program(request(vec![FlowIrExecutionArm {
            pin: "error".to_string(),
            steps: vec![FlowIrStep::Node {
                id: "stopError".to_string(),
                node_type: "stop_execution".to_string(),
                args: Vec::new(),
                continue_from: None,
                exec_arms: Vec::new(),
                anchor: None,
            }],
        }]));
        let compiled = compile_flow_ir(&safe_program, &catalog);
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        assert!(flow_like_ast::parse(&compiled.flowscript).is_ok());
    }

    #[test]
    fn capability_plan_requires_distinct_pins_and_usage() {
        let catalog = vec![node(
            "http_request",
            vec![pin("url", "String", "Normal")],
            Vec::new(),
        )];
        let mut request = FlowCapabilityPlanRequest {
            requirements: vec![FlowCapabilityRequirement {
                id: "request".to_string(),
                intent: "request".to_string(),
                required: true,
                exact_node_type: Some("http_request".to_string()),
                inputs: vec![FlowPinRequirement {
                    names: vec!["url".to_string()],
                    data_type: Some(FlowIrDataType::String),
                    container: Some(FlowIrContainer::Normal),
                    execution: false,
                }],
                outputs: Vec::new(),
            }],
            modules: Vec::new(),
        };
        let plan = plan_flow_capabilities(&request, &catalog);
        assert!(plan.feasible);
        let unused =
            validate_flow_capability_usage(&FlowIrProgram::default(), &request, &plan, &catalog);
        assert_eq!(unused[0].code, "IR_REQUIRED_CAPABILITY_UNUSED");

        let implemented = FlowIrProgram {
            modules: vec![FlowIrModule::Event {
                name: "run".to_string(),
                node_type: "events_simple".to_string(),
                params: Vec::new(),
                steps: vec![FlowIrStep::Node {
                    id: "request".to_string(),
                    node_type: "http_request".to_string(),
                    args: vec![FlowIrArg {
                        pin: "url".to_string(),
                        occurrence: 0,
                        value: string("https://example.com"),
                    }],
                    continue_from: None,
                    exec_arms: Vec::new(),
                    anchor: None,
                }],
                anchor: None,
            }],
            ..Default::default()
        };
        assert!(validate_flow_capability_usage(&implemented, &request, &plan, &catalog).is_empty());

        let duplicate_requirement = request.requirements[0].inputs[0].clone();
        request.requirements[0].inputs.push(duplicate_requirement);
        assert!(!plan_flow_capabilities(&request, &catalog).feasible);
    }

    #[test]
    fn exact_node_type_cannot_override_semantic_mismatch() {
        let catalog = vec![node(
            "log_info",
            vec![pin("message", "String", "Normal")],
            Vec::new(),
        )];
        let request = FlowCapabilityPlanRequest {
            requirements: vec![FlowCapabilityRequirement {
                id: "send_slack".to_string(),
                intent: "send a Slack notification".to_string(),
                required: true,
                exact_node_type: Some("log_info".to_string()),
                inputs: Vec::new(),
                outputs: Vec::new(),
            }],
            modules: Vec::new(),
        };
        let plan = plan_flow_capabilities(&request, &catalog);
        assert!(!plan.feasible);
        assert!(plan.requirements[0].candidates.is_empty());
    }

    #[test]
    fn semantic_discovery_filters_reported_hash_and_regex_decoys_and_requires_selection() {
        let mut regex = node(
            "utils_regex_capture",
            vec![pin("input", "String", "Normal")],
            vec![pin("captures", "String", "Array")],
        );
        regex.friendly_name = "Regex Capture".to_string();
        regex.description =
            "Matches a regular expression and returns all capture groups".to_string();
        let mut sha256 = node(
            "utils_hash_sha256",
            vec![pin("input", "String", "Normal")],
            vec![pin("hash", "String", "Normal")],
        );
        sha256.friendly_name = "SHA-256 Hash".to_string();
        sha256.description = "Computes the SHA-256 hash of the input string".to_string();
        let mut regex_replace = node(
            "utils_string_replace",
            vec![pin("input", "String", "Normal")],
            vec![pin("result", "String", "Normal")],
        );
        regex_replace.description =
            "Replaces a regex pattern; replacement supports capture groups".to_string();
        let catalog = vec![
            regex,
            sha256,
            regex_replace,
            node(
                "ai_generative_history_from_string",
                vec![pin("message", "String", "Normal")],
                vec![pin("history", "Struct", "Normal")],
            ),
            node(
                "ai_generative_llm_response_from_string",
                vec![pin("content", "String", "Normal")],
                vec![pin("response", "Struct", "Normal")],
            ),
            node(
                "selector_to_string",
                vec![pin("selector", "Struct", "Normal")],
                vec![pin("value", "String", "Normal")],
            ),
        ];
        let request = FlowCapabilityPlanRequest {
            requirements: vec![
                FlowCapabilityRequirement {
                    id: "string_match".to_string(),
                    intent: "regular expression match captures groups from string".to_string(),
                    required: true,
                    exact_node_type: None,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                },
                FlowCapabilityRequirement {
                    id: "hash".to_string(),
                    intent: "cryptographic hash SHA256 string to deterministic key".to_string(),
                    required: true,
                    exact_node_type: None,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                },
            ],
            modules: Vec::new(),
        };

        let plan = plan_flow_capabilities(&request, &catalog);
        assert!(!plan.feasible, "discovery is not an accepted exact plan");
        assert!(
            plan.requirements
                .iter()
                .all(|resolution| resolution.supported && resolution.selection_required)
        );
        assert_eq!(
            plan.requirements[0]
                .candidates
                .iter()
                .map(|candidate| candidate.node_type.as_str())
                .collect::<Vec<_>>(),
            vec!["utils_regex_capture"]
        );
        assert_eq!(
            plan.requirements[1]
                .candidates
                .iter()
                .map(|candidate| candidate.node_type.as_str())
                .collect::<Vec<_>>(),
            vec!["utils_hash_sha256"]
        );
        assert!(plan.requirements.iter().all(|resolution| {
            resolution
                .missing
                .iter()
                .any(|missing| missing.starts_with("selection_required:"))
        }));

        let mut wrong_exact = request.clone();
        wrong_exact.requirements[0].exact_node_type =
            Some("ai_generative_history_from_string".to_string());
        wrong_exact.requirements[1].exact_node_type = Some("selector_to_string".to_string());
        let wrong_plan = plan_flow_capabilities(&wrong_exact, &catalog);
        assert!(!wrong_plan.feasible);
        assert!(
            wrong_plan
                .requirements
                .iter()
                .all(|resolution| !resolution.supported && resolution.candidates.is_empty())
        );

        let mut selected = request.clone();
        selected.requirements[0].exact_node_type = Some("utils_regex_capture".to_string());
        selected.requirements[1].exact_node_type = Some("utils_hash_sha256".to_string());
        assert!(plan_flow_capabilities(&selected, &catalog).feasible);

        let optional = FlowCapabilityPlanRequest {
            requirements: vec![FlowCapabilityRequirement {
                required: false,
                ..request.requirements[1].clone()
            }],
            modules: Vec::new(),
        };
        let optional_plan = plan_flow_capabilities(&optional, &catalog);
        assert!(optional_plan.feasible);
        assert!(optional_plan.requirements[0].supported);
        assert!(!optional_plan.requirements[0].selection_required);
    }

    #[test]
    fn semantic_protocol_operations_reject_connect_disconnect_and_send_decoys() {
        let mut imap_connect = node("email_imap_connect", Vec::new(), Vec::new());
        imap_connect.friendly_name = "IMAP Connect".to_string();
        imap_connect.description = "Connects to an IMAP server".to_string();
        let mut imap_disconnect = node("email_imap_disconnect", Vec::new(), Vec::new());
        imap_disconnect.friendly_name = "IMAP Disconnect".to_string();
        imap_disconnect.description = "Closes and disconnects an IMAP session".to_string();
        let mut smtp_connect = node("email_smtp_connect", Vec::new(), Vec::new());
        smtp_connect.friendly_name = "SMTP Connect".to_string();
        smtp_connect.description = "Connects to an SMTP server".to_string();
        let mut smtp_send = node("email_smtp_send", Vec::new(), Vec::new());
        smtp_send.friendly_name = "SMTP Send".to_string();
        smtp_send.description = "Sends an email through an SMTP connection".to_string();
        let catalog = vec![imap_connect, imap_disconnect, smtp_connect, smtp_send];

        for (intent, expected) in [
            ("connect to IMAP server", "email_imap_connect"),
            (
                "close disconnect IMAP connection cleanly",
                "email_imap_disconnect",
            ),
            ("send SMTP email", "email_smtp_send"),
        ] {
            let discovery = FlowCapabilityPlanRequest {
                requirements: vec![FlowCapabilityRequirement {
                    id: "mail_operation".to_string(),
                    intent: intent.to_string(),
                    required: true,
                    exact_node_type: None,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                }],
                modules: Vec::new(),
            };
            let plan = plan_flow_capabilities(&discovery, &catalog);
            assert!(!plan.feasible);
            assert!(plan.requirements[0].selection_required);
            assert_eq!(plan.requirements[0].candidates.len(), 1, "{intent}");
            assert_eq!(plan.requirements[0].candidates[0].node_type, expected);

            let wrong = catalog
                .iter()
                .map(|metadata| metadata.name.as_str())
                .find(|node_type| *node_type != expected)
                .unwrap();
            let mut wrong_request = discovery.clone();
            wrong_request.requirements[0].exact_node_type = Some(wrong.to_string());
            let wrong_plan = plan_flow_capabilities(&wrong_request, &catalog);
            assert!(
                !wrong_plan.feasible,
                "{intent} accepted wrong exact {wrong}"
            );
            assert!(!wrong_plan.requirements[0].supported);
            assert!(
                wrong_plan.requirements[0]
                    .missing
                    .iter()
                    .any(|missing| { missing.starts_with("semantic anchors not satisfied:") })
            );

            let mut selected = discovery;
            selected.requirements[0].exact_node_type = Some(expected.to_string());
            assert!(plan_flow_capabilities(&selected, &catalog).feasible);
        }
    }

    #[test]
    fn exact_semantic_selection_and_pin_contract_are_both_required() {
        let mut sha256 = node(
            "utils_hash_sha256",
            vec![pin("input", "String", "Normal")],
            vec![pin("hash", "String", "Normal")],
        );
        sha256.friendly_name = "SHA-256 Hash".to_string();
        sha256.description = "Computes the SHA-256 digest".to_string();
        let mut selector = node(
            "selector_to_string",
            vec![pin("selector", "Struct", "Normal")],
            vec![pin("value", "String", "Normal")],
        );
        selector.description = "Converts a selector to its string representation".to_string();
        let catalog = vec![selector, sha256];
        let mut request = FlowCapabilityPlanRequest {
            requirements: vec![FlowCapabilityRequirement {
                id: "hash".to_string(),
                intent: "cryptographic SHA256 hash digest".to_string(),
                required: true,
                exact_node_type: Some("selector_to_string".to_string()),
                inputs: vec![FlowPinRequirement {
                    names: vec!["input".to_string()],
                    data_type: Some(FlowIrDataType::String),
                    container: Some(FlowIrContainer::Normal),
                    execution: false,
                }],
                outputs: vec![FlowPinRequirement {
                    names: vec!["hash".to_string()],
                    data_type: Some(FlowIrDataType::String),
                    container: Some(FlowIrContainer::Normal),
                    execution: false,
                }],
            }],
            modules: Vec::new(),
        };
        assert!(!plan_flow_capabilities(&request, &catalog).feasible);

        request.requirements[0].exact_node_type = Some("utils_hash_sha256".to_string());
        assert!(plan_flow_capabilities(&request, &catalog).feasible);

        request.requirements[0].outputs[0].names = vec!["missing_digest".to_string()];
        assert!(!plan_flow_capabilities(&request, &catalog).feasible);
    }

    #[test]
    fn required_capability_must_be_reachable_from_an_event() {
        let catalog = vec![
            node("events_simple", Vec::new(), Vec::new()),
            node("slack_send", Vec::new(), Vec::new()),
        ];
        let request = FlowCapabilityPlanRequest {
            requirements: vec![FlowCapabilityRequirement {
                id: "notify_slack".to_string(),
                intent: "send Slack notification".to_string(),
                required: true,
                exact_node_type: Some("slack_send".to_string()),
                inputs: Vec::new(),
                outputs: Vec::new(),
            }],
            modules: Vec::new(),
        };
        let plan = plan_flow_capabilities(&request, &catalog);
        assert!(plan.feasible);
        let helper = FlowIrModule::Function {
            name: "notify".to_string(),
            params: Vec::new(),
            returns: Vec::new(),
            cache: None,
            steps: vec![FlowIrStep::Node {
                id: "slack".to_string(),
                node_type: "slack_send".to_string(),
                args: Vec::new(),
                continue_from: None,
                exec_arms: Vec::new(),
                anchor: None,
            }],
            anchor: None,
        };
        let event = |steps| FlowIrModule::Event {
            name: "run".to_string(),
            node_type: "events_simple".to_string(),
            params: Vec::new(),
            steps,
            anchor: None,
        };
        let unreachable = FlowIrProgram {
            modules: vec![helper.clone(), event(Vec::new())],
            ..Default::default()
        };
        assert_eq!(
            validate_flow_capability_usage(&unreachable, &request, &plan, &catalog)[0].code,
            "IR_REQUIRED_CAPABILITY_UNUSED"
        );
        let called = FlowIrProgram {
            modules: vec![
                helper,
                event(vec![FlowIrStep::CallFunction {
                    id: "call_notify".to_string(),
                    function: "notify".to_string(),
                    args: Vec::new(),
                    anchor: None,
                }]),
            ],
            ..Default::default()
        };
        assert!(validate_flow_capability_usage(&called, &request, &plan, &catalog).is_empty());
    }

    #[test]
    fn nominal_interfaces_round_trip_and_missing_returns_are_diagnostic() {
        let payload_type = FlowIrType {
            data_type: FlowIrDataType::Struct,
            container: FlowIrContainer::Normal,
            interface: Some("Payload".to_string()),
        };
        let nominal = compile_flow_ir(
            &FlowIrProgram {
                interfaces: vec![FlowIrInterface {
                    name: "Payload".to_string(),
                    fields: vec![FlowIrInterfaceField {
                        name: "message".to_string(),
                        value_type: FlowIrType::scalar(FlowIrDataType::String),
                        optional: false,
                        default: None,
                    }],
                }],
                variables: vec![FlowIrVariable {
                    name: "payload".to_string(),
                    value_type: payload_type,
                    default: None,
                    exposed: false,
                    secret: false,
                    editable: true,
                    runtime_configured: false,
                    category: None,
                    description: None,
                    anchor: None,
                }],
                ..Default::default()
            },
            &[],
        );
        assert!(nominal.diagnostics.is_empty(), "{:?}", nominal.diagnostics);
        assert!(nominal.flowscript.contains("payload: Payload"));

        let missing_return = compile_flow_ir(
            &FlowIrProgram {
                modules: vec![FlowIrModule::Function {
                    name: "bad name".to_string(),
                    params: Vec::new(),
                    returns: vec![FlowIrParam {
                        name: "value".to_string(),
                        value_type: FlowIrType::scalar(FlowIrDataType::String),
                    }],
                    cache: None,
                    steps: Vec::new(),
                    anchor: None,
                }],
                ..Default::default()
            },
            &[],
        );
        assert!(
            missing_return
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "IR_MODULE_NAME_INVALID" })
        );
        assert!(
            missing_return
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "IR_RETURN_MISSING" })
        );
    }

    #[test]
    fn interface_date_fields_render_with_the_flowscript_date_type() {
        let compiled = compile_flow_ir(
            &FlowIrProgram {
                interfaces: vec![FlowIrInterface {
                    name: "AuditEntry".to_string(),
                    fields: vec![
                        FlowIrInterfaceField {
                            name: "createdAt".to_string(),
                            value_type: FlowIrType::scalar(FlowIrDataType::Date),
                            optional: false,
                            default: None,
                        },
                        FlowIrInterfaceField {
                            name: "previousRuns".to_string(),
                            value_type: FlowIrType {
                                data_type: FlowIrDataType::Date,
                                container: FlowIrContainer::Array,
                                interface: None,
                            },
                            optional: true,
                            default: None,
                        },
                    ],
                }],
                ..Default::default()
            },
            &[],
        );

        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        assert!(compiled.flowscript.contains("createdAt: Date;"));
        assert!(compiled.flowscript.contains("previousRuns?: Date[];"));
    }

    #[test]
    fn selected_execution_arm_exports_values_to_its_following_tail() {
        let catalog = vec![
            node(
                "http_request",
                vec![
                    pin("exec_in", "Execution", "Normal"),
                    pin("url", "String", "Normal"),
                ],
                vec![
                    pin("success", "Execution", "Normal"),
                    pin("error", "Execution", "Normal"),
                ],
            ),
            node(
                "string_format",
                vec![pin("format_string", "String", "Normal")],
                vec![pin("formatted_string", "String", "Normal")],
            ),
            node(
                "log_info",
                vec![pin("message", "Generic", "Normal")],
                Vec::new(),
            ),
        ];
        let program = FlowIrProgram {
            modules: vec![FlowIrModule::Function {
                name: "request".to_string(),
                params: Vec::new(),
                returns: Vec::new(),
                cache: None,
                steps: vec![
                    FlowIrStep::Node {
                        id: "request".to_string(),
                        node_type: "http_request".to_string(),
                        args: vec![FlowIrArg {
                            pin: "url".to_string(),
                            occurrence: 0,
                            value: string("https://example.com"),
                        }],
                        continue_from: Some("success".to_string()),
                        exec_arms: vec![FlowIrExecutionArm {
                            pin: "success".to_string(),
                            steps: vec![FlowIrStep::Node {
                                id: "message".to_string(),
                                node_type: "string_format".to_string(),
                                args: vec![FlowIrArg {
                                    pin: "format_string".to_string(),
                                    occurrence: 0,
                                    value: string("ok"),
                                }],
                                continue_from: None,
                                exec_arms: Vec::new(),
                                anchor: None,
                            }],
                        }],
                        anchor: None,
                    },
                    FlowIrStep::Node {
                        id: "after".to_string(),
                        node_type: "log_info".to_string(),
                        args: vec![FlowIrArg {
                            pin: "message".to_string(),
                            occurrence: 0,
                            value: FlowIrValue::Output {
                                step: "message".to_string(),
                                pin: "formatted_string".to_string(),
                                occurrence: 0,
                            },
                        }],
                        continue_from: None,
                        exec_arms: Vec::new(),
                        anchor: None,
                    },
                ],
                anchor: None,
            }],
            ..Default::default()
        };
        let compiled = compile_flow_ir(&program, &catalog);
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        assert!(
            compiled.flowscript.contains("message.formattedString"),
            "{}",
            compiled.flowscript
        );
    }

    #[test]
    fn dynamic_placeholder_inputs_and_function_refs_compile_through_typed_ir() {
        let catalog = vec![
            node(
                "events_simple",
                Vec::new(),
                vec![pin("exec_out", "Execution", "Normal")],
            ),
            node(
                "string_format",
                vec![pin("format_string", "String", "Normal")],
                vec![pin("string", "String", "Normal")],
            ),
            node(
                "agent_register_function_tools",
                vec![
                    pin("exec_in", "Execution", "Normal"),
                    pin("agent_in", "Generic", "Normal"),
                ],
                vec![pin("exec_out", "Execution", "Normal")],
            ),
        ];
        let format_step = FlowIrStep::Node {
            id: "formatted".to_string(),
            node_type: "string_format".to_string(),
            args: vec![
                FlowIrArg {
                    pin: "format_string".to_string(),
                    occurrence: 0,
                    value: string("Hello {name}"),
                },
                FlowIrArg {
                    pin: "name".to_string(),
                    occurrence: 0,
                    value: string("Felix"),
                },
            ],
            continue_from: None,
            exec_arms: Vec::new(),
            anchor: None,
        };
        let program = FlowIrProgram {
            modules: vec![
                FlowIrModule::Function {
                    name: "fetchPage".to_string(),
                    params: Vec::new(),
                    returns: vec![FlowIrParam {
                        name: "message".to_string(),
                        value_type: FlowIrType::scalar(FlowIrDataType::String),
                    }],
                    cache: None,
                    steps: vec![
                        format_step.clone(),
                        FlowIrStep::Return {
                            values: vec![FlowIrValue::Output {
                                step: "formatted".to_string(),
                                pin: "string".to_string(),
                                occurrence: 0,
                            }],
                        },
                    ],
                    anchor: None,
                },
                FlowIrModule::Event {
                    name: "registerTools".to_string(),
                    node_type: "events_simple".to_string(),
                    params: Vec::new(),
                    steps: vec![FlowIrStep::Node {
                        id: "register".to_string(),
                        node_type: "agent_register_function_tools".to_string(),
                        args: vec![
                            FlowIrArg {
                                pin: "agent_in".to_string(),
                                occurrence: 0,
                                value: string("agent"),
                            },
                            FlowIrArg {
                                pin: "tools".to_string(),
                                occurrence: 0,
                                value: FlowIrValue::FunctionRefs {
                                    functions: vec!["fetchPage".to_string()],
                                },
                            },
                        ],
                        continue_from: None,
                        exec_arms: Vec::new(),
                        anchor: None,
                    }],
                    anchor: None,
                },
            ],
            ..Default::default()
        };
        let compiled = compile_flow_ir(&program, &catalog);
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        assert!(compiled.flowscript.contains("name: \"Felix\""));
        assert!(compiled.flowscript.contains("tools: [fetchPage]"));
        assert!(flow_like_ast::parse(&compiled.flowscript).is_ok());

        let mut typo_program = program;
        let FlowIrModule::Function { steps, .. } = &mut typo_program.modules[0] else {
            unreachable!()
        };
        let FlowIrStep::Node { args, .. } = &mut steps[0] else {
            unreachable!()
        };
        args[1].pin = "typo".to_string();
        let typo = compile_flow_ir(&typo_program, &catalog);
        assert!(
            typo.diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "IR_INPUT_PIN_MISSING")
        );
    }

    #[test]
    fn capability_execution_usage_tracks_real_consumers_and_function_boundaries() {
        let catalog = vec![
            node(
                "events_simple",
                Vec::new(),
                vec![pin("exec_out", "Execution", "Normal")],
            ),
            node(
                "impure_action",
                vec![pin("exec_in", "Execution", "Normal")],
                vec![pin("exec_out", "Execution", "Normal")],
            ),
            node(
                "multi_action",
                vec![pin("exec_in", "Execution", "Normal")],
                vec![
                    pin("success", "Execution", "Normal"),
                    pin("error", "Execution", "Normal"),
                ],
            ),
        ];
        let event_request = FlowCapabilityPlanRequest {
            requirements: vec![FlowCapabilityRequirement {
                id: "event_exec".to_string(),
                intent: "event execution".to_string(),
                required: true,
                exact_node_type: Some("events_simple".to_string()),
                inputs: Vec::new(),
                outputs: vec![FlowPinRequirement {
                    names: vec!["exec_out".to_string()],
                    data_type: None,
                    container: None,
                    execution: true,
                }],
            }],
            modules: Vec::new(),
        };
        let event_plan = plan_flow_capabilities(&event_request, &catalog);
        let helper = FlowIrModule::Function {
            name: "doWork".to_string(),
            params: Vec::new(),
            returns: Vec::new(),
            cache: None,
            steps: vec![FlowIrStep::Node {
                id: "work".to_string(),
                node_type: "impure_action".to_string(),
                args: Vec::new(),
                continue_from: None,
                exec_arms: Vec::new(),
                anchor: None,
            }],
            anchor: None,
        };
        let event = |steps| FlowIrModule::Event {
            name: "run".to_string(),
            node_type: "events_simple".to_string(),
            params: Vec::new(),
            steps,
            anchor: None,
        };
        let impure_call = FlowIrProgram {
            modules: vec![
                helper,
                event(vec![FlowIrStep::CallFunction {
                    id: "call".to_string(),
                    function: "doWork".to_string(),
                    args: Vec::new(),
                    anchor: None,
                }]),
            ],
            ..Default::default()
        };
        assert!(
            validate_flow_capability_usage(&impure_call, &event_request, &event_plan, &catalog)
                .is_empty()
        );
        let local_only = FlowIrProgram {
            modules: vec![event(vec![FlowIrStep::Assign {
                target: "local".to_string(),
                value: string("value"),
            }])],
            ..Default::default()
        };
        assert_eq!(
            validate_flow_capability_usage(&local_only, &event_request, &event_plan, &catalog)[0]
                .code,
            "IR_REQUIRED_CAPABILITY_UNUSED"
        );

        let multi_request = FlowCapabilityPlanRequest {
            requirements: ["success", "error"]
                .into_iter()
                .map(|pin_name| FlowCapabilityRequirement {
                    id: pin_name.to_string(),
                    intent: pin_name.to_string(),
                    required: true,
                    exact_node_type: Some("multi_action".to_string()),
                    inputs: Vec::new(),
                    outputs: vec![FlowPinRequirement {
                        names: vec![pin_name.to_string()],
                        data_type: None,
                        container: None,
                        execution: true,
                    }],
                })
                .collect(),
            modules: Vec::new(),
        };
        let multi_plan = plan_flow_capabilities(&multi_request, &catalog);
        let final_multi = FlowIrProgram {
            modules: vec![FlowIrModule::Function {
                name: "request".to_string(),
                params: Vec::new(),
                returns: Vec::new(),
                cache: None,
                steps: vec![FlowIrStep::Node {
                    id: "multi".to_string(),
                    node_type: "multi_action".to_string(),
                    args: Vec::new(),
                    continue_from: None,
                    exec_arms: Vec::new(),
                    anchor: None,
                }],
                anchor: None,
            }],
            ..Default::default()
        };
        assert!(
            validate_flow_capability_usage(&final_multi, &multi_request, &multi_plan, &catalog)
                .is_empty()
        );
    }

    #[test]
    fn duplicate_typed_anchors_fail_closed() {
        let catalog = vec![node("pure", Vec::new(), Vec::new())];
        let step = |id: &str| FlowIrStep::Node {
            id: id.to_string(),
            node_type: "pure".to_string(),
            args: Vec::new(),
            continue_from: None,
            exec_arms: Vec::new(),
            anchor: Some("same-node".to_string()),
        };
        let program = FlowIrProgram {
            modules: vec![FlowIrModule::Function {
                name: "duplicates".to_string(),
                params: Vec::new(),
                returns: Vec::new(),
                cache: None,
                steps: vec![step("one"), step("two")],
                anchor: None,
            }],
            ..Default::default()
        };
        let compiled = compile_flow_ir(&program, &catalog);
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "IR_DUPLICATE_ANCHOR")
        );
    }

    #[test]
    fn unsafe_returns_shadowing_dynamic_composites_and_terminal_tails_fail_closed() {
        let catalog = vec![
            node(
                "log_info",
                vec![pin("message", "Generic", "Normal")],
                Vec::new(),
            ),
            node(
                "stop_execution",
                vec![pin("exec_in", "Execution", "Normal")],
                Vec::new(),
            ),
        ];
        let program = FlowIrProgram {
            modules: vec![FlowIrModule::Function {
                name: "unsafeFlow".to_string(),
                params: vec![FlowIrParam {
                    name: "message".to_string(),
                    value_type: FlowIrType::scalar(FlowIrDataType::String),
                }],
                returns: Vec::new(),
                cache: None,
                steps: vec![
                    FlowIrStep::Return { values: Vec::new() },
                    FlowIrStep::Node {
                        id: "message".to_string(),
                        node_type: "log_info".to_string(),
                        args: vec![FlowIrArg {
                            pin: "message".to_string(),
                            occurrence: 0,
                            value: FlowIrValue::List {
                                items: vec![FlowIrValue::Ref {
                                    name: "message".to_string(),
                                }],
                            },
                        }],
                        continue_from: None,
                        exec_arms: Vec::new(),
                        anchor: None,
                    },
                    FlowIrStep::Node {
                        id: "stop".to_string(),
                        node_type: "stop_execution".to_string(),
                        args: Vec::new(),
                        continue_from: None,
                        exec_arms: Vec::new(),
                        anchor: None,
                    },
                    FlowIrStep::Node {
                        id: "never".to_string(),
                        node_type: "log_info".to_string(),
                        args: Vec::new(),
                        continue_from: None,
                        exec_arms: Vec::new(),
                        anchor: None,
                    },
                ],
                anchor: None,
            }],
            ..Default::default()
        };
        let compiled = compile_flow_ir(&program, &catalog);
        for code in [
            "IR_FUNCTION_RETURN_POSITION",
            "IR_STEP_ID_INVALID",
            "IR_DYNAMIC_COMPOSITE_UNSUPPORTED",
            "IR_UNREACHABLE_STEP",
        ] {
            assert!(
                compiled
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "missing {code}: {:?}",
                compiled.diagnostics
            );
        }
    }

    #[test]
    fn capability_plan_detects_missing_custom_header_input() {
        let catalog = vec![node(
            "email_smtp_send",
            vec![
                pin("connection", "Struct", "Normal"),
                pin("to", "String", "Normal"),
                pin("subject", "String", "Normal"),
            ],
            vec![pin("message_id", "String", "Normal")],
        )];
        let request = FlowCapabilityPlanRequest {
            requirements: vec![FlowCapabilityRequirement {
                id: "thread_headers".to_string(),
                intent: "smtp send reply with custom headers".to_string(),
                required: true,
                exact_node_type: Some("email_smtp_send".to_string()),
                inputs: vec![FlowPinRequirement {
                    names: vec!["headers".to_string(), "custom_headers".to_string()],
                    data_type: None,
                    container: None,
                    execution: false,
                }],
                outputs: Vec::new(),
            }],
            modules: Vec::new(),
        };
        let plan = plan_flow_capabilities(&request, &catalog);
        assert!(!plan.feasible);
        assert!(!plan.requirements[0].supported);
    }
}
