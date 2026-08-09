//! Machine-readable FlowScript reconciliation diagnostics.
//!
//! Reconciliation historically exposed diagnostics as `Vec<String>`. A number of callers still
//! construct and inspect [`ReconcileResult`] directly, so replacing that field would be a breaking
//! change. This module provides a structured, serializable sidecar derived at the result boundary:
//! legacy strings remain authoritative and callers can opt into stable codes and metadata without
//! coordinating a flag day.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::reconcile::ReconcileResult;

/// Stable diagnostic category. Variant names are serialized as public `FS_*` codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlowScriptDiagnosticCode {
    FsParseError,
    FsCatalogDeclarationNotFound,
    FsCatalogDeclarationAmbiguous,
    FsCatalogMetadataRequired,
    FsTypeMismatch,
    FsTypeAmbiguous,
    FsUnknownInputPin,
    FsStructFieldUnknown,
    FsUnresolvedArgument,
    FsOutputPinUnresolved,
    FsExecutionPolicyAmbiguous,
    FsExecutionEntryUnconnected,
    FsExecutionChainAmbiguous,
    FsBranchLoweringUnsupported,
    FsBranchArmPinUnknown,
    FsNodeLimitExceeded,
    FsProhibitedNode,
    FsEventEmpty,
    FsHelperEmpty,
    FsHelperNoObservableEffect,
    FsHelperExecutionEntryUnconnected,
    FsHelperExecutionTailUnconnected,
    FsFunctionReturnMismatch,
    FsVariableUnresolved,
    FsAnchorUnresolved,
    FsRequestAcceptanceIncomplete,
    FsRequestAcceptanceForbidden,
    FsRequestApprovalInvalid,
    FsReconcileUnsupported,
}

impl FlowScriptDiagnosticCode {
    /// Public string representation used in telemetry and repair prompts.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FsParseError => "FS_PARSE_ERROR",
            Self::FsCatalogDeclarationNotFound => "FS_CATALOG_DECLARATION_NOT_FOUND",
            Self::FsCatalogDeclarationAmbiguous => "FS_CATALOG_DECLARATION_AMBIGUOUS",
            Self::FsCatalogMetadataRequired => "FS_CATALOG_METADATA_REQUIRED",
            Self::FsTypeMismatch => "FS_TYPE_MISMATCH",
            Self::FsTypeAmbiguous => "FS_TYPE_AMBIGUOUS",
            Self::FsUnknownInputPin => "FS_UNKNOWN_INPUT_PIN",
            Self::FsStructFieldUnknown => "FS_STRUCT_FIELD_UNKNOWN",
            Self::FsUnresolvedArgument => "FS_UNRESOLVED_ARGUMENT",
            Self::FsOutputPinUnresolved => "FS_OUTPUT_PIN_UNRESOLVED",
            Self::FsExecutionPolicyAmbiguous => "FS_EXECUTION_POLICY_AMBIGUOUS",
            Self::FsExecutionEntryUnconnected => "FS_EXECUTION_ENTRY_UNCONNECTED",
            Self::FsExecutionChainAmbiguous => "FS_EXECUTION_CHAIN_AMBIGUOUS",
            Self::FsBranchLoweringUnsupported => "FS_BRANCH_LOWERING_UNSUPPORTED",
            Self::FsBranchArmPinUnknown => "FS_BRANCH_ARM_PIN_UNKNOWN",
            Self::FsNodeLimitExceeded => "FS_NODE_LIMIT_EXCEEDED",
            Self::FsProhibitedNode => "FS_PROHIBITED_NODE",
            Self::FsEventEmpty => "FS_EVENT_EMPTY",
            Self::FsHelperEmpty => "FS_HELPER_EMPTY",
            Self::FsHelperNoObservableEffect => "FS_HELPER_NO_OBSERVABLE_EFFECT",
            Self::FsHelperExecutionEntryUnconnected => "FS_HELPER_EXECUTION_ENTRY_UNCONNECTED",
            Self::FsHelperExecutionTailUnconnected => "FS_HELPER_EXECUTION_TAIL_UNCONNECTED",
            Self::FsFunctionReturnMismatch => "FS_FUNCTION_RETURN_MISMATCH",
            Self::FsVariableUnresolved => "FS_VARIABLE_UNRESOLVED",
            Self::FsAnchorUnresolved => "FS_ANCHOR_UNRESOLVED",
            Self::FsRequestAcceptanceIncomplete => "FS_REQUEST_ACCEPTANCE_INCOMPLETE",
            Self::FsRequestAcceptanceForbidden => "FS_REQUEST_ACCEPTANCE_FORBIDDEN",
            Self::FsRequestApprovalInvalid => "FS_REQUEST_APPROVAL_INVALID",
            Self::FsReconcileUnsupported => "FS_RECONCILE_UNSUPPORTED",
        }
    }
}

/// Compiler phase that emitted (or best explains) a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowScriptDiagnosticPhase {
    Parse,
    CatalogResolution,
    TypeCheck,
    Lowering,
    ExecutionWiring,
    Validation,
}

/// One 1-based source coordinate plus its UTF-8 byte offset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowScriptSourcePosition {
    pub line: usize,
    pub column: usize,
    /// UTF-8 byte offset when source text was supplied to the classifier. Parser errors currently
    /// expose only line/column, so their offset is absent instead of using a misleading sentinel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

/// Best available source extent. Parse errors point at one coordinate; diagnostics enriched with
/// source text cover the declaration/call token that could be located.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowScriptSourceSpan {
    pub start: FlowScriptSourcePosition,
    pub end: FlowScriptSourcePosition,
}

/// Safe repair guidance. It deliberately describes a supported operation rather than inventing a
/// replacement declaration that may not exist in the active catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowScriptDiagnosticFix {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration_search: Option<String>,
    /// Exact, compact declarations from the active live catalog that are safe to use for this
    /// repair. A resolved call contributes its one authoritative signature; an unresolved call
    /// may contribute a small, deterministically ranked candidate set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub catalog_declarations: Vec<String>,
    /// Exact declarations for a bounded set of catalog-declared companion nodes. These make
    /// structural repairs (such as connect -> inbox -> list -> fetch) possible without treating
    /// a local pin rename as a complete fix.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub companion_declarations: Vec<String>,
}

/// Structured sidecar for one root diagnostic or one deduplicated diagnostic group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowScriptDiagnostic {
    /// Deterministic id derived from the stable code and semantic grouping fields.
    pub id: String,
    pub code: FlowScriptDiagnosticCode,
    pub phase: FlowScriptDiagnosticPhase,
    /// Original legacy text. This keeps the sidecar useful to callers during migration.
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_span: Option<FlowScriptSourceSpan>,
    /// Every source site of this group's subject, in source order, capped at [`MAX_SUBJECT_SPANS`].
    /// `source_span` is `spans[0]`.
    ///
    /// These are *all occurrences of the named subject*, not per-message attributions: the
    /// reconcile diagnostic channel is `Vec<String>` and carries no position, so which of
    /// `occurrences` messages belongs to which site is unknown. When `spans.len() == occurrences`
    /// the correspondence is 1:1 in source order; otherwise these are the candidate set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<FlowScriptSourceSpan>,
    /// Located sites beyond those carried in `spans`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub additional_sites: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ast_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<FlowScriptDiagnosticFix>,
    /// Id of a preceding root diagnostic when this message is a conservative, same-subject
    /// execution-wiring cascade.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caused_by: Option<String>,
    /// Count of legacy diagnostics represented by this group.
    pub occurrences: usize,
    /// Distinct legacy wordings folded into this semantic group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_messages: Vec<String>,
}

impl ReconcileResult {
    /// Return deduplicated, machine-readable diagnostics while preserving [`Self::diagnostics`]
    /// unchanged for legacy callers.
    pub fn structured_diagnostics(&self) -> Vec<FlowScriptDiagnostic> {
        structure_reconcile_diagnostics(&self.diagnostics, None)
    }

    /// As [`Self::structured_diagnostics`], with best-effort source extents for diagnostics whose
    /// call, event, or function name can be located in `source`.
    pub fn structured_diagnostics_for_source(&self, source: &str) -> Vec<FlowScriptDiagnostic> {
        structure_reconcile_diagnostics(&self.diagnostics, Some(source))
    }

    /// Only independently actionable roots. Same-subject execution cascades remain available from
    /// [`Self::structured_diagnostics`] through their `caused_by` link.
    pub fn root_cause_diagnostics(&self) -> Vec<FlowScriptDiagnostic> {
        self.structured_diagnostics()
            .into_iter()
            .filter(|diagnostic| diagnostic.caused_by.is_none())
            .collect()
    }
}

/// Classify legacy reconcile strings, deduplicate semantic repeats, and conservatively link
/// execution-wiring cascades to an earlier error about the same declaration or AST scope.
pub fn structure_reconcile_diagnostics(
    diagnostics: &[String],
    source: Option<&str>,
) -> Vec<FlowScriptDiagnostic> {
    let mut grouped = Vec::<FlowScriptDiagnostic>::new();
    let mut by_key = HashMap::<String, usize>::new();

    for message in diagnostics {
        let mut diagnostic = classify(message);
        if let Some(source) = source
            && diagnostic.source_span.is_none()
        {
            let (spans, additional) = locate_subject_sites(source, &diagnostic);
            diagnostic.source_span = spans.first().cloned();
            diagnostic.spans = spans;
            diagnostic.additional_sites = additional;
        }

        let key = semantic_group_key(&diagnostic);
        if let Some(index) = by_key.get(&key).copied() {
            let existing = &mut grouped[index];
            existing.occurrences += 1;
            if existing.message != diagnostic.message
                && !existing.related_messages.contains(&diagnostic.message)
            {
                existing.related_messages.push(diagnostic.message);
            }
            continue;
        }

        diagnostic.id = diagnostic_id(&key);
        by_key.insert(key, grouped.len());
        grouped.push(diagnostic);
    }

    link_execution_cascades(&mut grouped);
    grouped
}

fn classify(message: &str) -> FlowScriptDiagnostic {
    let ticks = backtick_values(message);
    let mut diagnostic = FlowScriptDiagnostic {
        id: String::new(),
        code: FlowScriptDiagnosticCode::FsReconcileUnsupported,
        phase: FlowScriptDiagnosticPhase::Lowering,
        message: message.to_string(),
        source_span: None,
        spans: Vec::new(),
        additional_sites: 0,
        ast_path: None,
        scope: None,
        expected: None,
        actual: None,
        declaration: None,
        pin: None,
        fix: None,
        caused_by: None,
        occurrences: 1,
        related_messages: Vec::new(),
    };

    if message.starts_with("FlowScript parse error") {
        diagnostic.code = FlowScriptDiagnosticCode::FsParseError;
        diagnostic.phase = FlowScriptDiagnosticPhase::Parse;
        diagnostic.source_span = parse_error_span(message);
        diagnostic.expected = expected_from_parse_message(message);
        diagnostic.actual = actual_from_parse_message(message);
        diagnostic.fix = fix("Correct the syntax at the reported source position.", None);
    } else if message.contains("does not match a catalog declaration")
        || message.contains("did not match the catalog")
        || message.contains("is missing from the catalog")
    {
        diagnostic.code = FlowScriptDiagnosticCode::FsCatalogDeclarationNotFound;
        diagnostic.phase = FlowScriptDiagnosticPhase::CatalogResolution;
        diagnostic.declaration = ticks.first().cloned();
        diagnostic.ast_path = diagnostic
            .declaration
            .as_deref()
            .map(|name| format!("calls.{name}"));
        diagnostic.expected = Some("an exact declaration present in the active catalog".into());
        diagnostic.actual = diagnostic.declaration.clone();
        diagnostic.fix = fix(
            "Use the exact catalog declaration name and signature.",
            diagnostic.declaration.clone(),
        );
    } else if message.contains("FlowScript call") && message.contains("is ambiguous; matched") {
        diagnostic.code = FlowScriptDiagnosticCode::FsCatalogDeclarationAmbiguous;
        diagnostic.phase = FlowScriptDiagnosticPhase::CatalogResolution;
        diagnostic.declaration = ticks.first().cloned();
        diagnostic.ast_path = diagnostic
            .declaration
            .as_deref()
            .map(|name| format!("calls.{name}"));
        diagnostic.expected = Some("one exact catalog declaration".into());
        diagnostic.actual = message
            .split_once("matched ")
            .map(|(_, candidates)| candidates.to_string());
        diagnostic.fix = fix(
            "Choose one exact declaration from the reported candidates.",
            diagnostic.declaration.clone(),
        );
    } else if message.contains("catalog metadata is required") {
        diagnostic.code = FlowScriptDiagnosticCode::FsCatalogMetadataRequired;
        diagnostic.phase = FlowScriptDiagnosticPhase::CatalogResolution;
        diagnostic.expected = Some("catalog metadata for every new call".into());
        diagnostic.actual = Some("catalog metadata was not supplied".into());
        diagnostic.fix = fix(
            "Reconcile through the catalog-aware FlowScript entry point.",
            None,
        );
    } else if message.contains("which has no field") || message.contains("which has no such field")
    {
        // A platform struct whose field list is authoritative — the repair is always an accessor
        // call, and the message already carries the list of them.
        diagnostic.code = FlowScriptDiagnosticCode::FsStructFieldUnknown;
        diagnostic.phase = FlowScriptDiagnosticPhase::Validation;
        let (struct_title, field) = if message.contains("which has no such field") {
            (ticks.get(2).cloned(), ticks.get(1).cloned())
        } else {
            (ticks.get(1).cloned(), ticks.get(2).cloned())
        };
        diagnostic.expected = struct_title.map(|title| format!("a declared field of `{title}`"));
        diagnostic.actual = field;
        diagnostic.fix = fix(
            message
                .split_once("; ")
                .map(|(_, hint)| hint)
                .unwrap_or("Use the catalog accessor call for this attribute."),
            None,
        );
    } else if message.contains("argument") && message.contains("has incompatible pin types") {
        diagnostic.code = FlowScriptDiagnosticCode::FsTypeMismatch;
        diagnostic.phase = FlowScriptDiagnosticPhase::TypeCheck;
        diagnostic.pin = ticks.first().cloned();
        diagnostic.declaration = ticks.get(1).cloned();
        diagnostic.actual = ticks.get(3).cloned();
        diagnostic.expected = ticks.get(5).cloned();
        diagnostic.ast_path = diagnostic.declaration.as_deref().map(|declaration| {
            diagnostic.pin.as_deref().map_or_else(
                || format!("calls.{declaration}.arguments"),
                |pin| format!("calls.{declaration}.arguments.{pin}"),
            )
        });
        diagnostic.fix = fix(
            "Insert a catalog-declared conversion whose output matches the target pin type.",
            diagnostic.expected.clone(),
        );
    } else if message.contains("binary comparison")
        && message.contains("incompatible operand types")
    {
        diagnostic.code = FlowScriptDiagnosticCode::FsTypeMismatch;
        diagnostic.phase = FlowScriptDiagnosticPhase::TypeCheck;
        diagnostic.expected = Some("matching concrete operand types".into());
        if ticks.len() >= 3 {
            diagnostic.actual = Some(format!("{} and {}", ticks[1], ticks[2]));
        }
        diagnostic.ast_path = Some("expressions.binaryComparison".into());
        diagnostic.fix = fix(
            "Convert both operands to the same concrete type before comparing them.",
            Some("type conversion comparison".into()),
        );
    } else if message.contains("binary comparison") && message.contains("ambiguous operand type") {
        diagnostic.code = FlowScriptDiagnosticCode::FsTypeAmbiguous;
        diagnostic.phase = FlowScriptDiagnosticPhase::TypeCheck;
        diagnostic.expected = Some("one concrete operand type".into());
        diagnostic.actual = Some("unknown or ambiguous operand type".into());
        diagnostic.ast_path = Some("expressions.binaryComparison".into());
        diagnostic.fix = fix(
            "Convert or bind the operands to one explicit concrete type before comparing them.",
            Some("type conversion comparison".into()),
        );
    } else if message.contains("binary comparison")
        && message.contains("has no suitable two-input catalog node")
    {
        diagnostic.code = FlowScriptDiagnosticCode::FsCatalogDeclarationNotFound;
        diagnostic.phase = FlowScriptDiagnosticPhase::CatalogResolution;
        diagnostic.expected = ticks
            .get(1)
            .map(|ty| format!("a two-input Boolean comparator for {ty}"));
        diagnostic.actual = Some("no compatible comparator declaration".into());
        diagnostic.ast_path = Some("expressions.binaryComparison".into());
        diagnostic.fix = fix(
            "Use a supported concrete type and its exact comparison declaration.",
            Some("comparison".into()),
        );
    } else if message.contains("has no input pin named") {
        diagnostic.code = FlowScriptDiagnosticCode::FsUnknownInputPin;
        diagnostic.phase = FlowScriptDiagnosticPhase::CatalogResolution;
        if ticks.len() >= 2 {
            diagnostic.declaration = Some(ticks[0].clone());
            diagnostic.pin = Some(ticks[1].clone());
        } else {
            diagnostic.declaration = message
                .strip_prefix("node ")
                .and_then(|rest| rest.split_once(" has no input pin named"))
                .map(|(node, _)| node.trim_matches(['`', '\'', '"']).to_string());
            diagnostic.pin = quoted_value_after(message, "named ");
        }
        set_call_path(&mut diagnostic);
        diagnostic.expected = diagnostic
            .pin
            .as_deref()
            .map(|pin| format!("an input pin named {pin}"));
        diagnostic.actual = Some("the resolved declaration has no such input pin".into());
        diagnostic.fix = fix(
            "Use an exact input name from the declaration signature.",
            diagnostic.declaration.clone(),
        );
    } else if message.contains("argument")
        && message.contains("not a literal or resolvable node output")
    {
        diagnostic.code = FlowScriptDiagnosticCode::FsUnresolvedArgument;
        diagnostic.phase = FlowScriptDiagnosticPhase::TypeCheck;
        diagnostic.pin = ticks.first().cloned();
        diagnostic.declaration = ticks.get(1).cloned();
        set_call_path(&mut diagnostic);
        diagnostic.expected = Some("a literal, variable, or resolvable node output".into());
        diagnostic.actual = Some("an unresolved expression".into());
        diagnostic.fix = fix(
            "Bind the argument to a literal, variable, or named output compatible with its pin.",
            diagnostic.declaration.clone(),
        );
    } else if message.contains("could not choose an output pin for argument") {
        diagnostic.code = FlowScriptDiagnosticCode::FsOutputPinUnresolved;
        diagnostic.phase = FlowScriptDiagnosticPhase::TypeCheck;
        diagnostic.pin = ticks.first().cloned();
        diagnostic.declaration = ticks.get(1).cloned();
        set_call_path(&mut diagnostic);
        diagnostic.expected = Some("one output compatible with the target input pin".into());
        diagnostic.actual = Some("no unambiguous compatible output".into());
        diagnostic.fix = fix(
            "Select a named output whose type matches the target input pin.",
            diagnostic.declaration.clone(),
        );
    } else if message.contains("comparison node")
        && message.contains("no unambiguous Boolean output")
    {
        diagnostic.code = FlowScriptDiagnosticCode::FsOutputPinUnresolved;
        diagnostic.phase = FlowScriptDiagnosticPhase::TypeCheck;
        diagnostic.declaration = ticks.first().cloned();
        diagnostic.expected = Some("one Boolean output pin".into());
        diagnostic.actual = Some("zero or multiple Boolean outputs".into());
        set_call_path(&mut diagnostic);
        diagnostic.fix = fix(
            "Use a comparator declaration with one unambiguous Boolean output.",
            diagnostic.declaration.clone(),
        );
    } else if message.contains("multiple execution outputs")
        && message.contains("no default continuation policy")
    {
        diagnostic.code = FlowScriptDiagnosticCode::FsExecutionPolicyAmbiguous;
        diagnostic.phase = FlowScriptDiagnosticPhase::ExecutionWiring;
        diagnostic.declaration = ticks.first().cloned();
        diagnostic.ast_path = diagnostic
            .declaration
            .as_deref()
            .map(|name| format!("calls.{name}.execution"));
        diagnostic.expected = Some("one explicit/default continuation output".into());
        diagnostic.actual = parenthesized_after(message, "multiple execution outputs");
        diagnostic.fix = fix(
            "Express the execution branches explicitly or add a catalog continuation policy.",
            diagnostic.declaration.clone(),
        );
    } else if message.contains("no incoming execution connection") {
        diagnostic.code = FlowScriptDiagnosticCode::FsExecutionEntryUnconnected;
        diagnostic.phase = FlowScriptDiagnosticPhase::ExecutionWiring;
        diagnostic.declaration = ticks.first().cloned();
        diagnostic.ast_path = diagnostic
            .declaration
            .as_deref()
            .map(|name| format!("calls.{name}.execution"));
        diagnostic.expected = Some("an incoming execution edge".into());
        diagnostic.actual = Some("no incoming execution edge".into());
        diagnostic.fix = fix(
            "Place the call in an executable chain after a statement with a continuation output.",
            diagnostic.declaration.clone(),
        );
    } else if message.contains("leaves multiple execution successors") {
        diagnostic.code = FlowScriptDiagnosticCode::FsExecutionChainAmbiguous;
        diagnostic.phase = FlowScriptDiagnosticPhase::ExecutionWiring;
        diagnostic.declaration = ticks.first().cloned();
        diagnostic.expected = Some("one execution successor".into());
        diagnostic.actual = Some("multiple execution successors".into());
        diagnostic.fix = fix(
            "Reconnect the intended execution successor explicitly.",
            None,
        );
    } else if message.contains("branch statements are not yet converted automatically") {
        diagnostic.code = FlowScriptDiagnosticCode::FsBranchLoweringUnsupported;
        diagnostic.phase = FlowScriptDiagnosticPhase::Lowering;
        diagnostic.ast_path = Some("statements.branch".into());
        diagnostic.expected = Some("a branch form supported by FlowScript lowering".into());
        diagnostic.actual = Some("an unsupported free-standing branch statement".into());
        diagnostic.fix = fix(
            "Use a typed if-condition or the exact control branch declaration with explicit arms.",
            Some("control branch".into()),
        );
    } else if message.contains("branch arm label")
        && message.contains("does not match an execution output")
    {
        diagnostic.code = FlowScriptDiagnosticCode::FsBranchArmPinUnknown;
        diagnostic.phase = FlowScriptDiagnosticPhase::ExecutionWiring;
        diagnostic.pin = ticks.first().cloned();
        diagnostic.declaration = ticks.get(1).cloned();
        set_call_path(&mut diagnostic);
        diagnostic.expected = Some("an exact execution output pin name".into());
        diagnostic.actual = diagnostic.pin.clone();
        diagnostic.fix = fix(
            "Rename the branch arm to an exact execution output pin.",
            diagnostic.declaration.clone(),
        );
    } else if message.contains("nodes (max") && message.contains("Nothing was queued") {
        diagnostic.code = FlowScriptDiagnosticCode::FsNodeLimitExceeded;
        diagnostic.phase = FlowScriptDiagnosticPhase::Validation;
        diagnostic.scope = if message.contains("function/layer") {
            ticks.first().map(|name| format!("function:{name}"))
        } else {
            Some("root".into())
        };
        diagnostic.ast_path = diagnostic.scope.as_deref().map(scope_ast_path);
        diagnostic.expected =
            number_after(message, "max ").map(|max| format!("at most {max} nodes"));
        diagnostic.actual = number_after(message, "with ").map(|count| format!("{count} nodes"));
        diagnostic.fix = fix(
            "Split the workflow into smaller non-empty functions and call them from the event.",
            None,
        );
    } else if message.contains("new function")
        && (message.contains("no executable body nodes")
            || message.contains("no materialized body nodes")
            || message.contains("runtime-empty helper"))
    {
        diagnostic.code = FlowScriptDiagnosticCode::FsHelperEmpty;
        diagnostic.phase = FlowScriptDiagnosticPhase::Validation;
        set_function_scope(&mut diagnostic, ticks.first());
        diagnostic.expected = Some("at least one materialized executable body node".into());
        diagnostic.actual = Some("an empty or non-materializable helper body".into());
        diagnostic.fix = fix(
            "Add supported executable catalog calls to the helper or remove the helper.",
            None,
        );
    } else if message.contains("new function") && message.contains("no observable runtime effect") {
        diagnostic.code = FlowScriptDiagnosticCode::FsHelperNoObservableEffect;
        diagnostic.phase = FlowScriptDiagnosticPhase::Validation;
        set_function_scope(&mut diagnostic, ticks.first());
        diagnostic.expected = Some("declared returns or an execution-reachable side effect".into());
        diagnostic.actual = Some("a pure helper with no return values".into());
        diagnostic.fix = fix(
            "Declare and return observable values, or add a supported impure operation.",
            None,
        );
    } else if message.contains("new impure function")
        && message.contains("no Function exec_in connection")
    {
        diagnostic.code = FlowScriptDiagnosticCode::FsHelperExecutionEntryUnconnected;
        diagnostic.phase = FlowScriptDiagnosticPhase::ExecutionWiring;
        set_function_scope(&mut diagnostic, ticks.first());
        diagnostic.expected = Some("Function exec_in connected to the first body node".into());
        diagnostic.actual = Some("no materialized execution entry edge".into());
        diagnostic.fix = fix(
            "Make the first helper statement an execution-reachable impure call.",
            None,
        );
    } else if message.contains("new impure function")
        && message.contains("body tail connected to Function exec_out")
    {
        diagnostic.code = FlowScriptDiagnosticCode::FsHelperExecutionTailUnconnected;
        diagnostic.phase = FlowScriptDiagnosticPhase::ExecutionWiring;
        set_function_scope(&mut diagnostic, ticks.first());
        diagnostic.expected =
            Some("the final body continuation connected to Function exec_out".into());
        diagnostic.actual = Some("no materialized execution tail edge".into());
        diagnostic.fix = fix(
            "End the helper with an impure statement that has a supported continuation output.",
            None,
        );
    } else if message.contains("return value") {
        diagnostic.code = FlowScriptDiagnosticCode::FsFunctionReturnMismatch;
        diagnostic.phase = FlowScriptDiagnosticPhase::TypeCheck;
        let function_name = message
            .split_once("in function `")
            .and_then(|(_, rest)| rest.split_once('`'))
            .map(|(name, _)| name.to_string());
        diagnostic.ast_path = Some(match function_name.as_deref() {
            Some(name) => format!("functions.{name}.statements.return"),
            None => "statements.return".to_string(),
        });
        diagnostic.scope = function_name.map(|name| format!("function:{name}"));
        diagnostic.actual = ticks
            .first()
            .map(|expression| format!("return value `{expression}`"))
            .or(Some("an unresolved or undeclared return value".into()));
        if message.contains("no matching function return pin") {
            diagnostic.expected =
                Some("a declared function return pin for every returned value".into());
            diagnostic.fix = fix(
                "Declare a named return pin in the function signature for each returned value (in order), or return fewer values.",
                None,
            );
        } else {
            diagnostic.expected = Some("a resolvable value and compatible output pin".into());
            diagnostic.fix = fix(
                "Return a literal (materialized automatically as a typed local variable), a call output bound earlier in the body (`const x = call(...)` then `return x.pinName`), or a variable reference.",
                None,
            );
        }
    } else if message.contains("variable reference") && message.contains("does not resolve") {
        diagnostic.code = FlowScriptDiagnosticCode::FsVariableUnresolved;
        diagnostic.phase = FlowScriptDiagnosticPhase::CatalogResolution;
        diagnostic.declaration = ticks.first().cloned();
        diagnostic.ast_path = diagnostic
            .declaration
            .as_deref()
            .map(|name| format!("variables.{name}"));
        diagnostic.expected = Some("a declared board or FlowScript variable".into());
        diagnostic.actual = diagnostic.declaration.clone();
        diagnostic.fix = fix(
            "Declare the variable at FlowScript top level before using it.",
            None,
        );
    } else if message.contains("anchor") && message.contains("no longer") {
        diagnostic.code = FlowScriptDiagnosticCode::FsAnchorUnresolved;
        diagnostic.phase = FlowScriptDiagnosticPhase::CatalogResolution;
        diagnostic.declaration = ticks
            .get(1)
            .cloned()
            .or_else(|| ticks.first().cloned())
            .or_else(|| {
                message
                    .strip_prefix("anchor ")
                    .and_then(|rest| rest.split_once(" no longer"))
                    .map(|(anchor, _)| anchor.to_string())
            });
        diagnostic.expected = Some("an anchor that resolves to a current board entity".into());
        diagnostic.actual = diagnostic.declaration.clone();
        diagnostic.fix = fix(
            "Refresh FlowScript from the current board before editing it.",
            None,
        );
    } else if message.contains("new event") && message.contains("no executable body nodes") {
        diagnostic.code = FlowScriptDiagnosticCode::FsEventEmpty;
        diagnostic.phase = FlowScriptDiagnosticPhase::Validation;
        if let Some(name) = ticks.first() {
            diagnostic.scope = Some(format!("event:{name}"));
            diagnostic.ast_path = Some(format!("events.{name}"));
        }
        diagnostic.expected = Some("at least one executable event body node".into());
        diagnostic.actual = Some("an empty event registration".into());
        diagnostic.fix = fix("Add supported executable calls to the event body.", None);
    }

    diagnostic
}

fn set_call_path(diagnostic: &mut FlowScriptDiagnostic) {
    diagnostic.ast_path = match (diagnostic.declaration.as_deref(), diagnostic.pin.as_deref()) {
        (Some(declaration), Some(pin)) => Some(format!("calls.{declaration}.args.{pin}")),
        (Some(declaration), None) => Some(format!("calls.{declaration}")),
        _ => None,
    };
}

fn set_function_scope(diagnostic: &mut FlowScriptDiagnostic, name: Option<&String>) {
    if let Some(name) = name {
        diagnostic.scope = Some(format!("function:{name}"));
        diagnostic.ast_path = Some(format!("functions.{name}"));
    }
}

fn scope_ast_path(scope: &str) -> String {
    scope
        .strip_prefix("function:")
        .map(|name| format!("functions.{name}"))
        .unwrap_or_else(|| "events".into())
}

fn semantic_group_key(diagnostic: &FlowScriptDiagnostic) -> String {
    let mut key = format!(
        "{}|{}|{}|{}|{}|{}",
        diagnostic.code.as_str(),
        diagnostic.scope.as_deref().unwrap_or_default(),
        diagnostic.ast_path.as_deref().unwrap_or_default(),
        diagnostic.declaration.as_deref().unwrap_or_default(),
        diagnostic.pin.as_deref().unwrap_or_default(),
        diagnostic.actual.as_deref().unwrap_or_default(),
    );
    if let Some(span) = diagnostic.source_span.as_ref() {
        key.push_str(&format!("|{}:{}", span.start.line, span.start.column));
    }
    // Unknown wording cannot be safely grouped by semantic fields because none were recognized.
    if diagnostic.code == FlowScriptDiagnosticCode::FsReconcileUnsupported {
        key.push('|');
        key.push_str(&diagnostic.message);
    }
    key
}

fn diagnostic_id(key: &str) -> String {
    // FNV-1a is intentionally local and deterministic; DefaultHasher does not promise stable ids
    // across compiler releases.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("FSD-{hash:016x}")
}

fn link_execution_cascades(diagnostics: &mut [FlowScriptDiagnostic]) {
    let mut roots_by_subject = HashMap::<String, String>::new();
    for diagnostic in diagnostics {
        let subject = diagnostic
            .scope
            .clone()
            .or_else(|| diagnostic.declaration.clone());
        let Some(subject) = subject else { continue };

        let is_root_candidate = matches!(
            diagnostic.code,
            FlowScriptDiagnosticCode::FsCatalogDeclarationNotFound
                | FlowScriptDiagnosticCode::FsCatalogDeclarationAmbiguous
                | FlowScriptDiagnosticCode::FsTypeMismatch
                | FlowScriptDiagnosticCode::FsTypeAmbiguous
                | FlowScriptDiagnosticCode::FsUnknownInputPin
                | FlowScriptDiagnosticCode::FsStructFieldUnknown
                | FlowScriptDiagnosticCode::FsUnresolvedArgument
                | FlowScriptDiagnosticCode::FsOutputPinUnresolved
                | FlowScriptDiagnosticCode::FsHelperEmpty
                | FlowScriptDiagnosticCode::FsHelperNoObservableEffect
        );
        let is_execution_cascade = matches!(
            diagnostic.code,
            FlowScriptDiagnosticCode::FsExecutionEntryUnconnected
                | FlowScriptDiagnosticCode::FsHelperExecutionEntryUnconnected
                | FlowScriptDiagnosticCode::FsHelperExecutionTailUnconnected
        );

        if is_execution_cascade {
            if let Some(root_id) = roots_by_subject.get(&subject) {
                diagnostic.caused_by = Some(root_id.clone());
            }
        } else if is_root_candidate {
            roots_by_subject
                .entry(subject)
                .or_insert_with(|| diagnostic.id.clone());
        }
    }
}

/// Every offset at which `needle` occurs as a standalone token start. `str::find` returned only the
/// first, which on a generated document is the wrong one: a helper called 40 times reported all 40
/// diagnostics at one coordinate, and for a declared helper the first textual `name(` is its own
/// `function name(` header rather than any call site.
fn subject_offsets(source: &str, needle: &str, skip_function_headers: bool) -> Vec<usize> {
    source
        .match_indices(needle)
        .map(|(offset, _)| offset)
        .filter(|offset| {
            // `catalog(` must not match the needle `log(`.
            let before = source[..*offset].chars().next_back();
            !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                && !(skip_function_headers && source[..*offset].ends_with("function "))
        })
        .collect()
}

fn locate_subject_sites(
    source: &str,
    diagnostic: &FlowScriptDiagnostic,
) -> (Vec<FlowScriptSourceSpan>, usize) {
    let (needle, token_len, skip_headers) = if let Some(scope) = diagnostic.scope.as_deref() {
        if let Some(name) = scope.strip_prefix("function:") {
            (
                format!("function {name}"),
                name.len() + "function ".len(),
                false,
            )
        } else if let Some(name) = scope.strip_prefix("event:") {
            (format!("{name}("), name.len(), false)
        } else {
            return (Vec::new(), 0);
        }
    } else if let Some(declaration) = diagnostic.declaration.as_deref() {
        (format!("{declaration}("), declaration.len(), true)
    } else {
        return (Vec::new(), 0);
    };
    let offsets = subject_offsets(source, &needle, skip_headers);
    let additional = offsets.len().saturating_sub(MAX_SUBJECT_SPANS);
    let spans = offsets
        .iter()
        .take(MAX_SUBJECT_SPANS)
        .map(|offset| FlowScriptSourceSpan {
            start: source_position(source, *offset),
            end: source_position(source, offset + token_len),
        })
        .collect();
    (spans, additional)
}

fn parse_error_span(message: &str) -> Option<FlowScriptSourceSpan> {
    let after_line = message.strip_prefix("FlowScript parse error at line ")?;
    let (line, after_line) = after_line.split_once(", col ")?;
    let (column, _) = after_line.split_once(':')?;
    let line = line.parse::<usize>().ok()?;
    let column = column.parse::<usize>().ok()?;
    let position = FlowScriptSourcePosition {
        line,
        column,
        // The parser does not currently expose a byte offset; source-enriched semantic
        // diagnostics do carry one.
        offset: None,
    };
    Some(FlowScriptSourceSpan {
        start: position.clone(),
        end: position,
    })
}

/// Upper bound on how many call sites a single grouped diagnostic carries. A generated document can
/// call one helper dozens of times; the model needs enough sites to locate the work, not all of them.
const MAX_SUBJECT_SPANS: usize = 12;

fn is_zero(value: &usize) -> bool {
    *value == 0
}

fn source_position(source: &str, offset: usize) -> FlowScriptSourcePosition {
    let prefix = &source[..offset.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.chars().count() + 1, |(_, tail)| {
            tail.chars().count() + 1
        });
    FlowScriptSourcePosition {
        line,
        column,
        offset: Some(offset),
    }
}

fn fix(
    summary: impl Into<String>,
    declaration_search: Option<String>,
) -> Option<FlowScriptDiagnosticFix> {
    Some(FlowScriptDiagnosticFix {
        summary: summary.into(),
        declaration_search,
        catalog_declarations: Vec::new(),
        companion_declarations: Vec::new(),
    })
}

fn backtick_values(message: &str) -> Vec<String> {
    message
        .split('`')
        .enumerate()
        .filter(|&(index, value)| (index % 2 == 1)).map(|(index, value)| value.to_string())
        .collect()
}

fn quoted_value_after(message: &str, marker: &str) -> Option<String> {
    let rest = message.split_once(marker)?.1;
    let quote = rest.chars().next()?;
    if !matches!(quote, '`' | '\'' | '"') {
        return rest.split_whitespace().next().map(str::to_string);
    }
    rest[quote.len_utf8()..]
        .split_once(quote)
        .map(|(value, _)| value.to_string())
}

fn parenthesized_after(message: &str, marker: &str) -> Option<String> {
    let rest = message.split_once(marker)?.1;
    let start = rest.find('(')? + 1;
    let end = rest[start..].find(')')? + start;
    Some(rest[start..end].to_string())
}

fn number_after(message: &str, marker: &str) -> Option<usize> {
    let rest = message.split_once(marker)?.1;
    let digits = rest
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse().ok()
}

fn expected_from_parse_message(message: &str) -> Option<String> {
    let details = message.rsplit_once(": ")?.1;
    let rest = details.strip_prefix("expected ")?;
    Some(
        rest.split_once(", found")
            .map_or(rest, |(expected, _)| expected)
            .to_string(),
    )
}

fn actual_from_parse_message(message: &str) -> Option<String> {
    message
        .rsplit_once(", found ")
        .map(|(_, actual)| actual.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(messages: &[&str]) -> ReconcileResult {
        ReconcileResult {
            commands: Vec::new(),
            corrections: Vec::new(),
            diagnostics: messages
                .iter()
                .map(|message| (*message).to_string())
                .collect(),
        }
    }

    #[test]
    fn structures_generic_comparison_with_stable_type_metadata() {
        let structured = result(&[
            "binary comparison `==` has incompatible operand types `Generic` and `String`",
        ])
        .structured_diagnostics();

        assert_eq!(structured.len(), 1);
        let diagnostic = &structured[0];
        assert_eq!(diagnostic.code, FlowScriptDiagnosticCode::FsTypeMismatch);
        assert_eq!(diagnostic.phase, FlowScriptDiagnosticPhase::TypeCheck);
        assert_eq!(
            diagnostic.expected.as_deref(),
            Some("matching concrete operand types")
        );
        assert_eq!(diagnostic.actual.as_deref(), Some("Generic and String"));
        assert!(diagnostic.id.starts_with("FSD-"));
        assert!(diagnostic.fix.is_some());
    }

    #[test]
    fn structures_argument_pin_type_mismatch_with_call_and_pin_metadata() {
        let structured = result(&[
            "argument `updatedAt` on `saveTicket` has incompatible pin types: source `utils_datetime_now.date` is `Date/Normal`, but input `updatedAt` requires `String/Normal`; use a catalog-declared conversion before connecting it",
        ])
        .structured_diagnostics();

        assert_eq!(structured.len(), 1);
        let diagnostic = &structured[0];
        assert_eq!(diagnostic.code, FlowScriptDiagnosticCode::FsTypeMismatch);
        assert_eq!(diagnostic.phase, FlowScriptDiagnosticPhase::TypeCheck);
        assert_eq!(diagnostic.declaration.as_deref(), Some("saveTicket"));
        assert_eq!(diagnostic.pin.as_deref(), Some("updatedAt"));
        assert_eq!(diagnostic.actual.as_deref(), Some("Date/Normal"));
        assert_eq!(diagnostic.expected.as_deref(), Some("String/Normal"));
        assert_eq!(
            diagnostic.ast_path.as_deref(),
            Some("calls.saveTicket.arguments.updatedAt")
        );
        assert!(diagnostic.fix.is_some());
    }

    #[test]
    fn structures_unknown_pin_with_call_ast_path_and_catalog_search() {
        let structured = result(&[
            "node `agentRegisterFunctionTools` has no input pin named `tools`; skipped that argument",
        ])
        .structured_diagnostics();

        let diagnostic = &structured[0];
        assert_eq!(diagnostic.code, FlowScriptDiagnosticCode::FsUnknownInputPin);
        assert_eq!(
            diagnostic.declaration.as_deref(),
            Some("agentRegisterFunctionTools")
        );
        assert_eq!(diagnostic.pin.as_deref(), Some("tools"));
        assert_eq!(
            diagnostic.ast_path.as_deref(),
            Some("calls.agentRegisterFunctionTools.args.tools")
        );
        assert_eq!(
            diagnostic
                .fix
                .as_ref()
                .and_then(|fix| fix.declaration_search.as_deref()),
            Some("agentRegisterFunctionTools")
        );
    }

    #[test]
    fn preserves_parse_position_and_expected_actual_tokens() {
        let structured = result(&[
            "FlowScript parse error at line 31, col 21: expected `Colon`, found `Assign`",
        ])
        .structured_diagnostics();

        let diagnostic = &structured[0];
        assert_eq!(diagnostic.code, FlowScriptDiagnosticCode::FsParseError);
        let span = diagnostic.source_span.as_ref().expect("parse span");
        assert_eq!((span.start.line, span.start.column), (31, 21));
        assert_eq!(diagnostic.expected.as_deref(), Some("`Colon`"));
        assert_eq!(diagnostic.actual.as_deref(), Some("`Assign`"));
    }

    #[test]
    fn deduplicates_repeated_argument_cascades_without_changing_legacy_strings() {
        let result = result(&[
            "argument `condition` on `controlBranch` is not a literal or resolvable node output; skipped connection",
            "argument `condition` on `controlBranch` is not a literal or resolvable node output; skipped connection",
            "argument `condition` on `controlBranch` is not a literal or resolvable node output; skipped connection",
        ]);
        let structured = result.structured_diagnostics();

        assert_eq!(
            result.diagnostics.len(),
            3,
            "legacy diagnostics stay untouched"
        );
        assert_eq!(structured.len(), 1);
        assert_eq!(structured[0].occurrences, 3);
        assert_eq!(structured[0].pin.as_deref(), Some("condition"));
    }

    #[test]
    fn source_enrichment_locates_function_scope() {
        let result = result(&[
            "new impure function `saveTicket` has no materialized body tail connected to Function exec_out; callers could not continue after it",
        ]);
        let source =
            "function saveTicket(ticket: Struct) {\n    databaseUpsert({ value: ticket })\n}\n";
        let structured = result.structured_diagnostics_for_source(source);

        let diagnostic = &structured[0];
        assert_eq!(
            diagnostic.code,
            FlowScriptDiagnosticCode::FsHelperExecutionTailUnconnected
        );
        assert_eq!(diagnostic.scope.as_deref(), Some("function:saveTicket"));
        let span = diagnostic.source_span.as_ref().expect("function span");
        assert_eq!((span.start.line, span.start.column), (1, 1));
        assert_eq!(span.end.offset, Some("function saveTicket".len()));
    }

    /// `str::find` returned only the first textual match, so a helper called many times reported
    /// every diagnostic at one coordinate. The model then patched the wrong line, produced an
    /// identical diagnostic set, and looked to the loop like it was making no progress.
    #[test]
    fn locates_every_call_site_not_only_the_first() {
        let result = result(&[
            "argument `value` on `stringFormat` is not a literal or resolvable node output",
        ]);
        let source = concat!(
            "eventsSimple() {\n",
            "    const a = stringFormat({ formatString: \"x\" })\n",
            "    const b = stringFormat({ formatString: \"y\" })\n",
            "    const c = stringFormat({ formatString: \"z\" })\n",
            "}\n",
        );
        let structured = result.structured_diagnostics_for_source(source);

        let diagnostic = &structured[0];
        assert_eq!(diagnostic.spans.len(), 3, "every call site must be located");
        assert_eq!(diagnostic.additional_sites, 0);
        let lines: Vec<usize> = diagnostic.spans.iter().map(|s| s.start.line).collect();
        assert_eq!(lines, vec![2, 3, 4], "spans must be in source order");
        assert_eq!(
            diagnostic.source_span.as_ref().map(|s| s.start.line),
            Some(2),
            "source_span stays spans[0] for existing consumers"
        );
    }

    /// For a helper declared in the same document the FIRST textual `name(` is its own
    /// `function name(` header, not a call. Sending the model to the declaration when the defect is
    /// in the calls is worse than sending it nowhere.
    #[test]
    fn a_call_site_is_never_reported_at_its_declaration_header() {
        let result = result(&[
            "argument `subject` on `saveTicket` is not a literal or resolvable node output",
        ]);
        let source = concat!(
            "function saveTicket(subject: string) {\n",
            "    logInfo({ message: subject })\n",
            "}\n",
            "\n",
            "eventsSimple() {\n",
            "    saveTicket({ subject: \"hi\" })\n",
            "}\n",
        );
        let structured = result.structured_diagnostics_for_source(source);

        let diagnostic = &structured[0];
        assert_eq!(
            diagnostic.spans.len(),
            1,
            "only the call site counts: {:?}",
            diagnostic.spans
        );
        assert_eq!(
            diagnostic.spans[0].start.line, 6,
            "the declaration header on line 1 must be skipped"
        );
    }

    /// A generated document can call one helper dozens of times. Carry enough sites to locate the
    /// work and report the remainder as a count rather than inflating every diagnostic.
    #[test]
    fn caps_carried_spans_and_reports_the_remainder() {
        let result = result(&[
            "argument `value` on `stringFormat` is not a literal or resolvable node output",
        ]);
        let mut source = String::from("eventsSimple() {\n");
        for index in 0..(MAX_SUBJECT_SPANS + 5) {
            source.push_str(&format!(
                "    const v{index} = stringFormat({{ formatString: \"x\" }})\n"
            ));
        }
        source.push_str("}\n");
        let structured = result.structured_diagnostics_for_source(&source);

        let diagnostic = &structured[0];
        assert_eq!(diagnostic.spans.len(), MAX_SUBJECT_SPANS);
        assert_eq!(diagnostic.additional_sites, 5);
    }

    /// A shorter declaration name must not match inside a longer one.
    #[test]
    fn subject_search_does_not_match_a_longer_identifier() {
        assert_eq!(
            subject_offsets(
                "catalogLookup({ x: 1 })\nlookup({ y: 2 })\n",
                "lookup(",
                false
            ),
            vec!["catalogLookup({ x: 1 })\n".len()],
        );
    }

    #[test]
    fn links_same_function_execution_cascade_to_empty_helper_root() {
        let structured = result(&[
            "new function `helper` contains no materialized body nodes in its Function layer; refusing to create a runtime-empty helper",
            "new impure function `helper` has no materialized body tail connected to Function exec_out; callers could not continue after it",
        ])
        .structured_diagnostics();

        assert_eq!(structured.len(), 2);
        assert_eq!(
            structured[1].caused_by.as_deref(),
            Some(structured[0].id.as_str())
        );
    }

    #[test]
    fn groups_function_return_mismatches_per_function_with_scope_and_expression() {
        let structured = result(&[
            "return value 1 (`missingRef`) in function `brokenOne` is not a resolvable FlowScript value; return a literal, a call output bound earlier in the body (`const x = call(...)`), or a variable",
            "return value 1 (`otherMissing`) in function `brokenTwo` is not a resolvable FlowScript value; return a literal, a call output bound earlier in the body (`const x = call(...)`), or a variable",
        ])
        .structured_diagnostics();

        assert_eq!(
            structured.len(),
            2,
            "two broken functions must stay two groups: {structured:?}"
        );
        assert!(structured.iter().all(|diagnostic| {
            diagnostic.code == FlowScriptDiagnosticCode::FsFunctionReturnMismatch
        }));
        assert_eq!(structured[0].scope.as_deref(), Some("function:brokenOne"));
        assert_eq!(
            structured[0].ast_path.as_deref(),
            Some("functions.brokenOne.statements.return")
        );
        assert_eq!(
            structured[0].actual.as_deref(),
            Some("return value `missingRef`")
        );
        assert_eq!(structured[1].scope.as_deref(), Some("function:brokenTwo"));
        let fix = structured[0].fix.as_ref().expect("repair guidance");
        assert!(
            fix.summary.contains("literal"),
            "fix must mention that literals are supported: {}",
            fix.summary
        );
    }

    #[test]
    fn classifies_report_failure_families() {
        let cases = [
            (
                "argument `condition` on `controlBranch` is not a literal or resolvable node output; skipped connection",
                FlowScriptDiagnosticCode::FsUnresolvedArgument,
                FlowScriptDiagnosticPhase::TypeCheck,
            ),
            (
                "could not choose an output pin for argument `condition` on `controlBranch`",
                FlowScriptDiagnosticCode::FsOutputPinUnresolved,
                FlowScriptDiagnosticPhase::TypeCheck,
            ),
            (
                "node `$43` has multiple execution outputs (success, error) and no default continuation policy; add an explicit policy before auto-wiring sequential FlowScript calls",
                FlowScriptDiagnosticCode::FsExecutionPolicyAmbiguous,
                FlowScriptDiagnosticPhase::ExecutionWiring,
            ),
            (
                "node `Now` has no incoming execution connection and will not run; wire its execution input from the previous statement's execution output",
                FlowScriptDiagnosticCode::FsExecutionEntryUnconnected,
                FlowScriptDiagnosticPhase::ExecutionWiring,
            ),
            (
                "new FlowScript branch statements are not yet converted automatically; use the exact control_branch declaration and supported FlowScript branch blocks because model-facing emit_commands cannot wire executable branches",
                FlowScriptDiagnosticCode::FsBranchLoweringUnsupported,
                FlowScriptDiagnosticPhase::Lowering,
            ),
            (
                "this edit would leave function/layer `pollInbox` with 60 nodes (max 50). Nothing was queued. Split the logic into smaller functions",
                FlowScriptDiagnosticCode::FsNodeLimitExceeded,
                FlowScriptDiagnosticPhase::Validation,
            ),
            (
                "new function `helper` contains no materialized body nodes in its Function layer; refusing to create a runtime-empty helper",
                FlowScriptDiagnosticCode::FsHelperEmpty,
                FlowScriptDiagnosticPhase::Validation,
            ),
        ];

        for (message, expected_code, expected_phase) in cases {
            let diagnostic = result(&[message])
                .structured_diagnostics()
                .pop()
                .expect("structured diagnostic");
            assert_eq!(diagnostic.code, expected_code, "{message}");
            assert_eq!(diagnostic.phase, expected_phase, "{message}");
            assert!(diagnostic.fix.is_some(), "missing fix for {message}");
        }
    }

    /// The accessor list is the whole point of this diagnostic — a classification that drops it
    /// leaves the model with "no such field" and no way to repair.
    #[test]
    fn closed_struct_field_rejection_carries_the_accessor_repair() {
        for message in [
            "catalog output `path_from_upload_dir.file` is a `FlowPath`, which has no field `filename`; a FlowPath is a store handle, not a file object; read file attributes with the `Data/Files/Path` accessors instead: `filename({ path })`, `extension({ path })`",
            "`structGet` reads field `extension` from a `FlowPath`, which has no such field; a FlowPath is a store handle, not a file object; read file attributes with the `Data/Files/Path` accessors instead: `filename({ path })`, `extension({ path })`",
        ] {
            let diagnostic = result(&[message])
                .structured_diagnostics()
                .pop()
                .expect("structured diagnostic");
            assert_eq!(
                diagnostic.code,
                FlowScriptDiagnosticCode::FsStructFieldUnknown,
                "{message}"
            );
            assert_eq!(
                diagnostic.phase,
                FlowScriptDiagnosticPhase::Validation,
                "{message}"
            );
            let fix = diagnostic.fix.expect("fix").summary;
            assert!(fix.contains("filename({ path })"), "{fix}");
            assert!(fix.contains("extension({ path })"), "{fix}");
        }
    }
}
