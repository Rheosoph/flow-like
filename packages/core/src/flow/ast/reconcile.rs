//! Reconcile: parsed FlowScript (`BoardAst`) → `BoardCommand`s against an existing board.
//!
//! This is the *write* half of the FlowScript pipeline. Rendering lowers a [`Board`] into a
//! [`BoardAst`] and emits text (with optional `//@n:<id>` anchors); editing that text and parsing
//! it back yields a new `BoardAst`. Reconcile diffs the new AST against the live board and emits
//! the minimal set of [`BoardCommand`]s needed to realize the edit — reusing the exact command
//! enum the copilot already validates and applies (so undo/redo, coordinates, and all
//! side-channel metadata of untouched nodes survive). See `todo/ast.md` §7.
//!
//! ## Scope
//!
//! The plain [`reconcile`] path targets the dominant "edit an existing board through text"
//! operations, which are fully and unambiguously reconstructable:
//!
//! - **Configuration edits**: a literal argument on an anchored call changed ⇒ [`BoardCommand::UpdateNodePin`].
//! - **Deletions**: a *text-visible* anchored node present in the board but absent from the new
//!   AST ⇒ [`BoardCommand::RemoveNode`].
//!
//! The catalog-aware [`reconcile_with_catalog`] path additionally turns new unanchored catalog
//! calls into structural commands (`AddNode`, `ConnectPins`, `UpdateNodePin`). Reconcile still
//! never guesses identities or ambiguous catalog matches: a partial or ambiguous edit produces
//! diagnostics instead of destructive commands.

use std::collections::{BTreeMap, HashMap, HashSet};

use flow_like_ast::model::*;
use flow_like_ast::to_camel_case;

use crate::flow::board::{Board, Layer, LayerCache, LayerCacheScope, LayerType};
use crate::flow::copilot::{
    BoardCommand, NodeMetadata, NodePosition, PinMetadata, PlaceholderPinDef, node_to_metadata,
};
use crate::flow::node::Node;
use crate::flow::pin::{Pin, PinType};
use crate::flow::variable::{Variable, VariableType};

/// Outcome of reconciling a parsed `BoardAst` against a live board.
#[derive(Debug, Default, Clone)]
pub struct ReconcileResult {
    /// Minimal board mutations to realize the edit, in apply order.
    pub commands: Vec<BoardCommand>,
    /// Non-blocking, deterministic source migrations used while deriving commands. Callers should
    /// surface these to the author and rewrite retained FlowScript to the canonical spelling.
    pub corrections: Vec<String>,
    /// Blocking representation issues. Apply/check paths treat any diagnostic as an atomic
    /// rejection, so informational notes must not be placed here.
    pub diagnostics: Vec<String>,
}

/// Controls whether omissions in the submitted AST are edits or simply outside its scope.
///
/// Raw FlowScript round-trips use [`ReconcileMode::Replace`] because their document represents the
/// whole visible board. Typed IR uses [`ReconcileMode::Additive`] by default: an IR program can be
/// a self-contained addition to an existing board and therefore must not erase unrelated anchors
/// or variables merely because it did not reproduce them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileMode {
    #[default]
    Replace,
    Additive,
}

/// Diff a parsed FlowScript AST against `existing` and emit the minimal `BoardCommand`s.
///
/// Only nodes carrying a stable anchor (`//@n:<id>`) are eligible for in-place edits or removal,
/// and removal is further gated on the node being *text-visible* (it appears in the board's own
/// lowered/rendered form) so inlined/sugared helper nodes are never deleted merely for being
/// absent from the text. See the module docs for the full contract.
pub fn reconcile(existing: &Board, new: &BoardAst) -> ReconcileResult {
    reconcile_inner(existing, new, None, None, ReconcileMode::Replace)
}

/// Diff a parsed FlowScript AST against `existing`, using catalog metadata to turn unanchored
/// calls into structural commands.
///
/// The non-catalog [`reconcile`] path remains intentionally conservative for callers that only
/// want anchored edits. FlowPilot should use this catalog-aware path so new text-domain calls can
/// become `AddNode` / `ConnectPins` / `UpdateNodePin` command batches automatically.
pub fn reconcile_with_catalog(
    existing: &Board,
    new: &BoardAst,
    catalog: &[NodeMetadata],
) -> ReconcileResult {
    reconcile_inner(existing, new, Some(catalog), None, ReconcileMode::Replace)
}

/// Catalog-aware reconcile with explicit document scope semantics.
pub fn reconcile_with_catalog_mode(
    existing: &Board,
    new: &BoardAst,
    catalog: &[NodeMetadata],
    mode: ReconcileMode,
) -> ReconcileResult {
    reconcile_inner(existing, new, Some(catalog), None, mode)
}

/// Like [`reconcile_with_catalog`] but runs `enricher` to materialize each new node's dynamic
/// (`on_update`-generated) pins from its literal args, so literals/connections targeting those pins
/// resolve against real pins instead of the predicted `synthesize_dynamic_input_pin` fallback.
pub fn reconcile_with_catalog_enriched(
    existing: &Board,
    new: &BoardAst,
    catalog: &[NodeMetadata],
    enricher: &MetadataEnricher,
) -> ReconcileResult {
    reconcile_inner(
        existing,
        new,
        Some(catalog),
        Some(enricher),
        ReconcileMode::Replace,
    )
}

/// Hook that lets a caller enrich a resolved node's metadata with the dynamic pins its `on_update`
/// would create for a call's literal arguments (see `apply_flowscript_to_board`). Runs only for
/// callers that supply one; `None` keeps the pure static-catalog behavior (predicted via
/// `synthesize_dynamic_input_pin`).
pub type MetadataEnricher = Box<
    dyn Fn(&NodeMetadata, &[(String, flow_like_types::Value)], &Board) -> Option<NodeMetadata>
        + Send
        + Sync,
>;

fn reconcile_inner(
    existing: &Board,
    new: &BoardAst,
    catalog: Option<&[NodeMetadata]>,
    enricher: Option<&MetadataEnricher>,
    mode: ReconcileMode,
) -> ReconcileResult {
    let mut result = ReconcileResult::default();
    let mut preflight_diagnostics = duplicate_ast_declaration_diagnostics(new);
    let duplicate_anchors = duplicate_ast_anchors(new);
    preflight_diagnostics.extend(duplicate_anchors.into_iter().map(|anchor| {
            format!(
                "duplicate FlowScript anchor `{anchor}` identifies more than one entity; no commands were derived"
            )
        }));
    if !preflight_diagnostics.is_empty() {
        preflight_diagnostics.sort();
        preflight_diagnostics.dedup();
        result.diagnostics = preflight_diagnostics;
        return result;
    }
    let board_ast = super::lower_to_ast(existing);
    let variable_refs = VariableRefLookup::from_board_and_ast(existing, new);

    let variable_changes = reconcile_variables(existing, &board_ast, new, mode);
    result.commands.extend(variable_changes.commands);
    result.diagnostics.extend(variable_changes.diagnostics);

    // Function cache decorators are layer metadata rather than graph nodes/pins, so reconcile
    // them on both the conservative and catalog-aware paths. This also makes removing a decorator
    // explicit: an active live cache becomes `cache: None` on the layer.
    result
        .commands
        .extend(reconcile_function_caches(existing, new));

    // Anchored `base.path = value` writes carry no `&Call`, so synthesize the equivalent
    // `struct_set` calls the anchor-keyed collectors below diff against / delete. These arenas own
    // the calls and must outlive the `new_calls`/`visible` maps that borrow them, so build them in
    // full up front — pushing to the Vec later would reallocate and invalidate the references.
    let new_field_assign_calls = collect_anchored_field_assign_calls(new);
    let board_field_assign_calls = collect_anchored_field_assign_calls(&board_ast);

    // 1. Index every anchored call in the new AST by node id.
    let mut new_calls: HashMap<String, &Call> = HashMap::new();
    collect_calls(new, &mut new_calls);
    for call in &new_field_assign_calls {
        if let Some(anchor) = call.anchor.as_deref() {
            new_calls.entry(anchor.to_string()).or_insert(call);
        }
    }

    // 2. Configuration edits: for each anchored call that still maps to a board node, diff its
    //    literal arguments against the node's current pin defaults.
    for (anchor, call) in &new_calls {
        let Some(node) = find_board_node(existing, anchor) else {
            result.diagnostics.push(format!(
                "anchor {anchor} no longer resolves to a board node; skipped"
            ));
            continue;
        };
        // Multi-pins (several input pins sharing one name) pair positionally with the
        // same-named args, mirroring the order lowering emitted them in.
        let mut same_name_seen: HashMap<&str, usize> = HashMap::new();
        for (arg_index, arg) in call.args.iter().enumerate() {
            let occurrence = {
                let counter = same_name_seen.entry(arg.name.as_str()).or_insert(0);
                let index = *counter;
                *counter += 1;
                index
            };
            let Some(mut new_value) = literal_expr_to_value(&arg.value) else {
                // References / nested calls describe wiring, which v1 does not rewrite.
                continue;
            };
            let direct_pins = matching_input_pins(node, &arg.name);
            let alias_target = direct_pins
                .get(occurrence)
                .is_none()
                .then(|| input_arg_alias_target(&node.name, call, arg_index))
                .flatten();
            let (pins, target_occurrence) = match alias_target {
                Some((name, alias_occurrence)) => {
                    (matching_input_pins(node, name), alias_occurrence)
                }
                None => (direct_pins, occurrence),
            };
            let Some(pin) = pins.get(target_occurrence).copied() else {
                // No pin at this occurrence: a literal on a pin the node's `on_update` will mint
                // (e.g. a placeholder added to an existing `string_format`) is deferred to apply,
                // which creates the pin and then applies the write; a genuinely unknown pin stays a
                // (non-fatal) skip.
                if arg_targets_predicted_dynamic_pin(node, anchor, call, arg, existing, enricher) {
                    normalize_variable_ref_value_for_pin(&mut new_value, &arg.name, &variable_refs);
                    result.commands.push(BoardCommand::UpdateNodePin {
                        node_id: anchor.clone(),
                        pin_id: arg.name.clone(),
                        value: new_value,
                        summary: Some(format!("Set {} on {}", arg.name, node.friendly_name)),
                    });
                } else if is_widget_dynamic_binding_arg(&arg.name) {
                    // These pins are minted by the node's `on_update` from the PERSISTED widget, so
                    // a missing one almost always means the widget itself is not there yet (wrong
                    // selector, or a page/widget build that has not landed). Saying only "no pin
                    // named X; skipped" reads like a typo and silently drops the data binding.
                    result.diagnostics.push(format!(
                        "node {anchor} has no input pin named {:?} (occurrence {}). This is a widget data binding, and those pins only exist once the widget is persisted — check that the widget selector names an existing widget and that its page/widget build has completed, then re-check. No part of this revision was applied.",
                        arg.name,
                        occurrence + 1
                    ));
                } else {
                    result.diagnostics.push(format!(
                        "node {anchor} has no input pin named {:?} (occurrence {}). No part of this revision was applied.",
                        arg.name,
                        occurrence + 1
                    ));
                }
                continue;
            };
            if alias_target.is_some() {
                result.corrections.push(input_arg_alias_correction(
                    call,
                    arg,
                    &pin.name,
                    target_occurrence,
                    pins.len(),
                ));
            }
            normalize_variable_ref_value_for_pin(&mut new_value, &pin.name, &variable_refs);
            let current = pin
                .default_value
                .as_deref()
                .and_then(|b| flow_like_types::json::from_slice::<flow_like_types::Value>(b).ok());
            if current.as_ref() == Some(&new_value) {
                continue; // unchanged
            }
            // Composite literals are how struct_make/make_array sugar (and unset composite
            // pins) render. A wired pin's value is carried by its edge — a default written
            // beneath it is dead weight — and `{}`/`[]` on an unset defaultless pin is the
            // representation of "unset". Neither is a configuration edit.
            if matches!(
                new_value,
                flow_like_types::Value::Object(_) | flow_like_types::Value::Array(_)
            ) {
                if !pin.depends_on.is_empty() {
                    continue;
                }
                let empty_composite = match &new_value {
                    flow_like_types::Value::Object(map) => map.is_empty(),
                    flow_like_types::Value::Array(items) => items.is_empty(),
                    _ => false,
                };
                if empty_composite && matches!(current, None | Some(flow_like_types::Value::Null)) {
                    continue;
                }
            }
            result.commands.push(BoardCommand::UpdateNodePin {
                node_id: anchor.clone(),
                // The name is ambiguous across multi-pins; address those by exact pin id.
                pin_id: if pins.len() > 1 {
                    pin.id.clone()
                } else {
                    pin.name.clone()
                },
                value: new_value,
                summary: Some(format!("Set {} on {}", arg.name, node.friendly_name)),
            });
        }
    }

    // 3. Deletions: a text-visible anchored node absent from the new AST is a removal. We compute
    //    "text-visible" from the board's own lowered AST so sugared/inlined nodes (reroutes,
    //    struct_make, pure helpers) are never removed just for lacking an anchor in the text.
    let mut visible: HashMap<String, &Call> = HashMap::new();
    collect_statement_calls(&board_ast, &mut visible);
    for call in &board_field_assign_calls {
        if let Some(anchor) = call.anchor.as_deref() {
            visible.entry(anchor.to_string()).or_insert(call);
        }
    }
    let new_anchors: HashSet<&String> = new_calls.keys().collect();
    let mut removed_ids: HashSet<String> = HashSet::new();
    if mode == ReconcileMode::Replace {
        for anchor in visible.keys() {
            if new_anchors.contains(anchor) {
                continue;
            }
            let Some(node) = find_board_node(existing, anchor) else {
                continue;
            };
            removed_ids.insert(anchor.clone());
            result.commands.push(BoardCommand::RemoveNode {
                node_id: anchor.clone(),
                summary: Some(format!("Remove {}", node.friendly_name)),
            });
        }
    }

    // 4. Structural authoring: catalog-aware FlowPilot calls can add new unanchored calls in the
    //    text. Translate those to the same command format the UI already reviews/applies.
    if let Some(catalog) = catalog {
        let structural = StructuralPlanner::new(existing, catalog, enricher).plan(new);
        if !structural.commands.is_empty() {
            let anchored_edits = std::mem::take(&mut result.commands);
            result.commands = structural.commands;
            result.commands.extend(anchored_edits);
        }
        result.corrections.extend(structural.corrections);
        result.diagnostics.extend(structural.diagnostics);
    } else if ast_has_unanchored_calls(new) {
        result.diagnostics.push(
            "FlowScript contains new unanchored calls; catalog metadata is required to turn them into board commands."
                .to_string(),
        );
    }

    // Bridge severed exec chains AFTER structural planning merged: when a removed statement was
    // REPLACED, the planner already re-targeted the predecessor's exec output (exec outputs are
    // single-target replace at apply), so a bridge from that same source would steal the new
    // node's wiring and orphan it. Skip sources the merged plan already drives.
    if !removed_ids.is_empty() {
        let occupied_sources: HashSet<(String, String)> = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    ..
                } => Some((from_node.clone(), from_pin.clone())),
                _ => None,
            })
            .collect();
        bridge_removed_exec_chains(
            existing,
            &removed_ids,
            &occupied_sources,
            &mut result.commands,
            &mut result.diagnostics,
        );
    }

    // Layer-size gate LAST, over the merged command set: a violating edit queues nothing.
    if let Some(violations) = layer_node_limit_violations(existing, &result.commands) {
        result.commands.clear();
        result.diagnostics.extend(violations);
    }

    result.corrections.sort();
    result.corrections.dedup();

    result
}

fn function_cache_to_layer_cache(cache: &FunctionCache) -> LayerCache {
    LayerCache {
        enabled: true,
        prefix: cache.namespace.clone(),
        ttl_seconds: cache.ttl_seconds,
        scope: match cache.scope {
            FunctionCacheScope::App => LayerCacheScope::App,
            FunctionCacheScope::User => LayerCacheScope::User,
        },
    }
}

fn matching_function_layer<'a>(existing: &'a Board, func: &FnDecl) -> Option<&'a Layer> {
    if let Some(anchor) = func.anchor.as_deref() {
        return existing.layers.get(anchor).filter(|layer| {
            matches!(layer.r#type, LayerType::Function)
                && to_camel_case(&layer.name) == to_camel_case(&func.name)
        });
    }

    let normalized_name = to_camel_case(&func.name);
    let mut matches = existing.layers.values().filter(|layer| {
        matches!(layer.r#type, LayerType::Function) && to_camel_case(&layer.name) == normalized_name
    });
    let only = matches.next()?;
    matches.next().is_none().then_some(only)
}

fn reconcile_function_caches(existing: &Board, ast: &BoardAst) -> Vec<BoardCommand> {
    ast.functions
        .iter()
        .filter_map(|func| {
            let layer = matching_function_layer(existing, func)?;
            let desired = func.cache.as_ref().map(function_cache_to_layer_cache);
            // Disabled cache records are equivalent to an absent decorator. Preserve those dormant
            // settings unless FlowScript explicitly enables caching, avoiding a no-op rewrite.
            let current = layer
                .cache
                .as_ref()
                .filter(|cache| cache.is_active())
                .map(|cache| {
                    let mut cache = cache.clone();
                    // Persisted `None` and explicit zero are the same permanent policy. Lowering
                    // spells both as `ttlSeconds: 0`, so compare their canonical forms here to
                    // keep an unchanged board -> FlowScript -> board round-trip command-free.
                    if cache.ttl_seconds.is_none() {
                        cache.ttl_seconds = Some(0);
                    }
                    cache
                });
            (current != desired).then(|| BoardCommand::UpdateLayerCache {
                layer_id: layer.id.clone(),
                summary: Some(if desired.is_some() {
                    format!("Update cache for function {}", func.name)
                } else {
                    format!("Disable cache for function {}", func.name)
                }),
                cache: desired,
            })
        })
        .collect()
}

/// Reject semantic declarations that would otherwise collide in the planner's name/id maps.
/// Reconcile is atomic, so an ambiguous document must produce diagnostics and zero commands rather
/// than creating duplicate variables or an orphan Function layer before a later declaration wins.
fn duplicate_ast_declaration_diagnostics(ast: &BoardAst) -> Vec<String> {
    fn register_duplicates<'a>(
        names: impl IntoIterator<Item = &'a str>,
        noun: &str,
        scope: &str,
        diagnostics: &mut Vec<String>,
    ) {
        let mut counts = BTreeMap::<&str, usize>::new();
        for name in names {
            *counts.entry(name).or_default() += 1;
        }
        for (name, count) in counts {
            if count > 1 {
                diagnostics.push(format!(
                    "duplicate FlowScript {noun} `{name}` {scope}; no commands were derived"
                ));
            }
        }
    }

    fn register_normalized_boundary_duplicates<'a>(
        names: impl IntoIterator<Item = &'a str>,
        noun: &str,
        scope: &str,
        diagnostics: &mut Vec<String>,
    ) {
        let mut originals = BTreeMap::<String, Vec<&str>>::new();
        for name in names {
            originals.entry(to_camel_case(name)).or_default().push(name);
        }
        for (normalized, mut names) in originals {
            names.sort_unstable();
            names.dedup();
            if names.len() > 1 {
                diagnostics.push(format!(
                    "FlowScript {noun} names {} collide as normalized boundary name `{normalized}` {scope}; no commands were derived",
                    names
                        .into_iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }

    fn visit_event(event: &EventBlock, scope: &str, diagnostics: &mut Vec<String>) {
        register_duplicates(
            event.params.iter().map(|param| param.name.as_str()),
            "event parameter",
            scope,
            diagnostics,
        );
        register_normalized_boundary_duplicates(
            event.params.iter().map(|param| param.name.as_str()),
            "event parameter",
            scope,
            diagnostics,
        );
        visit_block(&event.body, scope, diagnostics);
    }

    fn visit_block(block: &Block, scope: &str, diagnostics: &mut Vec<String>) {
        for stmt in &block.stmts {
            match stmt {
                Stmt::Branch { arms, .. } => {
                    register_duplicates(
                        arms.iter().map(|arm| arm.label.as_str()),
                        "branch arm label",
                        scope,
                        diagnostics,
                    );
                    for arm in arms {
                        visit_block(&arm.body, scope, diagnostics);
                    }
                }
                Stmt::Loop { body, .. } => visit_block(body, scope, diagnostics),
                Stmt::Handler(event) => visit_event(event, scope, diagnostics),
                Stmt::Let { .. }
                | Stmt::Call { .. }
                | Stmt::Assign { .. }
                | Stmt::FieldAssign { .. }
                | Stmt::LocalAlias { .. }
                | Stmt::Return { .. }
                | Stmt::Local(_)
                | Stmt::Comment(_) => {}
            }
        }
    }

    let mut diagnostics = Vec::new();
    register_duplicates(
        ast.variables.iter().map(|variable| variable.name.as_str()),
        "variable declaration",
        "at the top level",
        &mut diagnostics,
    );
    register_duplicates(
        ast.functions.iter().map(|function| function.name.as_str()),
        "function declaration",
        "at the top level",
        &mut diagnostics,
    );
    // Named events and functions are both registered as same-batch aliases by the apply planner.
    // Two named events (even of different catalog types), or a function and named event sharing a
    // name, therefore make `SetNodeFunctionRefs` resolution ambiguous. Reject that document here
    // so `check_flowscript` can return an actionable diagnostic instead of allowing commit to
    // queue a batch that apply must roll back atomically.
    // Existing anchored declarations may already share a friendly name: the apply planner records
    // that alias as ambiguous, but an unchanged round-trip remains valid because no command needs
    // to resolve it. Grandfather that persisted state; reject a collision only when this document
    // introduces at least one unanchored callable into the shared alias namespace.
    let mut callable_declarations = BTreeMap::<&str, (usize, usize, Vec<(&str, bool)>)>::new();
    for function in &ast.functions {
        let entry = callable_declarations
            .entry(function.name.as_str())
            .or_default();
        entry.0 += 1;
        if function.anchor.is_none() {
            entry.1 += 1;
        }
    }
    for event in &ast.events {
        let Some(event_name) = event
            .event_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        callable_declarations
            .entry(event_name)
            .or_default()
            .2
            .push((event.name.as_str(), event.anchor.is_none()));
    }
    for (name, (function_count, unanchored_function_count, mut events)) in callable_declarations {
        let introduces_callable =
            unanchored_function_count > 0 || events.iter().any(|(_, unanchored)| *unanchored);
        if events.is_empty() || function_count + events.len() < 2 || !introduces_callable {
            continue;
        }
        let mut event_types = events
            .drain(..)
            .map(|(event_type, _)| event_type)
            .collect::<Vec<_>>();
        event_types.sort_unstable();
        let function_origin = match function_count {
            0 => String::new(),
            1 => "a function and ".to_string(),
            count => format!("{count} functions and "),
        };
        let event_count = event_types.len();
        let event_noun = if event_count == 1 {
            "named event"
        } else {
            "named events"
        };
        diagnostics.push(format!(
            "top-level FlowScript callable name `{name}` is ambiguous across {function_origin}{event_count} {event_noun} ({}); named events and functions share the apply resolver namespace, so give each callable a unique name; no commands were derived",
            event_types
                .into_iter()
                .map(|event_type| format!("`{event_type}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    register_duplicates(
        ast.interfaces
            .iter()
            .map(|interface| interface.name.as_str()),
        "interface declaration",
        "at the top level",
        &mut diagnostics,
    );

    for interface in &ast.interfaces {
        let scope = format!("in interface `{}`", interface.name);
        register_duplicates(
            interface.fields.iter().map(|field| field.name.as_str()),
            "interface field",
            &scope,
            &mut diagnostics,
        );
    }
    for function in &ast.functions {
        let scope = format!("in function `{}`", function.name);
        register_duplicates(
            function.params.iter().map(|param| param.name.as_str()),
            "function parameter",
            &scope,
            &mut diagnostics,
        );
        register_normalized_boundary_duplicates(
            function.params.iter().map(|param| param.name.as_str()),
            "function parameter",
            &scope,
            &mut diagnostics,
        );
        register_duplicates(
            function.returns.iter().map(|param| param.name.as_str()),
            "function return",
            &scope,
            &mut diagnostics,
        );
        register_normalized_boundary_duplicates(
            function.returns.iter().map(|param| param.name.as_str()),
            "function return",
            &scope,
            &mut diagnostics,
        );
        let parameter_names = function
            .params
            .iter()
            .map(|param| param.name.as_str())
            .collect::<HashSet<_>>();
        let mut collisions = function
            .returns
            .iter()
            .map(|param| param.name.as_str())
            .filter(|name| parameter_names.contains(name))
            .collect::<Vec<_>>();
        collisions.sort_unstable();
        collisions.dedup();
        for name in collisions {
            diagnostics.push(format!(
                "FlowScript function boundary name `{name}` is used by both a parameter and return {scope}; no commands were derived"
            ));
        }
        let parameter_names = function
            .params
            .iter()
            .map(|param| to_camel_case(&param.name))
            .collect::<HashSet<_>>();
        let mut normalized_collisions = function
            .returns
            .iter()
            .filter_map(|param| {
                let normalized = to_camel_case(&param.name);
                parameter_names.contains(&normalized).then_some(normalized)
            })
            .collect::<Vec<_>>();
        normalized_collisions.sort_unstable();
        normalized_collisions.dedup();
        for name in normalized_collisions {
            diagnostics.push(format!(
                "FlowScript function normalized boundary name `{name}` is used by both a parameter and return {scope}; no commands were derived"
            ));
        }
        visit_block(&function.body, &scope, &mut diagnostics);
    }
    for (index, event) in ast.events.iter().enumerate() {
        let scope = format!("in event `{}` #{}", event.name, index + 1);
        visit_event(event, &scope, &mut diagnostics);
    }

    diagnostics.sort();
    diagnostics.dedup();
    diagnostics
}

fn duplicate_ast_anchors(ast: &BoardAst) -> Vec<String> {
    fn register(anchor: Option<&str>, seen: &mut HashSet<String>, duplicate: &mut HashSet<String>) {
        let Some(anchor) = anchor.map(str::trim).filter(|anchor| !anchor.is_empty()) else {
            return;
        };
        if !seen.insert(anchor.to_string()) {
            duplicate.insert(anchor.to_string());
        }
    }

    fn visit_call(
        call: &Call,
        fallback_anchor: Option<&str>,
        seen: &mut HashSet<String>,
        duplicate: &mut HashSet<String>,
    ) {
        register(call.anchor.as_deref().or(fallback_anchor), seen, duplicate);
        for argument in &call.args {
            visit_expr(&argument.value, None, seen, duplicate);
        }
    }

    fn visit_expr(
        expr: &Expr,
        fallback_anchor: Option<&str>,
        seen: &mut HashSet<String>,
        duplicate: &mut HashSet<String>,
    ) {
        match expr {
            Expr::Call(call) => visit_call(call, fallback_anchor, seen, duplicate),
            Expr::Field { base, .. } | Expr::Member { base, .. } => {
                visit_expr(base, fallback_anchor, seen, duplicate)
            }
            Expr::Object(fields) => {
                register(fallback_anchor, seen, duplicate);
                for field in fields {
                    visit_expr(&field.value, None, seen, duplicate);
                }
            }
            Expr::Array(items) => {
                register(fallback_anchor, seen, duplicate);
                for item in items {
                    visit_expr(item, None, seen, duplicate);
                }
            }
            Expr::Index { base, index }
            | Expr::Binary {
                lhs: base,
                rhs: index,
                ..
            } => {
                register(fallback_anchor, seen, duplicate);
                visit_expr(base, None, seen, duplicate);
                visit_expr(index, None, seen, duplicate);
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                register(fallback_anchor, seen, duplicate);
                visit_expr(cond, None, seen, duplicate);
                visit_expr(then, None, seen, duplicate);
                visit_expr(otherwise, None, seen, duplicate);
            }
            Expr::Ref(_) | Expr::Literal(_) => register(fallback_anchor, seen, duplicate),
        }
    }

    fn visit_event(
        event: &EventBlock,
        seen: &mut HashSet<String>,
        duplicate: &mut HashSet<String>,
    ) {
        register(event.anchor.as_deref(), seen, duplicate);
        // The lowerer renders an entry node with MULTIPLE exec outputs as its EventBlock header
        // plus an immediate arm-routing Branch carrying the SAME anchor (the arms select the
        // entry's own labelled outputs). That pair is one entity: the planner resolves the
        // anchored branch back to the existing entry node, so the reappearing anchor is legal
        // exactly once, as the first body statement.
        let mut statements = event.body.stmts.iter();
        if let (
            Some(event_anchor),
            Some(Stmt::Branch {
                call,
                condition,
                arms,
                anchor,
                ..
            }),
        ) = (
            event
                .anchor
                .as_deref()
                .map(str::trim)
                .filter(|anchor| !anchor.is_empty()),
            event.body.stmts.first(),
        ) {
            let branch_anchor = anchor
                .as_deref()
                .or(call.anchor.as_deref())
                .map(str::trim)
                .filter(|anchor| !anchor.is_empty());
            if branch_anchor == Some(event_anchor) {
                for argument in &call.args {
                    visit_expr(&argument.value, None, seen, duplicate);
                }
                if let Some(condition) = condition {
                    visit_expr(condition, None, seen, duplicate);
                }
                for arm in arms {
                    visit_block(&arm.body, seen, duplicate);
                }
                statements.next();
            }
        }
        for statement in statements {
            visit_stmt(statement, seen, duplicate);
        }
    }

    fn visit_block(block: &Block, seen: &mut HashSet<String>, duplicate: &mut HashSet<String>) {
        for statement in &block.stmts {
            visit_stmt(statement, seen, duplicate);
        }
    }

    fn visit_stmt(statement: &Stmt, seen: &mut HashSet<String>, duplicate: &mut HashSet<String>) {
        match statement {
            Stmt::Let { call, anchor, .. } | Stmt::Call { call, anchor } => {
                visit_call(call, anchor.as_deref(), seen, duplicate)
            }
            Stmt::Branch {
                call,
                condition,
                arms,
                anchor,
                ..
            } => {
                visit_call(call, anchor.as_deref(), seen, duplicate);
                if let Some(condition) = condition {
                    visit_expr(condition, None, seen, duplicate);
                }
                for arm in arms {
                    visit_block(&arm.body, seen, duplicate);
                }
            }
            Stmt::Loop {
                call, body, anchor, ..
            } => {
                visit_call(call, anchor.as_deref(), seen, duplicate);
                visit_block(body, seen, duplicate);
            }
            Stmt::Assign { value, anchor, .. } | Stmt::LocalAlias { value, anchor, .. } => {
                visit_expr(value, anchor.as_deref(), seen, duplicate)
            }
            Stmt::FieldAssign { value, anchor, .. } => {
                register(anchor.as_deref(), seen, duplicate);
                visit_expr(value, None, seen, duplicate);
            }
            Stmt::Return { values, anchor } => {
                register(anchor.as_deref(), seen, duplicate);
                for value in values {
                    visit_expr(value, None, seen, duplicate);
                }
            }
            Stmt::Local(variable) => register(variable.anchor.as_deref(), seen, duplicate),
            Stmt::Handler(event) => visit_event(event, seen, duplicate),
            Stmt::Comment(_) => {}
        }
    }

    let mut seen = HashSet::new();
    let mut duplicate = HashSet::new();
    for variable in &ast.variables {
        register(variable.anchor.as_deref(), &mut seen, &mut duplicate);
    }
    for function in &ast.functions {
        register(function.anchor.as_deref(), &mut seen, &mut duplicate);
        visit_block(&function.body, &mut seen, &mut duplicate);
    }
    for event in &ast.events {
        visit_event(event, &mut seen, &mut duplicate);
    }
    let mut duplicate = duplicate.into_iter().collect::<Vec<_>>();
    duplicate.sort();
    duplicate
}

/// Re-join execution chains severed by deletions: for each removed segment whose predecessor and
/// (transitive) successor survive, emit the bridging ConnectPins so the tail keeps running.
/// Layer boundary pins count as endpoints — deleting the first statement of a function body
/// re-joins its layer's `exec_in` to the surviving successor. Sources the merged plan already
/// drives (`occupied_sources`) are skipped: the planner's replacement wiring wins. Ambiguous
/// shapes (multiple predecessors or successors) get a diagnostic instead of a guess.
fn bridge_removed_exec_chains(
    existing: &Board,
    removed: &HashSet<String>,
    occupied_sources: &HashSet<(String, String)>,
    commands: &mut Vec<BoardCommand>,
    diagnostics: &mut Vec<String>,
) {
    // pin id → (owner ref for commands, pin, walkable node when the owner is a node).
    let mut pin_owner: HashMap<&str, (String, &Pin, Option<&Node>)> = HashMap::new();
    for node in all_board_nodes(existing) {
        let mut pins = node.pins.values().collect::<Vec<_>>();
        pins.sort_by(|left, right| left.id.cmp(&right.id));
        for pin in pins {
            pin_owner.insert(pin.id.as_str(), (node.id.clone(), pin, Some(node)));
        }
    }
    for layer in existing.layers.values() {
        for pin in layer.pins.values() {
            pin_owner.insert(pin.id.as_str(), (layer.id.clone(), pin, None));
        }
    }

    for node_id in removed {
        let Some(node) = find_board_node(existing, node_id) else {
            continue;
        };
        // Only bridge from a segment head: its predecessor survives the edit. Mid-segment
        // removed nodes are covered by their head's forward walk.
        let mut preds: Vec<(String, String)> = node
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Input && is_exec_pin(pin))
            .flat_map(|pin| pin.depends_on.iter())
            .filter_map(|pin_id| pin_owner.get(pin_id.as_str()))
            .filter(|(owner_id, _, _)| !removed.contains(owner_id))
            .map(|(owner_id, pin, _)| (owner_id.clone(), pin.name.clone()))
            .collect();
        preds.sort();
        preds.dedup();
        let [(pred_node, pred_pin)] = preds.as_slice() else {
            continue;
        };
        if occupied_sources.contains(&(pred_node.clone(), pred_pin.clone())) {
            continue;
        }

        let mut frontier = vec![node];
        let mut visited: HashSet<String> = HashSet::new();
        let mut successors: Vec<(String, String)> = Vec::new();
        while let Some(current) = frontier.pop() {
            if !visited.insert(current.id.clone()) {
                continue;
            }
            for pin in current
                .pins
                .values()
                .filter(|pin| pin.pin_type == PinType::Output && is_exec_pin(pin))
            {
                for target_pin_id in &pin.connected_to {
                    let Some((owner_id, target_pin, owner_node)) =
                        pin_owner.get(target_pin_id.as_str())
                    else {
                        continue;
                    };
                    if removed.contains(owner_id) {
                        if let Some(owner_node) = owner_node {
                            frontier.push(owner_node);
                        }
                    } else {
                        successors.push((owner_id.clone(), target_pin.name.clone()));
                    }
                }
            }
        }
        successors.sort();
        successors.dedup();

        match successors.as_slice() {
            [] => {}
            [(succ_node, succ_pin)] => {
                commands.push(BoardCommand::ConnectPins {
                    from_node: pred_node.clone(),
                    from_pin: pred_pin.clone(),
                    to_node: succ_node.clone(),
                    to_pin: succ_pin.clone(),
                    summary: Some(format!(
                        "Re-join execution around removed {}",
                        node.friendly_name
                    )),
                });
            }
            _ => diagnostics.push(format!(
                "removing `{}` leaves multiple execution successors; the chain after it was not re-joined automatically — reconnect the intended successor explicitly",
                node.friendly_name
            )),
        }
    }
}

/// Enforce [`MAX_NODES_PER_LAYER`]: existing per-layer populations plus the edit's additions
/// (minus its removals) must stay within the cap. Returns violation diagnostics, or `None` when
/// the edit fits.
fn layer_node_limit_violations(existing: &Board, commands: &[BoardCommand]) -> Option<Vec<String>> {
    let mut counts: HashMap<Option<String>, i64> = HashMap::new();
    let mut layer_names: HashMap<String, String> = HashMap::new();
    // Canonical boards keep every node in `board.nodes` and identify Function membership through
    // `node.layer`. Some legacy/readback paths also mirror those same identities in
    // `layer.nodes`. Count identities, not map entries, or a valid 26-node Function temporarily
    // represented in both stores looks like 52 nodes and every edit is rejected by the limit.
    // The flat representation is authoritative; layer-local maps only fill identities absent from
    // it. Normalize an empty layer id to root, matching placement and apply semantics.
    let normalize_layer = |layer: Option<&str>| {
        layer
            .filter(|layer_id| !layer_id.is_empty())
            .map(str::to_string)
    };
    let mut node_layers: HashMap<String, Option<String>> = HashMap::new();

    for node in existing.nodes.values() {
        node_layers.insert(node.id.clone(), normalize_layer(node.layer.as_deref()));
    }
    for layer in existing.layers.values() {
        layer_names.insert(layer.id.clone(), layer.name.clone());
        for node in layer.nodes.values() {
            node_layers
                .entry(node.id.clone())
                .or_insert_with(|| normalize_layer(Some(layer.id.as_str())));
        }
    }
    for layer in node_layers.values() {
        *counts.entry(layer.clone()).or_default() += 1;
    }

    let mut net_added: HashMap<Option<String>, i64> = HashMap::new();
    for command in commands {
        match command {
            BoardCommand::AddNode { target_layer, .. } => {
                *counts.entry(target_layer.clone()).or_default() += 1;
                *net_added.entry(target_layer.clone()).or_default() += 1;
            }
            BoardCommand::CreateLayer {
                ref_id: Some(ref_id),
                name,
                ..
            } => {
                layer_names.insert(ref_id.clone(), name.clone());
            }
            BoardCommand::RemoveNode { node_id, .. } => {
                if let Some(layer) = node_layers.get(node_id) {
                    *counts.entry(layer.clone()).or_default() -= 1;
                    *net_added.entry(layer.clone()).or_default() -= 1;
                }
            }
            _ => {}
        }
    }

    // Only an edit that GROWS a layer past the cap is rejected: a layer that already exceeds it
    // (legacy boards predating the limit) must stay editable — and re-appliable — as long as the
    // edit does not add net nodes there.
    let mut violations: Vec<String> = counts
        .iter()
        .filter(|(layer, count)| {
            **count > MAX_NODES_PER_LAYER as i64
                && net_added.get(*layer).copied().unwrap_or_default() > 0
        })
        .map(|(layer, count)| {
            let scope = match layer {
                Some(id) => format!(
                    "function/layer `{}`",
                    layer_names.get(id).cloned().unwrap_or_else(|| id.clone())
                ),
                None => "the root layer".to_string(),
            };
            format!(
                "this edit would leave {scope} with {count} nodes (max {MAX_NODES_PER_LAYER}). Nothing was queued. Split the logic into smaller `function name(...) {{ ... }}` declarations — each function layer has its own {MAX_NODES_PER_LAYER}-node budget — and call them instead"
            )
        })
        .collect();
    violations.sort();
    (!violations.is_empty()).then_some(violations)
}

/// Represent a JSON value as the literal expression form the parser would have produced for it,
/// so JSON-shaped text can flow through expression machinery that matches on `Expr::Literal`.
fn json_value_literal_expr(value: &flow_like_types::Value) -> Expr {
    use flow_like_types::Value;
    Expr::Literal(match value {
        Value::String(s) => Literal::String(s.clone()),
        Value::Bool(b) => Literal::Bool(*b),
        Value::Null => Literal::Null,
        Value::Number(n) => {
            if let Some(int) = n.as_i64() {
                Literal::Int(int)
            } else {
                Literal::Float(n.as_f64().unwrap_or_default())
            }
        }
        composite => Literal::Json(
            flow_like_types::json::to_string(composite).unwrap_or_else(|_| "null".to_string()),
        ),
    })
}

/// Convert an AST [`Literal`] into the JSON value a [`BoardCommand::UpdateNodePin`] carries.
fn literal_to_value(lit: &Literal) -> flow_like_types::Value {
    use flow_like_types::Value;
    match lit {
        Literal::String(s) => Value::String(s.clone()),
        Literal::Int(i) => Value::from(*i),
        Literal::Float(f) => Value::from(*f),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Null => Value::Null,
        Literal::Json(raw) => {
            flow_like_types::json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
        }
    }
}

fn reconcile_variables(
    live_board: &Board,
    existing: &BoardAst,
    new: &BoardAst,
    mode: ReconcileMode,
) -> ReconcileResult {
    let mut result = ReconcileResult::default();
    let mut existing_by_anchor: HashMap<&str, &VarDecl> = HashMap::new();
    let mut existing_by_name: HashMap<&str, &VarDecl> = HashMap::new();

    for var in &existing.variables {
        if let Some(anchor) = var.anchor.as_deref() {
            existing_by_anchor.insert(anchor, var);
        }
        existing_by_name.insert(var.name.as_str(), var);
    }

    let mut seen_existing = HashSet::new();
    for var in &new.variables {
        if secret_initializer_contains_value(var) {
            result.diagnostics.push(format!(
                "secret variable `{}` cannot take a non-empty FlowScript-authored default; use an empty placeholder and configure the value through trusted secret settings",
                var.name
            ));
            continue;
        }
        let matched = match var.anchor.as_deref() {
            Some(anchor) => match existing_by_anchor.get(anchor).copied() {
                Some(existing) => Some(existing),
                None => {
                    result.diagnostics.push(format!(
                        "variable `{}` anchors to `{anchor}`, which no longer exists on the board; the explicit anchor was not replaced by a name match",
                        var.name
                    ));
                    continue;
                }
            },
            None => existing_by_name.get(var.name.as_str()).copied(),
        };

        match matched {
            Some(old) => {
                if let Some(anchor) = old.anchor.as_deref() {
                    seen_existing.insert(anchor.to_string());
                }
                let hidden_default_present = old
                    .anchor
                    .as_deref()
                    .and_then(|anchor| live_board.variables.get(anchor))
                    .is_some_and(|variable| variable.secret && variable.default_value.is_some());
                match update_variable_command(existing, new, old, var, hidden_default_present) {
                    Err(diagnostic) => result.diagnostics.push(diagnostic),
                    Ok(update) => {
                        if mode == ReconcileMode::Additive
                            && var.anchor.is_none()
                            && update.is_some()
                        {
                            result.diagnostics.push(format!(
                                "additive variable `{}` collides with an existing variable but omits its exact anchor; the existing type/default/security configuration was preserved",
                                var.name
                            ));
                            continue;
                        }
                        if let Some(command) = update {
                            result.commands.push(command);
                        }
                    }
                }
            }
            None => {
                result.commands.push(create_variable_command(new, var));
            }
        }
    }

    if mode == ReconcileMode::Replace {
        for var in &existing.variables {
            let Some(anchor) = var.anchor.as_deref() else {
                continue;
            };
            if seen_existing.contains(anchor) {
                continue;
            }
            let still_present_by_name = new.variables.iter().any(|new_var| {
                new_var.anchor.as_deref() == Some(anchor)
                    || (new_var.anchor.is_none() && new_var.name == var.name)
            });
            if still_present_by_name {
                continue;
            }
            result.commands.push(BoardCommand::RemoveVariable {
                variable_id: anchor.to_string(),
                summary: Some(format!("Remove variable {}", var.name)),
            });
        }
    }

    result
}

fn create_variable_command(ast: &BoardAst, var: &VarDecl) -> BoardCommand {
    BoardCommand::CreateVariable {
        variable_id: Some(variable_id_for_decl(var)),
        name: var.name.clone(),
        data_type: variable_data_type(var).to_string(),
        value_type: variable_value_type(var).to_string(),
        // FlowScript is model-visible and therefore cannot be a credential write channel. A
        // secret declaration may carry an empty initializer for readability, but only a
        // trusted secret-setting path may populate the stored default.
        default_value: (!var.secret).then(|| variable_default_value(var)).flatten(),
        description: var.description.clone(),
        category: var.category.clone(),
        schema: visible_variable_schema(ast, var),
        exposed: Some(var.exposed),
        secret: Some(var.secret),
        editable: Some(var.editable),
        runtime_configured: Some(var.runtime_configured),
        target_layer: None,
        summary: Some(format!("Create variable {}", var.name)),
    }
}

fn variable_id_for_decl(var: &VarDecl) -> String {
    var.anchor
        .clone()
        .unwrap_or_else(|| generated_variable_id(&var.name))
}

fn generated_variable_id(name: &str) -> String {
    let mut out = String::from("var_");
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out == "var_" {
        out.push_str("value");
    }
    out
}

fn secret_initializer_contains_value(var: &VarDecl) -> bool {
    if !var.secret {
        return false;
    }
    match var.default.as_ref() {
        None | Some(Literal::Null) => false,
        Some(Literal::String(value)) => !value.is_empty(),
        Some(_) => true,
    }
}

fn update_variable_command(
    old_ast: &BoardAst,
    new_ast: &BoardAst,
    old: &VarDecl,
    new: &VarDecl,
    hidden_default_present: bool,
) -> Result<Option<BoardCommand>, String> {
    let Some(variable_id) = old.anchor.clone() else {
        return Ok(None);
    };
    let old_default = variable_default_value(old);
    let new_default = variable_default_value(new);
    let old_schema = visible_variable_schema(old_ast, old);
    let new_schema = visible_variable_schema(new_ast, new);

    let data_type_changed = variable_data_type(old) != variable_data_type(new);
    let value_type_changed = variable_value_type(old) != variable_value_type(new);
    let schema_changed =
        old_schema != new_schema && !schemas_structurally_equivalent(old_ast, new_ast, old, new);

    // A hidden default cannot be validated against a new representation without exposing it to
    // the text/model domain. Preserve it only while its type and schema stay unchanged. The user
    // can declassify in one atomic clear-and-flag update before changing the representation.
    if old.secret
        && new.secret
        && hidden_default_present
        && (data_type_changed || value_type_changed || schema_changed)
    {
        return Err(format!(
            "secret variable `{}` has a hidden default, so its type/value shape/schema cannot be changed while it remains secret; declassify it without an initializer to clear the hidden default atomically",
            new.name
        ));
    }

    // Never replace a hidden credential with model-authored text during declassification. The
    // first transition clears the credential and secret flag together; a later non-secret edit
    // may set an ordinary default through the normal path.
    if old.secret && !new.secret && hidden_default_present && new_default.is_some() {
        return Err(format!(
            "secret variable `{}` cannot be declassified with a FlowScript-authored default; remove the initializer so the hidden default can be cleared atomically",
            new.name
        ));
    }

    let mut changed = false;

    let name = changed_option(new.name.clone(), old.name != new.name, &mut changed);
    let data_type = changed_option(
        variable_data_type(new).to_string(),
        data_type_changed,
        &mut changed,
    );
    let value_type = changed_option(
        variable_value_type(new).to_string(),
        value_type_changed,
        &mut changed,
    );
    let (default_value, clear_default_value) = if new.secret {
        // This covers secret -> secret (the old value is deliberately absent from `old_default`)
        // as well as non-secret -> secret. Neither transition may write or clear a credential from
        // model-authored FlowScript.
        (None, false)
    } else if old.secret && hidden_default_present {
        // `new_default` was rejected above. Set both fields on one UpdateVariable so apply clears
        // the hidden bytes before the cloned variable becomes non-secret.
        changed = true;
        (None, true)
    } else if old.secret {
        // The live board proves there are no hidden bytes to preserve or expose, so an ordinary
        // default may be authored as part of the non-secret result.
        let default_value = changed_option(
            new_default.clone().unwrap_or(flow_like_types::Value::Null),
            new_default.is_some(),
            &mut changed,
        );
        (default_value, false)
    } else {
        let default_value = changed_option(
            new_default.clone().unwrap_or(flow_like_types::Value::Null),
            old_default != new_default && new_default.is_some(),
            &mut changed,
        );
        let clear_default_value = old_default.is_some() && new_default.is_none();
        changed |= clear_default_value;
        (default_value, clear_default_value)
    };

    let description = changed_option(
        new.description.clone().unwrap_or_default(),
        old.description != new.description && new.description.is_some(),
        &mut changed,
    );
    let clear_description = old.description.is_some() && new.description.is_none();
    changed |= clear_description;

    let category = changed_option(
        new.category.clone().unwrap_or_default(),
        old.category != new.category && new.category.is_some(),
        &mut changed,
    );
    let clear_category = old.category.is_some() && new.category.is_none();
    changed |= clear_category;

    let schema = changed_option(
        new_schema.clone().unwrap_or_default(),
        schema_changed && new_schema.is_some(),
        &mut changed,
    );
    let clear_schema = schema_changed && old_schema.is_some() && new_schema.is_none();
    changed |= clear_schema;

    let exposed = changed_option(new.exposed, old.exposed != new.exposed, &mut changed);
    let secret = changed_option(new.secret, old.secret != new.secret, &mut changed);
    let editable = changed_option(new.editable, old.editable != new.editable, &mut changed);
    let runtime_configured = changed_option(
        new.runtime_configured,
        old.runtime_configured != new.runtime_configured,
        &mut changed,
    );

    Ok(changed.then_some(BoardCommand::UpdateVariable {
        variable_id,
        name,
        data_type,
        value_type,
        default_value,
        clear_default_value,
        description,
        clear_description,
        category,
        clear_category,
        schema,
        clear_schema,
        exposed,
        secret,
        editable,
        runtime_configured,
        value: None,
        summary: Some(format!("Update variable {}", new.name)),
    }))
}

fn changed_option<T>(value: T, did_change: bool, changed: &mut bool) -> Option<T> {
    if did_change {
        *changed = true;
        Some(value)
    } else {
        None
    }
}

fn variable_default_value(var: &VarDecl) -> Option<flow_like_types::Value> {
    var.default.as_ref().map(literal_to_value)
}

fn variable_data_type(var: &VarDecl) -> &'static str {
    if var.schema.is_some() {
        return "Struct";
    }
    match var.ty.base.as_str() {
        "exec" | "Execution" => "Execution",
        "string" | "String" => "String",
        "int" | "Integer" => "Integer",
        "float" | "Float" => "Float",
        "bool" | "Boolean" => "Boolean",
        "Date" => "Date",
        "Path" | "PathBuf" => "PathBuf",
        "any" | "Generic" => "Generic",
        "Struct" => "Struct",
        "bytes" | "Byte" => "Byte",
        _ => "Struct",
    }
}

fn variable_value_type(var: &VarDecl) -> &'static str {
    match var.ty.container {
        Container::Normal => "Normal",
        Container::Array => "Array",
        Container::Map => "HashMap",
        Container::Set => "HashSet",
    }
}

/// Boundary execution pin names on function layers, mirrored onto `control_call_function` call
/// nodes by that node's `on_update`.
const FUNCTION_EXEC_IN: &str = "exec_in";
const FUNCTION_EXEC_OUT: &str = "exec_out";
/// The catalog node that invokes a function layer (`function_layer_id` holds the target layer).
const CALL_FUNCTION_NODE_TYPE: &str = "control_call_function";
/// Generic call-by-reference node; its FlowScript display is the referenced target rather than
/// this catalog type, so anchored display validation must treat it as intentional sugar.
const CALL_REFERENCE_NODE_TYPE: &str = "control_call_reference";
const FUNCTION_LAYER_ID_PIN: &str = "function_layer_id";

/// Catalog nodes that FlowScript renders as binary operators. This is the write-side counterpart
/// of `lower.rs::BINARY_OPS`: every two-data-input operator the reader emits must be materializable
/// again, including Boolean composition and arithmetic nested inside a condition or call argument.
///
/// Float equality nodes deliberately are not listed: unlike the two-operand sugar they also
/// require a tolerance input, so silently inventing one would change the authored semantics.
const BINARY_OPERATOR_NODES: &[(&str, &str, &str, &str)] = &[
    ("==", "String", "Boolean", "equal_string"),
    ("!=", "String", "Boolean", "not_equal_string"),
    ("==", "Boolean", "Boolean", "bool_equal"),
    ("&&", "Boolean", "Boolean", "bool_and"),
    ("||", "Boolean", "Boolean", "bool_or"),
    ("^", "Boolean", "Boolean", "bool_xor"),
    ("==", "Integer", "Boolean", "int_equal"),
    ("!=", "Integer", "Boolean", "int_unequal"),
    (">", "Integer", "Boolean", "int_greater_than"),
    (">=", "Integer", "Boolean", "int_greater_than_or_equal"),
    ("<", "Integer", "Boolean", "int_less_than"),
    ("<=", "Integer", "Boolean", "int_less_than_or_equal"),
    ("+", "Integer", "Integer", "int_add"),
    ("-", "Integer", "Integer", "int_subtract"),
    ("*", "Integer", "Integer", "int_multiply"),
    ("/", "Integer", "Integer", "int_divide"),
    ("%", "Integer", "Integer", "int_modulo"),
    ("**", "Integer", "Integer", "int_power"),
    (">", "Float", "Boolean", "float_greater_than"),
    (">=", "Float", "Boolean", "float_greater_than_or_equal"),
    ("<", "Float", "Boolean", "float_less_than"),
    ("<=", "Float", "Boolean", "float_less_than_or_equal"),
    ("+", "Float", "Float", "float_add"),
    ("-", "Float", "Float", "float_subtract"),
    ("*", "Float", "Float", "float_multiply"),
    ("/", "Float", "Float", "float_divide"),
    ("**", "Float", "Float", "float_power"),
];

fn canonical_binary_op(op: &str) -> &str {
    match op {
        "===" => "==",
        "!==" => "!=",
        _ => op,
    }
}

fn binary_operator_op(node_type: &str) -> Option<&'static str> {
    BINARY_OPERATOR_NODES
        .iter()
        .find(|(_, _, _, candidate)| *candidate == node_type)
        .map(|(op, _, _, _)| *op)
}

fn binary_data_inputs(meta: &NodeMetadata) -> Option<Vec<&PinMetadata>> {
    let inputs = meta
        .inputs
        .iter()
        .filter(|pin| pin.data_type != "Execution")
        .collect::<Vec<_>>();
    (inputs.len() == 2).then_some(inputs)
}

fn binary_operator_call(
    meta: &NodeMetadata,
    inputs: &[&PinMetadata],
    lhs: &Expr,
    rhs: &Expr,
) -> Call {
    Call {
        node_type: meta.name.clone(),
        display: to_camel_case(&meta.name),
        args: [lhs, rhs]
            .into_iter()
            .zip(inputs)
            .map(|(value, pin)| Arg {
                name: pin.name.clone(),
                value: value.clone(),
            })
            .collect(),
        anchor: None,
    }
}

/// Hard ceiling on nodes per layer (root, event scope, or one function layer). Oversized layers
/// make boards unreadable; reconcile rejects edits that would exceed it so the agent splits the
/// work into function layers instead.
pub const MAX_NODES_PER_LAYER: usize = 100;

fn function_layer_pins(
    func: &FnDecl,
    impure: bool,
    interface_schemas: &HashMap<String, String>,
) -> Vec<LayerPinMetadata> {
    let exec_pins = if impure {
        {
            vec![
                LayerPinMetadata {
                    name: FUNCTION_EXEC_IN.to_string(),
                    friendly_name: "Exec In".to_string(),
                    data_type: "Execution".to_string(),
                    value_type: "Normal".to_string(),
                    pin_type: "Input".to_string(),
                    schema: None,
                    enforce_schema: false,
                },
                LayerPinMetadata {
                    name: FUNCTION_EXEC_OUT.to_string(),
                    friendly_name: "Exec Out".to_string(),
                    data_type: "Execution".to_string(),
                    value_type: "Normal".to_string(),
                    pin_type: "Output".to_string(),
                    schema: None,
                    enforce_schema: false,
                },
            ]
        }
    } else {
        Default::default()
    };
    exec_pins
        .into_iter()
        .chain(
            func.params
                .iter()
                .map(|param| layer_pin_from_param(param, "Input", interface_schemas)),
        )
        .chain(
            func.returns
                .iter()
                .map(|param| layer_pin_from_param(param, "Output", interface_schemas)),
        )
        .collect()
}

fn execution_pin_metadata(name: &str, friendly_name: &str) -> PinMetadata {
    PinMetadata {
        name: name.to_string(),
        friendly_name: friendly_name.to_string(),
        description: String::new(),
        data_type: "Execution".to_string(),
        value_type: "Normal".to_string(),
        default_value: None,
        schema: None,
        is_generic: false,
        valid_values: None,
        enforce_schema: false,
    }
}

fn param_pin_metadata(param: &Param, interface_schemas: &HashMap<String, String>) -> PinMetadata {
    let schema = interface_schemas.get(&param.ty.base).cloned();
    let data_type = type_ref_data_type(&param.ty).to_string();
    PinMetadata {
        name: param.name.clone(),
        friendly_name: param.name.clone(),
        description: String::new(),
        data_type: data_type.clone(),
        value_type: type_ref_value_type(&param.ty).to_string(),
        default_value: None,
        schema: schema.clone(),
        is_generic: data_type == "Generic",
        valid_values: None,
        enforce_schema: schema.is_some(),
    }
}

fn variable_value_pin_metadata(
    name: &str,
    data_type: String,
    value_type: String,
    schema: Option<String>,
) -> PinMetadata {
    let is_generic = data_type == "Generic";
    PinMetadata {
        name: name.to_string(),
        friendly_name: name.to_string(),
        description: String::new(),
        data_type,
        value_type,
        default_value: None,
        schema,
        is_generic,
        valid_values: None,
        enforce_schema: false,
    }
}

fn param_output_pin_def(
    param: &Param,
    interface_schemas: &HashMap<String, String>,
) -> PlaceholderPinDef {
    let schema = interface_schemas.get(&param.ty.base).cloned();
    PlaceholderPinDef {
        name: param.name.clone(),
        friendly_name: param.name.clone(),
        description: None,
        pin_type: "Output".to_string(),
        data_type: type_ref_data_type(&param.ty).to_string(),
        value_type: Some(type_ref_value_type(&param.ty).to_string()),
        schema: schema.clone(),
        enforce_schema: schema.is_some(),
    }
}

/// A FlowScript `function` declaration resolved to its (existing or planned) layer, captured
/// before events/bodies are planned so call sites anywhere in the document can target it.
#[derive(Clone)]
struct PlannedFunction {
    entity: NodeEntity,
    impure: bool,
    /// At least one call in this function (or a transitively called FlowScript function) does not
    /// resolve against the catalog. Structural/purity diagnostics are derivative until that
    /// primary resolution error is repaired, so they must not drown out the actionable cause.
    has_unresolved_calls: bool,
    params: Vec<PinMetadata>,
    returns: Vec<PinMetadata>,
}

fn layer_pin_from_param(
    param: &Param,
    pin_type: &str,
    interface_schemas: &HashMap<String, String>,
) -> LayerPinMetadata {
    let schema = interface_schemas.get(&param.ty.base).cloned();
    LayerPinMetadata {
        name: param.name.clone(),
        friendly_name: param.name.clone(),
        data_type: type_ref_data_type(&param.ty).to_string(),
        value_type: type_ref_value_type(&param.ty).to_string(),
        pin_type: pin_type.to_string(),
        schema: schema.clone(),
        enforce_schema: schema.is_some(),
    }
}

fn interface_schema_map(ast: &BoardAst) -> HashMap<String, String> {
    ast.interfaces
        .iter()
        .filter_map(|interface| {
            flow_like_ast::schema_from_interface_with_defs(interface, &ast.interfaces)
                .or_else(|| interface.schema.clone())
                .map(|schema| (interface.name.clone(), schema))
        })
        .collect()
}

fn type_ref_data_type(ty: &TypeRef) -> &'static str {
    match ty.base.as_str() {
        "exec" | "Execution" => "Execution",
        "string" | "String" => "String",
        "int" | "Integer" => "Integer",
        "float" | "Float" => "Float",
        "bool" | "Boolean" => "Boolean",
        "Date" => "Date",
        "Path" | "PathBuf" => "PathBuf",
        "any" | "Generic" => "Generic",
        "Struct" => "Struct",
        "bytes" | "Byte" => "Byte",
        _ => "Struct",
    }
}

fn type_ref_value_type(ty: &TypeRef) -> &'static str {
    match ty.container {
        Container::Normal => "Normal",
        Container::Array => "Array",
        Container::Map => "HashMap",
        Container::Set => "HashSet",
    }
}

fn visible_variable_schema(ast: &BoardAst, var: &VarDecl) -> Option<String> {
    let schema = var.schema.as_deref()?;
    let interface_name = flow_like_ast::interface_name_for_schema(&ast.interfaces, schema)?;
    let interface = ast
        .interfaces
        .iter()
        .find(|interface| interface.name == interface_name)?;
    flow_like_ast::schema_from_interface_with_defs(interface, &ast.interfaces)
        .or_else(|| Some(schema.to_string()))
}

fn schemas_structurally_equivalent(
    old_ast: &BoardAst,
    new_ast: &BoardAst,
    old: &VarDecl,
    new: &VarDecl,
) -> bool {
    let Some(old_schema) = comparable_variable_schema(old_ast, old) else {
        return comparable_variable_schema(new_ast, new).is_none();
    };
    let Some(new_schema) = comparable_variable_schema(new_ast, new) else {
        return false;
    };
    if new_schema == old_schema {
        return true;
    }
    // The lowered (old) side carries the raw board schema while the parsed (new) side carries
    // its render→parse projection; equal fixed points mean the text did not change the schema.
    text_projected_schema(&old_schema)
        .and_then(|projection| flow_like_ast::normalize_schema(&projection))
        .is_some_and(|projection| projection == new_schema)
}

fn comparable_variable_schema(ast: &BoardAst, var: &VarDecl) -> Option<String> {
    let schema = visible_variable_schema(ast, var).or_else(|| var.schema.clone())?;
    flow_like_ast::normalize_schema(&schema)
}

fn find_board_node<'a>(board: &'a Board, node_id: &str) -> Option<&'a Node> {
    board.nodes.get(node_id).or_else(|| {
        board
            .layers
            .values()
            .find_map(|layer| layer.nodes.get(node_id))
    })
}

/// Return one semantic node per id in deterministic id order.
///
/// Canonical boards store nodes in `board.nodes` and use `node.layer` for membership. Legacy
/// boards can instead (or additionally) keep nodes in `layer.nodes`. A mirrored nested clone may
/// lag behind the canonical flat node, so flat nodes always win and nested nodes only fill ids
/// absent from the flat map. Layers are visited by id to make the fallback deterministic even for
/// malformed legacy boards that repeat one nested-only id in more than one layer.
fn all_board_nodes(board: &Board) -> Vec<&Node> {
    let mut nodes_by_id: BTreeMap<&str, &Node> = BTreeMap::new();
    for node in board.nodes.values() {
        nodes_by_id.insert(node.id.as_str(), node);
    }

    let mut layers = board.layers.values().collect::<Vec<_>>();
    layers.sort_by(|left, right| left.id.cmp(&right.id));
    for layer in layers {
        for node in layer.nodes.values() {
            nodes_by_id.entry(node.id.as_str()).or_insert(node);
        }
    }

    nodes_by_id.into_values().collect()
}

fn literal_expr_to_value(expr: &Expr) -> Option<flow_like_types::Value> {
    use flow_like_types::Value;
    match expr {
        Expr::Literal(lit) => Some(literal_to_value(lit)),
        Expr::Object(fields) => {
            let mut map = serde_json::Map::new();
            for field in fields {
                map.insert(field.key.clone(), literal_expr_to_value(&field.value)?);
            }
            Some(Value::Object(map))
        }
        Expr::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(literal_expr_to_value(item)?);
            }
            Some(Value::Array(values))
        }
        _ => None,
    }
}

/// Compact single-line rendering of an expression for diagnostics, truncated to ~80 chars.
fn describe_expr(expr: &Expr) -> String {
    fn render(expr: &Expr) -> String {
        match expr {
            Expr::Ref(name) => name.clone(),
            Expr::Call(call) => format!("{}(...)", call.display),
            Expr::Field { base, pin } => format!("{}.{pin}", render(base)),
            Expr::Member { base, field } => format!("{}.{field}", render(base)),
            Expr::Index { base, .. } => format!("{}[...]", render(base)),
            Expr::Binary { op, lhs, rhs } => format!("{} {op} {}", render(lhs), render(rhs)),
            Expr::Ternary { cond, .. } => format!("{} ? ... : ...", render(cond)),
            Expr::Object(_) => "{...}".to_string(),
            Expr::Array(_) => "[...]".to_string(),
            Expr::Literal(literal) => match literal {
                Literal::String(value) => format!("\"{value}\""),
                Literal::Int(value) => value.to_string(),
                Literal::Float(value) => value.to_string(),
                Literal::Bool(value) => value.to_string(),
                Literal::Null => "null".to_string(),
                Literal::Json(value) => value.clone(),
            },
        }
    }
    let rendered = render(expr);
    if rendered.chars().count() > 80 {
        let truncated: String = rendered.chars().take(77).collect();
        format!("{truncated}...")
    } else {
        rendered
    }
}

#[derive(Debug, Default, Clone)]
struct VariableRefLookup {
    by_ref: HashMap<String, String>,
}

impl VariableRefLookup {
    fn from_board(board: &Board) -> Self {
        let mut lookup = Self::default();
        for variable in board.variables.values() {
            lookup.insert(&variable.id, &variable.name);
        }
        for layer in board.layers.values() {
            for variable in layer.variables.values() {
                lookup.insert(&variable.id, &variable.name);
            }
        }
        lookup
    }

    fn from_board_and_ast(board: &Board, ast: &BoardAst) -> Self {
        let mut lookup = Self::from_board(board);
        for variable in &ast.variables {
            let id = variable_id_for_decl_against_board(board, variable);
            lookup.insert(&id, &variable.name);
        }
        lookup
    }

    fn insert(&mut self, id: &str, name: &str) {
        self.by_ref.insert(id.to_string(), id.to_string());
        self.by_ref.insert(name.to_string(), id.to_string());
        self.by_ref.insert(to_camel_case(name), id.to_string());
    }

    fn resolve(&self, value: &str) -> Option<String> {
        self.by_ref
            .get(value)
            .cloned()
            .or_else(|| self.by_ref.get(&to_camel_case(value)).cloned())
    }
}

fn variable_id_for_decl_against_board(board: &Board, var: &VarDecl) -> String {
    if let Some(anchor) = var.anchor.as_deref() {
        return anchor.to_string();
    }

    if let Some(existing) = board
        .variables
        .values()
        .find(|existing| existing.name == var.name)
    {
        return existing.id.clone();
    }

    for layer in board.layers.values() {
        if let Some(existing) = layer
            .variables
            .values()
            .find(|existing| existing.name == var.name)
        {
            return existing.id.clone();
        }
    }

    variable_id_for_decl(var)
}

fn is_variable_ref_pin_name(name: &str) -> bool {
    pin_name_matches(name, "var_ref") || pin_name_matches(name, "varRef")
}

fn normalize_variable_ref_value_for_pin(
    value: &mut flow_like_types::Value,
    pin_name: &str,
    lookup: &VariableRefLookup,
) {
    if !is_variable_ref_pin_name(pin_name) {
        return;
    }

    let flow_like_types::Value::String(raw) = value else {
        return;
    };

    if let Some(variable_id) = lookup.resolve(raw) {
        *raw = variable_id;
    }
}

fn find_input_pin<'a>(node: &'a Node, name: &str) -> Option<&'a Pin> {
    matching_input_pins(node, name).first().copied()
}

fn find_input_pin_by_ref<'a>(node: &'a Node, pin_ref: &str) -> Option<&'a Pin> {
    if let Some(pin) = node.pins.get(pin_ref)
        && pin.pin_type == PinType::Input
    {
        return Some(pin);
    }
    if let Some((name, occurrence)) = parse_pin_occurrence_ref(pin_ref) {
        let mut matching = node
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Input && node_pin_name_matches(pin, name))
            .collect::<Vec<_>>();
        matching.sort_by_key(|pin| (node_pin_match_rank(pin, name), pin.index, pin.id.clone()));
        return matching.get(occurrence).copied();
    }
    find_input_pin(node, pin_ref)
}

fn find_boundary_pin_by_ref<'a>(
    pins: &'a HashMap<String, Pin>,
    pin_ref: &str,
    expected: PinType,
) -> Option<&'a Pin> {
    if let Some(pin) = pins.get(pin_ref) {
        return (pin.pin_type == expected).then_some(pin);
    }
    let (name, occurrence) = parse_pin_occurrence_ref(pin_ref).unwrap_or((pin_ref, 0));
    let mut matching = pins
        .values()
        .filter(|pin| pin.pin_type == expected && node_pin_name_matches(pin, name))
        .collect::<Vec<_>>();
    matching.sort_by_key(|pin| (node_pin_match_rank(pin, name), pin.index, pin.id.clone()));
    matching.get(occurrence).copied()
}

fn boundary_pin_metadata(pin: &Pin) -> PinMetadata {
    PinMetadata {
        name: pin.name.clone(),
        friendly_name: pin.friendly_name.clone(),
        description: pin.description.clone(),
        data_type: format!("{:?}", pin.data_type),
        value_type: format!("{:?}", pin.value_type),
        default_value: None,
        schema: pin.schema.clone(),
        is_generic: pin.data_type == VariableType::Generic,
        valid_values: pin
            .options
            .as_ref()
            .and_then(|options| options.valid_values.clone()),
        enforce_schema: pin
            .options
            .as_ref()
            .and_then(|options| options.enforce_schema)
            .unwrap_or(false),
    }
}

/// All input pins matching `name`, deterministically ordered: populated pins (connected or
/// holding a default — the ones lowering emits args for) first, then empty ones, each sorted
/// by pin index. `node.pins` is a HashMap, so an unsorted `.find()` picks an arbitrary pin
/// among same-named multi-pins and corrupts them nondeterministically.
fn matching_input_pins<'a>(node: &'a Node, name: &str) -> Vec<&'a Pin> {
    let mut matching: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Input && node_pin_name_matches(p, name))
        .collect();
    matching.sort_by_key(|p| {
        let populated = !p.depends_on.is_empty() || p.default_value.is_some();
        (
            node_pin_match_rank(p, name),
            !populated,
            p.index,
            p.id.clone(),
        )
    });
    matching
}

/// Stable occurrence ref for one concrete live input pin, ordered the same way occurrence refs
/// are decoded after apply. This intentionally differs from `matching_input_pins`' populated-first
/// authored-argument pairing.
fn node_input_occurrence_ref(node: &Node, pin: &Pin) -> String {
    let mut matching = node
        .pins
        .values()
        .filter(|candidate| {
            candidate.pin_type == PinType::Input && node_pin_name_matches(candidate, &pin.name)
        })
        .collect::<Vec<_>>();
    matching.sort_by_key(|candidate| {
        (
            node_pin_match_rank(candidate, &pin.name),
            candidate.index,
            candidate.id.clone(),
        )
    });
    if matching.len() <= 1 {
        return pin.name.clone();
    }
    let occurrence = matching
        .iter()
        .position(|candidate| candidate.id == pin.id)
        .unwrap_or_default();
    pin_occurrence_ref(&pin.name, occurrence)
}

fn find_output_pin<'a>(node: &'a Node, name: &str) -> Option<&'a Pin> {
    let mut matching: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output && node_pin_name_matches(p, name))
        .collect();
    matching.sort_by_key(|p| (node_pin_match_rank(p, name), p.index, p.id.clone()));
    matching.first().copied()
}

/// Read a pin's configured literal default as a JSON string value.
fn node_pin_literal_string(node: &Node, pin_name: &str) -> Option<String> {
    let pin = find_input_pin(node, pin_name)?;
    let bytes = pin.default_value.as_deref()?;
    match flow_like_types::json::from_slice::<flow_like_types::Value>(bytes).ok()? {
        flow_like_types::Value::String(value) => Some(value),
        _ => None,
    }
}

pub(crate) fn pin_name_matches(raw: &str, requested: &str) -> bool {
    raw == requested || to_camel_case(raw) == requested
}

/// How closely a pin answers to `requested`: `0` when its own name matches, `1` when only its
/// friendly name does, `None` when neither. A pin's own name must outrank another pin's friendly
/// name — `string_format`'s config pin is named `format_string` but presented as "Input", so a
/// `{input}` placeholder resolves to the format string itself, and its value overwrites the
/// template, unless the real name wins.
fn pin_name_match_rank(name: &str, friendly_name: &str, requested: &str) -> Option<u8> {
    if pin_name_matches(name, requested) {
        return Some(0);
    }
    pin_name_matches(friendly_name, requested).then_some(1)
}

fn node_pin_match_rank(pin: &Pin, requested: &str) -> u8 {
    pin_name_match_rank(&pin.name, &pin.friendly_name, requested).unwrap_or(u8::MAX)
}

fn metadata_pin_match_rank(pin: &PinMetadata, requested: &str) -> u8 {
    pin_name_match_rank(&pin.name, &pin.friendly_name, requested).unwrap_or(u8::MAX)
}

fn node_pin_name_matches(pin: &Pin, requested: &str) -> bool {
    pin_name_match_rank(&pin.name, &pin.friendly_name, requested).is_some()
}

fn metadata_pin_name_matches(pin: &PinMetadata, requested: &str) -> bool {
    pin_name_match_rank(&pin.name, &pin.friendly_name, requested).is_some()
}

fn authored_arg_name_matches(raw: &str, canonical: &str) -> bool {
    raw == canonical || to_camel_case(raw) == to_camel_case(canonical)
}

/// Resolve only catalog-proven argument aliases whose target and position are deterministic.
/// Direct catalog pin matches always win before this fallback is consulted. Alias/canonical mixes
/// and duplicate aliases are rejected so an automatic repair can never overwrite one input with
/// another authored value.
fn input_arg_alias_target(
    node_type: &str,
    call: &Call,
    arg_index: usize,
) -> Option<(&'static str, usize)> {
    let arg_name = call.args.get(arg_index)?.name.as_str();
    let count = |name: &str| {
        call.args
            .iter()
            .filter(|arg| authored_arg_name_matches(&arg.name, name))
            .count()
    };

    match (node_type, arg_name) {
        ("string_replace", "regex") if count("regex") == 1 && count("isRegex") == 0 => {
            Some(("isRegex", 0))
        }
        ("bool_or", "a") if count("a") == 1 && count("b") <= 1 && count("boolean") == 0 => {
            Some(("boolean", 0))
        }
        ("bool_or", "b") if count("b") == 1 && count("a") <= 1 && count("boolean") == 0 => {
            Some(("boolean", 1))
        }
        _ => None,
    }
}

fn input_arg_alias_correction(
    call: &Call,
    arg: &Arg,
    canonical_name: &str,
    occurrence: usize,
    matching_pin_count: usize,
) -> String {
    let target = if matching_pin_count > 1 {
        format!(
            "`{}` (occurrence {} of {matching_pin_count})",
            to_camel_case(canonical_name),
            occurrence + 1
        )
    } else {
        format!("`{}`", to_camel_case(canonical_name))
    };
    format!(
        "Auto-corrected `{}` argument `{}` to {target}.",
        call.display, arg.name
    )
}

fn call_matches_node(call: &Call, node: &Node) -> bool {
    if !call.node_type.trim().is_empty() {
        return call.node_type == node.name;
    }

    let display = safe_catalog_call_alias(&call.display).unwrap_or(&call.display);
    pin_name_matches(&node.name, display) || pin_name_matches(&node.friendly_name, display)
}

fn is_exec_pin(pin: &Pin) -> bool {
    pin.data_type == VariableType::Execution
}

fn default_node_output_pin(node: &Node) -> Option<String> {
    let mut outputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output && !is_exec_pin(p))
        .collect();
    outputs.sort_by_key(|p| p.index);
    match outputs.as_slice() {
        [pin] => Some(pin.name.clone()),
        many => many
            .iter()
            .find(|p| {
                matches!(
                    p.name.as_str(),
                    "result" | "value" | "output" | "out" | "batch"
                )
            })
            .map(|p| p.name.clone()),
    }
}

fn exec_input_pin(node: &Node) -> Option<String> {
    node.pins
        .values()
        .filter(|p| p.pin_type == PinType::Input && is_exec_pin(p))
        .min_by_key(|p| p.index)
        .map(|p| p.name.clone())
}

fn exec_output_pin(node: &Node) -> Option<String> {
    let mut candidates: Vec<ExecPinCandidate> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output && is_exec_pin(p))
        .map(|p| ExecPinCandidate {
            name: p.name.clone(),
            friendly_name: p.friendly_name.clone(),
            index: p.index,
        })
        .collect();
    candidates.sort_by_key(|p| p.index);
    default_exec_output_by_policy(&node.name, &candidates)
}

fn metadata_input_pin<'a>(meta: &'a NodeMetadata, name: &str) -> Option<&'a PinMetadata> {
    metadata_input_pin_at(meta, name, 0)
}

/// The `occurrence`-th input pin matching `name` — multi-pins (several inputs sharing one name,
/// e.g. `bool_or`'s two `boolean` pins) pair positionally with same-named call arguments.
fn metadata_input_pin_at<'a>(
    meta: &'a NodeMetadata,
    name: &str,
    occurrence: usize,
) -> Option<&'a PinMetadata> {
    let mut matching: Vec<&PinMetadata> = meta
        .inputs
        .iter()
        .filter(|p| p.data_type != "Execution" && metadata_pin_name_matches(p, name))
        .collect();
    // Stable, so pins of equal rank keep their catalog order (`node_to_metadata` sorts by index).
    matching.sort_by_key(|p| metadata_pin_match_rank(p, name));
    matching.get(occurrence).copied()
}

/// Encode an occurrence of a same-named pin in a board-command pin reference. Catalog nodes can
/// legally expose several pins with the same internal name (for example the two `string` inputs
/// on `equal_string`). Pin ids are regenerated when a catalog node is added, so a reconcile batch
/// cannot address those future ids directly. The applier resolves this stable positional selector
/// after `AddNode` has materialized the node.
pub(crate) fn pin_occurrence_ref(name: &str, occurrence: usize) -> String {
    format!("{name}[#{}]", occurrence + 1)
}

/// Decode the positional selector produced by [`pin_occurrence_ref`]. Ordinary pin names are
/// deliberately left untouched; only a terminal, one-based `[#N]` suffix is reserved.
pub(crate) fn parse_pin_occurrence_ref(pin_ref: &str) -> Option<(&str, usize)> {
    let without_closing = pin_ref.strip_suffix(']')?;
    let (name, one_based) = without_closing.rsplit_once("[#")?;
    let one_based = one_based.parse::<usize>().ok()?;
    if name.is_empty() || one_based == 0 {
        return None;
    }
    Some((name, one_based - 1))
}

fn metadata_input_command_ref(
    meta: &NodeMetadata,
    input: &PinMetadata,
    occurrence: usize,
) -> String {
    let matching = meta
        .inputs
        .iter()
        .filter(|pin| pin.data_type != "Execution" && metadata_pin_name_matches(pin, &input.name))
        .count();
    if matching > 1 {
        pin_occurrence_ref(&input.name, occurrence)
    } else {
        input.name.clone()
    }
}

/// Node types whose `on_update` mints one input pin per template placeholder, paired with the
/// config pin that carries the template. Reconcile plans NEW nodes against static catalog metadata
/// that predates those pins, so it predicts them from the config value here. Extend this list as
/// more placeholder-driven nodes appear (the prediction machinery is otherwise node-agnostic).
pub(crate) fn dynamic_placeholder_config_pin(node_type: &str) -> Option<&'static str> {
    match node_type {
        "string_format" => Some("format_string"),
        "string_render_template" => Some("template"),
        node_type => sql_param_config_pin(node_type),
    }
}

/// Nodes that derive one input pin per `$placeholder` in their SQL literal, paired with the
/// pin that carries that literal. Kept in one place because the same list gates prediction
/// here, enrichment in `apply`, and the required-input lint in `executability`.
///
/// The two families do not share a config pin or a dialect: a DataFusion node parameterizes
/// its `query` and binds through the planner, while a LanceDB node parameterizes the `filter`
/// it hands to `only_if` and has its values substituted before the predicate is parsed.
pub(crate) fn sql_param_config_pin(node_type: &str) -> Option<&'static str> {
    match node_type {
        "df_sql_query"
        | "df_sql_query_cached"
        | "df_execute_sql"
        | "df_write_delta"
        | "graph_sql_query" => Some("query"),
        "filter_local_db"
        | "count_local_db"
        | "filter_delete_local_db"
        | "vector_search_local_db"
        | "fts_search_local_db"
        | "hybrid_search_local_db" => Some("filter"),
        _ => None,
    }
}

pub(crate) fn sql_param_node(node_type: &str) -> bool {
    sql_param_config_pin(node_type).is_some()
}

/// Why an argument did not resolve to an input pin, and what has to change for it to.
///
/// Every diagnostic aborts the whole revision (`reconcile_is_safe_to_apply`), so the wording must
/// never imply the rest of the call went through. The old "skipped that argument" said exactly
/// that, and the cheapest repair it invited — delete the argument and re-commit — leaves the value
/// producer on the canvas as a statement of its own, wired to nothing, next to a dynamic input pin
/// that stayed empty. Naming the config value that declares these pins is what lets the author fix
/// the cause instead.
fn missing_input_pin_diagnostic(meta: &NodeMetadata, call: &Call, arg: &Arg) -> String {
    let head = format!(
        "node `{}` has no input pin named `{}`",
        call.display, arg.name
    );
    let Some(config_pin) = dynamic_placeholder_config_pin(&meta.name) else {
        return format!("{head}; no part of this revision was applied");
    };

    let config = to_camel_case(config_pin);
    let declares = if sql_param_node(&meta.name) {
        format!("Each `$placeholder` in `{config}` becomes one `param<Name>` pin")
    } else {
        format!("Each placeholder in `{config}` becomes one pin")
    };
    let alternative = if sql_param_node(&meta.name) {
        ", so bind values through the `params` object instead"
    } else {
        ""
    };

    let config_value = call.args.iter().find_map(|candidate| {
        metadata_input_pin(meta, &candidate.name)
            .is_some_and(|pin| pin.name == config_pin)
            .then_some(&candidate.value)
    });
    match config_value {
        Some(Expr::Literal(Literal::String(_))) => format!(
            "{head}. {declares}, and `{config}` on this call does not declare `{}`. Add it there, or drop the argument. No part of this revision was applied.",
            arg.name
        ),
        Some(_) => format!(
            "{head}. {declares}, and only when `{config}` is a plain string literal on this same call — a computed or wired `{config}` declares nothing{alternative}. No part of this revision was applied."
        ),
        None => format!(
            "{head}. {declares}, and this call does not set `{config}`. Set it to a plain string literal on this same call. No part of this revision was applied."
        ),
    }
}

/// Whether `pin_name` on `node_type` is a pin that node's `on_update` both mints AND then re-types
/// from its source via [`Node::match_type`].
///
/// `match_type` copies the data type, value type and schema of whatever is wired into the pin ONTO
/// the pin, so a minted pin's stored shape describes its current source rather than its own
/// contract — the pin is always minted `Generic`. Deliberately decided from the node type and pin
/// name alone: a catalog lookup would make the rule depend on the catalog carrying this node type,
/// and silently stop applying wherever it does not.
///
/// The widget nodes are NOT here. Their `dyn_*` pins are typed once from the widget's contract and
/// never re-derived from the wire, so their type IS a contract and rejecting a mismatch is right.
fn minted_wire_typed_pin(node_type: &str, pin_name: &str) -> bool {
    if sql_param_node(node_type) {
        // `param_<placeholder>`. The static `params` object pin does not match this prefix.
        return pin_name.starts_with("param_");
    }
    // Every input except the template itself is a minted placeholder pin.
    match node_type {
        "string_format" => !pin_name_matches("format_string", pin_name),
        "string_render_template" => !pin_name_matches("template", pin_name),
        _ => false,
    }
}

/// Placeholder tokens in a template string, matching `string_format`'s `\{([a-zA-Z0-9_]+)\}`.
fn format_string_placeholders(template: &str) -> Vec<String> {
    let bytes = template.as_bytes();
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start && bytes.get(end) == Some(&b'}') {
                let name = &template[start..end];
                if seen.insert(name) {
                    names.push(name.to_string());
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    names
}

fn dynamic_template_placeholders(node_type: &str, template: &str) -> Option<Vec<String>> {
    match node_type {
        // The pin names, not the bare placeholders: these are matched against the argument
        // the call requests, and a SQL node names its pins `param_<placeholder>`.
        //
        // Discovery goes through the same tokenizer DataFusion parses with, so prediction
        // here and the pins the node actually mints cannot disagree — unlike the
        // hand-rolled `format_string` mirror above, which has to be kept in sync by eye.
        node_type if sql_param_node(node_type) => Some(
            match sql_param_config_pin(node_type) {
                // A LanceDB filter is tokenized with Lance's dialect, not DataFusion's, so
                // prediction has to ask the same module the node's `on_update` asks.
                Some("filter") => {
                    flow_like_storage::databases::lance_filter_params::declared_placeholders(
                        template,
                    )
                }
                _ => flow_like_storage::databases::sql_params::declared_placeholders(template),
            }
            .unwrap_or_default()
            .iter()
            .map(|placeholder| {
                flow_like_storage::databases::sql_params::param_pin_name(placeholder)
            })
            .collect(),
        ),
        "string_format" => Some(format_string_placeholders(template)),
        "string_render_template" => {
            let mut environment = flow_like_types::minijinja::Environment::new();
            environment.add_template("flowscript", template).ok()?;
            let parsed = environment.get_template("flowscript").ok()?;
            let mut names = parsed
                .undeclared_variables(false)
                .into_iter()
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            Some(names)
        }
        _ => None,
    }
}

/// The literal template string driving a placeholder node's dynamic pins: the value of the config
/// arg on this call if it is a string literal, else the existing node's stored config default.
fn placeholder_template_value(
    meta: &NodeMetadata,
    call: &Call,
    entity: &NodeEntity,
    existing: &Board,
    config_pin: &str,
) -> Option<String> {
    for arg in &call.args {
        if metadata_input_pin(meta, &arg.name).is_some_and(|pin| pin.name == config_pin) {
            return match &arg.value {
                Expr::Literal(Literal::String(template)) => Some(template.clone()),
                _ => None,
            };
        }
    }

    let NodeEntity::Existing(node_id) = entity else {
        return None;
    };
    let node = find_board_node(existing, node_id)?;
    let bytes = find_input_pin(node, config_pin)?.default_value.as_deref()?;
    flow_like_types::json::from_slice::<flow_like_types::Value>(bytes)
        .ok()?
        .as_str()
        .map(ToOwned::to_owned)
}

/// Permissive Generic input pin, mirroring the `VariableType::Generic` pins a placeholder node's
/// `on_update` adds. Generic short-circuits `metadata_pins_are_compatible`, so it accepts any wire.
fn generic_input_pin_metadata(name: &str) -> PinMetadata {
    PinMetadata {
        name: name.to_string(),
        friendly_name: name.to_string(),
        description: String::new(),
        data_type: "Generic".to_string(),
        value_type: "Normal".to_string(),
        default_value: None,
        schema: None,
        is_generic: true,
        valid_values: None,
        enforce_schema: false,
    }
}

/// Predict the dynamic input pin an `on_update` will add for `arg`, when it names a template
/// placeholder of a placeholder-driven node. Returns `None` for genuinely unknown pins so real
/// typos still surface as diagnostics.
fn synthesize_dynamic_input_pin(
    meta: &NodeMetadata,
    call: &Call,
    arg: &Arg,
    entity: &NodeEntity,
    existing: &Board,
) -> Option<PinMetadata> {
    if let Some(pin) = synthesize_chart_mode_input_pin(meta, call, arg, entity, existing) {
        return Some(pin);
    }
    if widget_dynamic_pin_node(&meta.name) && is_widget_dynamic_binding_arg(&arg.name) {
        return Some(generic_input_pin_metadata(&arg.name));
    }
    let config_pin = dynamic_placeholder_config_pin(&meta.name)?;
    let template = placeholder_template_value(meta, call, entity, existing, config_pin)?;
    synthesize_dynamic_input_pin_from_template(meta, &template, &arg.name)
}

/// Predict the mode-specific pins of `a2ui_push_csv_to_chart`. Its static catalog shape is JSON
/// mode (`data`); `on_update` swaps that pin for the CSV family, and can swap it back on an
/// existing CSV-mode node. The source checker cannot depend on server-only runtime enrichment, so
/// mirror this small audited state machine deterministically.
fn synthesize_chart_mode_input_pin(
    meta: &NodeMetadata,
    call: &Call,
    arg: &Arg,
    entity: &NodeEntity,
    existing: &Board,
) -> Option<PinMetadata> {
    if meta.name != "a2ui_push_csv_to_chart" {
        return None;
    }
    let format = placeholder_template_value(meta, call, entity, existing, "format")?;
    let (name, friendly_name, data_type, valid_values, enforce_schema) =
        match (format.as_str(), arg.name.as_str()) {
            ("JSON", requested) if pin_name_matches("data", requested) => {
                ("data", "Data", "Struct", None, false)
            }
            ("CSV", requested) if pin_name_matches("csv", requested) => {
                ("csv", "Data", "String", None, false)
            }
            ("CSV", requested) if pin_name_matches("table", requested) => {
                ("table", "Table", "Struct", None, false)
            }
            ("CSV", requested) if pin_name_matches("chart_type", requested) => (
                "chart_type",
                "Chart Type",
                "String",
                Some(
                    [
                        "Bar", "Line", "Pie", "Scatter", "Area", "Radar", "Heatmap", "Calendar",
                        "Sankey", "Tree",
                    ]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                ),
                false,
            ),
            ("CSV", requested) if pin_name_matches("delimiter", requested) => {
                ("delimiter", "Delimiter", "String", None, false)
            }
            _ => return None,
        };
    Some(PinMetadata {
        name: name.to_string(),
        friendly_name: friendly_name.to_string(),
        description: String::new(),
        data_type: data_type.to_string(),
        value_type: "Normal".to_string(),
        default_value: None,
        schema: None,
        is_generic: false,
        valid_values,
        enforce_schema,
    })
}

/// Pure counterpart shared with typed IR validation. It mirrors the exact placeholder grammar and
/// Generic pin shape reconciliation will use when the node is materialized.
pub(crate) fn synthesize_dynamic_input_pin_from_template(
    meta: &NodeMetadata,
    template: &str,
    requested_pin: &str,
) -> Option<PinMetadata> {
    dynamic_placeholder_config_pin(&meta.name)?;
    dynamic_template_placeholders(&meta.name, template)?
        .iter()
        .any(|token| token == requested_pin || to_camel_case(token) == requested_pin)
        .then(|| generic_input_pin_metadata(requested_pin))
}

/// Whether an arg name refers to a widget's dynamic data-binding pin, in either snake_case or
/// the camelCase form the UI surfaces use. Covers every prefix the widget nodes mint:
/// `dyn_path_*` / `dyn_prop_*` / `dyn_cust_*` (declarative widgets), `dyn_in_*` (package
/// contract inputs) and `dyn_arg_*` (widget queries).
pub(crate) fn is_widget_dynamic_binding_arg(name: &str) -> bool {
    const KINDS: [&str; 5] = ["path", "prop", "cust", "in", "arg"];
    KINDS.iter().any(|kind| {
        name.starts_with(&format!("dyn_{kind}_"))
            || name.starts_with(&to_camel_case(&format!("dyn_{kind}_")))
    })
}

/// Nodes whose dynamic input pins come from a persisted widget named by a **literal** on the
/// same call.
///
/// Reconcile has no handle to app storage, so unlike the placeholder nodes it cannot
/// enumerate these pins to check a name against. It accepts a well-formed `dyn*` argument and
/// lets apply resolve it after `on_update` has run — which is where the pin genuinely exists.
/// Refusing to plan the command instead is what used to make a *correct* widget binding on a
/// NEW node fail the whole batch.
///
/// This is deliberately only `a2ui_instantiate_widget`. Its pins derive from the
/// `widget_selector` literal, which apply writes in the setup phase — so `on_update` has
/// minted the pins by the time the deferred writes and connections run. The sibling nodes
/// (`a2ui_widget_update_inputs`, `a2ui_widget_query`) derive their pins from a *connected*
/// `element_ref` instead, and connections are the last commands in the batch, so their pins
/// cannot exist in time. Predicting for them would turn a recoverable check-time diagnostic
/// into an apply-time rollback; they get [`is_widget_dynamic_binding_arg`]'s diagnostic
/// naming the real cause instead.
pub(crate) fn widget_dynamic_pin_node(node_type: &str) -> bool {
    matches!(node_type, "a2ui_instantiate_widget")
}

/// Whether `arg` targets a dynamic input pin the node's `on_update` will mint (one not yet live on
/// the board node). Used by the config-edit path to DEFER a literal to apply — which creates the pin
/// via `on_update`, then applies the write — instead of reporting a missing pin. Prefers the
/// enricher (runs `on_update`); falls back to `synthesize_dynamic_input_pin` (string_format family).
fn arg_targets_predicted_dynamic_pin(
    node: &Node,
    node_id: &str,
    call: &Call,
    arg: &Arg,
    existing: &Board,
    enricher: Option<&MetadataEnricher>,
) -> bool {
    if find_input_pin(node, &arg.name).is_some() {
        return false;
    }
    let base = node_to_metadata(node);
    if let Some(enricher) = enricher {
        let literal_args: Vec<(String, flow_like_types::Value)> = call
            .args
            .iter()
            .filter_map(|a| literal_expr_to_value(&a.value).map(|value| (a.name.clone(), value)))
            .collect();
        if let Some(enriched) = enricher(&base, &literal_args, existing) {
            return enriched
                .inputs
                .iter()
                .any(|pin| metadata_pin_name_matches(pin, &arg.name));
        }
    }
    let entity = NodeEntity::Existing(node_id.to_string());
    synthesize_dynamic_input_pin(&base, call, arg, &entity, existing).is_some()
}

/// The node type + response pin an event/tool-entry `return` maps to (mirrors the `lower.rs`
/// `events_generic_return_result` sugar).
const EVENT_RETURN_RESULT_TYPE: &str = "events_generic_return_result";
const EVENT_RESPONSE_PIN: &str = "response";

/// Synthetic argument names the FlowScript decompiler emits for a node's function references
/// (`tools:` for agent tool-registration nodes, `fnRefs:` for generic references — see
/// `lower.rs::fn_ref_arg`). They are NOT board input pins, so reconcile must not treat them as
/// missing pins.
const SYNTHETIC_FN_REF_ARGS: &[&str] = &["tools", "fnRefs"];

/// A `tools:`/`fnRefs:` array carrying a node's function references, rather than a real pin
/// argument. Recognized so reconcile skips it (round-tripping to a no-op) instead of reporting a
/// missing pin — which otherwise surfaces to the user as a spurious "FlowScript apply blocked".
fn is_synthetic_fn_ref_arg(arg: &Arg) -> bool {
    SYNTHETIC_FN_REF_ARGS.contains(&arg.name.as_str()) && matches!(arg.value, Expr::Array(_))
}

/// Extract the referenced target names from a synthetic `tools:`/`fnRefs:` array argument
/// (`[fetchPage, …]`). Only bare references are recognized; the applier resolves each name to a
/// concrete node id.
fn synthetic_fn_ref_targets(arg: &Arg) -> Vec<String> {
    let Expr::Array(items) = &arg.value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Expr::Ref(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn metadata_output_pin<'a>(meta: &'a NodeMetadata, name: &str) -> Option<&'a PinMetadata> {
    let mut matching: Vec<&PinMetadata> = meta
        .outputs
        .iter()
        .filter(|p| p.data_type != "Execution" && metadata_pin_name_matches(p, name))
        .collect();
    matching.sort_by_key(|p| metadata_pin_match_rank(p, name));
    matching.first().copied()
}

fn canonical_schema_value(value: &flow_like_types::Value) -> String {
    match value {
        flow_like_types::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_schema_value)
                .collect::<Vec<_>>()
                .join(",")
        ),
        flow_like_types::Value::Object(fields) => {
            let mut names = fields.keys().collect::<Vec<_>>();
            names.sort_unstable();
            format!(
                "{{{}}}",
                names
                    .into_iter()
                    .map(|name| format!(
                        "{}:{}",
                        serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string()),
                        canonical_schema_value(&fields[name])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn normalized_pin_schema(schema: Option<&str>, refs: &HashMap<String, String>) -> Option<String> {
    let schema = schema?.trim();
    if schema.is_empty() {
        return None;
    }
    let expanded = refs
        .get(schema)
        .map(String::as_str)
        .unwrap_or(schema)
        .trim();
    flow_like_types::json::from_str::<flow_like_types::Value>(expanded)
        .map(|value| canonical_schema_value(&value))
        .ok()
        .or_else(|| Some(expanded.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn schema_constraints_are_compatible(
    input_name: &str,
    input_data_type: &str,
    input_value_type: &str,
    input_schema: Option<&str>,
    input_enforce_schema: bool,
    output_name: &str,
    output_data_type: &str,
    output_value_type: &str,
    output_schema: Option<&str>,
    output_enforce_schema: bool,
    refs: &HashMap<String, String>,
) -> bool {
    let input_schema = normalized_pin_schema(input_schema, refs);
    let output_schema = normalized_pin_schema(output_schema, refs);

    // struct_make/struct_break/struct_set boundary pins adopt the connected schema dynamically.
    // Preserve that behavior before applying the ordinary two-sided schema equality rule.
    if (matches!(input_name, "struct" | "struct_in" | "struct_out")
        || matches!(output_name, "struct" | "struct_in" | "struct_out"))
        && input_data_type == "Struct"
        && output_data_type == "Struct"
    {
        return input_value_type == output_value_type;
    }

    // Match the runtime/UI contract: descriptive schemas are permissive unless one endpoint opts
    // into enforcement. Canonical JSON still avoids rejecting whitespace/key-order-only changes.
    if (input_enforce_schema || output_enforce_schema)
        && !matches!(input_name, "value_ref" | "value_in")
        && !matches!(output_name, "value_ref" | "value_in")
        && input_data_type != "Generic"
        && output_data_type != "Generic"
    {
        return match (input_schema.as_deref(), output_schema.as_deref()) {
            // Two declared contracts: they must be the same one. This is what catches a genuinely
            // wrong struct, e.g. a `Bit` from findModel wired into embedDocument's
            // `CachedEmbeddingModel` input without loading the model first.
            (Some(input), Some(output)) => input == output,
            // Only one side declares a contract, so there is nothing to contradict. An untyped
            // `Struct` boundary pin — a FlowScript `function db(): (database: Struct)` parameter or
            // return — adopts the connected schema, exactly like the struct_make/break/set boundary
            // pins above. Rejecting these made factoring a shared handle into a helper impossible,
            // which is precisely the decomposition BOARD_ORGANIZATION_GUIDANCE asks for.
            _ => true,
        };
    }

    true
}

fn metadata_pins_are_compatible(
    input: &PinMetadata,
    output: &PinMetadata,
    refs: &HashMap<String, String>,
) -> bool {
    if !schema_constraints_are_compatible(
        &input.name,
        &input.data_type,
        &input.value_type,
        input.schema.as_deref(),
        input.enforce_schema,
        &output.name,
        &output.data_type,
        &output.value_type,
        output.schema.as_deref(),
        output.enforce_schema,
        refs,
    ) {
        return false;
    }
    let input_generic = input.is_generic || input.data_type == "Generic";
    let output_generic = output.is_generic || output.data_type == "Generic";
    if input_generic || output_generic {
        if input_generic
            && input.value_type != "Normal"
            && input.value_type != output.value_type
            && !(output_generic && output.value_type == "Normal")
        {
            return false;
        }
        if output_generic
            && output.value_type != "Normal"
            && output.value_type != input.value_type
            && !(input_generic && input.value_type == "Normal")
        {
            return false;
        }
        return true;
    }

    input.data_type == output.data_type && input.value_type == output.value_type
}

fn planned_output_is_compatible(
    input: &PinMetadata,
    output: &PlannedOutputType,
    refs: &HashMap<String, String>,
) -> bool {
    if !schema_constraints_are_compatible(
        &input.name,
        &input.data_type,
        &input.value_type,
        input.schema.as_deref(),
        input.enforce_schema,
        &output.pin_name,
        &output.data_type,
        &output.value_type,
        output.schema.as_deref(),
        output.enforce_schema,
        refs,
    ) {
        return false;
    }
    let input_generic = input.is_generic || input.data_type == "Generic";
    let output_generic = output.is_generic || output.data_type == "Generic";
    if input_generic || output_generic {
        // A dynamic Generic/Normal pin can specialize to any concrete shape. Once either side
        // declares a collection container, though, the container is part of the contract: do not
        // silently wire Generic[] to a scalar (or a scalar to Generic[]).
        if input_generic
            && input.value_type != "Normal"
            && input.value_type != output.value_type
            && !(output_generic && output.value_type == "Normal")
        {
            return false;
        }
        if output_generic
            && output.value_type != "Normal"
            && output.value_type != input.value_type
            && !(input_generic && input.value_type == "Normal")
        {
            return false;
        }
        return true;
    }

    input.data_type == output.data_type && input.value_type == output.value_type
}

/// Variable nodes specialize their Generic catalog pins from the selected variable. Their runtime
/// `on_update` contract permits a schema-less side, but if both sides carry schemas they must
/// describe the same structure. This check complements the generic pin compatibility rules, whose
/// `value_in`/`value_ref` exception is needed while those nodes are still unspecialized.
fn variable_assignment_schemas_are_compatible(
    input: &PinMetadata,
    output: &PlannedOutputType,
    refs: &HashMap<String, String>,
) -> bool {
    match (
        normalized_pin_schema(input.schema.as_deref(), refs),
        normalized_pin_schema(output.schema.as_deref(), refs),
    ) {
        (Some(input), Some(output)) => input == output,
        _ => true,
    }
}

fn default_metadata_output_pin(meta: &NodeMetadata) -> Option<String> {
    let outputs: Vec<&PinMetadata> = meta
        .outputs
        .iter()
        .filter(|p| p.data_type != "Execution")
        .collect();
    match outputs.as_slice() {
        [pin] => Some(pin.name.clone()),
        many => many
            .iter()
            .find(|p| {
                matches!(
                    p.name.as_str(),
                    "result" | "value" | "output" | "out" | "batch"
                )
            })
            .map(|p| p.name.clone()),
    }
}

fn metadata_exec_input_pin(meta: &NodeMetadata) -> Option<String> {
    meta.inputs
        .iter()
        .find(|p| p.data_type == "Execution")
        .map(|p| p.name.clone())
}

fn metadata_exec_output_pin(meta: &NodeMetadata) -> Option<String> {
    let candidates: Vec<ExecPinCandidate> = meta
        .outputs
        .iter()
        .filter(|p| p.data_type == "Execution")
        .enumerate()
        .map(|(index, p)| ExecPinCandidate {
            name: p.name.clone(),
            friendly_name: p.friendly_name.clone(),
            index: index as u16,
        })
        .collect();
    default_exec_output_by_policy(&meta.name, &candidates)
}

/// Catalog-level invariant for a node used as an Event registration target. Entry nodes start an
/// execution chain: they expose at least one execution output and cannot themselves require an
/// incoming execution edge. This deliberately derives capability from pins instead of a fixed
/// node-type allowlist so package-defined event nodes remain supported.
fn event_entry_incompatibility(meta: &NodeMetadata) -> Option<&'static str> {
    if meta.inputs.iter().any(|pin| pin.data_type == "Execution") {
        return Some("it has an Execution input and must run inside an existing chain");
    }
    if !meta.outputs.iter().any(|pin| pin.data_type == "Execution") {
        return Some("it has no Execution output to start a workflow chain");
    }
    None
}

#[derive(Debug, Clone)]
struct ExecPinCandidate {
    name: String,
    friendly_name: String,
    index: u16,
}

type ExecOutputSelector = fn(&[ExecPinCandidate]) -> Option<String>;

// Default sequential FlowScript execution is the reverse of lower.rs's statement-order rendering,
// but for multi-output nodes it is a semantic policy, not a pin-order guess.
// Nodes with more than one execution output must be listed here. Package/custom nodes should
// eventually surface this as catalog metadata; until then this registry is the authoritative
// callback map for "continue after this call" semantics.
const EXEC_OUTPUT_POLICIES: &[(&str, ExecOutputSelector)] = &[
    ("http_fetch", select_exec_success),
    ("streaming_http_fetch", select_exec_success),
    // Batch upsert exposes `exec_out` for success and `error` for failure. Sequential/function
    // continuation follows success; an unhandled error intentionally terminates that path.
    ("batch_upsert_local_db", select_exec_out),
];

fn default_exec_output_by_policy(
    node_type: &str,
    candidates: &[ExecPinCandidate],
) -> Option<String> {
    match candidates {
        [] => None,
        [single] => Some(single.name.clone()),
        many => {
            if let Some((_, selector)) = EXEC_OUTPUT_POLICIES
                .iter()
                .find(|(mapped_node_type, _)| *mapped_node_type == node_type)
            {
                return selector(many);
            }
            select_sole_forward_exec_pin(many).or_else(|| select_exec_done(many))
        }
    }
}

fn is_error_exec_pin_name(name: &str) -> bool {
    matches!(
        name,
        "error" | "exec_error" | "on_error" | "failure" | "failed"
    )
}

/// `exec_out` + `error` is the catalog's dominant multi-output shape (527 `exec_out` against 169
/// `error` pins), and hand-listing every one of those node types in [`EXEC_OUTPUT_POLICIES`] does
/// not scale — so every DB write and UI update demanded a hand-written arm block just to continue
/// sequentially. When the canonical `exec_out` is the ONLY non-error output, it is the only way
/// forward, and continuation follows it exactly as the hand-written `batch_upsert_local_db` policy
/// does; an unhandled error still terminates its own path.
///
/// Deliberately narrow: this recognizes the catalog's own `exec_out` convention only. A node with
/// a second genuine outcome (`exec_out` + `empty`) and a custom/package node naming its outputs
/// anything else (`success`/`exec_success`) stay ambiguous and keep demanding explicit arms rather
/// than being guessed at.
fn select_sole_forward_exec_pin(candidates: &[ExecPinCandidate]) -> Option<String> {
    let mut forward = candidates
        .iter()
        .filter(|pin| !is_error_exec_pin_name(&pin.name));
    let sole = forward.next()?;
    if forward.next().is_some() || sole.name != "exec_out" {
        return None;
    }
    Some(sole.name.clone())
}

fn select_exec_success(candidates: &[ExecPinCandidate]) -> Option<String> {
    select_named_exec_pin(candidates, &["exec_success"])
}

fn select_exec_out(candidates: &[ExecPinCandidate]) -> Option<String> {
    select_named_exec_pin(candidates, &["exec_out"])
}

fn select_exec_done(candidates: &[ExecPinCandidate]) -> Option<String> {
    select_named_exec_pin(candidates, &["exec_done", "done"])
}

fn select_named_exec_pin(candidates: &[ExecPinCandidate], names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        candidates
            .iter()
            .find(|pin| pin.name == *name || pin.friendly_name == *name)
            .map(|pin| pin.name.clone())
    })
}

fn is_streaming_data_output_pin_name(name: &str) -> bool {
    matches!(
        name,
        "chunk" | "token" | "delta" | "stream_chunk" | "streamed_chunk"
    )
}

#[derive(Debug, Clone)]
enum NodeEntity {
    Existing(String),
    New {
        ref_id: String,
        meta: NodeMetadata,
    },
    Layer {
        ref_id: String,
        pins: Vec<LayerPinMetadata>,
    },
}

impl NodeEntity {
    fn node_ref(&self) -> String {
        match self {
            Self::Existing(id) => id.clone(),
            Self::New { ref_id, .. } => ref_id.clone(),
            Self::Layer { ref_id, .. } => ref_id.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct LayerPinMetadata {
    name: String,
    friendly_name: String,
    data_type: String,
    value_type: String,
    pin_type: String,
    schema: Option<String>,
    enforce_schema: bool,
}

#[derive(Debug, Clone)]
struct ExecCursor {
    entity: NodeEntity,
    output_pin: Option<String>,
}

impl ExecCursor {
    fn new(entity: NodeEntity) -> Self {
        Self {
            entity,
            output_pin: None,
        }
    }

    fn with_output(entity: NodeEntity, output_pin: Option<String>) -> Self {
        Self { entity, output_pin }
    }
}

#[derive(Debug, Clone)]
struct PlannedStmt {
    entity: NodeEntity,
    next_exec_pin: Option<String>,
    input_sources: Vec<ValueSource>,
    /// This statement reuses an entity that the preceding statement already placed in the
    /// execution chain. Bound arm blocks (`const r = call(); r { ... }`) only refine that
    /// entity's outgoing branches; reconnecting its input would create a self-edge.
    skip_exec_input_connection: bool,
    /// Additional execution tails the NEXT statement must also wire from — the final cursors of
    /// branch arms. Execution inputs are legal fan-in points, so `if (x) { a() } b()` wires `b`
    /// from BOTH the untaken `false` pin and `a`'s exec output.
    extra_exec_tails: Vec<ExecCursor>,
    /// The statement's own entity does not continue the chain (a fully-armed branch): only
    /// `extra_exec_tails` carry execution forward.
    suppress_self_continuation: bool,
}

impl PlannedStmt {
    fn new(entity: NodeEntity) -> Self {
        Self {
            entity,
            next_exec_pin: None,
            input_sources: Vec::new(),
            skip_exec_input_connection: false,
            extra_exec_tails: Vec::new(),
            suppress_self_continuation: false,
        }
    }

    fn with_next_exec_pin(entity: NodeEntity, next_exec_pin: Option<String>) -> Self {
        Self {
            next_exec_pin,
            ..Self::new(entity)
        }
    }

    fn with_input_sources(entity: NodeEntity, input_sources: Vec<ValueSource>) -> Self {
        Self {
            input_sources,
            ..Self::new(entity)
        }
    }

    fn next_cursor(&self) -> ExecCursor {
        ExecCursor::with_output(self.entity.clone(), self.next_exec_pin.clone())
    }
}

#[derive(Debug, Clone)]
struct ValueSource {
    node: NodeEntity,
    output_pin: Option<String>,
}

#[derive(Debug, Clone)]
struct OutputShape {
    node_type: String,
    pin_name: String,
    data_type: String,
    value_type: String,
    schema: Option<String>,
}

/// Concrete type information for an output whose identity is unambiguous while planning new
/// FlowScript wiring. Function boundaries do not preserve Struct schemas, but catalog and existing
/// board pins do; newly authored edges must obey those exact contracts. Legacy edges are
/// grandfathered separately only when the same endpoints are already connected.
#[derive(Debug, Clone)]
struct PlannedOutputType {
    source: String,
    pin_name: String,
    data_type: String,
    value_type: String,
    is_generic: bool,
    schema: Option<String>,
    enforce_schema: bool,
}

/// Extract the declared keys from a concrete object schema. Catalog schemas generated by
/// `schemars` normally expose `properties` at the root, but optional/ref-wrapped structs occur as
/// well. Stay conservative around unions: only a single non-null object variant is authoritative
/// enough to reject an unknown key.
fn catalog_object_schema_fields(schema: &str) -> Option<(Option<String>, Vec<String>)> {
    fn is_null_schema(schema: &flow_like_types::Value) -> bool {
        schema.get("type").is_some_and(|value| match value {
            flow_like_types::Value::String(kind) => kind == "null",
            flow_like_types::Value::Array(kinds) => kinds
                .iter()
                .all(|kind| kind.as_str().is_some_and(|kind| kind == "null")),
            _ => false,
        })
    }

    fn fields(
        schema: &flow_like_types::Value,
        root: &flow_like_types::Value,
        depth: usize,
    ) -> Option<Vec<String>> {
        if depth > 8 {
            return None;
        }
        if let Some(properties) = schema
            .get("properties")
            .and_then(flow_like_types::Value::as_object)
        {
            // `enforce_schema` governs pin-to-pin connection identity, not JSON object closure.
            // JSON Schema allows extra properties by default, so a property list is exhaustive
            // only when the schema itself closes additional/unevaluated properties. Explicit
            // extension and pattern keys always keep runtime member access permissive.
            if schema
                .get("additionalProperties")
                .is_some_and(|value| value.as_bool() != Some(false))
                || schema
                    .get("unevaluatedProperties")
                    .is_some_and(|value| value.as_bool() != Some(false))
                || schema
                    .get("patternProperties")
                    .and_then(flow_like_types::Value::as_object)
                    .is_some_and(|patterns| !patterns.is_empty())
            {
                return None;
            }
            let explicitly_closed = schema
                .get("additionalProperties")
                .is_some_and(|value| value.as_bool() == Some(false))
                || schema
                    .get("unevaluatedProperties")
                    .is_some_and(|value| value.as_bool() == Some(false));
            if !explicitly_closed {
                return None;
            }
            let mut names = properties.keys().cloned().collect::<Vec<_>>();
            names.sort();
            return Some(names);
        }
        if let Some(reference) = schema.get("$ref").and_then(flow_like_types::Value::as_str)
            && let Some(pointer) = reference.strip_prefix('#')
        {
            return fields(root.pointer(pointer)?, root, depth + 1);
        }
        for union in ["anyOf", "oneOf"] {
            let Some(variants) = schema.get(union).and_then(flow_like_types::Value::as_array)
            else {
                continue;
            };
            let mut object_fields = None;
            for variant in variants {
                if is_null_schema(variant) {
                    continue;
                }
                let candidate = fields(variant, root, depth + 1)?;
                if object_fields.is_some() {
                    // Several object variants can legitimately expose different/dynamic keys.
                    return None;
                }
                object_fields = Some(candidate);
            }
            return object_fields;
        }
        None
    }

    let root = flow_like_types::json::from_str::<flow_like_types::Value>(schema).ok()?;
    let title = root
        .get("title")
        .and_then(flow_like_types::Value::as_str)
        .map(str::to_string);
    fields(&root, &root, 0).map(|fields| (title, fields))
}

/// Platform structs whose property list is authoritative even though `schemars` emits an open
/// object schema, so the generic permissive path above cannot reject anything. Reading an undeclared
/// member off one of these is a silent `null` at runtime — `struct_get` reports `found = false` and
/// keeps going — so the repair hint travels with the field list.
///
/// Keyed by the schema's root `title`. Entries are `(title, declared fields, repair hint)`.
const CLOSED_PLATFORM_STRUCTS: &[(&str, &[&str], &str)] = &[(
    "FlowPath",
    &["path", "store_ref", "cache_store_ref"],
    "a FlowPath is a store handle, not a file object; read file attributes with the `Data/Files/Path` \
     accessors instead: `filename({ path })`, `extension({ path })`, `rawPath({ path })`, \
     `parent({ path })`, `child({ parentPath, childName })`, `setFilename({ inPath, filename })`, \
     `setExtension({ path, extension })`, `fromRawPath({ basePath, rawPath })`, \
     `pathReplaceSegment({ inPath, from, to })`",
)];

fn closed_platform_struct(
    schema: &str,
) -> Option<&'static (&'static str, &'static [&'static str], &'static str)> {
    let root = flow_like_types::json::from_str::<flow_like_types::Value>(schema).ok()?;
    let title = root.get("title").and_then(flow_like_types::Value::as_str)?;
    CLOSED_PLATFORM_STRUCTS
        .iter()
        .find(|(known, _, _)| *known == title)
}

/// `Some((title, hint))` when `field` is provably absent from a known closed platform struct.
fn closed_platform_struct_rejection(
    schema: &str,
    field: &str,
) -> Option<(&'static str, &'static str)> {
    let (title, fields, hint) = closed_platform_struct(schema)?;
    if fields
        .iter()
        .any(|declared| pin_name_matches(declared, field))
    {
        return None;
    }
    Some((title, hint))
}

/// The declared (serialized) spelling of a member on a known closed struct. Member access accepts
/// the camel form, but the runtime reads the JSON key verbatim, so `.storeRef` has to be lowered as
/// `store_ref` or it selects nothing.
fn closed_platform_struct_field_name(schema: &str, field: &str) -> Option<&'static str> {
    let (_, fields, _) = closed_platform_struct(schema)?;
    fields
        .iter()
        .find(|declared| pin_name_matches(declared, field))
        .copied()
}

#[derive(Debug, Clone)]
enum SymbolValue {
    Source(ValueSource),
    Literal(flow_like_types::Value),
    VariableRef { variable_id: String },
}

struct CatalogIndex {
    by_display: HashMap<String, Vec<NodeMetadata>>,
    by_display_lower: HashMap<String, Vec<NodeMetadata>>,
    // Catalogs can contain one entry per installed package or live board instance. Keep every
    // declaration for an internal name: selecting the last HashMap insertion makes reconciliation
    // depend on provider iteration order when two packages expose conflicting contracts.
    by_type: HashMap<String, Vec<NodeMetadata>>,
}

impl CatalogIndex {
    fn new(catalog: &[NodeMetadata]) -> Self {
        let mut by_display: HashMap<String, Vec<NodeMetadata>> = HashMap::new();
        let mut by_display_lower: HashMap<String, Vec<NodeMetadata>> = HashMap::new();
        let mut by_type: HashMap<String, Vec<NodeMetadata>> = HashMap::new();
        for meta in catalog {
            let display = to_camel_case(&meta.name);
            by_display
                .entry(display.clone())
                .or_default()
                .push(meta.clone());
            by_display_lower
                .entry(display.to_lowercase())
                .or_default()
                .push(meta.clone());
            by_type
                .entry(meta.name.clone())
                .or_default()
                .push(meta.clone());
        }
        Self {
            by_display,
            by_display_lower,
            by_type,
        }
    }

    fn resolve_call(&self, call: &Call) -> Result<NodeMetadata, String> {
        if !call.node_type.trim().is_empty() {
            return self.resolve_type(&call.node_type).map_err(|reason| {
                format!(
                    "FlowScript call `{}` declares exact node_type `{}`: {reason}",
                    call.display, call.node_type
                )
            });
        }

        match self.resolve_display(&call.display) {
            Ok(meta) => Ok(meta),
            Err(original) => {
                let Some(replacement) = safe_catalog_call_alias(&call.display) else {
                    return Err(original);
                };
                let Ok(meta) = self.resolve_display(replacement) else {
                    return Err(original);
                };
                let unknown_args = call_arguments_without_exact_inputs(call, &meta);
                let missing_required = call_missing_required_exact_inputs(call, &meta);
                if !unknown_args.is_empty() || !missing_required.is_empty() {
                    let mut reasons = Vec::new();
                    if !unknown_args.is_empty() {
                        reasons.push(format!(
                            "{} {} not {} on `{replacement}`",
                            unknown_args
                                .iter()
                                .map(|name| format!("`{name}`"))
                                .collect::<Vec<_>>()
                                .join(", "),
                            if unknown_args.len() == 1 { "is" } else { "are" },
                            if unknown_args.len() == 1 {
                                "an input"
                            } else {
                                "inputs"
                            },
                        ));
                    }
                    if !missing_required.is_empty() {
                        reasons.push(format!(
                            "required {} {} missing",
                            if missing_required.len() == 1 {
                                "input"
                            } else {
                                "inputs"
                            },
                            missing_required
                                .iter()
                                .map(|name| format!("`{name}`"))
                                .collect::<Vec<_>>()
                                .join(", "),
                        ));
                    }
                    return Err(format!(
                        "FlowScript call `{}` does not match a catalog declaration. The exact node name is `{replacement}`, but it was not auto-corrected because {}. Use `emailSmtpSend({{ connection: ..., from: ..., to: ..., subject: ..., bodyText: ... }})`.",
                        call.display,
                        reasons.join(" and "),
                    ));
                }
                Ok(meta)
            }
        }
    }

    fn resolve_type(&self, node_type: &str) -> Result<NodeMetadata, String> {
        let Some(matches) = self.by_type.get(node_type) else {
            return Err("it is not available in the catalog".to_string());
        };
        one_catalog_match(&to_camel_case(node_type), matches)
    }

    fn resolve_display(&self, display: &str) -> Result<NodeMetadata, String> {
        if display.trim().is_empty() {
            return Err("empty call display cannot be resolved to a catalog node".to_string());
        }

        if let Some(matches) = self.by_display.get(display) {
            return one_catalog_match(display, matches);
        }

        if let Some(matches) = self.by_display_lower.get(&display.to_lowercase()) {
            return one_catalog_match(display, matches);
        }

        Err(format!(
            "FlowScript call `{display}` does not match a catalog declaration; call `get_declarations` and use the exact function name"
        ))
    }
}

/// Deliberately tiny catalog migration table. These are historical/obvious names with exactly one
/// semantic successor; fuzzy matching is intentionally excluded because selecting the wrong node
/// is worse than asking the author to repair a call.
fn safe_catalog_call_alias(display: &str) -> Option<&'static str> {
    match display {
        "emailSmtpSendMail" => Some("emailSmtpSend"),
        _ => None,
    }
}

/// Return authored arguments that do not bind positionally to an exact catalog input. Used to
/// ensure a call-name migration is shape-compatible before applying it automatically.
fn call_arguments_without_exact_inputs<'a>(call: &'a Call, meta: &NodeMetadata) -> Vec<&'a str> {
    let mut same_name_seen: HashMap<&str, usize> = HashMap::new();
    call.args
        .iter()
        .filter_map(|arg| {
            let occurrence = {
                let seen = same_name_seen.entry(arg.name.as_str()).or_insert(0);
                let current = *seen;
                *seen += 1;
                current
            };
            metadata_input_pin_at(meta, &arg.name, occurrence)
                .is_none()
                .then_some(arg.name.as_str())
        })
        .collect()
}

fn call_missing_required_exact_inputs(call: &Call, meta: &NodeMetadata) -> Vec<String> {
    let mut authored_indices = HashSet::new();
    let mut same_name_seen: HashMap<&str, usize> = HashMap::new();
    for arg in &call.args {
        let occurrence = {
            let seen = same_name_seen.entry(arg.name.as_str()).or_insert(0);
            let current = *seen;
            *seen += 1;
            current
        };
        if let Some((index, _)) = meta
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, pin)| {
                pin.data_type != "Execution" && metadata_pin_name_matches(pin, &arg.name)
            })
            .nth(occurrence)
        {
            authored_indices.insert(index);
        }
    }

    let mut claimed = HashSet::new();
    let mut missing = Vec::new();
    for required in &meta.required_inputs {
        let Some((index, input)) = meta.inputs.iter().enumerate().find(|(index, input)| {
            !claimed.contains(index)
                && input.data_type != "Execution"
                && metadata_pin_name_matches(input, required)
        }) else {
            missing.push(to_camel_case(required));
            continue;
        };
        claimed.insert(index);
        if input.default_value.is_none() && !authored_indices.contains(&index) {
            missing.push(to_camel_case(&input.name));
        }
    }
    missing
}

fn unsafe_catalog_call_shape_diagnostic(call: &Call, meta: &NodeMetadata) -> Option<String> {
    if meta.name != "email_imap_inbox_fetch_mail" {
        return None;
    }
    let unknown = call_arguments_without_exact_inputs(call, meta);
    if unknown.is_empty() {
        return None;
    }
    let names = unknown
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "node `emailImapInboxFetchMail` has no input pin named {names}; this mailbox-fetch shape cannot be auto-corrected. `emailImapInboxFetchMail` accepts exactly `emailRef` and returns one `email`. Use `const inbox = mailImapInbox({{ connection: connection, inbox: \"INBOX\" }})`, then `const refs = mailImapList({{ inbox: inbox, filter: \"UNSEEN\" }})`; iterate with `for (const item of controlForEach({{ array: refs }}))`, call `const email = emailImapInboxFetchMail({{ emailRef: item.value }})`, read body fields with `emailGetContent({{ email: email }})` and sender fields with `const headers = emailGetHeaders({{ email: email }})` plus `mailAddressFields({{ address: headers.from }})`, and only after successful processing call `emailImapMarkSeen({{ email: item.value, markAsSeen: true }})`."
    ))
}

fn one_catalog_match(display: &str, matches: &[NodeMetadata]) -> Result<NodeMetadata, String> {
    match matches {
        [single] => Ok(single.clone()),
        [] => Err(format!(
            "FlowScript call `{display}` did not match the catalog"
        )),
        many => {
            // Board-derived catalogs carry one entry per node INSTANCE — identical node types
            // are one logical declaration, not an ambiguity. Conflicting same-name declarations
            // must fail closed: otherwise a catalog reorder silently changes the executable pin
            // contract chosen by reconciliation while the order-independent catalog fingerprint
            // remains unchanged.
            let first = &many[0];
            if many.iter().all(|m| m.name == first.name) {
                if many
                    .iter()
                    .all(|candidate| reconcile_node_contract_eq(first, candidate))
                {
                    return Ok(deterministic_catalog_match(many));
                }
                return Err(format!(
                    "FlowScript call `{display}` matched conflicting catalog declarations for internal node type `{}`; no declaration was selected",
                    first.name
                ));
            }
            let node_types: Vec<&str> = many.iter().map(|m| m.name.as_str()).collect();
            Err(format!(
                "FlowScript call `{display}` is ambiguous; matched {}",
                node_types.join(", ")
            ))
        }
    }
}

/// Multiset intersection of `required_inputs` across conflicting same-type declarations: only a
/// requirement present on EVERY candidate can safely supplement an anchored live node.
fn common_required_inputs(matches: &[NodeMetadata]) -> Vec<String> {
    let Some((first, rest)) = matches.split_first() else {
        return Vec::new();
    };
    let mut common = first.required_inputs.clone();
    for other in rest {
        let mut counts = HashMap::<&str, usize>::new();
        for required in &other.required_inputs {
            *counts.entry(required.as_str()).or_default() += 1;
        }
        common.retain(|required| match counts.get_mut(required.as_str()) {
            Some(count) if *count > 0 => {
                *count -= 1;
                true
            }
            _ => false,
        });
    }
    common
}

fn reconcile_node_contract_eq(left: &NodeMetadata, right: &NodeMetadata) -> bool {
    left.name == right.name
        && left.required_inputs == right.required_inputs
        && left.inputs.len() == right.inputs.len()
        && left
            .inputs
            .iter()
            .zip(&right.inputs)
            .all(|(left, right)| reconcile_pin_contract_eq(left, right))
        && left.outputs.len() == right.outputs.len()
        && left
            .outputs
            .iter()
            .zip(&right.outputs)
            .all(|(left, right)| reconcile_pin_contract_eq(left, right))
}

fn reconcile_pin_contract_eq(left: &PinMetadata, right: &PinMetadata) -> bool {
    left.name == right.name
        && left.data_type == right.data_type
        && left.value_type == right.value_type
        && left.default_value == right.default_value
        && reconcile_schema_contract_eq(left.schema.as_deref(), right.schema.as_deref())
        && left.is_generic == right.is_generic
        && left.valid_values == right.valid_values
        && left.enforce_schema == right.enforce_schema
}

fn reconcile_schema_contract_eq(left: Option<&str>, right: Option<&str>) -> bool {
    let refs = HashMap::new();
    reconcile_schema_contract_eq_with_refs(left, right, &refs)
}

fn reconcile_schema_contract_eq_with_refs(
    left: Option<&str>,
    right: Option<&str>,
    refs: &HashMap<String, String>,
) -> bool {
    normalized_pin_schema(left, refs) == normalized_pin_schema(right, refs)
}

/// Project `schema` through the FULL text surface: interface generation, rendering to FlowScript,
/// and re-parsing. Parsing normalizes strictly more than a purely in-memory interface projection
/// would (e.g. an optional `anyOf[enum, null]` folds into an enum containing `null`), and an
/// authored roundtrip schema is by construction in THIS fixed point.
fn text_projected_schema(schema: &str) -> Option<String> {
    let source = VarDecl {
        name: "boundary".to_string(),
        ty: TypeRef::new("Struct", Container::Normal),
        default: None,
        exposed: false,
        secret: false,
        editable: true,
        runtime_configured: false,
        category: None,
        description: None,
        schema: Some(schema.to_string()),
        anchor: None,
    };
    let interfaces = flow_like_ast::interfaces_for_variables(std::slice::from_ref(&source));
    let interface_name = flow_like_ast::interface_name_for_schema(&interfaces, schema)?.to_string();
    let ast = BoardAst {
        interfaces,
        variables: vec![VarDecl {
            name: "boundary".to_string(),
            ty: TypeRef::new(&interface_name, Container::Normal),
            default: Some(Literal::Json("{}".to_string())),
            exposed: false,
            secret: false,
            editable: true,
            runtime_configured: false,
            category: None,
            description: None,
            schema: None,
            anchor: None,
        }],
        ..BoardAst::default()
    };
    let text = flow_like_ast::render(&ast, &flow_like_ast::RenderOptions::default());
    let parsed = flow_like_ast::parse(&text).ok()?;
    parsed.variables.first()?.schema.clone()
}

fn function_boundary_contract_matches(
    live: &PinMetadata,
    authored: &PinMetadata,
    refs: &HashMap<String, String>,
) -> bool {
    if to_camel_case(&live.name) != to_camel_case(&authored.name)
        || live.data_type != authored.data_type
        || live.value_type != authored.value_type
        || live.is_generic != authored.is_generic
    {
        return false;
    }

    // Legacy lowering could not surface a live boundary schema in FlowScript. Do not interpret a
    // plain `Struct` as a schema removal. An interface can surface field/type/required structure,
    // but not richer JSON Schema constraints such as `additionalProperties`; compare the authored
    // schema to that representable projection while retaining the exact live schema for wiring.
    // The authored schema went through the render→parse text surface, so compare against the
    // live schema's projection through that same surface (`text_projected_schema` subsumes the
    // in-memory interface projection and adds the extra parse normalizations).
    match authored.schema.as_deref() {
        Some(schema) => {
            authored.enforce_schema
                && live.enforce_schema
                && live
                    .schema
                    .as_deref()
                    .map(|schema| refs.get(schema).map(String::as_str).unwrap_or(schema))
                    .and_then(text_projected_schema)
                    .is_some_and(|live_schema| {
                        reconcile_schema_contract_eq(Some(&live_schema), Some(schema))
                    })
        }
        None => {
            // A representable enforced live schema is part of the visible signature now. Treat
            // replacing its nominal interface with bare Struct as drift instead of silently
            // ignoring the edit. Legacy/non-enforced or non-representable schemas stay compatible.
            !live.enforce_schema
                || live
                    .schema
                    .as_deref()
                    .map(|schema| refs.get(schema).map(String::as_str).unwrap_or(schema))
                    .and_then(text_projected_schema)
                    .is_none()
        }
    }
}

fn event_parameter_contracts_match(
    node: &Node,
    params: &[Param],
    interface_schemas: &HashMap<String, String>,
    refs: &HashMap<String, String>,
) -> bool {
    let mut live = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution)
        .collect::<Vec<_>>();
    live.sort_by_key(|pin| (pin.index, pin.id.clone()));
    live.len() == params.len()
        && live.iter().zip(params).all(|(pin, param)| {
            function_boundary_contract_matches(
                &boundary_pin_metadata(pin),
                &param_pin_metadata(param, interface_schemas),
                refs,
            )
        })
}

fn deterministic_catalog_match(matches: &[NodeMetadata]) -> NodeMetadata {
    matches
        .iter()
        .min_by_key(|metadata| {
            serde_json::to_vec(metadata)
                .expect("catalog metadata contains only deterministically serializable fields")
        })
        .expect("catalog match selection is called only for non-empty matches")
        .clone()
}

struct BoardIndex<'a> {
    pin_owner: HashMap<&'a str, (&'a Node, &'a Pin)>,
    boundary_sources: HashMap<&'a str, ValueSource>,
}

impl<'a> BoardIndex<'a> {
    fn new(board: &'a Board) -> Self {
        let mut pin_owner = HashMap::new();
        let mut boundary_sources = HashMap::new();
        for node in all_board_nodes(board) {
            let mut pins = node.pins.values().collect::<Vec<_>>();
            pins.sort_by(|left, right| left.id.cmp(&right.id));
            for pin in pins {
                pin_owner.insert(pin.id.as_str(), (node, pin));
            }
        }
        let mut layers = board.layers.values().collect::<Vec<_>>();
        layers.sort_by(|left, right| left.id.cmp(&right.id));
        for layer in layers {
            let mut pins = layer.pins.values().collect::<Vec<_>>();
            pins.sort_by(|left, right| left.id.cmp(&right.id));
            for pin in pins {
                boundary_sources.insert(
                    pin.id.as_str(),
                    ValueSource {
                        node: NodeEntity::Existing(layer.id.clone()),
                        output_pin: Some(pin.name.clone()),
                    },
                );
            }
        }
        Self {
            pin_owner,
            boundary_sources,
        }
    }

    fn exec_incoming_edges(&self, node: &Node, input_pin_name: &str) -> Vec<(String, String)> {
        let Some(input) = find_input_pin(node, input_pin_name) else {
            return Vec::new();
        };
        if !is_exec_pin(input) {
            return Vec::new();
        }
        input
            .depends_on
            .iter()
            .filter_map(|pin_id| {
                self.pin_owner.get(pin_id.as_str()).map_or_else(
                    || {
                        self.boundary_sources.get(pin_id.as_str()).map(|source| {
                            (
                                source.node.node_ref(),
                                source.output_pin.clone().unwrap_or_default(),
                            )
                        })
                    },
                    |(source_node, source_pin)| {
                        Some((source_node.id.clone(), source_pin.name.clone()))
                    },
                )
            })
            .collect()
    }

    fn data_source_for_input(&self, node: &Node, input_pin_name: &str) -> Option<ValueSource> {
        let input = find_input_pin(node, input_pin_name)?;
        self.data_source_for_pin(input)
    }

    fn data_source_for_pin(&self, input: &Pin) -> Option<ValueSource> {
        self.data_sources_for_pin(input).into_iter().next()
    }

    fn data_sources_for_pin(&self, input: &Pin) -> Vec<ValueSource> {
        if is_exec_pin(input) {
            return Vec::new();
        }
        input
            .depends_on
            .iter()
            .filter_map(|source_pin_id| self.data_source_for_pin_id(source_pin_id))
            .collect()
    }

    fn data_source_for_pin_id(&self, source_pin_id: &str) -> Option<ValueSource> {
        let mut source_pin_id = source_pin_id;
        // Reroutes are pure wire-bends that the renderer collapses; trace through them so the
        // reuse path sees the real origin node — the one the FlowScript text actually shows.
        // Otherwise every inline chain behind a reroute is re-created on reconcile.
        for _ in 0..64 {
            let Some((source_node, source_pin)) = self.pin_owner.get(source_pin_id) else {
                return self.boundary_sources.get(source_pin_id).cloned();
            };
            if source_node.name != "reroute" {
                return Some(ValueSource {
                    node: NodeEntity::Existing(source_node.id.clone()),
                    output_pin: Some(source_pin.name.clone()),
                });
            }
            let route_in = find_input_pin(source_node, "route_in")?;
            source_pin_id = route_in.depends_on.iter().next()?;
        }
        None
    }
}

struct StructuralPlanner<'a> {
    existing: &'a Board,
    board_index: BoardIndex<'a>,
    catalog: CatalogIndex,
    result: ReconcileResult,
    add_commands: Vec<BoardCommand>,
    /// Ref ids for newly planned Event registration nodes. Event node types are catalog-extensible,
    /// so registration ordering cannot rely on a hard-coded list of built-in `events_*` names.
    event_entry_refs: HashSet<String>,
    /// Existing Event entries already named by this FlowScript document. Stale-anchor recovery may
    /// only rebind to an unclaimed live entry; otherwise two declarations could silently collapse
    /// onto the same trigger node.
    claimed_event_entries: HashSet<String>,
    disconnect_commands: Vec<BoardCommand>,
    connect_commands: Vec<BoardCommand>,
    update_commands: Vec<BoardCommand>,
    symbols: Vec<HashMap<String, SymbolValue>>,
    /// Bindings made inside a loop body or a branch arm, kept resolvable after that block
    /// closes. FlowScript blocks are lexical, but the board they render is one flat node
    /// scope: a value produced inside a loop and read after it is an ordinary data edge, and
    /// `lower` emits exactly that text from such a board. Consulted only when the lexical
    /// chain has no such name, so an enclosing binding still wins.
    closed_block_symbols: HashMap<String, SymbolValue>,
    variable_refs: VariableRefLookup,
    /// Exact data contract of every board variable visible to this planned source revision. New
    /// variable get/set nodes start Generic in the static catalog, but their `on_update` handlers
    /// specialize to this contract before connections are applied.
    variable_value_contracts: HashMap<String, PinMetadata>,
    /// `(layer, declared return pins, function name)` for the function bodies currently being
    /// planned; the name keeps return diagnostics attributable to their function.
    function_return_targets: Vec<(NodeEntity, Vec<PinMetadata>, String)>,
    /// Canonical schemas for named FlowScript interfaces. Function/event boundary pins use these
    /// contracts instead of degrading every named Struct to an untyped `Struct` placeholder.
    interface_schemas: HashMap<String, String>,
    unresolved_variable_refs: HashSet<String>,
    /// Newly added impure nodes: (ref_id, execution input pin, friendly name). Checked at the end
    /// for a missing incoming execution edge.
    new_impure_nodes: Vec<(String, String, String)>,
    /// Ref ids exempt from the dangling-execution check (a function body's first node, which has no
    /// execution entry to wire from yet).
    exec_check_exempt: HashSet<String>,
    next_ref: usize,
    /// Canvas placement is local to each layer. A single global counter makes function bodies
    /// start thousands of pixels away from their own layer origin, so an otherwise populated
    /// function appears empty and large workflows stretch into an unreadable line.
    next_position_by_layer: HashMap<Option<String>, usize>,
    /// Impure calls planned in expression/argument position (e.g. `value: fakerFullName()`).
    /// They only receive data wiring where they are resolved; `plan_block` drains this queue and
    /// splices them into the exec chain ahead of the consuming statement (innermost call first —
    /// the natural push order of the expression recursion). Without this their exec pins dangle
    /// and the nodes never run.
    pending_exec_splices: Vec<NodeEntity>,
    /// `SetNodeFunctionRefs` commands synthesized from `tools:`/`fnRefs:` arguments on newly added
    /// nodes. Held separately so they emit after the add/connect commands (the applier resolves the
    /// referenced targets — function layers, events — once those nodes exist).
    fn_ref_commands: Vec<BoardCommand>,
    /// Declared FlowScript functions by camelCase name, pre-created before events/bodies are
    /// planned so `functionName(...)` call sites resolve to `control_call_function` nodes.
    planned_functions: HashMap<String, PlannedFunction>,
    /// Literal-return sources materialized during THIS plan, keyed by the deterministic
    /// `var_{function}_{pin}` id, so repeated literal returns (branch arms, re-planned bodies)
    /// share one variable + `variable_get` instead of minting suffixed duplicates.
    planned_literal_return_sources: HashMap<String, ValueSource>,
    /// Boundary pass-throughs materialized during THIS plan, keyed by `(function layer ref,
    /// parameter pin)`. `already_planned` in `queue_validated_data_connection` dedupes the EDGE,
    /// not the reroute NODE, so without this `return a, a` (or two branch arms returning the same
    /// parameter) would mint a sibling reroute per return statement.
    planned_boundary_passthroughs: HashMap<(String, String), ValueSource>,
    /// Optional hook to materialize a node's dynamic (`on_update`-generated) pins for a call, so a
    /// literal/connection targeting one resolves against a real pin instead of the predicted
    /// `synthesize_dynamic_input_pin` fallback. `None` for the pure static-catalog paths.
    enricher: Option<&'a MetadataEnricher>,
}

impl<'a> StructuralPlanner<'a> {
    fn new(
        existing: &'a Board,
        catalog: &[NodeMetadata],
        enricher: Option<&'a MetadataEnricher>,
    ) -> Self {
        Self {
            existing,
            board_index: BoardIndex::new(existing),
            catalog: CatalogIndex::new(catalog),
            result: ReconcileResult::default(),
            add_commands: Vec::new(),
            event_entry_refs: HashSet::new(),
            claimed_event_entries: HashSet::new(),
            disconnect_commands: Vec::new(),
            connect_commands: Vec::new(),
            update_commands: Vec::new(),
            symbols: Vec::new(),
            closed_block_symbols: HashMap::new(),
            variable_refs: VariableRefLookup::from_board(existing),
            variable_value_contracts: HashMap::new(),
            function_return_targets: Vec::new(),
            interface_schemas: HashMap::new(),
            unresolved_variable_refs: HashSet::new(),
            new_impure_nodes: Vec::new(),
            exec_check_exempt: HashSet::new(),
            next_ref: 0,
            next_position_by_layer: HashMap::new(),
            pending_exec_splices: Vec::new(),
            fn_ref_commands: Vec::new(),
            planned_functions: HashMap::new(),
            planned_literal_return_sources: HashMap::new(),
            planned_boundary_passthroughs: HashMap::new(),
            enricher,
        }
    }

    fn reserve_declared_event_entries(&mut self, ast: &BoardAst) {
        self.claimed_event_entries.extend(
            declared_event_anchors(ast)
                .into_iter()
                .filter(|anchor| find_board_node(self.existing, anchor).is_some()),
        );
    }

    /// Enrich a resolved node's metadata with the dynamic pins its `on_update` would create for this
    /// call's literal arguments, so the reconciler can resolve those pins. No-op without an enricher
    /// (the default for tests and the non-enriched entry points).
    fn enrich_meta(&self, meta: NodeMetadata, call: &Call) -> NodeMetadata {
        let Some(enricher) = self.enricher else {
            return meta;
        };
        let literal_args: Vec<(String, flow_like_types::Value)> = call
            .args
            .iter()
            .filter_map(|arg| {
                literal_expr_to_value(&arg.value).map(|value| (arg.name.clone(), value))
            })
            .collect();
        enricher(&meta, &literal_args, self.existing).unwrap_or(meta)
    }

    fn plan(mut self, ast: &BoardAst) -> ReconcileResult {
        self.interface_schemas = interface_schema_map(ast);
        // Reserve every still-live explicit event anchor before planning. A stale declaration that
        // appears earlier in the document must not steal the entry owned by a later declaration.
        self.reserve_declared_event_entries(ast);
        self.push_scope();
        self.seed_top_level_variables(ast);
        // Function layers are created (and their impurity decided) up front so call sites in
        // events and other functions resolve regardless of declaration order.
        self.prepare_functions(ast);
        for event in &ast.events {
            self.plan_event(event, None);
        }
        for func in &ast.functions {
            self.plan_function_body(func);
        }
        self.pop_scope();

        self.check_new_function_structure();
        self.check_function_ref_targets();
        self.check_dangling_impure_execution();

        // The entry node is the registration target for the outer app Event. Materialize every
        // layer, variable and workflow node first, then add the entry node last. Connections are
        // emitted afterwards, so their ref resolution remains deterministic.
        let (event_entries, setup_commands): (Vec<_>, Vec<_>) =
            self.add_commands.into_iter().partition(|command| {
                matches!(
                    command,
                    BoardCommand::AddNode {
                        ref_id: Some(ref_id),
                        ..
                    } if self.event_entry_refs.contains(ref_id)
                )
            });
        self.result.commands.extend(setup_commands);
        self.result.commands.extend(event_entries);
        // Literal/config updates can change node shape (for example format/schema/template
        // nodes whose on_update adds dynamic pins), so apply them before resolving connections.
        self.result.commands.extend(self.update_commands);
        self.result.commands.extend(self.disconnect_commands);
        self.result.commands.extend(self.connect_commands);
        // Function references resolve against the nodes/layers added above, so emit last.
        self.result.commands.extend(self.fn_ref_commands);
        self.result
    }

    fn prepare_functions(&mut self, ast: &BoardAst) {
        for func in &ast.functions {
            let mut seen = HashSet::new();
            seen.insert(func.name.clone());
            let impure = self.function_body_is_impure(ast, &func.body, &mut seen);
            let mut resolution_seen = HashSet::new();
            resolution_seen.insert(func.name.clone());
            let has_unresolved_calls =
                self.function_body_has_unresolved_calls(ast, &func.body, &mut resolution_seen);
            let Some(entity) = self.function_layer_entity(func, impure) else {
                continue;
            };
            let Some((params, returns)) = self.function_contract_metadata(func, &entity, impure)
            else {
                continue;
            };
            self.planned_functions.insert(
                func.name.clone(),
                PlannedFunction {
                    entity,
                    impure,
                    has_unresolved_calls,
                    params,
                    returns,
                },
            );
        }
    }

    fn function_contract_metadata(
        &mut self,
        func: &FnDecl,
        entity: &NodeEntity,
        impure: bool,
    ) -> Option<(Vec<PinMetadata>, Vec<PinMetadata>)> {
        let expected_params = func
            .params
            .iter()
            .map(|param| param_pin_metadata(param, &self.interface_schemas))
            .collect::<Vec<_>>();
        let expected_returns = func
            .returns
            .iter()
            .map(|param| param_pin_metadata(param, &self.interface_schemas))
            .collect::<Vec<_>>();

        let NodeEntity::Existing(layer_id) = entity else {
            return Some((expected_params, expected_returns));
        };
        let layer = self.existing.layers.get(layer_id)?;
        let has_exec_in = layer
            .pins
            .values()
            .any(|pin| pin.pin_type == PinType::Input && pin.data_type == VariableType::Execution);
        let has_exec_out = layer
            .pins
            .values()
            .any(|pin| pin.pin_type == PinType::Output && pin.data_type == VariableType::Execution);
        if has_exec_in != impure || has_exec_out != impure {
            // Live layers legitimately diverge from the prescan's ideal boundary shape: legacy
            // layers predate exec pins, and terminal functions carry `exec_in` without
            // `exec_out`. Reconcile never rewrites anchored boundary pins (the live layer is
            // authoritative, as in `add_function_call_node`), so the only fatal case is a text
            // edit that makes a genuinely pure layer impure — its body could never run.
            let live_body_impure = self
                .existing
                .nodes
                .values()
                .filter(|node| node.layer.as_deref() == Some(layer_id.as_str()))
                .chain(layer.nodes.values())
                .any(|node| exec_input_pin(node).is_some());
            if impure && !has_exec_in && !live_body_impure {
                self.result.diagnostics.push(format!(
                    "function `{}` changes the execution-boundary contract of anchored Function layer `{layer_id}`; anchored function signatures cannot be rewritten by FlowScript",
                    func.name
                ));
                return None;
            }
        }

        let mut live_params = layer
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Input && pin.data_type != VariableType::Execution
            })
            .collect::<Vec<_>>();
        let mut live_returns = layer
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution
            })
            .collect::<Vec<_>>();
        live_params.sort_by_key(|pin| (pin.index, pin.id.clone()));
        live_returns.sort_by_key(|pin| (pin.index, pin.id.clone()));
        let live_params = live_params
            .into_iter()
            .map(boundary_pin_metadata)
            .collect::<Vec<_>>();
        let live_returns = live_returns
            .into_iter()
            .map(boundary_pin_metadata)
            .collect::<Vec<_>>();

        let params_match = live_params.len() == expected_params.len()
            && live_params
                .iter()
                .zip(&expected_params)
                .all(|(live, expected)| {
                    function_boundary_contract_matches(live, expected, &self.existing.refs)
                });
        let returns_match = live_returns.len() == expected_returns.len()
            && live_returns
                .iter()
                .zip(&expected_returns)
                .all(|(live, expected)| {
                    function_boundary_contract_matches(live, expected, &self.existing.refs)
                });
        if !params_match || !returns_match {
            self.result.diagnostics.push(format!(
                "function `{}` changes the data-boundary contract of anchored Function layer `{layer_id}`; parameter/return names, order, types, containers, and authored schemas must match the live layer",
                func.name
            ));
            return None;
        }

        // The live layer remains authoritative for schemas not representable in legacy lowered
        // FlowScript. Synthetic call nodes and return wiring therefore use these exact contracts.
        Some((live_params, live_returns))
    }

    /// Static prescan: does this function body contain anything that needs an execution chain
    /// (impure catalog calls, control flow, board-variable writes, or calls to other impure
    /// functions)? Decides whether the layer gets `exec_in`/`exec_out` boundary pins — a pure
    /// function must NOT carry them (the runtime would then look for an entry node), and an
    /// impure one must (or its body never runs when called).
    fn function_body_is_impure(
        &self,
        ast: &BoardAst,
        block: &Block,
        seen: &mut HashSet<String>,
    ) -> bool {
        block.stmts.iter().any(|stmt| match stmt {
            Stmt::Branch { .. } | Stmt::Loop { .. } => true,
            Stmt::Let { call, .. } | Stmt::Call { call, .. } => {
                self.call_is_impure(ast, call, seen)
            }
            Stmt::Assign { target, value, .. } => {
                self.assignment_targets_board_variable(ast, target)
                    || self.expr_contains_impure_call(ast, value, seen)
            }
            // `base.path = value` plans as an Assign to `base` (struct_set accumulator) — a
            // board-variable base ends in an impure variable_set node just like a plain Assign.
            Stmt::FieldAssign { base, value, .. } => {
                self.assignment_targets_board_variable(ast, base)
                    || self.expr_contains_impure_call(ast, value, seen)
            }
            Stmt::LocalAlias { value, .. } => self.expr_contains_impure_call(ast, value, seen),
            Stmt::Return { values, .. } => values
                .iter()
                .any(|value| self.expr_contains_impure_call(ast, value, seen)),
            Stmt::Handler(_) | Stmt::Local(_) | Stmt::Comment(_) => false,
        })
    }

    fn call_is_impure(&self, ast: &BoardAst, call: &Call, seen: &mut HashSet<String>) -> bool {
        // Impure calls hidden inside the ARGUMENTS make the enclosing statement impure no matter
        // what the callee is — they get exec-spliced ahead of it at plan time.
        let args_impure = call
            .args
            .iter()
            .any(|arg| self.expr_contains_impure_call(ast, &arg.value, seen));
        if let Some(func) = ast.functions.iter().find(|func| func.name == call.display) {
            let body_impure = seen.insert(func.name.clone())
                && self.function_body_is_impure(ast, &func.body, seen);
            return body_impure || args_impure;
        }
        let impure_by_meta = match self.catalog.resolve_call(call) {
            Ok(meta) => metadata_exec_input_pin(&meta).is_some(),
            // An unresolvable call (e.g. conflicting same-type declarations in a board-derived
            // catalog) must not silently classify as pure: the anchored live node knows whether
            // it carries an execution input, and misclassifying flips the function layer's
            // exec-boundary contract.
            Err(_) => call
                .anchor
                .as_deref()
                .and_then(|anchor| find_board_node(self.existing, anchor))
                .is_some_and(|node| exec_input_pin(node).is_some()),
        };
        impure_by_meta || args_impure
    }

    fn expr_contains_impure_call(
        &self,
        ast: &BoardAst,
        expr: &Expr,
        seen: &mut HashSet<String>,
    ) -> bool {
        match expr {
            Expr::Call(call) => self.call_is_impure(ast, call, seen),
            Expr::Field { base, .. } | Expr::Member { base, .. } => {
                self.expr_contains_impure_call(ast, base, seen)
            }
            Expr::Object(fields) => fields
                .iter()
                .any(|field| self.expr_contains_impure_call(ast, &field.value, seen)),
            Expr::Array(items) => items
                .iter()
                .any(|item| self.expr_contains_impure_call(ast, item, seen)),
            Expr::Index { base, index } => {
                self.expr_contains_impure_call(ast, base, seen)
                    || self.expr_contains_impure_call(ast, index, seen)
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                self.expr_contains_impure_call(ast, cond, seen)
                    || self.expr_contains_impure_call(ast, then, seen)
                    || self.expr_contains_impure_call(ast, otherwise, seen)
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.expr_contains_impure_call(ast, lhs, seen)
                    || self.expr_contains_impure_call(ast, rhs, seen)
            }
            Expr::Ref(_) | Expr::Literal(_) => false,
        }
    }

    /// Whether this body contains a call whose identity cannot yet be resolved. Purity and graph
    /// completeness depend on resolved pin metadata; when a call is unknown, the catalog error is
    /// the primary actionable diagnostic and secondary "pure"/"missing tail" warnings are noise.
    fn function_body_has_unresolved_calls(
        &self,
        ast: &BoardAst,
        block: &Block,
        seen: &mut HashSet<String>,
    ) -> bool {
        block.stmts.iter().any(|stmt| match stmt {
            Stmt::Let { call, .. } | Stmt::Call { call, .. } => {
                self.call_has_unresolved_target(ast, call, seen)
            }
            Stmt::Assign { value, .. } | Stmt::LocalAlias { value, .. } => {
                self.expr_has_unresolved_call(ast, value, seen)
            }
            Stmt::FieldAssign { value, .. } => self.expr_has_unresolved_call(ast, value, seen),
            Stmt::Branch {
                call,
                condition,
                arms,
                ..
            } => {
                (!is_placeholder_call(call) && self.call_has_unresolved_target(ast, call, seen))
                    || condition.as_ref().is_some_and(|condition| {
                        self.expr_has_unresolved_call(ast, condition, seen)
                    })
                    || arms
                        .iter()
                        .any(|arm| self.function_body_has_unresolved_calls(ast, &arm.body, seen))
            }
            Stmt::Loop { call, body, .. } => {
                self.call_has_unresolved_target(ast, call, seen)
                    || self.function_body_has_unresolved_calls(ast, body, seen)
            }
            Stmt::Return { values, .. } => values
                .iter()
                .any(|value| self.expr_has_unresolved_call(ast, value, seen)),
            Stmt::Handler(event) => self.function_body_has_unresolved_calls(ast, &event.body, seen),
            Stmt::Local(_) | Stmt::Comment(_) => false,
        })
    }

    fn call_has_unresolved_target(
        &self,
        ast: &BoardAst,
        call: &Call,
        seen: &mut HashSet<String>,
    ) -> bool {
        let args_unresolved = call
            .args
            .iter()
            .any(|arg| self.expr_has_unresolved_call(ast, &arg.value, seen));
        if let Some(function) = ast
            .functions
            .iter()
            .find(|function| function.name == call.display)
        {
            return args_unresolved
                || (seen.insert(function.name.clone())
                    && self.function_body_has_unresolved_calls(ast, &function.body, seen));
        }
        if args_unresolved {
            return true;
        }
        let Ok(meta) = self.catalog.resolve_call(call) else {
            return true;
        };
        unsafe_catalog_call_shape_diagnostic(call, &meta).is_some()
    }

    fn expr_has_unresolved_call(
        &self,
        ast: &BoardAst,
        expr: &Expr,
        seen: &mut HashSet<String>,
    ) -> bool {
        match expr {
            Expr::Call(call) => self.call_has_unresolved_target(ast, call, seen),
            Expr::Field { base, .. } | Expr::Member { base, .. } => {
                self.expr_has_unresolved_call(ast, base, seen)
            }
            Expr::Object(fields) => fields
                .iter()
                .any(|field| self.expr_has_unresolved_call(ast, &field.value, seen)),
            Expr::Array(items) => items
                .iter()
                .any(|item| self.expr_has_unresolved_call(ast, item, seen)),
            Expr::Index { base, index } => {
                self.expr_has_unresolved_call(ast, base, seen)
                    || self.expr_has_unresolved_call(ast, index, seen)
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                self.expr_has_unresolved_call(ast, cond, seen)
                    || self.expr_has_unresolved_call(ast, then, seen)
                    || self.expr_has_unresolved_call(ast, otherwise, seen)
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.expr_has_unresolved_call(ast, lhs, seen)
                    || self.expr_has_unresolved_call(ast, rhs, seen)
            }
            Expr::Ref(_) | Expr::Literal(_) => false,
        }
    }

    fn assignment_targets_board_variable(&self, ast: &BoardAst, target: &str) -> bool {
        ast.variables.iter().any(|var| var.name == target)
            || self.variable_refs.resolve(target).is_some()
    }

    /// Resolve the exact catalog identity carried by an event declaration without invoking the
    /// Generic/Simple fallback. A stale anchor may only create a replacement when this succeeds;
    /// an alias-only header such as `wikiExplorerLoad()` does not retain enough type information
    /// to decide whether the deleted node was Simple, Generic, or package-defined.
    fn exact_event_metadata(&self, event: &EventBlock) -> Result<Option<NodeMetadata>, String> {
        if !event.node_type.trim().is_empty() {
            self.catalog
                .resolve_type(&event.node_type)
                .map(Some)
                .map_err(|reason| {
                    format!(
                        "event `{}` declares exact node_type `{}`: {reason}",
                        event.name, event.node_type
                    )
                })
        } else {
            Ok(self.catalog.resolve_display(&event.name).ok())
        }
    }

    fn node_is_in_target_layer(&self, node: &Node, target_layer: Option<&str>) -> bool {
        let direct_layer = node.layer.as_deref().filter(|layer| !layer.is_empty());
        // Canonical flat storage is authoritative even when `layer` is None (root). Consult a
        // containing layer only for legacy nested-only nodes; mirrored layer clones may be stale.
        if self.existing.nodes.contains_key(&node.id) {
            return direct_layer == target_layer;
        }

        let nested_layer = self
            .existing
            .layers
            .values()
            .find(|layer| layer.nodes.contains_key(&node.id))
            .map(|layer| layer.id.as_str());
        nested_layer == target_layer
    }

    fn event_entry_targets_node(&self, entry: &Node, target_node_id: &str) -> bool {
        entry
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Output && is_exec_pin(pin))
            .flat_map(|pin| pin.connected_to.iter())
            .any(|target_pin_id| {
                self.board_index
                    .pin_owner
                    .get(target_pin_id.as_str())
                    .is_some_and(|(owner, _)| owner.id == target_node_id)
            })
    }

    /// A whole-document rewrite that drops the entry anchor must not mint a duplicate entry node:
    /// the stranded old entry keeps every data edge hanging off its outputs (its `history` etc.
    /// still feed body nodes), which renders as the new block's parameters and re-reconciles to a
    /// spurious connect on every readback. Claim the live entry instead — but only on the strong
    /// signal that it ALREADY drives this exact authored body, so a genuinely new sibling event
    /// (whose body nodes are not yet driven by anything) still creates its own entry.
    fn claim_existing_entry_for_unanchored_event(
        &mut self,
        event: &EventBlock,
        target_layer: Option<&str>,
    ) -> Option<NodeEntity> {
        let exact_meta = self.exact_event_metadata(event).ok()?;
        let first_body_node = first_existing_exec_body_node(self.existing, &event.body)?;
        let first_body_node_id = first_body_node.id.clone();
        let candidates: Vec<&Node> = all_board_nodes(self.existing)
            .into_iter()
            .filter(|node| {
                !self.claimed_event_entries.contains(&node.id)
                    && self.node_is_in_target_layer(node, target_layer)
                    && node.start == Some(true)
                    && event_entry_incompatibility(&node_to_metadata(node)).is_none()
                    && exact_meta
                        .as_ref()
                        .is_none_or(|meta| node.name == meta.name)
                    && event_parameter_contracts_match(
                        node,
                        &event.params,
                        &self.interface_schemas,
                        &self.existing.refs,
                    )
                    && self.event_entry_targets_node(node, &first_body_node_id)
            })
            .collect();
        let [entry] = candidates.as_slice() else {
            return None;
        };
        let entry_id = entry.id.clone();
        let entry_friendly = entry.friendly_name.clone();
        self.claimed_event_entries.insert(entry_id.clone());
        let desired_name = event.event_name.as_deref().unwrap_or(&event.name);
        if !pin_name_matches(&entry_friendly, desired_name) {
            self.update_commands.push(BoardCommand::RenameNode {
                node_id: entry_id.clone(),
                friendly_name: desired_name.to_string(),
                summary: Some(format!("Rename event to {desired_name}")),
            });
        }
        self.result.corrections.push(format!(
            "Bound event `{}` to live entry `{entry_id}`, which already drives this body, instead of creating a duplicate entry node.",
            event.name
        ));
        Some(NodeEntity::Existing(entry_id))
    }

    /// Recover an event whose explicit identity anchor disappeared from the live board.
    ///
    /// Recovery is deliberately deterministic:
    /// - one compatible, unclaimed entry in the same scope is rebound;
    /// - no live match is recreated only when the catalog type is exact;
    /// - incompatible or ambiguous matches stay blocking conflicts.
    fn recover_missing_event_entry(
        &mut self,
        event: &EventBlock,
        stale_anchor: &str,
        target_layer: Option<String>,
    ) -> Option<NodeEntity> {
        let exact_meta = match self.exact_event_metadata(event) {
            Ok(meta) => meta,
            Err(diagnostic) => {
                self.result.diagnostics.push(format!(
                    "event `{}` anchors to `{stale_anchor}`, which no longer exists on the board; {diagnostic}",
                    event.name
                ));
                return None;
            }
        };
        let canonical_type_display = exact_meta.as_ref().map(|meta| to_camel_case(&meta.name));
        let desired_alias = event.event_name.as_deref().or_else(|| {
            canonical_type_display
                .as_deref()
                .filter(|display| !pin_name_matches(display, &event.name))
                .map(|_| event.name.as_str())
        });

        let mut identity_matches = all_board_nodes(self.existing)
            .into_iter()
            .filter(|node| {
                !self.claimed_event_entries.contains(&node.id)
                    && self.node_is_in_target_layer(node, target_layer.as_deref())
                    && node.start == Some(true)
                    && event_entry_incompatibility(&node_to_metadata(node)).is_none()
                    && match &exact_meta {
                        Some(meta) => {
                            node.name == meta.name
                                && desired_alias.is_none_or(|alias| {
                                    pin_name_matches(&node.friendly_name, alias)
                                })
                        }
                        None => {
                            (pin_name_matches(&node.name, &event.name)
                                || pin_name_matches(&node.friendly_name, &event.name))
                                && event.event_name.as_deref().is_none_or(|alias| {
                                    pin_name_matches(&node.friendly_name, alias)
                                })
                        }
                    }
            })
            .collect::<Vec<_>>();
        identity_matches.sort_by(|left, right| left.id.cmp(&right.id));

        let mut compatible = identity_matches
            .iter()
            .copied()
            .filter(|node| {
                event_parameter_contracts_match(
                    node,
                    &event.params,
                    &self.interface_schemas,
                    &self.existing.refs,
                )
            })
            .collect::<Vec<_>>();

        // Several entries may legitimately share one event type. The old body's still-live first
        // execution node is a stronger identity signal than name alone, so use it to narrow the
        // candidates when exactly one entry currently drives that body.
        if compatible.len() > 1
            && let Some(first_body_node) = first_existing_exec_body_node(self.existing, &event.body)
        {
            let connected = compatible
                .iter()
                .copied()
                .filter(|entry| self.event_entry_targets_node(entry, &first_body_node.id))
                .collect::<Vec<_>>();
            if connected.len() == 1 {
                compatible = connected;
            }
        }

        match compatible.as_slice() {
            [entry] => {
                self.claimed_event_entries.insert(entry.id.clone());
                self.result.corrections.push(format!(
                    "Re-anchored event `{}` from missing `{stale_anchor}` to live entry `{}`.",
                    event.name, entry.id
                ));
                return Some(NodeEntity::Existing(entry.id.clone()));
            }
            entries @ [_, _, ..] => {
                self.result.diagnostics.push(format!(
                    "event `{}` anchors to `{stale_anchor}`, which no longer exists on the board; {} compatible live entries in the same scope make automatic re-anchoring ambiguous ({})",
                    event.name,
                    entries.len(),
                    entries
                        .iter()
                        .map(|entry| format!("`{}`", entry.id))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                return None;
            }
            [] => {}
        }

        // A strong alias found the intended live entry but its payload changed. Recreating a
        // sibling would hide a real contract conflict and leave two identically named triggers.
        if desired_alias.is_some() && !identity_matches.is_empty() {
            self.result.diagnostics.push(format!(
                "event `{}` anchors to `{stale_anchor}`, which no longer exists on the board; the matching live entry has an incompatible parameter contract and cannot be re-anchored automatically",
                event.name
            ));
            return None;
        }

        if exact_meta.is_some() {
            if block_has_no_executable_statements(&event.body) {
                self.result.diagnostics.push(format!(
                    "event `{}` anchors to `{stale_anchor}`, which no longer exists on the board; refusing to recreate an event with no executable body nodes",
                    event.name
                ));
                return None;
            }
            let recreated = self.add_entry_node(
                &event.name,
                &event.node_type,
                event.event_name.as_deref(),
                target_layer,
                &event.params,
            );
            if recreated.is_some() {
                self.result.corrections.push(format!(
                    "Recreated event `{}` because anchor `{stale_anchor}` no longer exists; the applied board will assign a new anchor.",
                    event.name
                ));
            }
            return recreated;
        }

        self.result.diagnostics.push(format!(
            "event `{}` anchors to `{stale_anchor}`, which no longer exists on the board; no unique compatible live entry was found and the alias-only header does not preserve an exact event type. Refresh FlowScript, or declare an explicit type such as `eventsGeneric {}(...)` before removing the stale anchor",
            event.name, event.name
        ));
        None
    }

    fn plan_event(&mut self, event: &EventBlock, target_layer: Option<String>) {
        if event.anchor.is_none() && block_has_no_executable_statements(&event.body) {
            self.result.diagnostics.push(format!(
                "new event `{}` has no executable body nodes; an Event entry is a registration target, not a complete workflow",
                event.name
            ));
        }
        let entry = match &event.anchor {
            Some(anchor) => match find_board_node(self.existing, anchor) {
                Some(node) => {
                    if !event.node_type.trim().is_empty() && node.name != event.node_type {
                        self.result.diagnostics.push(format!(
                            "event `{}` declares exact node_type `{}`, but anchor `{anchor}` resolves to `{}`",
                            event.name, event.node_type, node.name
                        ));
                        None
                    } else if event.node_type.trim().is_empty()
                        && !pin_name_matches(&node.name, &event.name)
                        && !pin_name_matches(&node.friendly_name, &event.name)
                    {
                        self.result.diagnostics.push(format!(
                            "event `{}` keeps anchor `{anchor}`, but that anchor identifies `{}`; remove the anchor to replace the event type",
                            event.name, node.name
                        ));
                        None
                    } else if !event_parameter_contracts_match(
                        node,
                        &event.params,
                        &self.interface_schemas,
                        &self.existing.refs,
                    ) {
                        self.result.diagnostics.push(format!(
                            "event `{}` changes the parameter contract of anchored entry `{anchor}`; parameter names, order, types, containers, and authored schemas must match the live event outputs",
                            event.name
                        ));
                        None
                    } else {
                        if let Some(event_name) = event
                            .event_name
                            .as_deref()
                            .filter(|name| !name.trim().is_empty())
                            && !pin_name_matches(&node.friendly_name, event_name)
                        {
                            self.update_commands.push(BoardCommand::RenameNode {
                                node_id: anchor.clone(),
                                friendly_name: event_name.to_string(),
                                summary: Some(format!("Rename event to {event_name}")),
                            });
                        }
                        self.claimed_event_entries.insert(anchor.clone());
                        Some(NodeEntity::Existing(anchor.clone()))
                    }
                }
                None => self.recover_missing_event_entry(event, anchor, target_layer.clone()),
            },
            None => self
                .claim_existing_entry_for_unanchored_event(event, target_layer.as_deref())
                .or_else(|| {
                    self.add_entry_node(
                        &event.name,
                        &event.node_type,
                        event.event_name.as_deref(),
                        target_layer.clone(),
                        &event.params,
                    )
                }),
        };

        self.push_scope();
        if let Some(entry) = &entry {
            self.seed_params_from_entity(&event.params, entry);
        }
        // A handler body is an independent entry point: a `return` inside it is the event-return
        // sugar (`events_generic_return_result`), never a return of the enclosing function whose
        // layer the handler happens to live in.
        let enclosing_function_returns = std::mem::take(&mut self.function_return_targets);
        let enclosing_blocks = std::mem::take(&mut self.closed_block_symbols);
        self.plan_block(&event.body, entry.map(ExecCursor::new), target_layer);
        self.function_return_targets = enclosing_function_returns;
        self.closed_block_symbols = enclosing_blocks;
        self.pop_scope();
    }

    fn plan_function_body(&mut self, func: &FnDecl) {
        let Some(planned) = self.planned_functions.get(&func.name).cloned() else {
            return;
        };
        if func.anchor.is_none() {
            if block_has_no_executable_statements(&func.body) {
                self.result.diagnostics.push(format!(
                    "new function `{}` has no executable body nodes; empty helper layers are not actionable FlowScript",
                    func.name
                ));
            } else if !planned.impure && func.returns.is_empty() && !planned.has_unresolved_calls {
                self.result.diagnostics.push(format!(
                    "new function `{}` is pure and declares no return values; its body has no observable runtime effect and cannot be reached through execution wiring. Declare and return a value, or use a catalog node with an Execution input if the helper is intended to perform side effects",
                    func.name
                ));
            }
        }
        let layer = planned.entity.clone();
        let target_layer = Some(layer.node_ref());
        self.push_scope();
        self.seed_function_params(&func.params, &layer);
        self.function_return_targets.push((
            layer.clone(),
            planned.returns.clone(),
            func.name.clone(),
        ));
        let entry = if planned.impure {
            self.function_entry_cursor(&layer, &func.name)
        } else {
            None
        };
        let enclosing_blocks = std::mem::take(&mut self.closed_block_symbols);
        let final_cursors = self.plan_block(&func.body, entry, target_layer);
        if planned.impure {
            for cursor in final_cursors {
                self.wire_function_exit(&layer, cursor);
            }
        }
        self.closed_block_symbols = enclosing_blocks;
        self.function_return_targets.pop();
        self.pop_scope();
    }

    /// Validate the graph commands produced for new Function layers, rather than trusting only
    /// the source-level shape. A function can contain syntactically valid calls yet still render
    /// and execute as an empty layer when none of those calls materialize, or when its execution
    /// boundaries are not connected to the body.
    fn check_new_function_structure(&mut self) {
        let new_functions = self
            .planned_functions
            .iter()
            .filter_map(|(name, planned)| match &planned.entity {
                NodeEntity::Layer { ref_id, pins } => Some((
                    name.clone(),
                    ref_id.clone(),
                    pins.clone(),
                    planned.has_unresolved_calls,
                )),
                NodeEntity::Existing(_) | NodeEntity::New { .. } => None,
            })
            .collect::<Vec<_>>();

        for (name, layer_ref, pins, has_unresolved_calls) in new_functions {
            if has_unresolved_calls {
                // Planning already emitted the exact catalog-resolution failure. The apparent
                // empty/pure/disconnected layer is a consequence of that missing node, not an
                // independent repair target.
                continue;
            }
            let body_nodes = self
                .add_commands
                .iter()
                .filter_map(|command| match command {
                    BoardCommand::AddNode {
                        ref_id: Some(ref_id),
                        target_layer: Some(target_layer),
                        ..
                    } if target_layer == &layer_ref => Some(ref_id.clone()),
                    _ => None,
                })
                .collect::<HashSet<_>>();

            if body_nodes.is_empty() {
                self.result.diagnostics.push(format!(
                    "new function `{name}` contains no materialized body nodes in its Function layer; refusing to create a runtime-empty helper"
                ));
                continue;
            }

            let has_exec_in = pins.iter().any(|pin| {
                pin.name == FUNCTION_EXEC_IN
                    && pin.data_type == "Execution"
                    && pin.pin_type == "Input"
            });
            let has_exec_out = pins.iter().any(|pin| {
                pin.name == FUNCTION_EXEC_OUT
                    && pin.data_type == "Execution"
                    && pin.pin_type == "Output"
            });
            if !(has_exec_in && has_exec_out) {
                continue;
            }

            let entry_connected = self.connect_commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, from_pin, to_node, .. }
                        if from_node == &layer_ref
                            && from_pin == FUNCTION_EXEC_IN
                            && body_nodes.contains(to_node)
                )
            });
            if !entry_connected {
                self.result.diagnostics.push(format!(
                    "new impure function `{name}` has no Function exec_in connection to a materialized body node; its body would never run"
                ));
            }

            let exit_connected = self.connect_commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, to_node, to_pin, .. }
                        if body_nodes.contains(from_node)
                            && to_node == &layer_ref
                            && to_pin == FUNCTION_EXEC_OUT
                )
            });
            if !exit_connected {
                self.result.diagnostics.push(format!(
                    "new impure function `{name}` has no materialized body tail connected to Function exec_out; callers could not continue after it. End the body on a node with one execution output, use explicit labelled arms for terminal multi-output nodes, or add an exact continuation policy for that node type"
                ));
            }
        }
    }

    /// A `tools:`/`fnRefs:` target must resolve to a concrete entry NODE at run time: the executor
    /// looks it up in the flat node map (`ExecutionContext::get_referenced_functions`), derives the
    /// tool name from that node's friendly name and the tool's WHOLE parameter schema from its data
    /// OUTPUT pins, triggers it, and reads the tool result from the `set_result` an
    /// `events_generic_return_result` writes.
    ///
    /// A FlowScript `function` materializes as a Function LAYER with boundary pins and no entry
    /// node, so such a reference can never be registered: `apply` rejects it with "Function layer
    /// `X` has no referenceable event/handler entry" and rolls the WHOLE batch back, and
    /// `validate_and_deduplicate_fn_refs` would silently strip it even if apply accepted it.
    /// Reject it here instead, where `check_flowscript` reports it with a fix — an apply-phase
    /// `Err` is invisible to the model.
    ///
    /// Only NEW function layers are checked: an anchored layer may already hold an entry node that
    /// the text does not re-declare, and apply resolves that case correctly today.
    fn check_function_ref_targets(&mut self) {
        let new_layer_targets: Vec<(String, String)> = self
            .fn_ref_commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::SetNodeFunctionRefs { fn_refs, .. } => Some(fn_refs.clone()),
                _ => None,
            })
            .flatten()
            .filter_map(
                |name| match self.planned_functions.get(&name).map(|p| &p.entity) {
                    Some(NodeEntity::Layer { ref_id, .. }) => Some((name, ref_id.clone())),
                    _ => None,
                },
            )
            .collect();

        for (name, layer_ref) in new_layer_targets {
            let has_entry = self.add_commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::AddNode {
                        ref_id: Some(ref_id),
                        target_layer: Some(target_layer),
                        ..
                    } if target_layer == &layer_ref && self.event_entry_refs.contains(ref_id)
                )
            });
            if !has_entry {
                self.result.diagnostics.push(format!(
                    "new function `{name}` is referenced as an agent tool but its Function layer contains no event/handler entry node, so the reference cannot be registered at run time. Declare `{name}` as a handler block — `{name}(<params>) {{ … return <value> }}` — instead of `function {name}(…)`; the handler's data outputs become the tool's arguments and its `return` becomes the tool result"
                ));
            }
        }
    }

    /// Execution entry for an impure function body: the layer's `exec_in` boundary pin. The
    /// runtime `control_call_function` node finds the body's entry node by following exactly this
    /// edge, so without it the body would never run.
    fn function_entry_cursor(&mut self, layer: &NodeEntity, name: &str) -> Option<ExecCursor> {
        match layer {
            NodeEntity::Layer { .. } => Some(ExecCursor::with_output(
                layer.clone(),
                Some(FUNCTION_EXEC_IN.to_string()),
            )),
            NodeEntity::Existing(id) => {
                let exec_in = self.existing.layers.get(id).and_then(|existing_layer| {
                    existing_layer
                        .pins
                        .values()
                        .find(|pin| {
                            pin.pin_type == PinType::Input
                                && pin.data_type == VariableType::Execution
                        })
                        .map(|pin| pin.name.clone())
                });
                match exec_in {
                    Some(pin) => Some(ExecCursor::with_output(layer.clone(), Some(pin))),
                    None => {
                        self.result.diagnostics.push(format!(
                            "function `{name}` has impure statements but its existing layer has no execution boundary pins; its body will not run when called. Recreate the function (delete and re-add the `function` declaration) to migrate it"
                        ));
                        None
                    }
                }
            }
            NodeEntity::New { .. } => None,
        }
    }

    /// Close an impure function body: wire a final statement's execution output to the layer's
    /// `exec_out` boundary pin so the graph reads (and renders) as a complete chain. Called once
    /// per final cursor — branch tails legally fan into the boundary pin.
    fn wire_function_exit(&mut self, layer: &NodeEntity, cursor: ExecCursor) {
        if matches!(&cursor.entity, NodeEntity::Existing(_))
            && matches!(layer, NodeEntity::Existing(_))
        {
            return;
        }
        let Some(from_pin) = cursor
            .output_pin
            .clone()
            .or_else(|| self.entity_exec_output_pin(&cursor.entity))
        else {
            return;
        };
        // The body's first statement wires FROM the layer's exec_in; if the body is empty the
        // cursor still points at the layer itself — nothing to close then.
        if cursor.entity.node_ref() == layer.node_ref() {
            return;
        }
        let to_pin = match layer {
            NodeEntity::Layer { .. } => Some(FUNCTION_EXEC_OUT.to_string()),
            NodeEntity::Existing(id) => self.existing.layers.get(id).and_then(|existing_layer| {
                existing_layer
                    .pins
                    .values()
                    .find(|pin| {
                        pin.pin_type == PinType::Output && pin.data_type == VariableType::Execution
                    })
                    .map(|pin| pin.name.clone())
            }),
            NodeEntity::New { .. } => None,
        };
        let Some(to_pin) = to_pin else { return };
        self.connect_commands.push(BoardCommand::ConnectPins {
            from_node: cursor.entity.node_ref(),
            from_pin,
            to_node: layer.node_ref(),
            to_pin,
            summary: Some("Connect function body to its exec out".to_string()),
        });
    }

    /// Plan a statement block. Returns the final execution cursors — where the chain "ends" — so
    /// function bodies can close onto their layer's `exec_out` boundary pin. The cursor SET
    /// models fan-in: after `if (x) { a() }`, the next statement wires from both the untaken
    /// `false` pin and `a`'s exec output.
    fn plan_block(
        &mut self,
        block: &Block,
        entry: Option<ExecCursor>,
        target_layer: Option<String>,
    ) -> Vec<ExecCursor> {
        let mut previous_execs: Vec<ExecCursor> = entry.into_iter().collect();
        // The `(node, pin)` exec edge the current insertion streak branched off from — set
        // when a NEW node is first wired after an EXISTING one, and used to splice out only
        // that predecessor's old edge when the chain reaches the next existing node.
        let mut insertion_origin: Option<(String, String)> = None;
        let promoted_local_aliases = promoted_local_aliases(block);

        for stmt in &block.stmts {
            let planned = self.plan_stmt(
                stmt,
                target_layer.clone(),
                matches!(
                    stmt,
                    Stmt::LocalAlias { name, .. } if promoted_local_aliases.contains(name)
                ),
            );

            // Splice impure argument calls (data-wired during expression resolution) into the
            // exec chain ahead of the statement that consumes their outputs, innermost first,
            // so they actually execute at runtime.
            for splice in std::mem::take(&mut self.pending_exec_splices) {
                for previous in &previous_execs {
                    let connected_edge =
                        self.connect_exec(previous, &splice, insertion_origin.as_ref());
                    if let Some(edge) = connected_edge
                        && insertion_origin.is_none()
                        && matches!(previous.entity, NodeEntity::Existing(_))
                        && matches!(splice, NodeEntity::New { .. })
                    {
                        insertion_origin = Some(edge);
                    }
                }
                previous_execs = vec![ExecCursor::new(splice)];
            }

            let Some(current) = planned else {
                continue;
            };

            let accepts_exec = self.entity_exec_input_pin(&current.entity).is_some();
            let continues_exec = current.next_exec_pin.is_some()
                || (accepts_exec && !self.entity_exec_output_pins(&current.entity).is_empty());

            if accepts_exec && !current.skip_exec_input_connection {
                if previous_execs.is_empty() {
                    // No execution predecessor to wire from (e.g. a function body's first node):
                    // exempt it from the dangling-execution warning.
                    if let NodeEntity::New { ref_id, .. } = &current.entity {
                        self.exec_check_exempt.insert(ref_id.clone());
                    }
                } else {
                    // The streaming side-chain special case only exists for a single linear
                    // predecessor; fan-in sets (branch tails) never stream.
                    let single_previous = previous_execs.len() == 1;
                    let mut statement_is_side_chain = false;
                    for previous in previous_execs.clone() {
                        let preferred_output = self.preferred_exec_output_for_input_sources(
                            &previous.entity,
                            &current.input_sources,
                        );
                        // Only a streaming-preferred branch (on_stream/on_chunk/…) is a
                        // side-chain that must not advance the main exec cursor. A cursor that
                        // merely carries a non-default continuation pin (loop body entry, loop
                        // "done") IS the main chain.
                        let used_branch_output = single_previous
                            && preferred_output.is_some()
                            && self.entity_exec_output_pin(&previous.entity) != preferred_output;
                        let previous = match preferred_output {
                            Some(output_pin) => {
                                ExecCursor::with_output(previous.entity.clone(), Some(output_pin))
                            }
                            None => previous,
                        };
                        let connected_edge = self.connect_exec(
                            &previous,
                            &current.entity,
                            insertion_origin.as_ref(),
                        );
                        if let Some(edge) = connected_edge
                            && !used_branch_output
                            && insertion_origin.is_none()
                            && matches!(previous.entity, NodeEntity::Existing(_))
                            && matches!(current.entity, NodeEntity::New { .. })
                        {
                            insertion_origin = Some(edge);
                        }
                        if used_branch_output {
                            statement_is_side_chain = true;
                        }
                    }
                    if statement_is_side_chain {
                        continue;
                    }
                }
            }

            if matches!(current.entity, NodeEntity::Existing(_)) {
                insertion_origin = None;
            }

            let mut next_cursors = current.extra_exec_tails.clone();
            if continues_exec && !current.suppress_self_continuation {
                next_cursors.push(current.next_cursor());
            }
            if !next_cursors.is_empty() {
                previous_execs = next_cursors;
            } else if accepts_exec || current.suppress_self_continuation {
                previous_execs = Vec::new();
            }
        }
        previous_execs
    }

    fn plan_stmt(
        &mut self,
        stmt: &Stmt,
        target_layer: Option<String>,
        promote_local_alias: bool,
    ) -> Option<PlannedStmt> {
        match stmt {
            Stmt::Let { name, call, anchor } => {
                let planned_call =
                    self.plan_call_statement_with_sources(call, anchor.as_deref(), target_layer);
                let (entity, input_sources) = planned_call?;
                self.insert_symbol(
                    name.clone(),
                    SymbolValue::Source(ValueSource {
                        node: entity.clone(),
                        output_pin: None,
                    }),
                );
                Some(PlannedStmt::with_input_sources(entity, input_sources))
            }
            Stmt::Call { call, anchor } => {
                let (entity, input_sources) =
                    self.plan_call_statement_with_sources(call, anchor.as_deref(), target_layer)?;
                Some(PlannedStmt::with_input_sources(entity, input_sources))
            }
            Stmt::Assign {
                target,
                value,
                anchor,
            } => {
                if let Some(anchor) = anchor {
                    if let Some(variable_id) = self.variable_id_for_assignment_target(target) {
                        let entity = NodeEntity::Existing(anchor.clone());
                        self.plan_existing_variable_set_node(
                            &entity,
                            &variable_id,
                            value,
                            target_layer,
                        );
                        self.assign_symbol(
                            target.clone(),
                            SymbolValue::VariableRef {
                                variable_id: variable_id.clone(),
                            },
                        );
                        return Some(PlannedStmt::new(entity));
                    }

                    if let Some((call, output_pin)) = assigned_call_expr(value) {
                        let entity = self
                            .plan_call_statement(call, Some(anchor), target_layer)
                            .unwrap_or_else(|| NodeEntity::Existing(anchor.clone()));
                        let output_pin = output_pin
                            .and_then(|pin| self.resolve_entity_output_pin(&entity, Some(pin)));
                        self.insert_symbol(
                            target.clone(),
                            SymbolValue::Source(ValueSource {
                                node: entity.clone(),
                                output_pin,
                            }),
                        );
                        return Some(PlannedStmt::new(entity));
                    }

                    return Some(PlannedStmt::new(NodeEntity::Existing(anchor.clone())));
                }

                if let Some(variable_id) = self.variable_id_for_assignment_target(target) {
                    let entity =
                        self.add_variable_set_node(&variable_id, value, target_layer.clone());
                    self.assign_symbol(
                        target.clone(),
                        SymbolValue::VariableRef {
                            variable_id: variable_id.clone(),
                        },
                    );
                    return entity.map(PlannedStmt::new);
                }

                // Rebinding an outer-scope `const` node output inside a nested block is
                // last-writer-wins on the symbol table: statements after the block would
                // silently wire into whichever arm was planned last. Surface it instead.
                if let Some(declared_scope) = self
                    .symbols
                    .iter()
                    .rposition(|scope| scope.contains_key(target.as_str()))
                    && declared_scope + 1 < self.symbols.len()
                    && matches!(
                        self.symbols[declared_scope].get(target.as_str()),
                        Some(SymbolValue::Source(_))
                    )
                {
                    self.result.diagnostics.push(format!(
                        "assignment to `{target}` inside a nested block rebinds an outer-scope binding (a function parameter or `const` node output); statements after the block would silently read only this arm's value. For a parameter, assign to a variable with a different name instead; for a call output, declare that variable with `let` and a literal initializer before the block, then assign it in each arm"
                    ));
                }

                let Some(resolved) = self.resolve_expr(value, target_layer.clone()) else {
                    self.result.diagnostics.push(format!(
                        "assignment to `{target}` is not a literal or resolvable node output; skipped local alias"
                    ));
                    return None;
                };
                let entity = match (&resolved, value) {
                    (SymbolValue::Source(source), Expr::Call(_)) => Some(source.node.clone()),
                    (SymbolValue::Source(source), Expr::Field { base, .. })
                        if matches!(base.as_ref(), Expr::Call(_)) =>
                    {
                        Some(source.node.clone())
                    }
                    _ => None,
                };
                if let Some(entity) = &entity {
                    self.undefer_statement_call_splice(entity);
                }
                self.assign_symbol(target.clone(), resolved);
                entity.map(PlannedStmt::new)
            }
            Stmt::FieldAssign {
                base,
                path,
                value,
                anchor,
            } => {
                // Expand the `base.path = value` struct-field write to its `struct_set` accumulator
                // form and reuse the `Stmt::Assign`+call planning path: it wires `struct_in` from
                // `base`'s prior source, rebinds `base` to `struct_out`, and (via the non-anchored
                // path) drops the struct_set's own splice so it never self-connects.
                let call = field_assign_struct_set_call(base, path, value, anchor.as_deref());
                let assign = Stmt::Assign {
                    target: base.clone(),
                    value: Expr::Call(call),
                    anchor: anchor.clone(),
                };
                self.plan_stmt(&assign, target_layer, promote_local_alias)
            }
            Stmt::LocalAlias {
                name,
                value,
                anchor,
            } => {
                if let Some(anchor) = anchor {
                    if let Some((call, output_pin)) = assigned_call_expr(value) {
                        let entity = self
                            .plan_call_statement(call, Some(anchor), target_layer)
                            .unwrap_or_else(|| NodeEntity::Existing(anchor.clone()));
                        let output_pin = output_pin
                            .and_then(|pin| self.resolve_entity_output_pin(&entity, Some(pin)));
                        self.insert_symbol(
                            name.clone(),
                            SymbolValue::Source(ValueSource {
                                node: entity.clone(),
                                output_pin,
                            }),
                        );
                        return Some(PlannedStmt::new(entity));
                    }

                    return Some(PlannedStmt::new(NodeEntity::Existing(anchor.clone())));
                }

                if promote_local_alias {
                    let variable_id = self.create_local_variable(name, value, target_layer.clone());
                    self.insert_symbol(
                        name.clone(),
                        SymbolValue::VariableRef {
                            variable_id: variable_id.clone(),
                        },
                    );
                    if literal_expr_to_value(value).is_some() {
                        return None;
                    }
                    // A non-literal initializer (`let x = call(...)`) cannot become the
                    // variable's default value; seed it with an explicit variable_set so the
                    // initializing expression still materializes and joins the exec chain.
                    let entity = self.add_variable_set_node(&variable_id, value, target_layer)?;
                    return Some(PlannedStmt::new(entity));
                }

                let Some(resolved) = self.resolve_expr(value, target_layer.clone()) else {
                    self.result.diagnostics.push(format!(
                        "local alias `{name}` is not a literal or resolvable node output; skipped"
                    ));
                    return None;
                };
                let entity = match (&resolved, value) {
                    (SymbolValue::Source(source), Expr::Call(_)) => Some(source.node.clone()),
                    (SymbolValue::Source(source), Expr::Field { base, .. })
                        if matches!(base.as_ref(), Expr::Call(_)) =>
                    {
                        Some(source.node.clone())
                    }
                    _ => None,
                };
                if let Some(entity) = &entity {
                    self.undefer_statement_call_splice(entity);
                }
                self.insert_symbol(name.clone(), resolved);
                entity.map(PlannedStmt::new)
            }
            Stmt::Branch {
                bind,
                call,
                condition,
                arms,
                anchor,
            } => {
                let mut synthesized_branch = false;
                let entity = if let Some(bind) = bind
                    && is_placeholder_call(call)
                {
                    match self.lookup_symbol(bind) {
                        Some(SymbolValue::Source(source)) => Some(source.node),
                        _ => {
                            self.result.diagnostics.push(format!(
                                "branch binding `{bind}` does not resolve to a node output"
                            ));
                            None
                        }
                    }
                } else if anchor.is_none()
                    && bind.is_none()
                    && is_placeholder_call(call)
                    && condition.is_some()
                {
                    // New `if (cond) { ... } [else { ... }]` sugar: synthesize the
                    // `control_branch` node and wire the condition through the normal
                    // argument machinery. Arm labels True/False match its exec pins.
                    synthesized_branch = true;
                    let branch_call = Call {
                        node_type: "control_branch".to_string(),
                        display: "controlBranch".to_string(),
                        args: vec![Arg {
                            name: "condition".to_string(),
                            value: condition.clone().expect("condition checked above"),
                        }],
                        anchor: None,
                    };
                    self.plan_call_statement(&branch_call, None, target_layer.clone())
                } else {
                    let entity =
                        self.plan_call_statement(call, anchor.as_deref(), target_layer.clone());
                    if let (Some(bind), Some(entity)) = (bind, entity.as_ref()) {
                        self.insert_symbol(
                            bind.clone(),
                            SymbolValue::Source(ValueSource {
                                node: entity.clone(),
                                output_pin: None,
                            }),
                        );
                    }
                    entity
                };

                if let Some(cond) = condition {
                    // Condition-form branches use a placeholder call in the AST. For an anchored
                    // branch, reconcile the condition against its existing input source so edits
                    // inside sugared comparisons update/reuse that source. A new synthesized
                    // branch already wired the same expression through its call arguments.
                    if let Some(existing @ NodeEntity::Existing(node_id)) = entity.as_ref()
                        && is_placeholder_call(call)
                        && let Some(node) = find_board_node(self.existing, node_id)
                    {
                        let meta = node_to_metadata(node);
                        let condition_call = Call {
                            node_type: node.name.clone(),
                            display: to_camel_case(&node.name),
                            args: vec![Arg {
                                name: "condition".to_string(),
                                value: cond.clone(),
                            }],
                            anchor: Some(node_id.clone()),
                        };
                        self.plan_call_arguments(
                            &condition_call,
                            existing,
                            &meta,
                            target_layer.clone(),
                            true,
                        );
                    } else if !synthesized_branch {
                        let _ = self.resolve_expr(cond, target_layer.clone());
                    }
                }
                // The branch's own argument/condition splices must chain BEFORE the branch
                // node, not inside an arm — stash them across the nested blocks.
                let mut stashed_splices = std::mem::take(&mut self.pending_exec_splices);
                let mut arm_tails: Vec<ExecCursor> = Vec::new();
                for arm in arms {
                    self.push_scope();
                    // Each arm chains from ITS labelled exec output. Wiring every arm from the
                    // default/policy pin would make later arms silently steal earlier arms' (and
                    // the default continuation's) single-target exec edge.
                    let arm_cursor = entity.clone().and_then(|entity| {
                        match self.resolve_arm_exec_pin(&entity, &arm.label) {
                            Some(pin) => Some(ExecCursor::with_output(entity, Some(pin))),
                            None => {
                                if !arm.body.stmts.is_empty() {
                                    // Name the node and list the real labels: without them the
                                    // model guesses spellings (`error`, `execError`, ...) for
                                    // many rounds instead of fixing the label in one.
                                    let node_name = if call.display.trim().is_empty() {
                                        entity.node_ref()
                                    } else {
                                        call.display.clone()
                                    };
                                    let available = self.entity_exec_output_pins(&entity);
                                    let available = if available.is_empty() {
                                        "none".to_string()
                                    } else {
                                        available
                                            .iter()
                                            .map(|name| to_camel_case(name))
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    };
                                    self.result.diagnostics.push(format!(
                                        "branch arm label `{}` does not match an execution output on `{node_name}` (available execution outputs: {available}); its body was not wired — use the exact exec pin name as the arm label",
                                        arm.label
                                    ));
                                }
                                None
                            }
                        }
                    });
                    // Each arm's final cursors carry execution to whatever follows the branch
                    // (exec inputs are fan-in points). An empty arm's tail is the labelled pin
                    // itself — the pass-through case.
                    arm_tails.extend(self.plan_block(&arm.body, arm_cursor, target_layer.clone()));
                    self.pop_block_scope();
                }
                stashed_splices.append(&mut self.pending_exec_splices);
                self.pending_exec_splices = stashed_splices;
                entity.map(|entity| {
                    // The statement AFTER a branch continues from the arm tails plus the one
                    // exec output no arm claimed (e.g. `false` for a lone `if`). When every
                    // output is claimed, only the arm tails carry execution forward.
                    let arm_pins: HashSet<String> = arms
                        .iter()
                        .filter_map(|arm| self.resolve_arm_exec_pin(&entity, &arm.label))
                        .collect();
                    let remaining: Vec<String> = self
                        .entity_exec_output_pin_refs(&entity)
                        .into_iter()
                        .filter(|pin| !arm_pins.contains(pin))
                        .collect();
                    let mut planned = PlannedStmt::new(entity);
                    planned.extra_exec_tails = arm_tails;
                    planned.skip_exec_input_connection =
                        bind.is_some() && is_placeholder_call(call);
                    match remaining.as_slice() {
                        [single] => planned.next_exec_pin = Some(single.clone()),
                        [] => planned.suppress_self_continuation = true,
                        // Several unclaimed outputs (a bare multi-output call with one
                        // labelled arm): leave the default policy machinery to decide.
                        _ => {}
                    }
                    planned
                })
            }
            Stmt::Loop {
                bind,
                call,
                body,
                anchor,
                ..
            } => {
                let entity =
                    self.plan_call_statement(call, anchor.as_deref(), target_layer.clone());
                // The loop's own argument splices must chain BEFORE the loop node, not inside
                // its body — stash them across the nested plan_block.
                let mut stashed_splices = std::mem::take(&mut self.pending_exec_splices);
                self.push_scope();
                if let (Some(bind), Some(entity)) = (bind, entity.as_ref()) {
                    self.insert_symbol(
                        bind.clone(),
                        SymbolValue::Source(ValueSource {
                            node: entity.clone(),
                            output_pin: None,
                        }),
                    );
                }
                if let Some(entity) = entity.as_ref() {
                    let body_pin =
                        self.entity_exec_output_pin_named(entity, &["exec_out", "loop", "body"]);
                    self.plan_block(
                        body,
                        Some(ExecCursor::with_output(entity.clone(), body_pin)),
                        target_layer,
                    );
                }
                self.pop_block_scope();
                stashed_splices.append(&mut self.pending_exec_splices);
                self.pending_exec_splices = stashed_splices;
                entity.map(|entity| {
                    PlannedStmt::with_next_exec_pin(
                        entity.clone(),
                        self.entity_exec_output_pin_named(&entity, &["done", "exec_done"]),
                    )
                })
            }
            Stmt::Handler(event) => {
                // A nested handler is an independent entry point but lives (and counts) in the
                // enclosing scope's layer.
                self.plan_event(event, target_layer);
                None
            }
            Stmt::Return { values, anchor } => {
                self.plan_return(values, anchor.as_deref(), target_layer)
            }
            Stmt::Local(var) => {
                self.register_local_variable_decl(var, target_layer.as_deref());
                None
            }
            Stmt::Comment(_) => None,
        }
    }

    /// Register a function-local `let name: Type` declaration as a variable symbol when it
    /// identifies a live variable (by `//@v:` anchor, else by name — the current layer first).
    /// The lowered form of a promoted local is this declaration plus an anchored seed
    /// assignment; without the symbol that assignment can no longer resolve as a `variable_set`
    /// and the board's own round-trip stops applying.
    fn register_local_variable_decl(&mut self, var: &VarDecl, target_layer: Option<&str>) {
        let variable_id = var.anchor.clone().or_else(|| {
            let name_matches = |existing: &&Variable| {
                existing.name == var.name || to_camel_case(&existing.name) == var.name
            };
            target_layer
                .and_then(|layer_id| self.existing.layers.get(layer_id))
                .and_then(|layer| layer.variables.values().find(name_matches))
                .or_else(|| {
                    self.existing
                        .layers
                        .values()
                        .find_map(|layer| layer.variables.values().find(name_matches))
                })
                .or_else(|| self.existing.variables.values().find(name_matches))
                .map(|existing| existing.id.clone())
        });
        let Some(variable_id) = variable_id else {
            return;
        };
        self.variable_value_contracts.insert(
            variable_id.clone(),
            variable_value_pin_metadata(
                "value_in",
                type_ref_data_type(&var.ty).to_string(),
                type_ref_value_type(&var.ty).to_string(),
                var.schema.clone(),
            ),
        );
        self.variable_refs.insert(&variable_id, &var.name);
        self.insert_symbol(var.name.clone(), SymbolValue::VariableRef { variable_id });
    }

    fn function_layer_entity(&mut self, func: &FnDecl, impure: bool) -> Option<NodeEntity> {
        if let Some(anchor) = &func.anchor {
            let Some(existing) = self
                .existing
                .layers
                .get(anchor)
                .filter(|layer| matches!(layer.r#type, LayerType::Function))
            else {
                self.result.diagnostics.push(format!(
                    "function `{}` anchors to `{anchor}`, which is not an existing Function layer",
                    func.name
                ));
                return None;
            };
            if to_camel_case(&existing.name) != to_camel_case(&func.name) {
                self.result.diagnostics.push(format!(
                    "function `{}` anchors to Function layer `{anchor}` named `{}`; anchored function identity cannot be retargeted",
                    func.name, existing.name
                ));
                return None;
            }
            return Some(NodeEntity::Existing(anchor.clone()));
        }

        // An unanchored declaration still has a stable identity: its name. FlowScript forbids two
        // top-level functions sharing one name, so a same-named Function layer IS this function.
        // Without this, a fresh full-document draft (a repair, or any run that did not carry the
        // `//@n` anchors) re-creates every layer, and the board's own canonical readback then holds
        // duplicate declarations that no longer reconcile — the board stops round-tripping.
        let normalized_name = to_camel_case(&func.name);
        let existing_by_name: Vec<String> = self
            .existing
            .layers
            .iter()
            .filter(|(_, layer)| {
                matches!(layer.r#type, LayerType::Function)
                    && to_camel_case(&layer.name) == normalized_name
            })
            .map(|(id, _)| id.clone())
            .collect();
        if let [only] = existing_by_name.as_slice() {
            return Some(NodeEntity::Existing(only.clone()));
        }

        let ref_id = format!("${}", self.next_ref);
        self.next_ref += 1;
        let pins = function_layer_pins(func, impure, &self.interface_schemas);
        let position = self.next_position(None);
        self.add_commands.push(BoardCommand::CreateLayer {
            name: func.name.clone(),
            ref_id: Some(ref_id.clone()),
            layer_type: Some("Function".to_string()),
            node_ids: Vec::new(),
            pins: Some(
                pins.iter()
                    .map(|pin| PlaceholderPinDef {
                        name: pin.name.clone(),
                        friendly_name: pin.friendly_name.clone(),
                        description: None,
                        pin_type: pin.pin_type.clone(),
                        data_type: pin.data_type.clone(),
                        value_type: Some(pin.value_type.clone()),
                        schema: pin.schema.clone(),
                        enforce_schema: pin.enforce_schema,
                    })
                    .collect(),
            ),
            position: Some(position),
            color: None,
            target_layer: None,
            cache: func.cache.as_ref().map(function_cache_to_layer_cache),
            summary: Some(format!("Create function {}", func.name)),
        });
        Some(NodeEntity::Layer { ref_id, pins })
    }

    fn seed_function_params(&mut self, params: &[Param], layer: &NodeEntity) {
        for param in params {
            self.insert_symbol(
                param.name.clone(),
                SymbolValue::Source(ValueSource {
                    node: layer.clone(),
                    output_pin: Some(param.name.clone()),
                }),
            );
        }
    }

    fn plan_return(
        &mut self,
        values: &[Expr],
        anchor: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<PlannedStmt> {
        // Inside a function layer: connect each value to the layer's boundary output pin.
        if let Some((layer, return_params, function_name)) =
            self.function_return_targets.last().cloned()
        {
            for (index, value) in values.iter().enumerate() {
                let rendered = describe_expr(value);
                let Some(return_param) = return_params.get(index).cloned() else {
                    self.result.diagnostics.push(format!(
                        "return value {} (`{rendered}`) in function `{function_name}` has no matching function return pin; the signature declares {} return value(s) — declare a return pin for it or drop the extra value",
                        index + 1,
                        return_params.len()
                    ));
                    continue;
                };
                let source = self.resolve_return_value_source(
                    value,
                    &layer,
                    &return_param,
                    &function_name,
                    target_layer.clone(),
                );
                let Some(source) = source else {
                    self.result.diagnostics.push(format!(
                        "return value {} (`{rendered}`) in function `{function_name}` is not a resolvable FlowScript value; return a literal, a call output bound earlier in the body (`const x = call(...)`), or a variable",
                        index + 1
                    ));
                    continue;
                };
                let Some(output_pin) =
                    self.resolve_source_output_pin_for_input(&source, &return_param)
                else {
                    self.result.diagnostics.push(format!(
                        "could not choose output pin for return value {} (`{rendered}`) in function `{function_name}`; read a named output (e.g. `call().pinName`) so the value has one exact source",
                        index + 1
                    ));
                    continue;
                };
                // `return <bare parameter>`: `seed_function_params` binds parameters to this
                // layer's own boundary Input pins, so this edge would be layer -> itself, which
                // `connect_pins` rejects and which rolls the whole apply batch back. Splice the
                // board's pass-through primitive between the two boundary pins instead. This is
                // the data-side twin of `wire_function_exit`'s existing self-reference guard.
                let (source, output_pin) = if source.node.node_ref() == layer.node_ref() {
                    match self.boundary_return_passthrough(
                        &layer,
                        &output_pin,
                        &return_param,
                        &function_name,
                    ) {
                        Some(spliced) => {
                            let pin = spliced.output_pin.clone().unwrap_or_default();
                            (spliced, pin)
                        }
                        None => continue,
                    }
                } else {
                    (source, output_pin)
                };
                self.queue_validated_data_connection(
                    &source,
                    output_pin,
                    &layer,
                    &return_param,
                    &return_param.name,
                    "Connect FlowScript function return".to_string(),
                    &format!(
                        "return value {} (`{rendered}`) for `{}` in function `{function_name}`",
                        index + 1,
                        return_param.name
                    ),
                    false,
                );
            }
            return None;
        }

        // Outside a function layer: an event/tool-entry `return` reverses the
        // `events_generic_return_result` sugar so agent tools and event handlers can return a value.
        self.plan_event_return(values, anchor, target_layer)
    }

    /// Resolve one function `return` value to a data source, consulting the boundary pin's
    /// existing wiring FIRST (like `plan_call_arguments` does for arguments) so an unchanged
    /// round-trip reuses the live producer instead of minting a duplicate `variable_get` (and a
    /// stale half-edge) on every apply.
    fn resolve_return_value_source(
        &mut self,
        value: &Expr,
        layer: &NodeEntity,
        return_param: &PinMetadata,
        function_name: &str,
        target_layer: Option<String>,
    ) -> Option<ValueSource> {
        for existing in self.existing_sources_for_input_ref(layer, &return_param.name) {
            if let Some(symbol) =
                self.resolve_expr_using_existing_source(value, existing, target_layer.clone())
            {
                return self.symbol_to_source(symbol, target_layer);
            }
        }
        match self.resolve_expr(value, target_layer.clone())? {
            // A literal has no producing node to wire from; materialize it as a typed
            // layer-local variable read through a `variable_get` in the function body.
            SymbolValue::Literal(literal) => self.literal_return_source(
                layer,
                function_name,
                return_param,
                literal,
                target_layer,
            ),
            symbol => self.symbol_to_source(symbol, target_layer),
        }
    }

    /// Reverse the `events_generic_return_result` sugar: reuse the anchored result node (or add a
    /// fresh one), wire the returned value into its `response` input, and chain it into the exec
    /// flow as a terminal statement.
    /// Bridge a Function layer's parameter pin to one of its own return pins (`return <param>`).
    ///
    /// That value both enters and leaves the SAME layer, and the direct edge is not representable:
    /// `connect_pins` rejects `from_node == to_node` outright ("Cannot connect a node to itself",
    /// aborting the whole apply); it could not be persisted anyway, because the command mutates two
    /// independent clones of one entity and the second upsert erases the first; and
    /// `control_call_function::read_outputs` resolves a return pin's dependency by searching the
    /// execution graph's NODES, so a boundary-owned dependency leaves the output silently unset.
    ///
    /// The board's primitive for a wire that must become two edges is `reroute`: a pure
    /// pass-through that `lower::resolve_source` already collapses back to the bare parameter
    /// reference and that `BoardIndex::data_source_for_pin_id` already traces through, so the text
    /// round-trips unchanged and the deletion planner never treats it as authored.
    ///
    /// Returns the source to wire the return pin from, or `None` when the bridge already exists on
    /// the live board (no-op) or could not be built (diagnosed).
    fn boundary_return_passthrough(
        &mut self,
        layer: &NodeEntity,
        param_pin: &str,
        return_param: &PinMetadata,
        function_name: &str,
    ) -> Option<ValueSource> {
        // An applied bridge collapses back to `{layer, param}` in `BoardIndex`
        // (`data_source_for_pin_id` traces through reroutes and falls back to `boundary_sources`),
        // so the reconciler's ordinary already-wired test recognises it and the round-trip is a
        // no-op. Without this the reroute chain would grow by one node on every apply.
        let already_wired = self
            .existing_sources_for_input_ref(layer, &return_param.name)
            .into_iter()
            .any(|existing| {
                existing.node.node_ref() == layer.node_ref()
                    && existing.output_pin.as_deref() == Some(param_pin)
            });
        if already_wired {
            return None;
        }

        // Both reroute pins are Generic, so `planned_output_is_compatible` short-circuits and the
        // two spliced edges validate against ANY contract. Type-check the parameter against the
        // declared return pin here, which is what the (unrepresentable) direct edge did.
        let boundary_source = ValueSource {
            node: layer.clone(),
            output_pin: Some(param_pin.to_string()),
        };
        let Some(output) = self.planned_source_output_type(&boundary_source) else {
            self.result.diagnostics.push(format!(
                "return of parameter `{param_pin}` as `{}` in function `{function_name}` has no exact source output contract; skipped connection",
                return_param.name
            ));
            return None;
        };
        if !planned_output_is_compatible(return_param, &output, &self.existing.refs) {
            self.result.diagnostics.push(format!(
                "return of parameter `{param_pin}` for `{}` in function `{function_name}` has incompatible pin types or schemas: the parameter is `{}/{}`, but the return pin requires `{}/{}`; use a catalog-declared conversion before returning it",
                return_param.name,
                output.data_type,
                output.value_type,
                return_param.data_type,
                return_param.value_type
            ));
            return None;
        }

        let key = (layer.node_ref(), param_pin.to_string());
        if let Some(source) = self.planned_boundary_passthroughs.get(&key) {
            return Some(source.clone());
        }

        let meta = self.resolve_variable_node("reroute", "Reroute")?;
        let route_in = metadata_input_pin(&meta, "route_in")?.clone();
        let route_out_name = metadata_output_pin(&meta, "route_out")?.name.clone();

        let entity = self.queue_add_node(meta, Some(layer.node_ref()));
        if !self.queue_validated_data_connection(
            &boundary_source,
            param_pin.to_string(),
            &entity,
            &route_in,
            &route_in.name,
            "Connect FlowScript function parameter pass-through".to_string(),
            &format!(
                "parameter `{param_pin}` returned as `{}` in function `{function_name}`",
                return_param.name
            ),
            false,
        ) {
            // The incoming half was rejected (and diagnosed); queueing only the outgoing half
            // would leave a reroute wired to nothing and the return pin reading Null.
            return None;
        }

        let source = ValueSource {
            node: entity,
            output_pin: Some(route_out_name),
        };
        self.planned_boundary_passthroughs
            .insert(key, source.clone());
        Some(source)
    }

    fn plan_event_return(
        &mut self,
        values: &[Expr],
        anchor: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<PlannedStmt> {
        if values.len() > 1 {
            self.result.diagnostics.push(format!(
                "event returns accept a single value; got {} — wrap the extra values in a struct and return that",
                values.len()
            ));
        }
        let entity = match anchor {
            Some(anchor) if find_board_node(self.existing, anchor).is_some() => {
                NodeEntity::Existing(anchor.to_string())
            }
            _ => {
                let meta =
                    self.resolve_variable_node(EVENT_RETURN_RESULT_TYPE, "returnGenericResult")?;
                self.queue_add_node(meta, target_layer.clone())
            }
        };

        if let Some(value) = values.first() {
            self.wire_return_response(&entity, value, target_layer);
        }
        Some(PlannedStmt::new(entity))
    }

    /// Wire a return value into an `events_generic_return_result` node's `response` input (literal
    /// set or data connection), reusing existing wiring on an unchanged roundtrip.
    fn wire_return_response(
        &mut self,
        entity: &NodeEntity,
        value: &Expr,
        target_layer: Option<String>,
    ) {
        let meta = match entity {
            NodeEntity::Existing(id) => find_board_node(self.existing, id).map(node_to_metadata),
            NodeEntity::New { meta, .. } => Some(meta.clone()),
            NodeEntity::Layer { .. } => None,
        };
        let Some(meta) = meta else { return };
        let Some(input) = metadata_input_pin(&meta, EVENT_RESPONSE_PIN) else {
            return;
        };

        if let Some(mut literal) = literal_expr_to_value(value) {
            self.normalize_input_value(input, &mut literal);
            self.queue_update_input(entity, input, literal, &meta);
            return;
        }

        let Some(source) = self
            .resolve_expr_for_argument(value, entity, &input.name, target_layer.clone())
            .and_then(|symbol| self.symbol_to_source(symbol, target_layer))
        else {
            self.result
                .diagnostics
                .push("return value is not a resolvable FlowScript value".to_string());
            return;
        };
        let Some(output_pin) = self.resolve_source_output_pin_for_input(&source, input) else {
            return;
        };
        self.queue_validated_data_connection(
            &source,
            output_pin,
            entity,
            input,
            &input.name,
            "Connect FlowScript return value".to_string(),
            "event return value",
            false,
        );
    }

    fn plan_call_statement(
        &mut self,
        call: &Call,
        anchor: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<NodeEntity> {
        self.plan_call_statement_with_sources(call, anchor, target_layer)
            .map(|(entity, _)| entity)
    }

    fn plan_call_statement_with_sources(
        &mut self,
        call: &Call,
        anchor: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<(NodeEntity, Vec<ValueSource>)> {
        if let Some(anchor) = anchor {
            let Some(node) = find_board_node(self.existing, anchor) else {
                self.result.diagnostics.push(format!(
                    "call `{}` anchors to `{anchor}`, which no longer exists on the board",
                    call.display
                ));
                return None;
            };
            // Validate anchored calls against their live instance pins/defaults while retaining
            // any explicit catalog requirements (including repeated names). Dynamic instance
            // pins exist only in the live metadata, so neither source can safely replace the
            // other wholesale.
            let mut meta = node_to_metadata(node);
            self.merge_catalog_required_inputs(&mut meta);
            let planned_function_target = self
                .planned_functions
                .get(&call.display)
                .map(|planned| planned.entity.node_ref());
            let expected_node_type = if planned_function_target.is_some() {
                CALL_FUNCTION_NODE_TYPE
            } else {
                call.node_type.as_str()
            };
            if !expected_node_type.trim().is_empty() && meta.name != expected_node_type {
                self.result.diagnostics.push(format!(
                    "call `{}` declares exact node_type `{expected_node_type}`, but anchor `{anchor}` resolves to `{}`",
                    call.display, meta.name
                ));
                return None;
            }
            if expected_node_type.trim().is_empty()
                && meta.name != CALL_REFERENCE_NODE_TYPE
                && !(is_placeholder_call(call) && meta.name == "control_branch")
                && !call_matches_node(call, node)
            {
                self.result.diagnostics.push(format!(
                    "call `{}` keeps anchor `{anchor}`, but that anchor identifies `{}`; remove the anchor to replace the node type",
                    call.display, meta.name
                ));
                return None;
            }
            if let Some(expected_layer) = planned_function_target
                && node_pin_literal_string(node, FUNCTION_LAYER_ID_PIN).as_deref()
                    != Some(expected_layer.as_str())
            {
                self.result.diagnostics.push(format!(
                    "function call `{}` keeps anchor `{anchor}`, but that call targets a different Function layer; remove the anchor to retarget it",
                    call.display
                ));
                return None;
            }
            let entity = NodeEntity::Existing(anchor.to_string());
            let input_sources = self.plan_call_arguments(call, &entity, &meta, target_layer, false);
            self.check_required_inputs_after_planning(call, &entity, &meta);
            return Some((entity, input_sources));
        }

        if call.display.trim().is_empty() {
            return None;
        }

        self.add_call_node_with_sources(call, target_layer)
    }

    /// Resolve an event block's entry node. A non-empty `node_type` is an exact catalog identity
    /// supplied by the typed AST and is therefore authoritative; `display` remains the authored
    /// handler alias. Text-parsed FlowScript has no `node_type`, so that legacy surface continues
    /// to resolve by display and then fall back to a Generic/Simple event. A given `event_name`
    /// (`eventsSimple dashboardLoad() { }`) always becomes the entry node's friendly name.
    fn add_entry_node(
        &mut self,
        display: &str,
        node_type: &str,
        event_name: Option<&str>,
        target_layer: Option<String>,
        params: &[Param],
    ) -> Option<NodeEntity> {
        let event_name = event_name.filter(|name| !name.trim().is_empty());
        if !node_type.trim().is_empty() {
            let meta = match self.catalog.resolve_type(node_type) {
                Ok(meta) => meta,
                Err(reason) => {
                    self.result.diagnostics.push(format!(
                        "event `{display}` declares exact node_type `{node_type}`: {reason}"
                    ));
                    return None;
                }
            };
            if let Some(reason) = event_entry_incompatibility(&meta) {
                self.result.diagnostics.push(format!(
                    "event `{display}` declares node_type `{node_type}`, which cannot be used as an event entry: {reason}"
                ));
                return None;
            }
            let friendly_name = event_name
                .map(str::to_string)
                .or_else(|| (display != to_camel_case(&meta.name)).then(|| display.to_string()));
            return Some(self.queue_event_entry_node(meta, target_layer, params, friendly_name));
        }

        match self.catalog.resolve_display(display) {
            Ok(meta) => {
                if let Some(reason) = event_entry_incompatibility(&meta) {
                    self.result.diagnostics.push(format!(
                        "FlowScript handler `{display}` resolved to `{}`, which cannot be used as an event entry: {reason}",
                        meta.name
                    ));
                    return None;
                }
                Some(self.queue_event_entry_node(
                    meta,
                    target_layer,
                    params,
                    event_name.map(str::to_string),
                ))
            }
            Err(err) => {
                let fallbacks = if params.is_empty() {
                    ["eventsSimple", "eventsGeneric"]
                } else {
                    ["eventsGeneric", "eventsSimple"]
                };
                for fallback in fallbacks {
                    if let Ok(meta) = self.catalog.resolve_display(fallback) {
                        if event_entry_incompatibility(&meta).is_some() {
                            continue;
                        }
                        // Preserve an arbitrary handler's authored name on its generic fallback.
                        // The apply planner registers this friendly name as a same-batch alias so
                        // `tools: [fetchPage]` resolves directly to this concrete entry node. A
                        // successful fallback is normal handler lowering, not a diagnostic: all
                        // reconcile diagnostics block atomic FlowScript application.
                        return Some(self.queue_event_entry_node(
                            meta,
                            target_layer,
                            params,
                            Some(event_name.unwrap_or(display).to_string()),
                        ));
                    }
                }
                self.result.diagnostics.push(err);
                None
            }
        }
    }

    /// Add a new event entry, materializing FlowScript-declared custom data outputs only for the
    /// generic event node. Static Simple/Chat interfaces must continue to use their catalog pins.
    fn queue_event_entry_node(
        &mut self,
        mut meta: NodeMetadata,
        target_layer: Option<String>,
        params: &[Param],
        friendly_name: Option<String>,
    ) -> NodeEntity {
        let mut additional_pins = Vec::new();

        for param in params {
            if metadata_output_pin(&meta, &param.name).is_some() {
                continue;
            }

            if meta.name != "events_generic" {
                self.result.diagnostics.push(format!(
                    "could not choose an output pin for event parameter `{}` on `{}`; only a new eventsGeneric entry may declare custom output parameters",
                    param.name, meta.friendly_name
                ));
                continue;
            }

            let pin_meta = param_pin_metadata(param, &self.interface_schemas);
            if pin_meta.data_type == "Execution" {
                self.result.diagnostics.push(format!(
                    "could not choose an output pin for event parameter `{}`: custom Generic Event parameters must be data values, not Execution",
                    param.name
                ));
                continue;
            }

            additional_pins.push(param_output_pin_def(param, &self.interface_schemas));
            meta.outputs.push(pin_meta);
        }

        let entity = self.queue_add_node_with_additional_pins(
            meta,
            target_layer,
            additional_pins,
            friendly_name,
        );
        if let NodeEntity::New { ref_id, .. } = &entity {
            self.event_entry_refs.insert(ref_id.clone());
        }
        entity
    }

    fn add_call_node(&mut self, call: &Call, target_layer: Option<String>) -> Option<NodeEntity> {
        self.add_call_node_with_sources(call, target_layer)
            .map(|(entity, _)| entity)
    }

    fn add_call_node_with_sources(
        &mut self,
        call: &Call,
        target_layer: Option<String>,
    ) -> Option<(NodeEntity, Vec<ValueSource>)> {
        // `functionName(...)` calling a FlowScript `function` declaration: create a
        // `control_call_function` node targeting that function's layer instead of resolving
        // against the catalog.
        if self.planned_functions.contains_key(&call.display) {
            return self.add_function_call_node(call, target_layer);
        }
        let meta = match self.catalog.resolve_call(call) {
            Ok(meta) => meta,
            Err(err) => {
                self.result.diagnostics.push(err);
                return None;
            }
        };
        if let Some(diagnostic) = unsafe_catalog_call_shape_diagnostic(call, &meta) {
            self.result.diagnostics.push(diagnostic);
            return None;
        }
        if let Some(replacement) = safe_catalog_call_alias(&call.display)
            && pin_name_matches(&meta.name, replacement)
            && !pin_name_matches(&meta.name, &call.display)
        {
            self.result.corrections.push(format!(
                "Auto-corrected FlowScript call `{}` to `{replacement}`.",
                call.display
            ));
        }
        // Materialize this call's dynamic (`on_update`-generated) pins so its args resolve against
        // real pins; a no-op when no enricher is supplied (falls back to `synthesize_dynamic_input_pin`).
        let meta = self.enrich_meta(meta, call);
        let entity = self.queue_add_node(meta.clone(), target_layer.clone());

        let input_sources = self.plan_call_arguments(call, &entity, &meta, target_layer, true);
        self.check_required_inputs_after_planning(call, &entity, &meta);

        Some((entity, input_sources))
    }

    /// Merge catalog-declared requirements into live instance metadata as a multiset union. The
    /// live metadata contributes dynamic pins and actual defaults; the catalog contributes
    /// requirements that an older or shape-shifted instance may no longer expose. Taking the
    /// maximum occurrence count avoids doubling the same requirement when both sources contain it.
    ///
    /// The anchored live node is authoritative here, so conflicting same-type declarations (a
    /// board-derived catalog carries one entry per node instance, and instances legitimately
    /// diverge through specialized generics or dynamic pins) never invalidate the call; only the
    /// requirements every candidate agrees on are merged in that case.
    fn merge_catalog_required_inputs(&self, live: &mut NodeMetadata) {
        let Some(matches) = self.catalog.by_type.get(&live.name) else {
            return;
        };
        let catalog_required = match one_catalog_match(&to_camel_case(&live.name), matches) {
            Ok(catalog) => catalog.required_inputs,
            Err(_) => common_required_inputs(matches),
        };

        let canonical_name = |required: &str| {
            live.inputs
                .iter()
                .find(|input| {
                    input.data_type != "Execution" && metadata_pin_name_matches(input, required)
                })
                .map(|input| input.name.clone())
                .unwrap_or_else(|| required.to_string())
        };
        let mut merged = live.required_inputs.clone();
        let mut merged_counts = HashMap::<String, usize>::new();
        for required in &merged {
            *merged_counts.entry(canonical_name(required)).or_default() += 1;
        }

        let mut catalog_counts = HashMap::<String, usize>::new();
        for required in &catalog_required {
            let key = canonical_name(required);
            let catalog_count = catalog_counts.entry(key.clone()).or_default();
            *catalog_count += 1;
            if merged_counts.get(&key).copied().unwrap_or_default() >= *catalog_count {
                continue;
            }
            merged.push(required.clone());
            *merged_counts.entry(key).or_default() += 1;
        }
        live.required_inputs = merged;
    }

    /// Catalog metadata carries the data inputs that must be configured for a node to be usable.
    /// Argument planning is the source of truth here: a required pin is satisfied only when it
    /// retained a catalog default or planning actually queued a literal/update or incoming edge.
    /// Pair duplicate pin names positionally so one `value:` argument cannot accidentally satisfy
    /// every same-named required input.
    fn check_required_inputs_after_planning(
        &mut self,
        call: &Call,
        entity: &NodeEntity,
        meta: &NodeMetadata,
    ) {
        if meta.required_inputs.is_empty() {
            return;
        }

        let node_ref = entity.node_ref();
        // Anchored statement literals are deliberately applied by the legacy outer diff, so the
        // structural planner does not queue a duplicate UpdateNodePin for them. Reconstruct their
        // exact positional pin refs here so they still satisfy required-input validation.
        let mut authored_literal_inputs = HashSet::new();
        let mut same_name_seen: HashMap<&str, usize> = HashMap::new();
        for (arg_index, arg) in call.args.iter().enumerate() {
            let occurrence = {
                let seen = same_name_seen.entry(arg.name.as_str()).or_insert(0);
                let current = *seen;
                *seen += 1;
                current
            };
            if literal_expr_to_value(&arg.value).is_none() {
                continue;
            }
            let pin_ref = match entity {
                // Mirror the legacy anchored diff's populated-first pairing, then translate the
                // concrete live pin back to its stable positional ref for required matching.
                NodeEntity::Existing(node_id) => {
                    find_board_node(self.existing, node_id).and_then(|node| {
                        let direct = matching_input_pins(node, &arg.name);
                        if let Some(pin) = direct.get(occurrence).copied() {
                            return Some(node_input_occurrence_ref(node, pin));
                        }
                        let (name, alias_occurrence) =
                            input_arg_alias_target(&meta.name, call, arg_index)?;
                        matching_input_pins(node, name)
                            .get(alias_occurrence)
                            .copied()
                            .map(|pin| node_input_occurrence_ref(node, pin))
                    })
                }
                _ => {
                    if let Some(input) = metadata_input_pin_at(meta, &arg.name, occurrence) {
                        Some(metadata_input_command_ref(meta, input, occurrence))
                    } else {
                        input_arg_alias_target(&meta.name, call, arg_index).and_then(
                            |(name, alias_occurrence)| {
                                metadata_input_pin_at(meta, name, alias_occurrence).map(|input| {
                                    metadata_input_command_ref(meta, input, alias_occurrence)
                                })
                            },
                        )
                    }
                }
            };
            if let Some(pin_ref) = pin_ref {
                authored_literal_inputs.insert(pin_ref);
            }
        }

        let mut claimed_inputs = HashSet::new();
        let mut missing = Vec::new();
        let mut required_seen: HashMap<&str, usize> = HashMap::new();

        for required in &meta.required_inputs {
            let required_occurrence = {
                let occurrence = required_seen.entry(required.as_str()).or_insert(0);
                let current = *occurrence;
                *occurrence += 1;
                current
            };
            // `required_inputs` is name-based. Prefer a no-default matching pin because catalog
            // enrichment derives this list from precisely those pins; the fallback preserves
            // manually-authored metadata where a required pin also has a usable default.
            let matching_indices = meta
                .inputs
                .iter()
                .enumerate()
                .filter(|(index, input)| {
                    !claimed_inputs.contains(index)
                        && input.data_type != "Execution"
                        && metadata_pin_name_matches(input, required)
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let input_index = matching_indices
                .iter()
                .copied()
                .find(|index| meta.inputs[*index].default_value.is_none())
                .or_else(|| matching_indices.first().copied());

            let Some(input_index) = input_index else {
                let label = if meta
                    .required_inputs
                    .iter()
                    .filter(|name| name.as_str() == required.as_str())
                    .count()
                    > 1
                {
                    pin_occurrence_ref(required, required_occurrence)
                } else {
                    required.clone()
                };
                missing.push(label);
                continue;
            };
            claimed_inputs.insert(input_index);

            let input = &meta.inputs[input_index];
            if input.default_value.is_some() {
                continue;
            }
            let occurrence = meta.inputs[..input_index]
                .iter()
                .filter(|candidate| {
                    candidate.data_type != "Execution"
                        && metadata_pin_name_matches(candidate, &input.name)
                })
                .count();
            let pin_ref = metadata_input_command_ref(meta, input, occurrence);
            let has_literal_or_value = authored_literal_inputs.contains(&pin_ref)
                || self.update_commands.iter().any(|command| {
                    matches!(
                        command,
                        BoardCommand::UpdateNodePin { node_id, pin_id, .. }
                            if node_id == &node_ref && pin_id == &pin_ref
                    )
                });
            let has_planned_connection = self.connect_commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { to_node, to_pin, .. }
                        if to_node == &node_ref && to_pin == &pin_ref
                )
            });
            // An unchanged anchored edge is intentionally not re-queued. Inspect the exact live
            // pin occurrence rather than `existing_source_for_input`'s name-only lookup, which
            // would let one connected repeated pin satisfy all of its siblings.
            let has_retained_existing_connection = if let NodeEntity::Existing(node_id) = entity {
                find_board_node(self.existing, node_id)
                    .and_then(|node| find_input_pin_by_ref(node, &pin_ref))
                    .is_some_and(|pin| {
                        if pin.depends_on.is_empty() {
                            return false;
                        }
                        !self.disconnect_commands.iter().any(|command| {
                            matches!(
                                command,
                                BoardCommand::DisconnectPins { to_node, to_pin, .. }
                                    if to_node == node_id
                                        && (to_pin == &pin_ref
                                            || to_pin == &pin.id
                                            || to_pin == &pin.name)
                            )
                        })
                    })
            } else {
                false
            };
            // A required pin that was ALREADY unset on the live anchored node (no default, no
            // incoming edge) is the board's status quo; re-anchoring the same call — or editing
            // an unrelated statement — must not start failing it. New nodes and pins this batch
            // actively unwires keep full enforcement.
            let grandfathered_unset = if let NodeEntity::Existing(node_id) = entity {
                find_board_node(self.existing, node_id)
                    .and_then(|node| find_input_pin_by_ref(node, &pin_ref))
                    .is_some_and(|pin| pin.depends_on.is_empty() && pin.default_value.is_none())
            } else {
                false
            };
            if !has_literal_or_value
                && !has_planned_connection
                && !has_retained_existing_connection
                && !grandfathered_unset
            {
                missing.push(pin_ref);
            }
        }

        if !missing.is_empty() {
            self.result.diagnostics.push(format!(
                "node `{}` is missing required inputs: {}",
                call.display,
                missing.join(", ")
            ));
        }
    }

    /// Create the `control_call_function` node for a call to a FlowScript `function` declaration.
    ///
    /// The synthetic metadata mirrors what the node's `on_update` will mint once its
    /// `function_layer_id` pin is set (the layer's params as inputs, returns as outputs, exec
    /// boundary pins when the function is impure), so argument and execution wiring plan against
    /// the pins that will exist at apply time.
    fn add_function_call_node(
        &mut self,
        call: &Call,
        target_layer: Option<String>,
    ) -> Option<(NodeEntity, Vec<ValueSource>)> {
        let planned = self.planned_functions.get(&call.display).cloned()?;
        let base = match self.catalog.resolve_type(CALL_FUNCTION_NODE_TYPE) {
            Ok(meta) => meta,
            Err(reason) => {
                self.result.diagnostics.push(format!(
                    "cannot call function `{}`: `{CALL_FUNCTION_NODE_TYPE}` {reason}",
                    call.display
                ));
                return None;
            }
        };

        let mut meta = base;
        meta.friendly_name = format!("Call {}", call.display);
        // The call node's `on_update` mirrors the layer's ACTUAL boundary pins, so for an
        // existing layer trust its pins over the prescan (legacy function layers may predate
        // exec boundary pins).
        let impure = match &planned.entity {
            NodeEntity::Existing(id) => self
                .existing
                .layers
                .get(id)
                .map(|layer| {
                    layer.pins.values().any(|pin| {
                        pin.pin_type == PinType::Input && pin.data_type == VariableType::Execution
                    })
                })
                .unwrap_or(planned.impure),
            _ => planned.impure,
        };
        if impure {
            meta.inputs
                .push(execution_pin_metadata(FUNCTION_EXEC_IN, "Exec In"));
            meta.outputs
                .push(execution_pin_metadata(FUNCTION_EXEC_OUT, "Exec Out"));
        }
        for param in &planned.params {
            meta.inputs.push(param.clone());
        }
        for ret in &planned.returns {
            meta.outputs.push(ret.clone());
        }

        let entity = self.queue_add_node(meta.clone(), target_layer.clone());
        if let Some(input) = metadata_input_pin(&meta, FUNCTION_LAYER_ID_PIN) {
            // For a new layer this is its `$n` ref; the applier resolves it to the real layer id.
            self.queue_update_input(
                &entity,
                input,
                flow_like_types::Value::String(planned.entity.node_ref()),
                &meta,
            );
        }

        let input_sources = self.plan_call_arguments(call, &entity, &meta, target_layer, true);
        Some((entity, input_sources))
    }

    fn plan_call_arguments(
        &mut self,
        call: &Call,
        entity: &NodeEntity,
        meta: &NodeMetadata,
        target_layer: Option<String>,
        include_direct_literals: bool,
    ) -> Vec<ValueSource> {
        let mut input_sources = Vec::new();
        let mut same_name_seen: HashMap<&str, usize> = HashMap::new();
        for (arg_index, arg) in call.args.iter().enumerate() {
            // Multi-pins (several inputs sharing one name) pair positionally with the
            // same-named args, mirroring the order lowering emitted them in.
            let occurrence = {
                let counter = same_name_seen.entry(arg.name.as_str()).or_insert(0);
                let index = *counter;
                *counter += 1;
                index
            };
            // Pins minted by a node's `on_update` (for example each `{placeholder}` of a
            // `string_format` node) are absent from the STATIC catalog metadata used to plan a NEW
            // node, so they must be predicted from the call's own config args. `synthesized_pin`
            // backs the `&PinMetadata` borrow for such a predicted dynamic pin.
            let synthesized_pin;
            let (input, target_occurrence) = match metadata_input_pin_at(
                meta, &arg.name, occurrence,
            ) {
                Some(pin) => (pin, occurrence),
                None => {
                    if let Some((name, alias_occurrence)) =
                        input_arg_alias_target(&meta.name, call, arg_index)
                        && let Some(pin) = metadata_input_pin_at(meta, name, alias_occurrence)
                    {
                        let matching_pin_count = meta
                            .inputs
                            .iter()
                            .filter(|candidate| {
                                candidate.data_type != "Execution"
                                    && metadata_pin_name_matches(candidate, &pin.name)
                            })
                            .count();
                        self.result.corrections.push(input_arg_alias_correction(
                            call,
                            arg,
                            &pin.name,
                            alias_occurrence,
                            matching_pin_count,
                        ));
                        (pin, alias_occurrence)
                    } else {
                        // `tools:`/`fnRefs:` carry a node's function references, not pin values —
                        // they are not board pins. For a NEWLY added node, materialize them as a
                        // `SetNodeFunctionRefs` command (the applier resolves each named target once
                        // the referenced functions/events exist). For an EXISTING node, leave its
                        // references untouched: an unchanged document round-trips to a clean no-op,
                        // and rewriting them would need exact ref→entry-node resolution against the
                        // live board.
                        if is_synthetic_fn_ref_arg(arg) {
                            if let NodeEntity::New { .. } = entity {
                                let refs = synthetic_fn_ref_targets(arg);
                                if !refs.is_empty() {
                                    self.fn_ref_commands
                                        .push(BoardCommand::SetNodeFunctionRefs {
                                            node_id: entity.node_ref(),
                                            fn_refs: refs,
                                            summary: Some(format!(
                                                "Register {} on {}",
                                                arg.name, meta.friendly_name
                                            )),
                                        });
                                }
                            }
                            continue;
                        }
                        // A dynamic pin the node's `on_update` will add from its config (for example
                        // a `string_format` placeholder that appears in the format string):
                        // synthesize a permissive Generic pin so its value/connection is still
                        // planned. Apply sets the config pin first, runs `on_update`, then resolves
                        // the wire against the now-live pin (see `plan()`'s
                        // update-before-connect ordering).
                        match synthesize_dynamic_input_pin(meta, call, arg, entity, self.existing) {
                            Some(pin) => {
                                synthesized_pin = pin;
                                (&synthesized_pin, occurrence)
                            }
                            None if is_widget_dynamic_binding_arg(&arg.name) => {
                                // A widget binding on a node that derives its pins from a
                                // connected `element_ref` rather than from a literal. The
                                // generic wording sends the model hunting for a typo; the
                                // actual cause is that these pins only exist once the
                                // widget is persisted and the instance is wired up.
                                self.result.diagnostics.push(format!(
                                        "node `{}` has no input pin named `{}`. Widget binding pins come from the persisted widget, and on this node they appear only once its `elementRef` input is connected to a live instance. Set the inputs on `a2uiInstantiateWidget` itself — every connection in a revision is applied after every pin write, so a later call in the same revision cannot see them either. `ui_inspect` with operation `widget` lists a widget's exact pin names. No part of this revision was applied.",
                                        call.display, arg.name
                                    ));
                                continue;
                            }
                            None => {
                                self.result
                                    .diagnostics
                                    .push(missing_input_pin_diagnostic(meta, call, arg));
                                continue;
                            }
                        }
                    }
                }
            };
            // Catalog pin ids are randomized by AddNodeCommand, so same-named inputs on a NEW
            // node cannot be addressed by their catalog ids. Carry their stable occurrence in
            // the command pin ref; apply resolves it after the node exists.
            let input_command_ref = metadata_input_command_ref(meta, input, target_occurrence);

            if let Some(mut value) = literal_expr_to_value(&arg.value) {
                if self.reuse_existing_composite_literal_source(
                    &arg.value,
                    entity,
                    &input_command_ref,
                    target_layer.clone(),
                ) {
                    continue;
                }
                self.normalize_input_value(input, &mut value);
                if include_direct_literals {
                    self.queue_update_input_at(entity, input, &input_command_ref, value, meta);
                }
                continue;
            }

            let Some(source) = self.resolve_expr_for_argument(
                &arg.value,
                entity,
                &input_command_ref,
                target_layer.clone(),
            ) else {
                // Expression forms v1 cannot resolve (ternaries, binaries, event returns) but
                // whose input is already wired on the board: keep the existing wiring silently —
                // the rendered text CAME from that wiring, so this is not an authoring error.
                if let Some(existing_source) =
                    self.existing_source_for_input_ref(entity, &input_command_ref)
                {
                    input_sources.push(existing_source);
                    continue;
                }
                self.result.diagnostics.push(format!(
                    "argument `{}` on `{}` is not a literal or resolvable node output; skipped connection",
                    arg.name, call.display
                ));
                continue;
            };
            let source = match source {
                SymbolValue::Literal(mut value) => {
                    self.normalize_input_value(input, &mut value);
                    self.queue_update_input_at(entity, input, &input_command_ref, value, meta);
                    continue;
                }
                SymbolValue::VariableRef { variable_id }
                    if is_variable_ref_pin_name(&input.name) =>
                {
                    self.queue_update_input_at(
                        entity,
                        input,
                        &input_command_ref,
                        flow_like_types::Value::String(variable_id),
                        meta,
                    );
                    continue;
                }
                SymbolValue::Source(source) => source,
                SymbolValue::VariableRef { variable_id } => {
                    let Some(source) =
                        self.add_variable_get_source(&variable_id, target_layer.clone())
                    else {
                        self.result.diagnostics.push(format!(
                            "could not create variable read for argument `{}` on `{}`",
                            arg.name, call.display
                        ));
                        continue;
                    };
                    source
                }
            };
            // Same reason as in `queue_validated_data_connection`: selecting the source's output
            // by the dynamic pin's CURRENT specialization hides every output of a differently
            // typed replacement, and reports it as a node with no usable output.
            let rewirable_target = self.rewirable_dynamic_input(entity, input);
            let selector_input = rewirable_target.as_ref().unwrap_or(input);
            let Some(output_pin) =
                self.resolve_source_output_pin_for_input(&source, selector_input)
            else {
                self.result.diagnostics.push(format!(
                    "could not choose an output pin for argument `{}` on `{}`",
                    arg.name, call.display
                ));
                continue;
            };
            // An explicit `structGet`/`structSet` never reaches the member-access check, so the same
            // closed-struct rejection has to guard the literal-call spelling of it.
            if let Some((title, hint)) =
                self.closed_struct_field_call_rejection(call, meta, input, &source, &output_pin)
            {
                let selected = self.literal_struct_field_argument(call).unwrap_or_default();
                self.result.diagnostics.push(format!(
                    "`{}` reads field `{selected}` from a `{title}`, which has no such field; {hint}",
                    call.display
                ));
                continue;
            }
            if !self.queue_validated_data_connection(
                &source,
                output_pin.clone(),
                entity,
                input,
                &input_command_ref,
                format!("Connect {} into {}", arg.name, meta.friendly_name),
                &format!("argument `{}` on `{}`", arg.name, call.display),
                false,
            ) {
                continue;
            }
            input_sources.push(ValueSource {
                node: source.node,
                output_pin: Some(output_pin),
            });
        }
        input_sources
    }

    fn resolve_expr_for_argument(
        &mut self,
        expr: &Expr,
        target_entity: &NodeEntity,
        input_command_ref: &str,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        for source in self.existing_sources_for_input_ref(target_entity, input_command_ref) {
            if let Some(symbol) =
                self.resolve_expr_using_existing_source(expr, source, target_layer.clone())
            {
                return Some(symbol);
            }
        }

        self.resolve_expr(expr, target_layer)
    }

    fn existing_source_for_input_ref(
        &self,
        target_entity: &NodeEntity,
        input_ref: &str,
    ) -> Option<ValueSource> {
        self.existing_sources_for_input_ref(target_entity, input_ref)
            .into_iter()
            .next()
    }

    fn existing_sources_for_input_ref(
        &self,
        target_entity: &NodeEntity,
        input_ref: &str,
    ) -> Vec<ValueSource> {
        let NodeEntity::Existing(node_id) = target_entity else {
            return Vec::new();
        };
        let input = if let Some(node) = find_board_node(self.existing, node_id) {
            find_input_pin_by_ref(node, input_ref)
        } else {
            self.existing
                .layers
                .get(node_id)
                .and_then(|layer| find_boundary_pin_by_ref(&layer.pins, input_ref, PinType::Output))
        };
        input
            .map(|input| self.board_index.data_sources_for_pin(input))
            .unwrap_or_default()
    }

    fn existing_target_contract_matches(
        &self,
        target_entity: &NodeEntity,
        input_ref: &str,
        authored: &PinMetadata,
    ) -> bool {
        let NodeEntity::Existing(node_id) = target_entity else {
            return false;
        };
        let node = find_board_node(self.existing, node_id);
        let live = if let Some(node) = node {
            find_input_pin_by_ref(node, input_ref)
        } else {
            self.existing
                .layers
                .get(node_id)
                .and_then(|layer| find_boundary_pin_by_ref(&layer.pins, input_ref, PinType::Output))
        };
        // A `variable_set` value pin's live shape is a representation detail: it may be an
        // unspecialized Generic or carry the specialization of whatever source fed it (with a
        // schema that need not equal the variable's normalized one). As with `variable_get`
        // sources, only an authored variable contract change in this revision invalidates
        // grandfathering the already-wired edge.
        if let (Some(node), Some(live_pin)) = (node, live)
            && node.name == "variable_set"
            && matches!(live_pin.name.as_str(), "value_in" | "new_value" | "value")
            && let Some(variable_id) = node_pin_literal_string(node, "var_ref")
        {
            return !self.variable_contract_changed_from_board(&variable_id);
        }
        live.map(boundary_pin_metadata).is_some_and(|live| {
            live.data_type == authored.data_type
                && live.value_type == authored.value_type
                && live.is_generic == (authored.is_generic || authored.data_type == "Generic")
                && live.enforce_schema == authored.enforce_schema
                && reconcile_schema_contract_eq_with_refs(
                    live.schema.as_deref(),
                    authored.schema.as_deref(),
                    &self.existing.refs,
                )
        })
    }

    fn existing_source_contract_matches(&self, source: &ValueSource, output_pin: &str) -> bool {
        let NodeEntity::Existing(node_id) = &source.node else {
            return false;
        };
        let live = if let Some(node) = find_board_node(self.existing, node_id) {
            if node.name == "variable_get"
                && output_pin == "value_ref"
                && let Some(variable_id) = node_pin_literal_string(node, "var_ref")
            {
                // Generic variable nodes may be intentionally unspecialized on the live board.
                // What invalidates grandfathering is an authored variable contract change in
                // this revision, not that legacy representation detail.
                return !self.variable_contract_changed_from_board(&variable_id);
            }
            find_output_pin(node, output_pin)
        } else {
            self.existing
                .layers
                .get(node_id)
                .and_then(|layer| find_boundary_pin_by_ref(&layer.pins, output_pin, PinType::Input))
        };
        let Some(live) = live.map(boundary_pin_metadata) else {
            return false;
        };
        let effective_source = ValueSource {
            node: source.node.clone(),
            output_pin: Some(output_pin.to_string()),
        };
        let Some(effective) = self.planned_source_output_type(&effective_source) else {
            return false;
        };
        live.data_type == effective.data_type
            && live.value_type == effective.value_type
            && live.is_generic == effective.is_generic
            && live.enforce_schema == effective.enforce_schema
            && reconcile_schema_contract_eq_with_refs(
                live.schema.as_deref(),
                effective.schema.as_deref(),
                &self.existing.refs,
            )
    }

    fn variable_contract_changed_from_board(&self, variable_id: &str) -> bool {
        let Some(authored) = self.variable_value_contracts.get(variable_id) else {
            return false;
        };
        let Some(existing) = self.existing.variables.get(variable_id).or_else(|| {
            self.existing
                .layers
                .values()
                .find_map(|layer| layer.variables.get(variable_id))
        }) else {
            return true;
        };
        if authored.data_type != format!("{:?}", existing.data_type)
            || authored.value_type != format!("{:?}", existing.value_type)
        {
            return true;
        }
        // The text surface cannot carry every live schema: non-representable schemas render as a
        // bare type (authored `None` keeps the stored schema), and representable ones render as an
        // interface whose generated schema is a lossy projection. Only an authored schema that
        // matches neither the live schema nor its representable projection is a contract change.
        let Some(authored_schema) = authored.schema.as_deref() else {
            return false;
        };
        if reconcile_schema_contract_eq_with_refs(
            Some(authored_schema),
            existing.schema.as_deref(),
            &self.existing.refs,
        ) {
            return false;
        }
        let existing_expanded = existing.schema.as_deref().map(|schema| {
            self.existing
                .refs
                .get(schema)
                .map(String::as_str)
                .unwrap_or(schema)
        });
        !existing_expanded.is_some_and(|existing_schema| {
            // The authored schema already went through the render→parse text surface; compare
            // it against the live schema's projection through that SAME surface.
            text_projected_schema(existing_schema).is_some_and(|projection| {
                reconcile_schema_contract_eq(Some(&projection), Some(authored_schema))
            })
        })
    }

    /// Queue one newly authored DATA edge only after resolving both endpoints to concrete live or
    /// planned contracts. Exact pre-existing endpoints are retained without re-validating legacy
    /// wiring; every changed endpoint must pass type, container, and schema checks.
    /// The `Generic` shape a wire-typed dynamic pin is actually minted with, when `input` is one.
    ///
    /// A pin minted by [`respecializes_dynamic_pins_from_their_source`] stores the contract of the
    /// source currently feeding it, because `on_update` re-runs `match_type` on every board pass.
    /// Validating a REPLACEMENT source against that stored shape rejects every rewire — and makes
    /// the output-pin picker report that the new source node has no usable output at all — even
    /// though the pin re-specializes to the new source the moment the edge lands.
    fn rewirable_dynamic_input(
        &self,
        target_entity: &NodeEntity,
        input: &PinMetadata,
    ) -> Option<PinMetadata> {
        if input.is_generic || input.data_type == "Generic" {
            return None;
        }
        let node_type = match target_entity {
            NodeEntity::Existing(node_id) => find_board_node(self.existing, node_id)?.name.as_str(),
            NodeEntity::New { meta, .. } => meta.name.as_str(),
            NodeEntity::Layer { .. } => return None,
        };
        minted_wire_typed_pin(node_type, &input.name)
            .then(|| generic_input_pin_metadata(&input.name))
    }

    #[allow(clippy::too_many_arguments)]
    fn queue_validated_data_connection(
        &mut self,
        source: &ValueSource,
        output_pin: String,
        target_entity: &NodeEntity,
        input: &PinMetadata,
        input_command_ref: &str,
        summary: String,
        context: &str,
        variable_schema_contract: bool,
    ) -> bool {
        let already_wired = self
            .existing_sources_for_input_ref(target_entity, input_command_ref)
            .into_iter()
            .any(|existing| {
                existing.node.node_ref() == source.node.node_ref()
                    && existing.output_pin.as_deref() == Some(output_pin.as_str())
            });
        if already_wired
            && self.existing_target_contract_matches(target_entity, input_command_ref, input)
            && self.existing_source_contract_matches(source, &output_pin)
        {
            return true;
        }

        // The identical edge may already be planned this run (e.g. two branch arms returning a
        // literal through one shared materialized getter); connecting it twice is meaningless.
        let already_planned = self.connect_commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == &source.node.node_ref()
                        && from_pin == &output_pin
                        && to_node == &target_entity.node_ref()
                        && to_pin == input_command_ref
            )
        });
        if already_planned {
            return true;
        }

        // Past the already-wired guards, `input` is only used to validate a NEW edge. A dynamic
        // pin's stored type is a copy of the source it holds today, so check the replacement
        // against the `Generic` pin the node really mints instead.
        let rewirable = self.rewirable_dynamic_input(target_entity, input);
        let input = rewirable.as_ref().unwrap_or(input);

        let resolved_source = ValueSource {
            node: source.node.clone(),
            output_pin: Some(output_pin.clone()),
        };
        let Some(output) = self.planned_source_output_type(&resolved_source) else {
            self.result.diagnostics.push(format!(
                "{context} has no exact source output contract for `{}.{output_pin}`; skipped connection",
                source.node.node_ref()
            ));
            return false;
        };
        let compatible = planned_output_is_compatible(input, &output, &self.existing.refs)
            && (!variable_schema_contract
                || variable_assignment_schemas_are_compatible(input, &output, &self.existing.refs));
        if !compatible {
            self.result.diagnostics.push(format!(
                "{context} has incompatible pin types or schemas: source `{}` is `{}/{}`, but input `{}` requires `{}/{}`; use a catalog-declared conversion before connecting it",
                output.source,
                output.data_type,
                output.value_type,
                input.name,
                input.data_type,
                input.value_type
            ));
            return false;
        }

        self.connect_commands.push(BoardCommand::ConnectPins {
            from_node: source.node.node_ref(),
            from_pin: output_pin,
            to_node: target_entity.node_ref(),
            to_pin: input_command_ref.to_string(),
            summary: Some(summary),
        });
        true
    }

    fn resolve_expr_using_existing_source(
        &mut self,
        expr: &Expr,
        source: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        match expr {
            Expr::Call(call) => self.reuse_existing_call_source(call, source, None, target_layer),
            Expr::Field { base, pin } => {
                // `<base>.field` is ambiguous in text: it selects an output pin when the base
                // node has one of that name, but the SAME spelling is how a collapsed
                // struct_get/struct_break field read renders. Prefer whatever shape the wired
                // source actually is: the base call itself, or an accessor node for this field.
                if let Expr::Call(call) = base.as_ref() {
                    self.reuse_existing_call_source(
                        call,
                        source.clone(),
                        Some(pin),
                        target_layer.clone(),
                    )
                    .or_else(|| self.reuse_existing_member_source(base, pin, source, target_layer))
                } else {
                    self.reuse_existing_member_source(base, pin, source, target_layer)
                }
            }
            // Sugared data sources: these text forms lower FROM a specific node shape, and
            // when the consumer's existing source IS that node, it must be reused. Falling
            // back to `resolve_expr` would materialize a duplicate node on every apply, and
            // resolving only the base would silently drop the field/index selection.
            Expr::Ref(name) => self.reuse_existing_variable_get(name, source),
            Expr::Member { base, field } => {
                self.reuse_existing_member_source(base, field, source, target_layer)
            }
            Expr::Index { base, index } => {
                self.reuse_existing_index_source(base, index, source, target_layer)
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => self.reuse_existing_select_source(cond, then, otherwise, source, target_layer),
            Expr::Binary { op, lhs, rhs } => {
                self.reuse_existing_binary_source(op, lhs, rhs, source, target_layer)
            }
            Expr::Object(fields) => {
                self.reuse_existing_struct_make_source(fields, source, target_layer)
            }
            // `[]` is how an existing wired `make_array` renders; keep that source.
            Expr::Array(items) if items.is_empty() => {
                let NodeEntity::Existing(node_id) = &source.node else {
                    return None;
                };
                let node = find_board_node(self.existing, node_id)?;
                (node.name == "make_array").then_some(SymbolValue::Source(source))
            }
            _ => None,
        }
    }

    /// A fully-literal `{...}`/`[]` argument may be the text rendering of a WIRED
    /// `struct_make`/`struct_make_from_schema`/`make_array` source rather than a pin default.
    /// Reuse that source (diffing its fields in place) instead of shadowing the edge with a
    /// literal pin write on every roundtrip. Returns whether an existing source was reused.
    fn reuse_existing_composite_literal_source(
        &mut self,
        expr: &Expr,
        entity: &NodeEntity,
        input_command_ref: &str,
        target_layer: Option<String>,
    ) -> bool {
        // `{}`/`[]` (and JSON object literals) parse as `Literal::Json`, not as structural
        // Object/Array expressions; normalize so the struct_make/make_array reuse arms see them.
        let normalized;
        let expr = match expr {
            Expr::Object(_) | Expr::Array(_) => expr,
            _ => match literal_expr_to_value(expr) {
                Some(flow_like_types::Value::Object(map)) => {
                    normalized = Expr::Object(
                        map.iter()
                            .map(|(key, value)| ObjectField {
                                key: key.clone(),
                                value: json_value_literal_expr(value),
                            })
                            .collect(),
                    );
                    &normalized
                }
                Some(flow_like_types::Value::Array(items)) if items.is_empty() => {
                    normalized = Expr::Array(Vec::new());
                    &normalized
                }
                _ => return false,
            },
        };
        self.existing_sources_for_input_ref(entity, input_command_ref)
            .into_iter()
            .any(|source| {
                self.resolve_expr_using_existing_source(expr, source, target_layer.clone())
                    .is_some()
            })
    }

    /// Reuse the existing `struct_make`/`struct_make_from_schema` node an object literal with
    /// non-literal members lowered from, diffing literal fields in place and recursing into
    /// wired ones. Without this the sugar is one-way: `x = { a: node.out }` re-renders from the
    /// live board but can never re-apply against it.
    fn reuse_existing_struct_make_source(
        &mut self,
        fields: &[ObjectField],
        source: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        match node.name.as_str() {
            "struct_make" if fields.is_empty() => Some(SymbolValue::Source(source)),
            "struct_make_from_schema" => {
                let meta = node_to_metadata(node);
                for field in fields {
                    let pin_name = format!("{}{}", super::lower::MAKE_STRUCT_PREFIX, field.key);
                    // A wired field (including a nested struct_make chain rendered as a literal
                    // object) keeps its source; only unwired literal fields become pin writes.
                    if let Some(sub_source) =
                        self.board_index.data_source_for_input(node, &pin_name)
                        && self
                            .resolve_expr_using_existing_source(
                                &field.value,
                                sub_source,
                                target_layer.clone(),
                            )
                            .is_some()
                    {
                        continue;
                    }
                    if let Some(value) = literal_expr_to_value(&field.value)
                        && let Some(pin) = metadata_input_pin(&meta, &pin_name)
                    {
                        let entity = NodeEntity::Existing(node_id.clone());
                        self.queue_update_input(&entity, pin, value, &meta);
                    }
                }
                Some(SymbolValue::Source(source))
            }
            _ => None,
        }
    }

    /// Reuse an existing variable accessor feeding this input when the text ref names the same
    /// variable it reads: a `variable_get`, or a `variable_set` sibling whose `value_ref`
    /// passthrough output already carries the variable's value (lowering renders both as the
    /// bare variable name).
    fn reuse_existing_variable_get(
        &mut self,
        name: &str,
        source: ValueSource,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        if node.name != "variable_get" && node.name != "variable_set" {
            return None;
        }
        let SymbolValue::VariableRef { variable_id } = self.lookup_symbol(name)? else {
            return None;
        };
        let configured = node_pin_literal_string(node, "var_ref")?;
        (configured == variable_id).then_some(SymbolValue::Source(source))
    }

    /// Reuse the existing struct/container accessor node a `base.field` member access lowered
    /// from (`struct_get`, `struct_break`, `map_get`, or a container-size node). The base is
    /// recursed so literal edits deeper in the chain still apply.
    fn reuse_existing_member_source(
        &mut self,
        base: &Expr,
        field: &str,
        source: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        let base_input = match node.name.as_str() {
            "struct_get" if node_pin_literal_string(node, "field").as_deref() == Some(field) => {
                "struct"
            }
            "struct_break"
                if source
                    .output_pin
                    .as_deref()
                    .and_then(|pin| pin.strip_prefix(super::lower::BREAK_STRUCT_PREFIX))
                    == Some(field) =>
            {
                "struct_in"
            }
            "array_length" if field == "length" => "array",
            "set_get_size" if field == "length" => "set_in",
            "map_size" if field == "length" => "map_in",
            "map_get" if node_pin_literal_string(node, "key").as_deref() == Some(field) => "map_in",
            _ => return None,
        };
        if let Some(base_source) = self.board_index.data_source_for_input(node, base_input) {
            let _ = self.resolve_expr_using_existing_source(base, base_source, target_layer);
        }
        Some(SymbolValue::Source(source))
    }

    /// Reuse the existing `array_get` node a `base[index]` access lowered from. A changed
    /// literal index becomes a pin update on the same node.
    fn reuse_existing_index_source(
        &mut self,
        base: &Expr,
        index: &Expr,
        source: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        if node.name != "array_get" || source.output_pin.as_deref() != Some("element") {
            return None;
        }
        if let Some(base_source) = self.board_index.data_source_for_input(node, "array_in") {
            let _ = self.resolve_expr_using_existing_source(base, base_source, target_layer);
        }
        if let Some(value) = literal_expr_to_value(index) {
            let meta = node_to_metadata(node);
            if let Some(pin) = metadata_input_pin(&meta, "index") {
                let entity = NodeEntity::Existing(node_id.clone());
                self.queue_update_input(&entity, pin, value, &meta);
            }
        }
        Some(SymbolValue::Source(source))
    }

    /// Reuse the existing `utils_types_select` node a `cond ? a : b` ternary lowered from,
    /// diffing its literal inputs and recursing into wired ones.
    fn reuse_existing_select_source(
        &mut self,
        cond: &Expr,
        then: &Expr,
        otherwise: &Expr,
        source: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        if node.name != "utils_types_select" {
            return None;
        }
        let meta = node_to_metadata(node);
        for (pin_name, expr) in [("condition", cond), ("a", then), ("b", otherwise)] {
            if let Some(value) = literal_expr_to_value(expr) {
                if let Some(pin) = metadata_input_pin(&meta, pin_name) {
                    let entity = NodeEntity::Existing(node_id.clone());
                    self.queue_update_input(&entity, pin, value, &meta);
                }
            } else if let Some(sub_source) = self.board_index.data_source_for_input(node, pin_name)
            {
                let _ =
                    self.resolve_expr_using_existing_source(expr, sub_source, target_layer.clone());
            }
        }
        Some(SymbolValue::Source(source))
    }

    /// Reuse the pure comparison node that an existing binary expression lowered from. Besides
    /// keeping an unchanged round-trip a no-op, this lets a literal edit such as
    /// `sender == "old"` -> `sender == "new"` update the comparator in place instead of adding a
    /// duplicate node behind the branch.
    fn reuse_existing_binary_source(
        &mut self,
        op: &str,
        lhs: &Expr,
        rhs: &Expr,
        source: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        if binary_operator_op(&node.name) != Some(canonical_binary_op(op)) {
            return None;
        }

        let meta = node_to_metadata(node);
        let inputs = binary_data_inputs(&meta)?;
        let call = binary_operator_call(&meta, &inputs, lhs, rhs);
        let entity = NodeEntity::Existing(node_id.clone());
        self.plan_call_arguments(&call, &entity, &meta, target_layer, true);

        Some(SymbolValue::Source(ValueSource {
            node: entity.clone(),
            output_pin: source
                .output_pin
                .or_else(|| self.resolve_entity_output_pin(&entity, None)),
        }))
    }

    fn reuse_existing_call_source(
        &mut self,
        call: &Call,
        source: ValueSource,
        requested_output: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(source_node_id) = &source.node else {
            return None;
        };
        let source_node = find_board_node(self.existing, source_node_id)?;
        // Always describe the reused node by ITS OWN pins: catalog metadata is a per-type sample
        // and misses instance pins of dynamic nodes (string_format placeholders, added pins).
        // A call to a declared FlowScript function reuses the existing `control_call_function`
        // node that targets that function's layer (catalog resolution can never match it, and
        // recreating it on every reconcile would duplicate call nodes).
        let is_function_call_reuse = source_node.name == CALL_FUNCTION_NODE_TYPE
            && self
                .planned_functions
                .get(&call.display)
                .is_some_and(|planned| {
                    node_pin_literal_string(source_node, FUNCTION_LAYER_ID_PIN)
                        .is_some_and(|layer_id| layer_id == planned.entity.node_ref())
                });
        let meta = if is_function_call_reuse {
            node_to_metadata(source_node)
        } else {
            match self.catalog.resolve_call(call) {
                Ok(meta) if meta.name == source_node.name => node_to_metadata(source_node),
                Ok(_) => return None,
                Err(_) if call_matches_node(call, source_node) => node_to_metadata(source_node),
                Err(_) => return None,
            }
        };
        let entity = NodeEntity::Existing(source_node_id.clone());
        self.plan_call_arguments(call, &entity, &meta, target_layer, true);

        let output_pin = requested_output
            .and_then(|pin| self.resolve_entity_output_pin(&entity, Some(pin)))
            .or(source.output_pin)
            .or_else(|| self.resolve_entity_output_pin(&entity, None));

        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin,
        }))
    }

    fn normalize_input_value(&mut self, input: &PinMetadata, value: &mut flow_like_types::Value) {
        if !is_variable_ref_pin_name(&input.name) {
            return;
        }

        let flow_like_types::Value::String(raw) = value else {
            return;
        };

        if let Some(variable_id) = self.variable_refs.resolve(raw) {
            *raw = variable_id;
            return;
        }

        if self.unresolved_variable_refs.insert(raw.clone()) {
            self.result.diagnostics.push(format!(
                "variable reference `{raw}` does not resolve to a board or FlowScript variable; declare it as a top-level FlowScript variable so it can be created"
            ));
        }
    }

    fn queue_update_input(
        &mut self,
        entity: &NodeEntity,
        input: &PinMetadata,
        value: flow_like_types::Value,
        meta: &NodeMetadata,
    ) {
        self.queue_update_input_at(entity, input, &input.name, value, meta);
    }

    fn queue_update_input_at(
        &mut self,
        entity: &NodeEntity,
        input: &PinMetadata,
        pin_ref: &str,
        value: flow_like_types::Value,
        meta: &NodeMetadata,
    ) {
        if let NodeEntity::Existing(node_id) = entity
            && let Some(node) = find_board_node(self.existing, node_id)
            && let Some(pin) = find_input_pin_by_ref(node, pin_ref)
        {
            let current = pin.default_value.as_deref().and_then(|bytes| {
                flow_like_types::json::from_slice::<flow_like_types::Value>(bytes).ok()
            });
            if current.as_ref() == Some(&value) {
                return;
            }
            // An unset composite pin (no/null default, no edge) reads as its empty value;
            // lowering renders it as `{}`/`[]`, so writing that back is a representation no-op.
            let empty_composite = match &value {
                flow_like_types::Value::Object(map) => map.is_empty(),
                flow_like_types::Value::Array(items) => items.is_empty(),
                _ => false,
            };
            let unset = matches!(current, None | Some(flow_like_types::Value::Null));
            if empty_composite && unset && pin.depends_on.is_empty() {
                return;
            }
        }

        self.update_commands.push(BoardCommand::UpdateNodePin {
            node_id: entity.node_ref(),
            pin_id: pin_ref.to_string(),
            value,
            summary: Some(format!("Set {} on {}", input.name, meta.friendly_name)),
        });
    }

    /// Warn (non-blocking) about newly added impure nodes — nodes with an execution input — that end
    /// up with no incoming execution edge and would therefore never run. Event/entry nodes have no
    /// execution input and are skipped implicitly; a function body's first node is exempt (no entry
    /// to wire from yet); existing anchored nodes are never recorded, so only new nodes are checked.
    fn check_dangling_impure_execution(&mut self) {
        let nodes = std::mem::take(&mut self.new_impure_nodes);
        for (ref_id, exec_in, friendly_name) in &nodes {
            if self.exec_check_exempt.contains(ref_id) {
                continue;
            }
            let connected = self.connect_commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { to_node, to_pin, .. }
                        if to_node == ref_id && to_pin == exec_in
                )
            });
            if !connected {
                self.result.diagnostics.push(format!(
                    "node `{friendly_name}` has no incoming execution connection and will not run; wire its execution input from the previous statement's execution output"
                ));
            }
        }
    }

    fn queue_add_node(&mut self, meta: NodeMetadata, target_layer: Option<String>) -> NodeEntity {
        self.queue_add_node_with_additional_pins(meta, target_layer, Vec::new(), None)
    }

    fn queue_add_node_with_additional_pins(
        &mut self,
        meta: NodeMetadata,
        target_layer: Option<String>,
        additional_pins: Vec<PlaceholderPinDef>,
        friendly_name: Option<String>,
    ) -> NodeEntity {
        let ref_id = format!("${}", self.next_ref);
        self.next_ref += 1;
        if let Some(exec_in) = metadata_exec_input_pin(&meta) {
            self.new_impure_nodes
                .push((ref_id.clone(), exec_in, meta.friendly_name.clone()));
        }
        let position = self.next_position(target_layer.as_deref());
        self.add_commands.push(BoardCommand::AddNode {
            node_type: meta.name.clone(),
            ref_id: Some(ref_id.clone()),
            position: Some(position),
            friendly_name,
            additional_pins: (!additional_pins.is_empty()).then_some(additional_pins),
            target_layer,
            summary: Some(format!("Add {}", meta.friendly_name)),
        });
        NodeEntity::New { ref_id, meta }
    }

    fn next_position(&mut self, target_layer: Option<&str>) -> NodePosition {
        const COLUMNS: usize = 4;
        let layer_key = target_layer.map(str::to_string);
        let index = self
            .next_position_by_layer
            .entry(layer_key.clone())
            .or_insert(0);
        let current = *index;
        *index += 1;
        let (base_x, base_y) = self.rightmost_existing_position(target_layer);

        NodePosition {
            x: base_x + 260.0 * ((current % COLUMNS) as f64 + 1.0),
            y: base_y + 160.0 * ((current / COLUMNS) as f64),
        }
    }

    fn rightmost_existing_position(&self, target_layer: Option<&str>) -> (f64, f64) {
        let mut rightmost: Option<(f32, f32)> = None;
        let mut seen = HashSet::new();
        let mut nodes = Vec::new();
        match target_layer {
            Some(layer_id) => {
                // Canonical boards keep layer members flat in `board.nodes` and identify their
                // scope via `node.layer`; retain the nested map only as a legacy compatibility
                // source. De-duplicate boards that temporarily contain both representations.
                if let Some(layer) = self.existing.layers.get(layer_id) {
                    for node in layer.nodes.values() {
                        if seen.insert(node.id.as_str()) {
                            nodes.push(node);
                        }
                    }
                }
                for node in self
                    .existing
                    .nodes
                    .values()
                    .filter(|node| node.layer.as_deref() == Some(layer_id))
                {
                    if seen.insert(node.id.as_str()) {
                        nodes.push(node);
                    }
                }
            }
            None => {
                // Layer members also live in `board.nodes`; exclude them from root placement or a
                // wide Function body pushes unrelated root/event nodes thousands of pixels away.
                nodes.extend(self.existing.nodes.values().filter(|node| {
                    node.layer
                        .as_deref()
                        .is_none_or(|layer_id| layer_id.is_empty())
                }));
            }
        };
        for node in nodes {
            if let Some((x, y, _)) = node.coordinates
                && rightmost.is_none_or(|(rx, _)| x > rx)
            {
                rightmost = Some((x, y));
            }
        }
        rightmost
            .map(|(x, y)| (x as f64, y as f64))
            .unwrap_or((0.0, 200.0))
    }

    /// Wire `previous` into `current`'s exec input. Returns the `(from_node, from_pin)`
    /// edge that was connected, or `None` when no connection was made.
    ///
    /// `insertion_origin` is the existing predecessor edge the current insertion streak
    /// branched off from. When new nodes were spliced before an existing target, ONLY that
    /// edge is disconnected — an exec input is a legal fan-in point, and the other incoming
    /// edges belong to unrelated events/branches that must stay wired.
    fn connect_exec(
        &mut self,
        previous: &ExecCursor,
        current: &NodeEntity,
        insertion_origin: Option<&(String, String)>,
    ) -> Option<(String, String)> {
        // Existing→existing pairs keep their board wiring (v1 does not rewrite exec edges) —
        // check FIRST so unchanged roundtrips don't emit no-continuation-policy diagnostics for
        // multi-output nodes whose successors are already wired.
        if matches!(previous.entity, NodeEntity::Existing(_))
            && matches!(current, NodeEntity::Existing(_))
        {
            return None;
        }

        let Some(from_pin) = previous
            .output_pin
            .clone()
            .or_else(|| self.entity_exec_output_pin(&previous.entity))
        else {
            let outputs = self.entity_exec_output_pins(&previous.entity);
            if outputs.len() > 1 {
                let arm_labels = outputs
                    .iter()
                    .map(|name| format!("{}: {{ ... }}", to_camel_case(name)))
                    .collect::<Vec<_>>()
                    .join(" ");
                self.result.diagnostics.push(format!(
                    "node `{}` has multiple execution outputs ({}) and no default continuation policy; bind it (`const r = call({{ ... }})`) and handle its outputs in an arm block `r {{ {arm_labels} }}` — arm labels must be these exact execution output names — instead of a plain sequential statement",
                    previous.entity.node_ref(),
                    outputs.join(", ")
                ));
            }
            return None;
        };
        let to_pin = self.entity_exec_input_pin(current)?;

        if let Some((origin_node, origin_pin)) = insertion_origin
            && let NodeEntity::Existing(node_id) = current
            && let Some(node) = find_board_node(self.existing, node_id)
        {
            for (from_node, from_pin) in self.board_index.exec_incoming_edges(node, &to_pin) {
                if &from_node == origin_node && &from_pin == origin_pin {
                    self.disconnect_commands.push(BoardCommand::DisconnectPins {
                        from_node,
                        from_pin,
                        to_node: node_id.clone(),
                        to_pin: to_pin.clone(),
                        summary: Some(format!("Rewire execution into {}", node.friendly_name)),
                    });
                }
            }
        }

        self.connect_commands.push(BoardCommand::ConnectPins {
            from_node: previous.entity.node_ref(),
            from_pin: from_pin.clone(),
            to_node: current.node_ref(),
            to_pin,
            summary: Some("Connect FlowScript execution order".to_string()),
        });
        Some((previous.entity.node_ref(), from_pin))
    }

    fn preferred_exec_output_for_input_sources(
        &self,
        previous: &NodeEntity,
        input_sources: &[ValueSource],
    ) -> Option<String> {
        let previous_ref = previous.node_ref();
        input_sources.iter().find_map(|source| {
            if source.node.node_ref() != previous_ref {
                return None;
            }
            let output_pin = source.output_pin.as_deref()?;
            self.exec_output_for_data_output(previous, output_pin)
        })
    }

    fn exec_output_for_data_output(&self, entity: &NodeEntity, output_pin: &str) -> Option<String> {
        if !is_streaming_data_output_pin_name(output_pin) {
            return None;
        }
        self.entity_exec_output_pin_named(
            entity,
            &["on_stream", "on_chunk", "for_chunk", "on_delta", "on_token"],
        )
    }

    fn entity_exec_input_pin(&self, entity: &NodeEntity) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => find_board_node(self.existing, id).and_then(exec_input_pin),
            NodeEntity::New { meta, .. } => metadata_exec_input_pin(meta),
            NodeEntity::Layer { .. } => None,
        }
    }

    fn entity_exec_output_pin(&self, entity: &NodeEntity) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => {
                find_board_node(self.existing, id).and_then(exec_output_pin)
            }
            NodeEntity::New { meta, .. } => metadata_exec_output_pin(meta),
            NodeEntity::Layer { .. } => None,
        }
    }

    fn entity_exec_output_pins(&self, entity: &NodeEntity) -> Vec<String> {
        match entity {
            NodeEntity::Existing(id) => find_board_node(self.existing, id)
                .map(|node| {
                    let mut pins: Vec<&Pin> = node
                        .pins
                        .values()
                        .filter(|p| p.pin_type == PinType::Output && is_exec_pin(p))
                        .collect();
                    pins.sort_by_key(|p| (p.index, p.id.clone()));
                    pins.into_iter().map(|p| p.name.clone()).collect()
                })
                .unwrap_or_default(),
            NodeEntity::New { meta, .. } => meta
                .outputs
                .iter()
                .filter(|p| p.data_type == "Execution")
                .map(|p| p.name.clone())
                .collect(),
            NodeEntity::Layer { .. } => Vec::new(),
        }
    }

    /// Resolve the catalog/board shape of the exact data output represented by `source`. An
    /// unselected call result uses the same deterministic default-output rule as connection
    /// planning, so validation never guesses among several unrelated outputs.
    fn source_output_shape(&self, source: &ValueSource) -> Option<OutputShape> {
        match &source.node {
            NodeEntity::Existing(id) => {
                if let Some(node) = find_board_node(self.existing, id) {
                    let pin = source
                        .output_pin
                        .as_deref()
                        .and_then(|name| find_output_pin(node, name))
                        .or_else(|| {
                            default_node_output_pin(node)
                                .as_deref()
                                .and_then(|name| find_output_pin(node, name))
                        })?;
                    return Some(OutputShape {
                        node_type: node.name.clone(),
                        pin_name: pin.name.clone(),
                        data_type: format!("{:?}", pin.data_type),
                        value_type: format!("{:?}", pin.value_type),
                        schema: pin.schema.clone(),
                    });
                }
                let layer = self.existing.layers.get(id)?;
                let requested = source.output_pin.as_deref()?;
                let pin = find_boundary_pin_by_ref(&layer.pins, requested, PinType::Input)?;
                Some(OutputShape {
                    node_type: layer.name.clone(),
                    pin_name: pin.name.clone(),
                    data_type: format!("{:?}", pin.data_type),
                    value_type: format!("{:?}", pin.value_type),
                    schema: pin.schema.clone(),
                })
            }
            NodeEntity::New { meta, .. } => {
                let pin = source
                    .output_pin
                    .as_deref()
                    .and_then(|name| metadata_output_pin(meta, name))
                    .or_else(|| {
                        default_metadata_output_pin(meta)
                            .as_deref()
                            .and_then(|name| metadata_output_pin(meta, name))
                    })?;
                Some(OutputShape {
                    node_type: meta.name.clone(),
                    pin_name: pin.name.clone(),
                    data_type: pin.data_type.clone(),
                    value_type: pin.value_type.clone(),
                    schema: pin.schema.clone(),
                })
            }
            NodeEntity::Layer { ref_id, pins } => {
                let requested = source.output_pin.as_deref()?;
                let pin = pins.iter().find(|pin| {
                    pin.pin_type == "Input"
                        && pin.data_type != "Execution"
                        && pin_name_matches(&pin.name, requested)
                })?;
                Some(OutputShape {
                    node_type: ref_id.clone(),
                    pin_name: pin.name.clone(),
                    data_type: pin.data_type.clone(),
                    value_type: pin.value_type.clone(),
                    schema: pin.schema.clone(),
                })
            }
        }
    }

    /// Resolve only output types that are authoritative for newly planned wiring. An explicit
    /// output is deterministic; an unselected new-node result is deterministic only when it has
    /// exactly one data output. Function-layer parameters retain their complete declared
    /// FlowScript contract, including a named-interface schema when one was authored.
    fn planned_source_output_type(&self, source: &ValueSource) -> Option<PlannedOutputType> {
        match &source.node {
            NodeEntity::New { meta, .. } => {
                let output = if let Some(requested) = source.output_pin.as_deref() {
                    metadata_output_pin(meta, requested)?
                } else {
                    let mut outputs = meta
                        .outputs
                        .iter()
                        .filter(|pin| pin.data_type != "Execution");
                    let output = outputs.next()?;
                    if outputs.next().is_some() {
                        return None;
                    }
                    output
                };
                Some(PlannedOutputType {
                    source: format!("{}.{}", meta.name, output.name),
                    pin_name: output.name.clone(),
                    data_type: output.data_type.clone(),
                    value_type: output.value_type.clone(),
                    is_generic: output.is_generic || output.data_type == "Generic",
                    schema: output.schema.clone(),
                    enforce_schema: output.enforce_schema,
                })
            }
            NodeEntity::Layer { ref_id, pins } => {
                let requested = source.output_pin.as_deref()?;
                let output = pins.iter().find(|pin| {
                    pin.pin_type == "Input"
                        && pin.data_type != "Execution"
                        && pin_name_matches(&pin.name, requested)
                })?;
                Some(PlannedOutputType {
                    source: format!("{ref_id}.{}", output.name),
                    pin_name: output.name.clone(),
                    data_type: output.data_type.clone(),
                    value_type: output.value_type.clone(),
                    is_generic: output.data_type == "Generic",
                    schema: output.schema.clone(),
                    enforce_schema: output.enforce_schema,
                })
            }
            NodeEntity::Existing(id) => {
                let requested = source.output_pin.as_deref()?;
                if let Some(node) = find_board_node(self.existing, id) {
                    let output = find_output_pin(node, requested)?;
                    if node.name == "variable_get"
                        && output.name == "value_ref"
                        && let Some(variable_id) = node_pin_literal_string(node, "var_ref")
                        && let Some(contract) =
                            self.variable_value_contract(&variable_id, &output.name)
                    {
                        return Some(PlannedOutputType {
                            source: format!("{}.{}", node.name, output.name),
                            pin_name: output.name.clone(),
                            data_type: contract.data_type,
                            value_type: contract.value_type,
                            is_generic: contract.is_generic,
                            schema: contract.schema,
                            enforce_schema: contract.enforce_schema,
                        });
                    }
                    return Some(PlannedOutputType {
                        source: format!("{}.{}", node.name, output.name),
                        pin_name: output.name.clone(),
                        data_type: format!("{:?}", output.data_type),
                        value_type: format!("{:?}", output.value_type),
                        is_generic: output.data_type == VariableType::Generic,
                        schema: output.schema.clone(),
                        enforce_schema: output
                            .options
                            .as_ref()
                            .and_then(|options| options.enforce_schema)
                            .unwrap_or(false),
                    });
                }

                // Existing Function layers use their boundary parameter pins as sources inside
                // the body. They share `NodeEntity::Existing` with ordinary nodes, so resolve the
                // layer only after node lookup fails.
                let layer = self.existing.layers.get(id)?;
                let output = find_boundary_pin_by_ref(&layer.pins, requested, PinType::Input)?;
                Some(PlannedOutputType {
                    source: format!("{}.{}", layer.name, output.name),
                    pin_name: output.name.clone(),
                    data_type: format!("{:?}", output.data_type),
                    value_type: format!("{:?}", output.value_type),
                    is_generic: output.data_type == VariableType::Generic,
                    schema: output.schema.clone(),
                    enforce_schema: output
                        .options
                        .as_ref()
                        .and_then(|options| options.enforce_schema)
                        .unwrap_or(false),
                })
            }
        }
    }

    /// Reject the generic `struct_get` fallback when catalog metadata proves it wrong.
    /// Schema-less scalar Struct and dynamic Generic/Normal outputs remain intentionally open;
    /// concrete scalars and collection containers are authoritative even without a schema.
    /// The literal `field` argument of an explicit struct-field call, when it is a plain string.
    fn literal_struct_field_argument(&self, call: &Call) -> Option<String> {
        call.args
            .iter()
            .find(|candidate| candidate.name == "field")
            .and_then(|candidate| literal_expr_to_value(&candidate.value))
            .and_then(|value| value.as_str().map(str::to_string))
    }

    /// `Some((title, hint))` when an explicit `structGet`/`structSet` selects a member that a known
    /// closed platform struct does not declare. Member-access sugar is covered separately by
    /// [`Self::schema_allows_member_access`]; this is the literal-call spelling of the same mistake.
    fn closed_struct_field_call_rejection(
        &self,
        call: &Call,
        meta: &NodeMetadata,
        input: &PinMetadata,
        source: &ValueSource,
        output_pin: &str,
    ) -> Option<(&'static str, &'static str)> {
        if !matches!(meta.name.as_str(), "struct_get" | "struct_set") {
            return None;
        }
        if !matches!(input.name.as_str(), "struct" | "struct_in") {
            return None;
        }
        let field = self.literal_struct_field_argument(call)?;
        let shape = self.source_output_shape(&ValueSource {
            node: source.node.clone(),
            output_pin: Some(output_pin.to_string()),
        })?;
        closed_platform_struct_rejection(shape.schema.as_deref()?, &field)
    }

    fn schema_allows_member_access(&mut self, source: &ValueSource, field: &str) -> bool {
        let Some(shape) = self.source_output_shape(source) else {
            return true;
        };

        if matches!(shape.value_type.as_str(), "Array" | "HashSet") {
            self.result.diagnostics.push(format!(
                "catalog output `{}.{}` has collection type `{}`, so member `{field}` cannot use generic struct-field fallback; select `{}` and index or loop over it instead",
                shape.node_type,
                shape.pin_name,
                shape.value_type,
                shape.pin_name
            ));
            return false;
        }
        if shape.value_type == "HashMap"
            || (shape.data_type == "Generic" && shape.value_type == "Normal")
        {
            return true;
        }
        if shape.data_type != "Struct" {
            self.result.diagnostics.push(format!(
                "catalog output `{}.{}` has scalar type `{}`, so member `{field}` cannot use struct-field fallback",
                shape.node_type, shape.pin_name, shape.data_type
            ));
            return false;
        }
        let Some(schema) = shape.schema.as_deref() else {
            return true;
        };
        if shape.value_type != "Normal" {
            return true;
        }

        // Platform structs the generic path cannot close, checked before it so an open schema does
        // not wave through a member that silently resolves to null at runtime.
        if let Some((title, hint)) = closed_platform_struct_rejection(schema, field) {
            self.result.diagnostics.push(format!(
                "catalog output `{}.{}` is a `{title}`, which has no field `{field}`; {hint}",
                shape.node_type, shape.pin_name
            ));
            return false;
        }

        let Some((title, fields)) = catalog_object_schema_fields(schema) else {
            return true;
        };
        if fields
            .iter()
            .any(|declared| pin_name_matches(declared, field))
        {
            return true;
        }

        let schema_name = title
            .map(|title| format!(" schema `{title}`"))
            .unwrap_or_else(|| " its catalog schema".to_string());
        let available = if fields.is_empty() {
            "none".to_string()
        } else {
            fields
                .iter()
                .map(|field| to_camel_case(field))
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.result.diagnostics.push(format!(
            "catalog output `{}.{}` uses{}, which declares no field `{field}` (available fields: {available}); refusing generic struct-field fallback",
            shape.node_type, shape.pin_name, schema_name
        ));
        false
    }

    /// Resolve a branch arm's label to an exec output pin. Labels come in two shapes: lowered
    /// text uses raw pin names (`exec_out_exists`), while the if/else sugar uses `True`/`False`
    /// — so the camelized label is tried as well (`True` → `true`).
    fn resolve_arm_exec_pin(&self, entity: &NodeEntity, label: &str) -> Option<String> {
        if let Some((name, occurrence)) = parse_pin_occurrence_ref(label) {
            let camel = to_camel_case(name);
            return self
                .entity_exec_output_pin_occurrence(entity, name, occurrence)
                .or_else(|| self.entity_exec_output_pin_occurrence(entity, &camel, occurrence));
        }
        let camel = to_camel_case(label);
        self.entity_exec_output_pin_named(entity, &[label, camel.as_str()])
    }

    /// Resolve the `occurrence`-th same-named exec output to the positional ref board commands
    /// use (`name[#N]`; the first occurrence stays the plain name). Ordering matches
    /// [`arm_label`](super::lower) and the apply-side occurrence decoding: index, then id.
    fn entity_exec_output_pin_occurrence(
        &self,
        entity: &NodeEntity,
        name: &str,
        occurrence: usize,
    ) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => {
                let node = find_board_node(self.existing, id)?;
                let mut matching: Vec<&Pin> = node
                    .pins
                    .values()
                    .filter(|pin| {
                        pin.pin_type == PinType::Output
                            && is_exec_pin(pin)
                            && node_pin_name_matches(pin, name)
                    })
                    .collect();
                matching.sort_by_key(|pin| (pin.index, pin.id.clone()));
                let pin = matching.get(occurrence)?;
                Some(if occurrence == 0 {
                    pin.name.clone()
                } else {
                    pin_occurrence_ref(&pin.name, occurrence)
                })
            }
            NodeEntity::New { meta, .. } => {
                let pin = meta
                    .outputs
                    .iter()
                    .filter(|pin| {
                        pin.data_type == "Execution" && metadata_pin_name_matches(pin, name)
                    })
                    .nth(occurrence)?;
                Some(if occurrence == 0 {
                    pin.name.clone()
                } else {
                    pin_occurrence_ref(&pin.name, occurrence)
                })
            }
            NodeEntity::Layer { .. } => None,
        }
    }

    /// Every exec output of `entity` as the positional ref an arm label resolves to: plain names,
    /// with `name[#N]` selectors for repeated names. Pairs with [`Self::resolve_arm_exec_pin`] so
    /// claimed-arm bookkeeping and the unclaimed-continuation pin agree on one addressing scheme.
    fn entity_exec_output_pin_refs(&self, entity: &NodeEntity) -> Vec<String> {
        let mut seen = HashMap::<String, usize>::new();
        self.entity_exec_output_pins(entity)
            .into_iter()
            .map(|name| {
                let occurrence = seen.entry(name.clone()).or_default();
                let current = *occurrence;
                *occurrence += 1;
                if current == 0 {
                    name
                } else {
                    pin_occurrence_ref(&name, current)
                }
            })
            .collect()
    }

    fn entity_exec_output_pin_named(&self, entity: &NodeEntity, names: &[&str]) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => {
                let node = find_board_node(self.existing, id)?;
                names
                    .iter()
                    .find_map(|name| find_output_pin(node, name).filter(|pin| is_exec_pin(pin)))
                    .map(|pin| pin.name.clone())
            }
            NodeEntity::New { meta, .. } => names
                .iter()
                .find_map(|name| {
                    meta.outputs.iter().find(|pin| {
                        pin.data_type == "Execution" && metadata_pin_name_matches(pin, name)
                    })
                })
                .map(|pin| pin.name.clone()),
            NodeEntity::Layer { .. } => None,
        }
    }

    /// Lower a struct-field access (`<source>.field`, where `<source>` already selected an output
    /// pin) to a `struct_get` node: `struct_get({ struct: <source>, field: "field" }).value`. This
    /// makes dot-notation field access on a node output resolve. Returns `None` (leaving the original
    /// diagnostic to fire) if the catalog has no `struct_get` node.
    fn lower_struct_field_access(
        &mut self,
        base: ValueSource,
        field: &str,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let probe = Call {
            node_type: "struct_get".to_string(),
            display: "structGet".to_string(),
            args: Vec::new(),
            anchor: None,
        };
        let meta = self.catalog.resolve_call(&probe).ok()?;
        let entity = self.queue_add_node(meta.clone(), target_layer);

        // Member access accepts the camel spelling, but the runtime selects the JSON key verbatim,
        // so a known struct's declared name wins over what the author typed.
        let selector = self
            .source_output_shape(&base)
            .and_then(|shape| {
                shape
                    .schema
                    .as_deref()
                    .and_then(|schema| closed_platform_struct_field_name(schema, field))
            })
            .unwrap_or(field)
            .to_string();

        if let Some(field_pin) = metadata_input_pin(&meta, "field") {
            self.queue_update_input(
                &entity,
                field_pin,
                flow_like_types::Value::String(selector),
                &meta,
            );
        }

        if let Some(struct_pin) = metadata_input_pin(&meta, "struct") {
            let from_pin = base
                .output_pin
                .clone()
                .or_else(|| self.resolve_entity_output_pin(&base.node, None));
            if let Some(from_pin) = from_pin
                && !self.queue_validated_data_connection(
                    &base,
                    from_pin,
                    &entity,
                    struct_pin,
                    &struct_pin.name,
                    format!("Read struct field `{field}`"),
                    &format!("struct member access `{field}`"),
                    false,
                )
            {
                return None;
            }
        }

        let output = self.resolve_entity_output_pin(&entity, Some("value"))?;
        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin: Some(output),
        }))
    }

    /// Materialize an `array_length` node for a `.length` member access.
    fn lower_array_length_access(
        &mut self,
        base: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let probe = Call {
            node_type: "array_length".to_string(),
            display: "arrayLength".to_string(),
            args: Vec::new(),
            anchor: None,
        };
        let meta = self.catalog.resolve_call(&probe).ok()?;
        let entity = self.queue_add_node(meta.clone(), target_layer);

        if let Some(array_pin) = metadata_input_pin(&meta, "array") {
            let from_pin = base
                .output_pin
                .clone()
                .or_else(|| self.resolve_entity_output_pin(&base.node, None));
            if let Some(from_pin) = from_pin
                && !self.queue_validated_data_connection(
                    &base,
                    from_pin,
                    &entity,
                    array_pin,
                    &array_pin.name,
                    "Read array length".to_string(),
                    "array length access",
                    false,
                )
            {
                return None;
            }
        }

        let output = self
            .resolve_entity_output_pin(&entity, Some("length"))
            .or_else(|| self.resolve_entity_output_pin(&entity, None));
        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin: output,
        }))
    }

    fn lower_container_size_access(
        &mut self,
        base: ValueSource,
        node_type: &str,
        display: &str,
        input_name: &str,
        output_name: &str,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let probe = Call {
            node_type: node_type.to_string(),
            display: display.to_string(),
            args: Vec::new(),
            anchor: None,
        };
        let meta = self.catalog.resolve_call(&probe).ok()?;
        let entity = self.queue_add_node(meta.clone(), target_layer);
        let input = metadata_input_pin(&meta, input_name)?;
        let from_pin = base
            .output_pin
            .clone()
            .or_else(|| self.resolve_entity_output_pin(&base.node, None))?;
        if !self.queue_validated_data_connection(
            &base,
            from_pin,
            &entity,
            input,
            &input.name,
            format!("Read {display}"),
            &format!("{display} access"),
            false,
        ) {
            return None;
        }
        let output = self
            .resolve_entity_output_pin(&entity, Some(output_name))
            .or_else(|| self.resolve_entity_output_pin(&entity, None));
        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin: output,
        }))
    }

    fn lower_collection_length_access(
        &mut self,
        base: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        match self
            .source_output_shape(&base)
            .map(|shape| shape.value_type)
            .as_deref()
        {
            Some("HashSet") => self.lower_container_size_access(
                base,
                "set_get_size",
                "set size",
                "set_in",
                "size",
                target_layer,
            ),
            Some("HashMap") => self.lower_container_size_access(
                base,
                "map_size",
                "map size",
                "map_in",
                "size",
                target_layer,
            ),
            Some("Array") | None => self.lower_array_length_access(base, target_layer),
            Some(_) => None,
        }
    }

    fn lower_hash_map_member_access(
        &mut self,
        base: ValueSource,
        field: &str,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let probe = Call {
            node_type: "map_get".to_string(),
            display: "mapGet".to_string(),
            args: Vec::new(),
            anchor: None,
        };
        let meta = self.catalog.resolve_call(&probe).ok()?;
        let entity = self.queue_add_node(meta.clone(), target_layer);
        if let Some(key_pin) = metadata_input_pin(&meta, "key") {
            self.queue_update_input(
                &entity,
                key_pin,
                flow_like_types::Value::String(field.to_string()),
                &meta,
            );
        }
        let map_pin = metadata_input_pin(&meta, "map_in")?;
        let from_pin = base
            .output_pin
            .clone()
            .or_else(|| self.resolve_entity_output_pin(&base.node, None))?;
        if !self.queue_validated_data_connection(
            &base,
            from_pin,
            &entity,
            map_pin,
            &map_pin.name,
            format!("Read map key `{field}`"),
            &format!("map member access `{field}`"),
            false,
        ) {
            return None;
        }
        let output = self.resolve_entity_output_pin(&entity, Some("value"))?;
        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin: Some(output),
        }))
    }

    fn lower_member_access(
        &mut self,
        base: ValueSource,
        field: &str,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let shape = self.source_output_shape(&base);
        match shape.as_ref().map(|shape| shape.value_type.as_str()) {
            Some("HashMap") if field == "length" => {
                self.lower_collection_length_access(base, target_layer)
            }
            Some("HashMap") => self.lower_hash_map_member_access(base, field, target_layer),
            Some("Array" | "HashSet") if field == "length" => {
                self.lower_collection_length_access(base, target_layer)
            }
            Some("Array" | "HashSet") => {
                let _ = self.schema_allows_member_access(&base, field);
                None
            }
            _ if self.schema_allows_member_access(&base, field) => {
                self.lower_struct_field_access(base, field, target_layer)
            }
            _ => None,
        }
    }

    /// Materialize an `array_get` node for a `base[index]` access, reading its `element`
    /// output. The synthetic call routes base/index through the standard argument planner.
    fn lower_array_index_access(
        &mut self,
        base: &Expr,
        index: &Expr,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let call = Call {
            node_type: "array_get".to_string(),
            display: "arrayGet".to_string(),
            args: vec![
                Arg {
                    name: "array_in".to_string(),
                    value: base.clone(),
                },
                Arg {
                    name: "index".to_string(),
                    value: index.clone(),
                },
            ],
            anchor: None,
        };
        let entity = self.add_call_node(&call, target_layer)?;
        let output = self
            .resolve_entity_output_pin(&entity, Some("element"))
            .or_else(|| self.resolve_entity_output_pin(&entity, None));
        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin: output,
        }))
    }

    /// Drop a statement's own primary call node from the exec-splice queue.
    ///
    /// A bare `x = impureCall(...)` reassignment (or a `let`/`const` alias such as
    /// `x = call(...).pin`) resolves its RHS through [`Self::resolve_expr`], whose `Expr::Call`
    /// arm queues every impure call for exec-splicing — the machinery that threads impure calls
    /// buried in *arguments* into the execution chain ahead of the consuming statement. But the
    /// RHS's own top-level call is the statement's primary node, and `plan_block` already
    /// exec-wires it as the `current` statement. Leaving it queued wires it twice, and because the
    /// splice makes it `previous_exec` immediately before it becomes `current`, `connect_exec`
    /// runs its execution output straight back into its own input — a self-connection that aborts
    /// apply with "Cannot connect a node to itself". Removing it leaves the statement path as its
    /// single, correct execution wiring; genuine argument-position splices stay queued.
    fn undefer_statement_call_splice(&mut self, entity: &NodeEntity) {
        let ref_id = entity.node_ref();
        self.pending_exec_splices
            .retain(|splice| splice.node_ref() != ref_id);
    }

    fn resolve_expr(&mut self, expr: &Expr, target_layer: Option<String>) -> Option<SymbolValue> {
        if let Some(value) = literal_expr_to_value(expr) {
            return Some(SymbolValue::Literal(value));
        }

        match expr {
            Expr::Ref(name) => self.lookup_symbol(name),
            Expr::Field { base, pin } => {
                let mut source = match self.resolve_expr(base, target_layer.clone())? {
                    SymbolValue::Source(source) => source,
                    SymbolValue::VariableRef { variable_id } => {
                        self.add_variable_get_source(&variable_id, target_layer.clone())?
                    }
                    SymbolValue::Literal(_) => return None,
                };
                match self.resolve_entity_output_pin(&source.node, Some(pin)) {
                    Some(output) => {
                        source.output_pin = Some(output);
                        Some(SymbolValue::Source(source))
                    }
                    // The name is not an output pin: models routinely write camelCase struct
                    // DATA keys (`mail.subject`), which the parser classifies as Field. Fall
                    // back to the struct_get lowering the Member arm uses instead of dropping
                    // the connection.
                    None => self.lower_member_access(source, pin, target_layer),
                }
            }
            Expr::Call(call) => self.add_call_node(call, target_layer).map(|node| {
                // Impure calls in expression position only get data wiring here; queue them so
                // plan_block splices them into the exec chain before the consuming statement.
                if self.entity_exec_input_pin(&node).is_some() {
                    self.pending_exec_splices.push(node.clone());
                }
                SymbolValue::Source(ValueSource {
                    node,
                    output_pin: None,
                })
            }),
            Expr::Member { base, field } => {
                // `base.field` is the text form of struct_get / array_length; rebuild the
                // accessor node instead of silently dropping the selection (which would
                // wire the consumer straight to the base value).
                let base_symbol = self.resolve_expr(base, target_layer.clone())?;
                let base_source = self.symbol_to_source(base_symbol, target_layer.clone())?;
                self.lower_member_access(base_source, field, target_layer)
            }
            Expr::Index { base, index } => self.lower_array_index_access(base, index, target_layer),
            Expr::Binary { op, lhs, rhs } => self.lower_binary_operator(op, lhs, rhs, target_layer),
            Expr::Object(_) | Expr::Array(_) | Expr::Ternary { .. } => None,
            Expr::Literal(_) => None,
        }
    }

    /// Materialize the catalog node represented by a FlowScript binary expression and return its
    /// output as a value source. Selection is type-directed: a literal on either side is enough to
    /// choose an Integer/Float/String/Boolean family while typed refs and calls use output metadata.
    fn lower_binary_operator(
        &mut self,
        op: &str,
        lhs: &Expr,
        rhs: &Expr,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let meta = self.resolve_binary_operator_meta(op, lhs, rhs)?;
        let inputs = binary_data_inputs(&meta)?;
        let call = binary_operator_call(&meta, &inputs, lhs, rhs);
        let entity = self.queue_add_node(meta.clone(), target_layer.clone());
        self.plan_call_arguments(&call, &entity, &meta, target_layer, true);

        let Some(output_pin) = default_metadata_output_pin(&meta) else {
            self.result.diagnostics.push(format!(
                "binary operator node `{}` has no unambiguous data output",
                meta.name
            ));
            return None;
        };
        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin: Some(output_pin),
        }))
    }

    fn resolve_binary_operator_meta(
        &mut self,
        op: &str,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Option<NodeMetadata> {
        let op = canonical_binary_op(op);
        if !BINARY_OPERATOR_NODES
            .iter()
            .any(|(candidate_op, _, _, _)| *candidate_op == op)
        {
            self.result.diagnostics.push(format!(
                "binary operator `{op}` is not supported by FlowScript reconcile"
            ));
            return None;
        }

        let lhs_type = self.expr_data_type_hint(lhs);
        let rhs_type = self.expr_data_type_hint(rhs);
        let operand_type = match (lhs_type.as_deref(), rhs_type.as_deref()) {
            (Some(lhs), Some(rhs)) if lhs == rhs => Some(lhs.to_string()),
            (Some(lhs), Some(rhs)) => {
                self.result.diagnostics.push(format!(
                    "binary operator `{op}` has incompatible operand types `{lhs}` and `{rhs}`"
                ));
                return None;
            }
            (Some(known), None) | (None, Some(known)) => Some(known.to_string()),
            (None, None) => None,
        };

        let candidates = BINARY_OPERATOR_NODES
            .iter()
            .filter(|(candidate_op, data_type, _, _)| {
                *candidate_op == op
                    && operand_type
                        .as_deref()
                        .is_none_or(|known| known == *data_type)
            })
            .filter_map(|(_, _, result_type, node_type)| {
                self.catalog
                    .resolve_type(node_type)
                    .ok()
                    .map(|meta| (*result_type, meta))
            })
            .filter(|(result_type, meta)| {
                binary_data_inputs(meta).is_some()
                    && meta.outputs.iter().any(|pin| pin.data_type == *result_type)
            })
            .map(|(_, meta)| meta)
            .collect::<Vec<_>>();

        match candidates.as_slice() {
            [meta] => Some(meta.clone()),
            [] => {
                let type_name = operand_type.as_deref().unwrap_or("unknown");
                self.result.diagnostics.push(format!(
                    "binary operator `{op}` for `{type_name}` has no suitable two-input catalog node"
                ));
                None
            }
            many => {
                self.result.diagnostics.push(format!(
                    "binary operator `{op}` has ambiguous operand type; candidates are {}",
                    many.iter()
                        .map(|meta| meta.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                None
            }
        }
    }

    fn expr_data_type_hint(&self, expr: &Expr) -> Option<String> {
        if let Some(value) = literal_expr_to_value(expr) {
            return value_data_type(&value).map(str::to_string);
        }

        match expr {
            Expr::Ref(name) => self
                .lookup_symbol(name)
                .and_then(|symbol| self.symbol_data_type_hint(&symbol)),
            Expr::Call(call) => {
                let meta = self.catalog.resolve_call(call).ok()?;
                let output = default_metadata_output_pin(&meta)?;
                metadata_output_pin(&meta, &output).map(|pin| pin.data_type.clone())
            }
            Expr::Field { base, pin } => match base.as_ref() {
                Expr::Call(call) => self
                    .catalog
                    .resolve_call(call)
                    .ok()
                    .and_then(|meta| metadata_output_pin(&meta, pin).cloned())
                    .map(|output| output.data_type),
                Expr::Ref(name) => self.lookup_symbol(name).and_then(|symbol| match symbol {
                    SymbolValue::Source(source) => {
                        self.entity_output_data_type(&source.node, Some(pin))
                    }
                    _ => None,
                }),
                _ => None,
            },
            Expr::Ternary {
                then, otherwise, ..
            } => {
                let then_type = self.expr_data_type_hint(then)?;
                (self.expr_data_type_hint(otherwise).as_deref() == Some(then_type.as_str()))
                    .then_some(then_type)
            }
            Expr::Binary { op, lhs, rhs } => {
                let op = canonical_binary_op(op);
                let lhs_type = self.expr_data_type_hint(lhs);
                let rhs_type = self.expr_data_type_hint(rhs);
                let operand_type = match (lhs_type.as_deref(), rhs_type.as_deref()) {
                    (Some(lhs), Some(rhs)) if lhs == rhs => Some(lhs),
                    (Some(_), Some(_)) => return None,
                    (Some(known), None) | (None, Some(known)) => Some(known),
                    (None, None) => None,
                };
                let mut result_types = BINARY_OPERATOR_NODES
                    .iter()
                    .filter(|(candidate_op, data_type, _, _)| {
                        *candidate_op == op && operand_type.is_none_or(|known| known == *data_type)
                    })
                    .map(|(_, _, result_type, _)| *result_type)
                    .collect::<Vec<_>>();
                result_types.sort_unstable();
                result_types.dedup();
                match result_types.as_slice() {
                    [result_type] => Some((*result_type).to_string()),
                    _ => None,
                }
            }
            Expr::Object(_) => Some("Struct".to_string()),
            Expr::Array(_) => Some("Generic".to_string()),
            Expr::Member { .. } | Expr::Index { .. } | Expr::Literal(_) => None,
        }
    }

    fn symbol_data_type_hint(&self, symbol: &SymbolValue) -> Option<String> {
        match symbol {
            SymbolValue::Source(source) => self.source_data_type_hint(source),
            SymbolValue::Literal(value) => value_data_type(value).map(str::to_string),
            SymbolValue::VariableRef { variable_id } => self
                .existing
                .variables
                .get(variable_id)
                .or_else(|| {
                    self.existing
                        .layers
                        .values()
                        .find_map(|layer| layer.variables.get(variable_id))
                })
                .map(|variable| format!("{:?}", variable.data_type)),
        }
    }

    fn source_data_type_hint(&self, source: &ValueSource) -> Option<String> {
        let output = source
            .output_pin
            .clone()
            .or_else(|| self.resolve_entity_output_pin(&source.node, None))?;
        self.entity_output_data_type(&source.node, Some(&output))
    }

    fn entity_output_data_type(
        &self,
        entity: &NodeEntity,
        requested: Option<&str>,
    ) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => {
                let node = find_board_node(self.existing, id)?;
                let output = requested
                    .and_then(|name| find_output_pin(node, name))
                    .or_else(|| {
                        default_node_output_pin(node).and_then(|name| find_output_pin(node, &name))
                    })?;
                Some(format!("{:?}", output.data_type))
            }
            NodeEntity::New { meta, .. } => {
                let output = requested
                    .and_then(|name| metadata_output_pin(meta, name))
                    .or_else(|| {
                        default_metadata_output_pin(meta)
                            .and_then(|name| metadata_output_pin(meta, &name))
                    })?;
                Some(output.data_type.clone())
            }
            NodeEntity::Layer { pins, .. } => pins
                .iter()
                .find(|pin| {
                    pin.pin_type == "Output"
                        && requested.is_none_or(|name| pin_name_matches(&pin.name, name))
                })
                .map(|pin| pin.data_type.clone()),
        }
    }

    fn symbol_to_source(
        &mut self,
        symbol: SymbolValue,
        target_layer: Option<String>,
    ) -> Option<ValueSource> {
        match symbol {
            SymbolValue::Source(source) => Some(source),
            SymbolValue::VariableRef { variable_id } => {
                self.add_variable_get_source(&variable_id, target_layer)
            }
            SymbolValue::Literal(_) => None,
        }
    }

    fn variable_value_contract(&self, variable_id: &str, pin_name: &str) -> Option<PinMetadata> {
        let mut contract = if let Some(contract) = self.variable_value_contracts.get(variable_id) {
            contract.clone()
        } else {
            let variable = self.existing.variables.get(variable_id).or_else(|| {
                self.existing
                    .layers
                    .values()
                    .find_map(|layer| layer.variables.get(variable_id))
            })?;
            variable_value_pin_metadata(
                pin_name,
                format!("{:?}", variable.data_type),
                format!("{:?}", variable.value_type),
                variable.schema.clone(),
            )
        };
        contract.name = pin_name.to_string();
        contract.friendly_name = pin_name.to_string();
        Some(contract)
    }

    fn specialize_variable_node_metadata(&self, meta: &mut NodeMetadata, variable_id: &str) {
        for input in &mut meta.inputs {
            if matches!(input.name.as_str(), "value_in" | "new_value" | "value")
                && let Some(contract) =
                    self.variable_value_contract(variable_id, input.name.as_str())
            {
                input.data_type = contract.data_type;
                input.value_type = contract.value_type;
                input.schema = contract.schema;
                input.is_generic = contract.is_generic;
            }
        }
        for output in &mut meta.outputs {
            if output.name == "value_ref"
                && let Some(contract) =
                    self.variable_value_contract(variable_id, output.name.as_str())
            {
                output.data_type = contract.data_type;
                output.value_type = contract.value_type;
                output.schema = contract.schema;
                output.is_generic = contract.is_generic;
            }
        }
    }

    fn add_variable_get_source(
        &mut self,
        variable_id: &str,
        target_layer: Option<String>,
    ) -> Option<ValueSource> {
        let mut meta = self.resolve_variable_node("variable_get", "variableGet")?;
        self.specialize_variable_node_metadata(&mut meta, variable_id);
        let entity = self.queue_add_node(meta.clone(), target_layer);
        if let Some(input) = metadata_input_pin(&meta, "var_ref") {
            self.update_commands.push(BoardCommand::UpdateNodePin {
                node_id: entity.node_ref(),
                pin_id: input.name.clone(),
                value: flow_like_types::Value::String(variable_id.to_string()),
                summary: Some("Select FlowScript variable".to_string()),
            });
        }
        Some(ValueSource {
            node: entity,
            output_pin: Some("value_ref".to_string()),
        })
    }

    fn add_variable_set_node(
        &mut self,
        variable_id: &str,
        value: &Expr,
        target_layer: Option<String>,
    ) -> Option<NodeEntity> {
        let mut meta = self.resolve_variable_node("variable_set", "variableSet")?;
        self.specialize_variable_node_metadata(&mut meta, variable_id);
        let entity = self.queue_add_node(meta.clone(), target_layer.clone());

        if let Some(input) = metadata_input_pin(&meta, "var_ref") {
            self.update_commands.push(BoardCommand::UpdateNodePin {
                node_id: entity.node_ref(),
                pin_id: input.name.clone(),
                value: flow_like_types::Value::String(variable_id.to_string()),
                summary: Some("Select FlowScript variable".to_string()),
            });
        }

        let Some(input) = metadata_input_pin(&meta, "value_in") else {
            self.result
                .diagnostics
                .push("variable_set has no value input pin".to_string());
            return Some(entity);
        };

        if let Some(literal) = literal_expr_to_value(value) {
            self.update_commands.push(BoardCommand::UpdateNodePin {
                node_id: entity.node_ref(),
                pin_id: input.name.clone(),
                value: literal,
                summary: Some("Set FlowScript variable value".to_string()),
            });
            return Some(entity);
        }

        let Some(source) = self
            .resolve_expr(value, target_layer.clone())
            .and_then(|symbol| self.symbol_to_source(symbol, target_layer))
        else {
            self.result.diagnostics.push(format!(
                "assignment to variable `{variable_id}` is not a resolvable value"
            ));
            return Some(entity);
        };

        if let Some(output_pin) = self.resolve_source_output_pin_for_input(&source, input) {
            self.queue_validated_data_connection(
                &source,
                output_pin,
                &entity,
                input,
                &input.name,
                "Set FlowScript variable value".to_string(),
                &format!("assignment to variable `{variable_id}`"),
                true,
            );
        }

        Some(entity)
    }

    fn plan_existing_variable_set_node(
        &mut self,
        entity: &NodeEntity,
        variable_id: &str,
        value: &Expr,
        target_layer: Option<String>,
    ) {
        let NodeEntity::Existing(node_id) = entity else {
            return;
        };
        let Some(node) = find_board_node(self.existing, node_id) else {
            return;
        };
        let meta = node_to_metadata(node);

        if let Some(input) = metadata_input_pin(&meta, "var_ref") {
            self.queue_update_input(
                entity,
                input,
                flow_like_types::Value::String(variable_id.to_string()),
                &meta,
            );
        }

        let Some(input) = metadata_input_pin(&meta, "value_in")
            .or_else(|| metadata_input_pin(&meta, "new_value"))
            .or_else(|| metadata_input_pin(&meta, "value"))
        else {
            self.result.diagnostics.push(format!(
                "variable set node `{node_id}` has no value input pin"
            ));
            return;
        };
        let target_input = self
            .variable_value_contract(variable_id, &input.name)
            .unwrap_or_else(|| input.clone());

        if let Some(mut literal) = literal_expr_to_value(value) {
            if self.reuse_existing_composite_literal_source(
                value,
                entity,
                &input.name,
                target_layer.clone(),
            ) {
                return;
            }
            self.normalize_input_value(&target_input, &mut literal);
            self.queue_update_input(entity, input, literal, &meta);
            return;
        }

        let Some(source) = self
            .resolve_expr_for_argument(value, entity, &input.name, target_layer.clone())
            .and_then(|symbol| self.symbol_to_source(symbol, target_layer))
        else {
            self.result.diagnostics.push(format!(
                "assignment to variable `{variable_id}` is not a resolvable value"
            ));
            return;
        };

        if let Some(output_pin) = self.resolve_source_output_pin_for_input(&source, &target_input) {
            self.queue_validated_data_connection(
                &source,
                output_pin,
                entity,
                &target_input,
                &input.name,
                "Set FlowScript variable value".to_string(),
                &format!("assignment to variable `{variable_id}`"),
                true,
            );
        }
    }

    fn resolve_variable_node(&mut self, node_type: &str, display: &str) -> Option<NodeMetadata> {
        match self.catalog.resolve_type(node_type) {
            Ok(meta) => return Some(meta),
            Err(reason) if self.catalog.by_type.contains_key(node_type) => {
                // Same-type declarations that conflict only in their instance specialization
                // (board-derived catalogs carry one entry per node instance, and accessor pins
                // are regenerated by `on_update` from the selected variable anyway) still
                // identify one usable node type; pick the deterministic candidate.
                let matches = &self.catalog.by_type[node_type];
                if matches.iter().all(|meta| meta.name == node_type) {
                    return Some(deterministic_catalog_match(matches));
                }
                self.result.diagnostics.push(format!(
                    "catalog node `{node_type}` required for `{display}` is unusable: {reason}"
                ));
                return None;
            }
            Err(_) => {}
        }
        match self.catalog.resolve_display(display) {
            Ok(meta) => Some(meta),
            Err(err) => {
                self.result.diagnostics.push(err);
                None
            }
        }
    }

    fn create_local_variable(
        &mut self,
        name: &str,
        value: &Expr,
        target_layer: Option<String>,
    ) -> String {
        let default_value = literal_expr_to_value(value);
        let (data_type, value_type) = if default_value.is_some() {
            infer_variable_types(default_value.as_ref())
        } else {
            // A non-literal initializer carries no default, but its output contract still
            // types the variable so downstream connections are validated against it.
            (
                self.expr_data_type_hint(value)
                    .unwrap_or_else(|| "Generic".to_string()),
                "Normal".to_string(),
            )
        };
        self.create_typed_local_variable(name, default_value, data_type, value_type, target_layer)
    }

    fn create_typed_local_variable(
        &mut self,
        name: &str,
        default_value: Option<flow_like_types::Value>,
        data_type: String,
        value_type: String,
        target_layer: Option<String>,
    ) -> String {
        let variable_id = self.unique_local_variable_id(name);
        self.variable_refs.insert(&variable_id, name);
        self.variable_value_contracts.insert(
            variable_id.clone(),
            variable_value_pin_metadata("value_in", data_type.clone(), value_type.clone(), None),
        );
        self.add_commands.push(BoardCommand::CreateVariable {
            variable_id: Some(variable_id.clone()),
            name: name.to_string(),
            data_type,
            value_type,
            default_value,
            description: None,
            category: None,
            schema: None,
            exposed: Some(false),
            secret: Some(false),
            editable: Some(true),
            runtime_configured: Some(false),
            target_layer,
            summary: Some(format!("Create local FlowScript variable {name}")),
        });
        variable_id
    }

    /// Resolve a literal `return` value to its materialized variable source, reusing (in order)
    /// the getter already wired into the boundary pin, the source planned for this return earlier
    /// in this run (another branch arm), and an existing layer-local variable, before creating
    /// anything new — re-applying the same script must not grow the board.
    fn literal_return_source(
        &mut self,
        layer: &NodeEntity,
        function_name: &str,
        return_param: &PinMetadata,
        literal: flow_like_types::Value,
        target_layer: Option<String>,
    ) -> Option<ValueSource> {
        let base_name = format!("{function_name}_{}", return_param.name);
        let deterministic_id = generated_variable_id(&base_name);

        for existing in self.existing_sources_for_input_ref(layer, &return_param.name) {
            let Some(variable_id) = self.materialized_return_variable_id(&existing, &base_name)
            else {
                continue;
            };
            self.queue_literal_return_default_update(&variable_id, &literal);
            self.planned_literal_return_sources
                .insert(deterministic_id, existing.clone());
            return Some(existing);
        }

        if let Some(source) = self.planned_literal_return_sources.get(&deterministic_id) {
            return Some(source.clone());
        }

        // The variable survived but its getter/edge was removed: reuse it instead of minting a
        // `_2` sibling. Restricted to THIS function layer so an unrelated same-named variable in
        // another scope is never hijacked.
        let orphaned = target_layer
            .as_deref()
            .and_then(|layer_id| self.existing.layers.get(layer_id))
            .is_some_and(|existing_layer| existing_layer.variables.contains_key(&deterministic_id));
        if orphaned {
            self.queue_literal_return_default_update(&deterministic_id, &literal);
            let source = self.add_variable_get_source(&deterministic_id, target_layer)?;
            self.planned_literal_return_sources
                .insert(deterministic_id, source.clone());
            return Some(source);
        }

        let source = self.materialize_literal_return_source(
            function_name,
            return_param,
            literal,
            target_layer,
        )?;
        self.planned_literal_return_sources
            .insert(deterministic_id, source.clone());
        Some(source)
    }

    /// If `source` is a live `variable_get` reading a variable materialized for this literal
    /// return — its name is the deterministic `{function}_{pin}`, historically with a `_N`
    /// uniqueness suffix — return that variable's id.
    fn materialized_return_variable_id(
        &self,
        source: &ValueSource,
        base_name: &str,
    ) -> Option<String> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        if node.name != "variable_get" {
            return None;
        }
        let variable_id = node_pin_literal_string(node, "var_ref")?;
        let variable = self.find_existing_variable(&variable_id)?;
        let matches = variable.name == base_name
            || variable
                .name
                .strip_prefix(base_name)
                .and_then(|rest| rest.strip_prefix('_'))
                .is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
                });
        matches.then_some(variable_id)
    }

    fn find_existing_variable(&self, variable_id: &str) -> Option<&Variable> {
        self.existing.variables.get(variable_id).or_else(|| {
            self.existing
                .layers
                .values()
                .find_map(|layer| layer.variables.get(variable_id))
        })
    }

    /// Emit an `UpdateVariable` for a reused materialized return variable when the authored
    /// literal differs from its stored default (type follows the literal).
    fn queue_literal_return_default_update(
        &mut self,
        variable_id: &str,
        literal: &flow_like_types::Value,
    ) {
        let Some(existing) = self.find_existing_variable(variable_id) else {
            return;
        };
        let current = existing
            .default_value
            .as_deref()
            .and_then(|bytes| flow_like_types::json::from_slice(bytes).ok());
        if current.as_ref() == Some(literal) {
            return;
        }
        let (data_type, value_type) = infer_variable_types(Some(literal));
        let data_type = (data_type != format!("{:?}", existing.data_type)).then_some(data_type);
        let value_type = (value_type != format!("{:?}", existing.value_type)).then_some(value_type);
        self.update_commands.push(BoardCommand::UpdateVariable {
            variable_id: variable_id.to_string(),
            name: None,
            data_type,
            value_type,
            default_value: Some(literal.clone()),
            clear_default_value: false,
            description: None,
            clear_description: false,
            category: None,
            clear_category: false,
            schema: None,
            clear_schema: false,
            exposed: None,
            secret: None,
            editable: None,
            runtime_configured: None,
            value: None,
            summary: Some("Update FlowScript literal return value".to_string()),
        });
    }

    /// A literal `return` value has no producing node; materialize it as a typed layer-local
    /// variable whose default is the literal, read through a `variable_get` inside the function
    /// layer, so the boundary return pin gets a real data source.
    fn materialize_literal_return_source(
        &mut self,
        function_name: &str,
        return_param: &PinMetadata,
        literal: flow_like_types::Value,
        target_layer: Option<String>,
    ) -> Option<ValueSource> {
        let name =
            self.unique_local_variable_name(&format!("{function_name}_{}", return_param.name));
        let (data_type, value_type) = infer_variable_types(Some(&literal));
        let variable_id = self.create_typed_local_variable(
            &name,
            Some(literal),
            data_type,
            value_type,
            target_layer.clone(),
        );
        self.add_variable_get_source(&variable_id, target_layer)
    }

    fn unique_local_variable_name(&self, base: &str) -> String {
        let mut candidate = base.to_string();
        let mut suffix = 2;
        loop {
            if !self.local_variable_id_taken(&generated_variable_id(&candidate)) {
                return candidate;
            }
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
    }

    /// Local variable ids derive from bare local names, which legally repeat across function
    /// layers (`let count = …` in two functions). Ids must stay unique board-wide, so suffix the
    /// id — not the display name — when the derived id is already claimed.
    fn unique_local_variable_id(&self, name: &str) -> String {
        let base = generated_variable_id(name);
        let mut candidate = base.clone();
        let mut suffix = 2;
        loop {
            if !self.local_variable_id_taken(&candidate) {
                return candidate;
            }
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
    }

    fn local_variable_id_taken(&self, id: &str) -> bool {
        self.variable_value_contracts.contains_key(id)
            || self.existing.variables.contains_key(id)
            || self
                .existing
                .layers
                .values()
                .any(|layer| layer.variables.contains_key(id))
    }

    fn variable_id_for_assignment_target(&mut self, target: &str) -> Option<String> {
        match self.lookup_symbol(target) {
            Some(SymbolValue::VariableRef { variable_id }) => Some(variable_id),
            _ => None,
        }
    }

    fn resolve_source_output_pin(&self, source: &ValueSource) -> Option<String> {
        if source.output_pin.is_some() {
            return source.output_pin.clone();
        }
        self.resolve_entity_output_pin(&source.node, None)
    }

    fn resolve_source_output_pin_for_input(
        &self,
        source: &ValueSource,
        input: &PinMetadata,
    ) -> Option<String> {
        if source.output_pin.is_some() {
            return source.output_pin.clone();
        }

        match &source.node {
            NodeEntity::New { meta, .. } => {
                let compatible: Vec<&PinMetadata> = meta
                    .outputs
                    .iter()
                    .filter(|pin| pin.data_type != "Execution")
                    .filter(|pin| metadata_pins_are_compatible(input, pin, &self.existing.refs))
                    .collect();
                match compatible.as_slice() {
                    [pin] => Some(pin.name.clone()),
                    many => many
                        .iter()
                        .find(|pin| {
                            matches!(pin.name.as_str(), "result" | "value" | "output" | "out")
                        })
                        .map(|pin| pin.name.clone()),
                }
            }
            NodeEntity::Existing(id) => self.resolve_source_output_pin(source).or_else(|| {
                // Multi-output live node without a default alias: the consuming pin's type can
                // still disambiguate (e.g. `return user` into a `bool` return pin selects the one
                // Boolean output).
                let node = find_board_node(self.existing, id)?;
                let meta = node_to_metadata(node);
                let compatible: Vec<&PinMetadata> = meta
                    .outputs
                    .iter()
                    .filter(|pin| pin.data_type != "Execution")
                    .filter(|pin| metadata_pins_are_compatible(input, pin, &self.existing.refs))
                    .collect();
                match compatible.as_slice() {
                    [pin] => Some(pin.name.clone()),
                    _ => None,
                }
            }),
            NodeEntity::Layer { .. } => self.resolve_source_output_pin(source),
        }
    }

    fn resolve_entity_output_pin(
        &self,
        entity: &NodeEntity,
        requested: Option<&str>,
    ) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => {
                let node = find_board_node(self.existing, id)?;
                if let Some(requested) = requested {
                    return find_output_pin(node, requested).map(|p| p.name.clone());
                }
                default_node_output_pin(node)
            }
            NodeEntity::New { meta, .. } => {
                if let Some(requested) = requested {
                    return metadata_output_pin(meta, requested).map(|p| p.name.clone());
                }
                default_metadata_output_pin(meta)
            }
            NodeEntity::Layer { pins, .. } => {
                if let Some(requested) = requested {
                    return pins
                        .iter()
                        .find(|pin| pin_name_matches(&pin.name, requested))
                        .map(|pin| pin.name.clone());
                }
                pins.iter()
                    .find(|pin| pin.pin_type == "Output")
                    .map(|pin| pin.name.clone())
            }
        }
    }

    fn seed_params_from_entity(&mut self, params: &[Param], entity: &NodeEntity) {
        for param in params {
            if let Some(pin) = self.resolve_entity_output_pin(entity, Some(&param.name)) {
                self.insert_symbol(
                    param.name.clone(),
                    SymbolValue::Source(ValueSource {
                        node: entity.clone(),
                        output_pin: Some(pin),
                    }),
                );
            }
        }
    }

    fn seed_top_level_variables(&mut self, ast: &BoardAst) {
        for var in &ast.variables {
            let variable_id = self.variable_id_for_decl(var);
            self.variable_value_contracts.insert(
                variable_id.clone(),
                variable_value_pin_metadata(
                    "value_in",
                    type_ref_data_type(&var.ty).to_string(),
                    type_ref_value_type(&var.ty).to_string(),
                    visible_variable_schema(ast, var).or_else(|| var.schema.clone()),
                ),
            );
            self.variable_refs.insert(&variable_id, &var.name);
            self.insert_symbol(var.name.clone(), SymbolValue::VariableRef { variable_id });
        }
    }

    fn variable_id_for_decl(&self, var: &VarDecl) -> String {
        if let Some(anchor) = var.anchor.as_deref() {
            return anchor.to_string();
        }

        if let Some(existing) = self
            .existing
            .variables
            .values()
            .find(|existing| existing.name == var.name)
        {
            return existing.id.clone();
        }

        for layer in self.existing.layers.values() {
            if let Some(existing) = layer
                .variables
                .values()
                .find(|existing| existing.name == var.name)
            {
                return existing.id.clone();
            }
        }

        variable_id_for_decl(var)
    }

    fn push_scope(&mut self) {
        self.symbols.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.symbols.pop();
    }

    /// Close a loop body or branch arm, retiring its bindings into [`Self::closed_block_symbols`]
    /// instead of dropping them.
    fn pop_block_scope(&mut self) {
        if let Some(scope) = self.symbols.pop() {
            self.closed_block_symbols.extend(scope);
        }
    }

    fn insert_symbol(&mut self, name: String, source: SymbolValue) {
        if let Some(scope) = self.symbols.last_mut() {
            scope.insert(name, source);
        }
    }

    fn assign_symbol(&mut self, name: String, source: SymbolValue) {
        if let Some(scope) = self
            .symbols
            .iter_mut()
            .rev()
            .find(|scope| scope.contains_key(&name))
        {
            scope.insert(name, source);
            return;
        }
        self.insert_symbol(name, source);
    }

    fn lookup_symbol(&self, name: &str) -> Option<SymbolValue> {
        self.symbols
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned())
            .or_else(|| self.closed_block_symbols.get(name).cloned())
    }
}

fn declared_event_anchors(ast: &BoardAst) -> HashSet<String> {
    fn visit_event(event: &EventBlock, anchors: &mut HashSet<String>) {
        if let Some(anchor) = &event.anchor {
            anchors.insert(anchor.clone());
        }
        visit_block(&event.body, anchors);
    }

    fn visit_block(block: &Block, anchors: &mut HashSet<String>) {
        for statement in &block.stmts {
            match statement {
                Stmt::Branch { arms, .. } => {
                    for arm in arms {
                        visit_block(&arm.body, anchors);
                    }
                }
                Stmt::Loop { body, .. } => visit_block(body, anchors),
                Stmt::Handler(event) => visit_event(event, anchors),
                Stmt::Let { .. }
                | Stmt::Call { .. }
                | Stmt::Assign { .. }
                | Stmt::FieldAssign { .. }
                | Stmt::LocalAlias { .. }
                | Stmt::Return { .. }
                | Stmt::Local(_)
                | Stmt::Comment(_) => {}
            }
        }
    }

    let mut anchors = HashSet::new();
    for event in &ast.events {
        visit_event(event, &mut anchors);
    }
    for function in &ast.functions {
        visit_block(&function.body, &mut anchors);
    }
    anchors
}

/// The first still-live execution statement in an authored event body. Pure aliases/comments are
/// skipped because an Event entry connects to the first node carrying an Execution input.
fn first_existing_exec_body_node<'a>(board: &'a Board, block: &Block) -> Option<&'a Node> {
    for statement in &block.stmts {
        let anchor = match statement {
            Stmt::Let { call, anchor, .. } | Stmt::Call { call, anchor } => {
                anchor.as_deref().or(call.anchor.as_deref())
            }
            Stmt::Branch { call, anchor, .. } | Stmt::Loop { call, anchor, .. } => {
                anchor.as_deref().or(call.anchor.as_deref())
            }
            Stmt::Assign { anchor, .. }
            | Stmt::FieldAssign { anchor, .. }
            | Stmt::LocalAlias { anchor, .. }
            | Stmt::Return { anchor, .. } => anchor.as_deref(),
            // A nested handler is an independent entry point, not part of this event's chain.
            Stmt::Handler(_) | Stmt::Local(_) | Stmt::Comment(_) => None,
        };
        if let Some(node) = anchor.and_then(|anchor| find_board_node(board, anchor))
            && exec_input_pin(node).is_some()
        {
            return Some(node);
        }
    }
    None
}

fn ast_has_unanchored_calls(ast: &BoardAst) -> bool {
    for ev in &ast.events {
        if ev.anchor.is_none() {
            return true;
        }
        if block_has_unanchored_calls(&ev.body) {
            return true;
        }
    }
    for f in &ast.functions {
        if f.anchor.is_none() {
            return true;
        }
        if block_has_unanchored_calls(&f.body) {
            return true;
        }
    }
    false
}

/// Empty event/function shells are registration/layer scaffolding, not workflow logic. Keep the
/// check syntactic and conservative: calls, assignments, control flow, handlers and returns may
/// all lower to real graph work; comments, typed locals and literal aliases cannot by themselves.
fn block_has_no_executable_statements(block: &Block) -> bool {
    block.stmts.iter().all(|stmt| match stmt {
        Stmt::Comment(_) | Stmt::Local(_) => true,
        Stmt::LocalAlias { value, .. } => !expr_has_unanchored_calls(value),
        Stmt::Let { .. }
        | Stmt::Call { .. }
        | Stmt::Assign { .. }
        | Stmt::FieldAssign { .. }
        | Stmt::Branch { .. }
        | Stmt::Loop { .. }
        | Stmt::Handler(_)
        | Stmt::Return { .. } => false,
    })
}

fn block_has_unanchored_calls(block: &Block) -> bool {
    block.stmts.iter().any(stmt_has_unanchored_calls)
}

fn stmt_has_unanchored_calls(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let { call, anchor, .. } | Stmt::Call { call, anchor } => {
            anchor.is_none() || call_args_have_unanchored_calls(call)
        }
        Stmt::Branch {
            call,
            condition,
            arms,
            anchor,
            ..
        } => {
            (anchor.is_none() && !is_placeholder_call(call))
                || call_args_have_unanchored_calls(call)
                || condition.as_ref().is_some_and(expr_has_unanchored_calls)
                || arms.iter().any(|arm| block_has_unanchored_calls(&arm.body))
        }
        Stmt::Loop {
            call, body, anchor, ..
        } => {
            anchor.is_none()
                || call_args_have_unanchored_calls(call)
                || block_has_unanchored_calls(body)
        }
        Stmt::Assign { value, anchor, .. }
        | Stmt::FieldAssign { value, anchor, .. }
        | Stmt::LocalAlias { value, anchor, .. } => {
            anchor.is_none() || expr_has_unanchored_calls(value)
        }
        Stmt::Handler(event) => event.anchor.is_none() || block_has_unanchored_calls(&event.body),
        Stmt::Return { values, .. } => values.iter().any(expr_has_unanchored_calls),
        Stmt::Local(_) | Stmt::Comment(_) => false,
    }
}

fn call_args_have_unanchored_calls(call: &Call) -> bool {
    call.args
        .iter()
        .any(|arg| expr_has_unanchored_calls(&arg.value))
}

fn expr_has_unanchored_calls(expr: &Expr) -> bool {
    match expr {
        Expr::Call(call) => call.anchor.is_none() || call_args_have_unanchored_calls(call),
        Expr::Field { base, .. } | Expr::Member { base, .. } => expr_has_unanchored_calls(base),
        Expr::Object(fields) => fields.iter().any(|f| expr_has_unanchored_calls(&f.value)),
        Expr::Array(items) => items.iter().any(expr_has_unanchored_calls),
        Expr::Index { base, index } => {
            expr_has_unanchored_calls(base) || expr_has_unanchored_calls(index)
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            expr_has_unanchored_calls(cond)
                || expr_has_unanchored_calls(then)
                || expr_has_unanchored_calls(otherwise)
        }
        Expr::Binary { lhs, rhs, .. } => {
            expr_has_unanchored_calls(lhs) || expr_has_unanchored_calls(rhs)
        }
        Expr::Ref(_) | Expr::Literal(_) => false,
    }
}

/// Synthesize the `struct_set` [`Call`] a `base.path = value` struct-field write
/// (`Stmt::FieldAssign`) expands to: `structSet({ structIn: base, field: "path", value })`.
///
/// Shared by the structural planner's `Stmt::FieldAssign` arm (which turns it into an `AddNode`)
/// and the reconcile collectors (which key it by anchor to drive config-edits and deletions) so
/// the two expansions can never drift.
fn field_assign_struct_set_call(
    base: &str,
    path: &str,
    value: &Expr,
    anchor: Option<&str>,
) -> Call {
    Call {
        node_type: String::new(),
        display: "structSet".to_string(),
        args: vec![
            Arg {
                name: "structIn".to_string(),
                value: Expr::Ref(base.to_string()),
            },
            Arg {
                name: "field".to_string(),
                value: Expr::Literal(Literal::String(path.to_string())),
            },
            Arg {
                name: "value".to_string(),
                value: value.clone(),
            },
        ],
        anchor: anchor.map(str::to_string),
    }
}

/// Materialize the synthesized `struct_set` [`Call`] for every *anchored* `Stmt::FieldAssign` in
/// `ast`, at any nesting depth. `FieldAssign` carries no `&Call`, so without this the config-edit
/// (`new_calls`) and deletion (`visible`) collectors would never see a `base.path = value` write's
/// underlying `struct_set` node. The returned owned calls back the `&Call` entries those maps key
/// by anchor. Anchorless writes are fresh (handled by the structural planner) and are skipped.
fn collect_anchored_field_assign_calls(ast: &BoardAst) -> Vec<Call> {
    let mut out = Vec::new();
    for ev in &ast.events {
        collect_field_assign_calls_in_block(&ev.body, &mut out);
    }
    for f in &ast.functions {
        collect_field_assign_calls_in_block(&f.body, &mut out);
    }
    out
}

fn collect_field_assign_calls_in_block(block: &Block, out: &mut Vec<Call>) {
    for stmt in &block.stmts {
        collect_field_assign_calls_in_stmt(stmt, out);
    }
}

fn collect_field_assign_calls_in_stmt(stmt: &Stmt, out: &mut Vec<Call>) {
    match stmt {
        Stmt::FieldAssign {
            base,
            path,
            value,
            anchor: Some(anchor),
        } => out.push(field_assign_struct_set_call(
            base,
            path,
            value,
            Some(anchor),
        )),
        Stmt::Branch { arms, .. } => {
            for arm in arms {
                collect_field_assign_calls_in_block(&arm.body, out);
            }
        }
        Stmt::Loop { body, .. } => collect_field_assign_calls_in_block(body, out),
        Stmt::Handler(event) => collect_field_assign_calls_in_block(&event.body, out),
        _ => {}
    }
}

/// Walk a `BoardAst`, recording every [`Call`] that carries an anchor, keyed by node id.
fn collect_calls<'a>(ast: &'a BoardAst, out: &mut HashMap<String, &'a Call>) {
    for ev in &ast.events {
        collect_block(&ev.body, out);
    }
    for f in &ast.functions {
        collect_block(&f.body, out);
    }
}

fn collect_block<'a>(block: &'a Block, out: &mut HashMap<String, &'a Call>) {
    for stmt in &block.stmts {
        collect_stmt(stmt, out);
    }
}

/// Walk only statement-owned calls. This is intentionally shallower than [`collect_calls`]:
/// inlined helper expressions are render conveniences, not standalone text-visible statements,
/// and must not be removed just because their anchors are absent from edited FlowScript.
fn collect_statement_calls<'a>(ast: &'a BoardAst, out: &mut HashMap<String, &'a Call>) {
    for ev in &ast.events {
        collect_statement_block(&ev.body, out);
    }
    for f in &ast.functions {
        collect_statement_block(&f.body, out);
    }
}

fn collect_statement_block<'a>(block: &'a Block, out: &mut HashMap<String, &'a Call>) {
    for stmt in &block.stmts {
        collect_statement_stmt(stmt, out);
    }
}

fn collect_statement_stmt<'a>(stmt: &'a Stmt, out: &mut HashMap<String, &'a Call>) {
    match stmt {
        Stmt::Let { call, anchor, .. } | Stmt::Call { call, anchor } => {
            collect_call_anchor_only(call, anchor.as_deref(), out)
        }
        Stmt::Branch {
            call, arms, anchor, ..
        } => {
            // Sugared boolean branches carry a placeholder call, but the statement anchor is
            // still the branch node itself — it must register as present or reconcile deletes
            // the branch node on every roundtrip.
            collect_call_anchor_only(call, anchor.as_deref(), out);
            for arm in arms {
                collect_statement_block(&arm.body, out);
            }
        }
        Stmt::Loop {
            call, body, anchor, ..
        } => {
            collect_call_anchor_only(call, anchor.as_deref(), out);
            collect_statement_block(body, out);
        }
        Stmt::Assign { value, anchor, .. } | Stmt::LocalAlias { value, anchor, .. } => {
            // The renderer prints only the STATEMENT anchor on assignments, so an assigned call
            // whose own anchor names a DIFFERENT node (a `variable_set` line inlining its
            // initializer call) is a render convenience: that anchor can never survive the text
            // round-trip, and keying deletions on it would remove the initializer node from
            // every unchanged apply.
            if let Some(anchor) = anchor.as_deref()
                && let Some((call, _)) = assigned_call_expr(value)
                && call
                    .anchor
                    .as_deref()
                    .is_none_or(|call_anchor| call_anchor == anchor)
            {
                collect_call_anchor_only(call, Some(anchor), out);
            }
        }
        Stmt::Handler(event) => collect_statement_block(&event.body, out),
        // A `base.path = value` write's own `struct_set` node has no `&Call` to key here; its
        // anchor is registered into `visible` separately from the owned arena built by
        // `collect_anchored_field_assign_calls`, so a deleted dot-form line still flags a removal.
        Stmt::FieldAssign { .. } | Stmt::Return { .. } | Stmt::Local(_) | Stmt::Comment(_) => {}
    }
}

fn collect_call_anchor_only<'a>(
    call: &'a Call,
    fallback_anchor: Option<&str>,
    out: &mut HashMap<String, &'a Call>,
) {
    if let Some(anchor) = call.anchor.as_deref().or(fallback_anchor) {
        out.insert(anchor.to_string(), call);
    }
}

fn collect_stmt<'a>(stmt: &'a Stmt, out: &mut HashMap<String, &'a Call>) {
    match stmt {
        Stmt::Let { call, anchor, .. } | Stmt::Call { call, anchor } => {
            collect_call_with_anchor(call, anchor.as_deref(), out)
        }
        Stmt::Branch {
            call, arms, anchor, ..
        } => {
            // Collect the anchor even for placeholder-call (sugared boolean) branches — the
            // statement anchor is the branch node itself; see collect_statement_stmt. A
            // condition-form `if (cond)` round-trip keeps its control_branch node this way
            // instead of being mistaken for a deletion.
            collect_call_with_anchor(call, anchor.as_deref(), out);
            for arm in arms {
                collect_block(&arm.body, out);
            }
        }
        Stmt::Loop {
            call, body, anchor, ..
        } => {
            collect_call_with_anchor(call, anchor.as_deref(), out);
            collect_block(body, out);
        }
        Stmt::Assign { value, anchor, .. } => {
            if let Some(anchor) = anchor.as_deref()
                && collect_assigned_call_with_anchor(value, anchor, out)
            {
                return;
            }
            collect_expr(value, out);
        }
        Stmt::LocalAlias { value, anchor, .. } => {
            if let Some(anchor) = anchor.as_deref()
                && collect_assigned_call_with_anchor(value, anchor, out)
            {
                return;
            }
            collect_expr(value, out);
        }
        // A `base.path = value` write's own `struct_set` has no `&Call` to key here; its anchor is
        // registered into `new_calls` separately (see `collect_anchored_field_assign_calls`). Still
        // walk the RHS so any anchored calls feeding `value` are tracked.
        Stmt::FieldAssign { value, .. } => collect_expr(value, out),
        Stmt::Return { values, .. } => {
            for v in values {
                collect_expr(v, out);
            }
        }
        Stmt::Handler(event) => collect_block(&event.body, out),
        Stmt::Local(_) | Stmt::Comment(_) => {}
    }
}

fn collect_call<'a>(call: &'a Call, out: &mut HashMap<String, &'a Call>) {
    collect_call_with_anchor(call, None, out);
}

fn collect_call_with_anchor<'a>(
    call: &'a Call,
    fallback_anchor: Option<&str>,
    out: &mut HashMap<String, &'a Call>,
) {
    if let Some(anchor) = call.anchor.as_deref().or(fallback_anchor) {
        out.insert(anchor.to_string(), call);
    }
    for arg in &call.args {
        collect_expr(&arg.value, out);
    }
}

fn collect_expr<'a>(expr: &'a Expr, out: &mut HashMap<String, &'a Call>) {
    match expr {
        Expr::Call(call) => collect_call(call, out),
        Expr::Field { base, .. } | Expr::Member { base, .. } => collect_expr(base, out),
        Expr::Object(fields) => {
            for f in fields {
                collect_expr(&f.value, out);
            }
        }
        Expr::Array(items) => {
            for it in items {
                collect_expr(it, out);
            }
        }
        Expr::Index { base, index } => {
            collect_expr(base, out);
            collect_expr(index, out);
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            collect_expr(cond, out);
            collect_expr(then, out);
            collect_expr(otherwise, out);
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_expr(lhs, out);
            collect_expr(rhs, out);
        }
        Expr::Ref(_) | Expr::Literal(_) => {}
    }
}

fn collect_assigned_call_with_anchor<'a>(
    expr: &'a Expr,
    anchor: &str,
    out: &mut HashMap<String, &'a Call>,
) -> bool {
    match expr {
        Expr::Field { base, .. } => {
            if let Expr::Call(call) = base.as_ref() {
                collect_call_with_anchor(call, Some(anchor), out);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn assigned_call_expr(expr: &Expr) -> Option<(&Call, Option<&str>)> {
    match expr {
        Expr::Call(call) => Some((call, None)),
        Expr::Field { base, pin } => {
            let Expr::Call(call) = base.as_ref() else {
                return None;
            };
            Some((call, Some(pin.as_str())))
        }
        _ => None,
    }
}

fn is_placeholder_call(call: &Call) -> bool {
    call.node_type.is_empty() && call.display.is_empty() && call.args.is_empty()
}

fn promoted_local_aliases(block: &Block) -> HashSet<String> {
    let local_names: HashSet<String> = block
        .stmts
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::LocalAlias { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();

    if local_names.is_empty() {
        return HashSet::new();
    }

    let mut promoted = HashSet::new();
    for stmt in &block.stmts {
        collect_nested_assignments_to(&local_names, stmt, false, &mut promoted);
    }
    promoted
}

fn collect_nested_assignments_to(
    local_names: &HashSet<String>,
    stmt: &Stmt,
    nested: bool,
    out: &mut HashSet<String>,
) {
    match stmt {
        Stmt::Assign { target, .. } if nested && local_names.contains(target) => {
            out.insert(target.clone());
        }
        Stmt::FieldAssign { base, .. } if nested && local_names.contains(base) => {
            out.insert(base.clone());
        }
        Stmt::Branch { arms, .. } => {
            for arm in arms {
                for stmt in &arm.body.stmts {
                    collect_nested_assignments_to(local_names, stmt, true, out);
                }
            }
        }
        Stmt::Loop { body, .. } => {
            for stmt in &body.stmts {
                collect_nested_assignments_to(local_names, stmt, true, out);
            }
        }
        Stmt::Handler(event) => {
            for stmt in &event.body.stmts {
                collect_nested_assignments_to(local_names, stmt, true, out);
            }
        }
        _ => {}
    }
}

fn infer_variable_types(value: Option<&flow_like_types::Value>) -> (String, String) {
    use flow_like_types::Value;
    match value {
        Some(Value::Array(items)) => {
            let data_type = items
                .iter()
                .find_map(value_data_type)
                .unwrap_or("Generic")
                .to_string();
            (data_type, "Array".to_string())
        }
        Some(value) => (
            value_data_type(value).unwrap_or("Generic").to_string(),
            "Normal".to_string(),
        ),
        None => ("Generic".to_string(), "Normal".to_string()),
    }
}

fn value_data_type(value: &flow_like_types::Value) -> Option<&'static str> {
    use flow_like_types::Value;
    match value {
        Value::String(_) => Some("String"),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some("Integer"),
        Value::Number(_) => Some("Float"),
        Value::Bool(_) => Some("Boolean"),
        Value::Object(_) => Some("Struct"),
        Value::Array(_) => Some("Generic"),
        Value::Null => None,
    }
}

/// Convenience: parse FlowScript text and reconcile it against `existing` in one step.
///
/// Parse errors are surfaced as a single diagnostic with no commands, so callers can render the
/// failure without applying a partial mutation.
pub fn reconcile_text(existing: &Board, text: &str) -> ReconcileResult {
    match flow_like_ast::parse(text) {
        Ok(ast) => reconcile(existing, &ast),
        Err(err) => ReconcileResult {
            commands: Vec::new(),
            corrections: Vec::new(),
            diagnostics: vec![format!(
                "FlowScript parse error at line {}, col {}: {}",
                err.line, err.col, err.message
            )],
        },
    }
}

/// Parse FlowScript text and reconcile it with catalog metadata, enabling new unanchored node
/// calls to be translated into reviewable board commands.
pub fn reconcile_text_with_catalog(
    existing: &Board,
    text: &str,
    catalog: &[NodeMetadata],
) -> ReconcileResult {
    match flow_like_ast::parse(text) {
        Ok(ast) => reconcile_with_catalog(existing, &ast, catalog),
        Err(err) => ReconcileResult {
            commands: Vec::new(),
            corrections: Vec::new(),
            diagnostics: vec![format!(
                "FlowScript parse error at line {}, col {}: {}",
                err.line, err.col, err.message
            )],
        },
    }
}

/// Like [`reconcile_text_with_catalog`] but with a dynamic-pin [`MetadataEnricher`] (see
/// [`reconcile_with_catalog_enriched`]).
pub fn reconcile_text_with_catalog_enriched(
    existing: &Board,
    text: &str,
    catalog: &[NodeMetadata],
    enricher: &MetadataEnricher,
) -> ReconcileResult {
    match flow_like_ast::parse(text) {
        Ok(ast) => reconcile_with_catalog_enriched(existing, &ast, catalog, enricher),
        Err(err) => ReconcileResult {
            commands: Vec::new(),
            corrections: Vec::new(),
            diagnostics: vec![format!(
                "FlowScript parse error at line {}, col {}: {}",
                err.line, err.col, err.message
            )],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::{
        Board, ExecutionMode, ExecutionStage, Layer, LayerCache, LayerCacheScope, LayerType,
    };

    fn exec_pin(name: &str, index: u16) -> ExecPinCandidate {
        ExecPinCandidate {
            name: name.to_string(),
            friendly_name: name.to_string(),
            index,
        }
    }

    #[test]
    fn exec_out_plus_error_continues_from_exec_out_without_a_hand_listed_policy() {
        // The catalog's dominant multi-output shape. Requiring an arm block for every one of these
        // made plain sequential persistence uncompilable unless the author hand-wired each call.
        let candidates = [exec_pin("exec_out", 0), exec_pin("error", 1)];
        for node_type in ["insert_local_db", "upsert_local_db", "a2ui_update_table"] {
            assert_eq!(
                default_exec_output_by_policy(node_type, &candidates),
                Some("exec_out".to_string()),
                "{node_type} should continue from its only forward output"
            );
        }
    }

    #[test]
    fn an_unanchored_function_updates_the_same_named_layer_instead_of_duplicating_it() {
        let mut board = empty_board();
        let layer = Layer::new(
            "layer-render".to_string(),
            "renderThreads".to_string(),
            LayerType::Function,
        );
        board.layers.insert(layer.id.clone(), layer);

        let result = reconcile_text_with_catalog(
            &board,
            "function renderThreads() {\n    logInfo({ message: \"threads\" })\n}\n",
            &[catalog_meta(
                "log_info",
                "Log Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            )],
        );

        assert!(
            !result
                .commands
                .iter()
                .any(|command| matches!(command, BoardCommand::CreateLayer { name, .. } if name == "renderThreads")),
            "a second layer named renderThreads would make the board's own readback invalid: {:?}",
            result.commands
        );
    }

    #[test]
    fn an_undeclared_boundary_schema_adopts_the_connected_contract() {
        // `function adventureDb(): (database: Struct)` — an untyped boundary pin carries no schema,
        // so an enforcing consumer has nothing to contradict and the handle may flow through.
        assert!(schema_constraints_are_compatible(
            "database",
            "Struct",
            "Normal",
            Some("{\"title\":\"LocalDatabase\"}"),
            true,
            "database",
            "Struct",
            "Normal",
            None,
            false,
            &HashMap::new(),
        ));
    }

    #[test]
    fn two_declared_schemas_must_still_match() {
        // findModel returns a `Bit`; embedDocument wants a loaded `CachedEmbeddingModel`. Both
        // sides declare a contract, so this stays a real error the author has to fix.
        assert!(!schema_constraints_are_compatible(
            "model",
            "Struct",
            "Normal",
            Some("{\"title\":\"CachedEmbeddingModel\"}"),
            true,
            "model",
            "Struct",
            "Normal",
            Some("{\"title\":\"Bit\"}"),
            false,
            &HashMap::new(),
        ));
    }

    #[test]
    fn non_canonical_forward_pins_are_still_never_guessed() {
        // Custom/package nodes keep the "do not guess" stance: only the catalog's own `exec_out`
        // convention is recognized, everything else needs an explicit policy or explicit arms.
        for forward in ["success", "exec_success", "next"] {
            assert_eq!(
                default_exec_output_by_policy(
                    "custom_split",
                    &[exec_pin(forward, 0), exec_pin("error", 1)]
                ),
                None,
                "{forward} must not be auto-wired"
            );
        }
    }

    #[test]
    fn two_real_outcomes_still_demand_explicit_arms() {
        let candidates = [
            exec_pin("exec_out", 0),
            exec_pin("empty", 1),
            exec_pin("error", 2),
        ];
        assert_eq!(
            default_exec_output_by_policy("vector_search_local_db", &candidates),
            None,
            "a genuine second outcome must not be auto-wired away"
        );
    }

    #[test]
    fn hand_listed_policies_still_win_over_the_general_rule() {
        let candidates = [
            exec_pin("exec_success", 0),
            exec_pin("exec_error", 1),
            exec_pin("exec_out", 2),
        ];
        assert_eq!(
            default_exec_output_by_policy("http_fetch", &candidates),
            Some("exec_success".to_string())
        );
    }

    use crate::flow::execution::LogLevel;
    use crate::flow::node::Node;
    use crate::flow::pin::{PinOptions, ValueType};
    use crate::flow::variable::{Variable, VariableType};
    use flow_like_storage::Path;
    use flow_like_types::tokio;
    use std::collections::HashMap;
    use std::time::SystemTime;

    fn empty_board() -> Board {
        Board {
            id: "board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
        }
    }

    fn connect(board: &mut Board, from_node: &str, from_pin: &str, to_node: &str, to_pin: &str) {
        crate::flow::board::commands::pins::connect_pins::connect_pins(
            board, from_node, from_pin, to_node, to_pin,
        )
        .expect("connect pins");
    }

    #[test]
    fn all_board_nodes_prefers_flat_nodes_and_orders_nested_fallbacks() {
        let mut board = empty_board();

        let mut canonical = Node::new("canonical", "Canonical", "", "test");
        canonical.id = "shared".to_string();
        board.nodes.insert(canonical.id.clone(), canonical.clone());

        let mut flat_last = Node::new("flat_last", "Flat Last", "", "test");
        flat_last.id = "z-flat".to_string();
        board.nodes.insert(flat_last.id.clone(), flat_last);

        let mut layer_z = Layer::new(
            "layer-z".to_string(),
            "Layer Z".to_string(),
            LayerType::Function,
        );
        let mut stale_mirror = canonical;
        stale_mirror.name = "stale_mirror".to_string();
        layer_z.nodes.insert(stale_mirror.id.clone(), stale_mirror);
        let mut fallback_z = Node::new("fallback_z", "Fallback Z", "", "test");
        fallback_z.id = "nested-only".to_string();
        layer_z.nodes.insert(fallback_z.id.clone(), fallback_z);

        let mut layer_a = Layer::new(
            "layer-a".to_string(),
            "Layer A".to_string(),
            LayerType::Function,
        );
        let mut fallback_a = Node::new("fallback_a", "Fallback A", "", "test");
        fallback_a.id = "nested-only".to_string();
        layer_a.nodes.insert(fallback_a.id.clone(), fallback_a);

        // Insert in reverse lexical order: fallback selection must depend on layer id, not the
        // backing HashMap's iteration order.
        board.layers.insert(layer_z.id.clone(), layer_z);
        board.layers.insert(layer_a.id.clone(), layer_a);

        let nodes = all_board_nodes(&board);
        assert_eq!(
            nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            vec!["nested-only", "shared", "z-flat"]
        );
        assert_eq!(
            nodes
                .iter()
                .find(|node| node.id == "shared")
                .map(|node| node.name.as_str()),
            Some("canonical"),
            "a mirrored legacy clone must not shadow the canonical flat node"
        );
        assert_eq!(
            nodes
                .iter()
                .find(|node| node.id == "nested-only")
                .map(|node| node.name.as_str()),
            Some("fallback_a"),
            "nested-only duplicates must choose the lexically first layer deterministically"
        );
    }

    #[test]
    fn board_index_prefers_flat_pins_and_keeps_nested_only_fallbacks() {
        let mut board = empty_board();

        let mut producer = Node::new("producer", "Producer", "", "test");
        producer.id = "producer".to_string();
        let producer_output = producer
            .add_output_pin(
                "canonical_value",
                "Canonical Value",
                "",
                VariableType::String,
            )
            .id
            .clone();

        let mut legacy = Node::new("legacy", "Legacy", "", "test");
        legacy.id = "legacy".to_string();
        let legacy_output = legacy
            .add_output_pin("legacy_value", "Legacy Value", "", VariableType::String)
            .id
            .clone();

        let mut consumer = Node::new("consumer", "Consumer", "", "test");
        consumer.id = "consumer".to_string();
        consumer
            .add_input_pin("flat_input", "Flat Input", "", VariableType::String)
            .depends_on
            .insert(producer_output.clone());
        consumer
            .add_input_pin("legacy_input", "Legacy Input", "", VariableType::String)
            .depends_on
            .insert(legacy_output);

        let mut stale_producer = producer.clone();
        stale_producer
            .pins
            .get_mut(&producer_output)
            .expect("mirrored output")
            .name = "stale_value".to_string();

        board.nodes.insert(producer.id.clone(), producer);
        board.nodes.insert(consumer.id.clone(), consumer);
        let mut layer = Layer::new(
            "function-layer".to_string(),
            "Function Layer".to_string(),
            LayerType::Function,
        );
        layer
            .nodes
            .insert(stale_producer.id.clone(), stale_producer);
        layer.nodes.insert(legacy.id.clone(), legacy);
        board.layers.insert(layer.id.clone(), layer);

        let index = BoardIndex::new(&board);
        let consumer = board.nodes.get("consumer").expect("consumer");
        let flat_source = index
            .data_source_for_input(consumer, "flat_input")
            .expect("flat source");
        assert_eq!(flat_source.node.node_ref(), "producer");
        assert_eq!(
            flat_source.output_pin.as_deref(),
            Some("canonical_value"),
            "the nested stale pin name must not shadow canonical flat metadata"
        );

        let legacy_source = index
            .data_source_for_input(consumer, "legacy_input")
            .expect("legacy nested-only source");
        assert_eq!(legacy_source.node.node_ref(), "legacy");
        assert_eq!(legacy_source.output_pin.as_deref(), Some("legacy_value"));
    }

    /// Build an `event → log` board where the log node carries a literal `text` default.
    fn board_with_log(text_default: &str) -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let exec_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let exec_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let text_pin = log.add_input_pin("text", "Text", "", VariableType::String);
        text_pin.default_value = Some(format!("\"{text_default}\"").into_bytes());
        board.nodes.insert(log.id.clone(), log);

        connect(&mut board, "event", &exec_out, "log", &exec_in);
        board
    }

    fn board_with_variable() -> Board {
        let mut board = empty_board();
        let mut variable = Variable::new("apiKey", VariableType::String, ValueType::Normal);
        variable.id = "var_api".to_string();
        variable.description = Some("API key".to_string());
        variable.set_default_value(flow_like_types::Value::String("old".to_string()));
        board.variables.insert(variable.id.clone(), variable);
        board
    }

    fn board_with_secret_variable() -> Board {
        let mut board = board_with_variable();
        board
            .variables
            .get_mut("var_api")
            .expect("secret variable")
            .secret = true;
        board
    }

    fn board_with_secret_struct_variable() -> Board {
        let mut board = empty_board();
        let mut variable = Variable::new("apiKey", VariableType::Struct, ValueType::Normal);
        variable.id = "var_api".to_string();
        variable.secret = true;
        variable.schema = Some(
            r#"{"type":"object","properties":{"token":{"type":"string"}},"required":["token"]}"#
                .to_string(),
        );
        variable.set_default_value(
            flow_like_types::json::from_str(r#"{"token":"hidden"}"#).expect("valid secret object"),
        );
        board.variables.insert(variable.id.clone(), variable);
        board
    }

    /// Finds the anchored `log` call in a lowered AST and overwrites its `text` literal.
    fn set_text_arg(ast: &mut BoardAst, value: &str) {
        for ev in &mut ast.events {
            for stmt in &mut ev.body.stmts {
                if let Stmt::Call { call, .. } = stmt
                    && call.anchor.as_deref() == Some("log")
                {
                    for arg in &mut call.args {
                        if arg.name == "text" {
                            arg.value = Expr::Literal(Literal::String(value.to_string()));
                        }
                    }
                }
            }
        }
    }

    /// Build an `event → control_branch(true) → log` board (condition-form `if` sugar).
    fn board_with_condition_branch() -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut branch = Node::new("control_branch", "Branch", "", "control");
        branch.id = "branch".to_string();
        let branch_in = branch
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let condition = branch.add_input_pin("condition", "Condition", "", VariableType::Boolean);
        condition.default_value = Some(b"true".to_vec());
        let branch_true = branch
            .add_output_pin("true", "True", "", VariableType::Execution)
            .id
            .clone();
        branch.add_output_pin("false", "False", "", VariableType::Execution);
        board.nodes.insert(branch.id.clone(), branch);

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let log_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        log.add_input_pin("text", "Text", "", VariableType::String);
        board.nodes.insert(log.id.clone(), log);

        connect(&mut board, "event", &event_out, "branch", &branch_in);
        connect(&mut board, "branch", &branch_true, "log", &log_in);
        board
    }

    /// Build an `event → single_choice` board whose two same-named `options` input pins
    /// hold different literal defaults (the dynamic multi-pin pattern). Returns the board
    /// plus the two option-pin ids in index order.
    fn board_with_multi_pin_node() -> (Board, String, String) {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut choice = Node::new("single_choice", "Single Choice", "", "interaction");
        choice.id = "choice".to_string();
        let choice_in = choice
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        choice.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let first = choice.add_input_pin("options", "Options", "", VariableType::String);
        first.default_value = Some(b"\"a\"".to_vec());
        let first_id = first.id.clone();
        let second = choice.add_input_pin("options", "Options", "", VariableType::String);
        second.default_value = Some(b"\"b\"".to_vec());
        let second_id = second.id.clone();
        board.nodes.insert(choice.id.clone(), choice);

        connect(&mut board, "event", &event_out, "choice", &choice_in);
        (board, first_id, second_id)
    }

    fn anchored_text(board: &Board) -> String {
        super::super::board_to_flowscript(
            board,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
    }

    #[test]
    fn unchanged_multi_pin_roundtrip_emits_nothing() {
        let (board, _, _) = board_with_multi_pin_node();
        let text = anchored_text(&board);
        let result = reconcile_text(&board, &text);
        assert!(
            result.commands.is_empty(),
            "no-op multi-pin round-trip must be empty; got {:?} from text:\n{text}",
            result.commands
        );
    }

    #[test]
    fn multi_pin_edit_targets_the_right_pin_by_id() {
        let (board, _first_id, second_id) = board_with_multi_pin_node();
        let text = anchored_text(&board).replace("options: \"b\"", "options: \"c\"");
        let result = reconcile_text(&board, &text);
        let updates: Vec<_> = result
            .commands
            .iter()
            .filter_map(|c| match c {
                BoardCommand::UpdateNodePin { pin_id, value, .. } => {
                    Some((pin_id.clone(), value.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            updates,
            vec![(second_id, flow_like_types::Value::String("c".to_string()))],
            "editing the second occurrence must update exactly the second pin, by id"
        );
    }

    #[test]
    fn new_multi_pin_call_emits_stable_occurrence_refs() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "equal_string",
                "Equal String",
                vec![
                    pin_meta("string", "String", PinType::Input),
                    pin_meta("string", "String", PinType::Input),
                ],
                vec![pin_meta("result", "Boolean", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            "eventsSimple() {\n    const matches = equalString({ string: \"sender\", string: \"example@example.com\" })\n}\n",
            &catalog,
        );

        assert!(
            result.diagnostics.is_empty(),
            "same-named inputs on a new node must reconcile cleanly: {:?}",
            result.diagnostics
        );
        let updates = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::UpdateNodePin { pin_id, value, .. } => {
                    Some((pin_id.as_str(), value.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            updates,
            vec![
                (
                    "string[#1]",
                    flow_like_types::Value::String("sender".to_string())
                ),
                (
                    "string[#2]",
                    flow_like_types::Value::String("example@example.com".to_string())
                ),
            ]
        );
    }

    #[test]
    fn string_replace_regex_alias_targets_canonical_pin_and_reports_correction() {
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "string_replace",
                "String Replace",
                vec![
                    pin_meta("string", "String", PinType::Input),
                    pin_meta("pattern", "String", PinType::Input),
                    pin_meta("replacement", "String", PinType::Input),
                    pin_meta("is_regex", "Boolean", PinType::Input),
                ],
                vec![pin_meta("string", "String", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const replaced = stringReplace({ string: "abc", pattern: "a", replacement: "z", regex: true })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { pin_id, value, .. }
                if pin_id == "is_regex" && value == &flow_like_types::Value::Bool(true)
        )));
        assert_eq!(
            result.corrections,
            vec!["Auto-corrected `stringReplace` argument `regex` to `isRegex`.".to_string()]
        );
    }

    #[test]
    fn bool_or_a_b_aliases_target_distinct_boolean_occurrences() {
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "bool_or",
                "Boolean Or",
                vec![
                    pin_meta("boolean", "Boolean", PinType::Input),
                    pin_meta("boolean", "Boolean", PinType::Input),
                ],
                vec![pin_meta("result", "Boolean", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &empty_board(),
            "eventsSimple() {\n    const either = boolOr({ a: true, b: false })\n}\n",
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let updates = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::UpdateNodePin { pin_id, value, .. }
                    if pin_id.starts_with("boolean[#") =>
                {
                    Some((pin_id.as_str(), value.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            updates,
            vec![
                ("boolean[#1]", flow_like_types::Value::Bool(true)),
                ("boolean[#2]", flow_like_types::Value::Bool(false)),
            ]
        );
        assert_eq!(result.corrections.len(), 2);
        assert!(
            result
                .corrections
                .iter()
                .any(|correction| correction.contains("`a` to `boolean` (occurrence 1 of 2)"))
        );
        assert!(
            result
                .corrections
                .iter()
                .any(|correction| correction.contains("`b` to `boolean` (occurrence 2 of 2)"))
        );
    }

    fn smtp_send_catalog() -> Vec<NodeMetadata> {
        let mut send = catalog_meta(
            "email_smtp_send",
            "Send Email",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("connection", "Struct", PinType::Input),
                pin_meta("from", "String", PinType::Input),
                pin_meta("to", "String", PinType::Input),
                pin_meta("subject", "String", PinType::Input),
                pin_meta("body_text", "String", PinType::Input),
            ],
            vec![
                pin_meta("exec_out", "Execution", PinType::Output),
                pin_meta("message_id", "String", PinType::Output),
            ],
        );
        send.required_inputs = vec!["connection".into(), "from".into(), "to".into()];
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            send,
        ]
    }

    #[test]
    fn smtp_send_legacy_name_is_repaired_only_for_a_compatible_complete_call() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    emailSmtpSendMail({ connection: {}, from: "support@example.com", to: "user@example.com", bodyText: "hello" })
}
"#,
            &smtp_send_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "email_smtp_send"
        )));
        assert!(result.corrections.iter().any(|correction| correction
            == "Auto-corrected FlowScript call `emailSmtpSendMail` to `emailSmtpSend`."));
    }

    #[test]
    fn smtp_send_legacy_name_with_incompatible_shape_is_not_partially_repaired() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    emailSmtpSendMail({ email: {}, to: "user@example.com", subject: "Hello", body: "text" })
}
"#,
            &smtp_send_catalog(),
        );

        assert!(result.corrections.is_empty());
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "email_smtp_send"
        )));
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.contains("emailSmtpSendMail"))
            .expect("actionable legacy-name diagnostic");
        assert!(diagnostic.contains("emailSmtpSend"));
        assert!(diagnostic.contains("not auto-corrected"));
        assert!(diagnostic.contains("connection"));
        assert!(diagnostic.contains("from"));
        assert!(diagnostic.contains("bodyText"));
    }

    #[test]
    fn imap_mailbox_fetch_misuse_gets_one_structural_replacement_diagnostic() {
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "email_imap_inbox_fetch_mail",
                "Fetch Mail",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("email_ref", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("email", "Struct", PinType::Output),
                ],
            ),
        ];
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const fetched = emailImapInboxFetchMail({ email: {}, unseenOnly: true, markSeen: true })
}
"#,
            &catalog,
        );

        assert!(result.corrections.is_empty());
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. }
                if node_type == "email_imap_inbox_fetch_mail"
        )));
        let matching = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.contains("emailImapInboxFetchMail"))
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "{:?}", result.diagnostics);
        assert!(matching[0].contains("accepts exactly `emailRef`"));
        assert!(matching[0].contains("mailImapList"));
        assert!(matching[0].contains("inbox: inbox"));
        assert!(matching[0].contains("controlForEach({ array: refs })"));
        assert!(matching[0].contains("emailRef: item.value"));
        assert!(matching[0].contains("emailGetContent"));
        assert!(matching[0].contains("email: email"));
        assert!(matching[0].contains("emailGetHeaders"));
        assert!(matching[0].contains("mailAddressFields"));
        assert!(matching[0].contains("address: headers.from"));
        assert!(matching[0].contains("emailImapMarkSeen"));
        assert!(matching[0].contains("email: item.value"));
    }

    #[test]
    fn unchanged_condition_branch_roundtrip_emits_no_removals() {
        // `if (cond)` sugar parses with a placeholder call; its anchor must still count as
        // present, otherwise a no-op round-trip deletes the control_branch node.
        let board = board_with_condition_branch();
        let text = super::super::board_to_flowscript(
            &board,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );
        let result = reconcile_text(&board, &text);
        let removals: Vec<_> = result
            .commands
            .iter()
            .filter(|c| matches!(c, BoardCommand::RemoveNode { .. }))
            .collect();
        assert!(
            removals.is_empty(),
            "no-op round-trip must not delete nodes; got {removals:?} from text:\n{text}"
        );
    }

    #[test]
    fn literal_edit_emits_update_pin() {
        let board = board_with_log("hello");
        let mut ast = super::super::lower_to_ast(&board);
        set_text_arg(&mut ast, "world");

        let result = reconcile(&board, &ast);

        let updates: Vec<_> = result
            .commands
            .iter()
            .filter_map(|c| match c {
                BoardCommand::UpdateNodePin {
                    node_id,
                    pin_id,
                    value,
                    ..
                } => Some((node_id.as_str(), pin_id.as_str(), value.clone())),
                _ => None,
            })
            .collect();

        assert_eq!(
            updates,
            vec![(
                "log",
                "text",
                flow_like_types::Value::String("world".into())
            )],
            "changed literal should emit exactly one UpdateNodePin"
        );
    }

    #[test]
    fn unchanged_literal_emits_nothing() {
        let board = board_with_log("hello");
        let ast = super::super::lower_to_ast(&board);

        let result = reconcile(&board, &ast);

        assert!(
            result.commands.is_empty(),
            "round-tripping the board's own AST must be a no-op, got {:?}",
            result.commands
        );
    }

    #[test]
    fn unchanged_variable_roundtrip_emits_nothing() {
        let board = board_with_variable();
        let text =
            super::super::board_to_flowscript(&board, &flow_like_ast::RenderOptions::default());

        let result = reconcile_text(&board, &text);

        assert!(
            result.commands.is_empty(),
            "round-tripping variable declarations must be a no-op, got {:?}",
            result.commands
        );
    }

    #[test]
    fn new_variable_emits_create_variable() {
        let board = empty_board();
        let ast = flow_like_ast::parse("const apiKey: string = \"secret\"\n").expect("parse");

        let result = reconcile(&board, &ast);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::CreateVariable {
                name,
                data_type,
                value_type,
                default_value: Some(value),
                ..
            } if name == "apiKey"
                && data_type == "String"
                && value_type == "Normal"
                && value == &flow_like_types::Value::String("secret".to_string())
        ));
    }

    #[test]
    fn new_secret_variable_accepts_empty_placeholder_without_storing_it() {
        let ast = flow_like_ast::parse("@secret\nconst apiKey: string = \"\"\n").expect("parse");

        let result = reconcile(&empty_board(), &ast);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::CreateVariable {
                name,
                default_value: None,
                secret: Some(true),
                ..
            } if name == "apiKey"
        ));
    }

    #[test]
    fn new_secret_variable_rejects_nonempty_flowscript_default_without_echoing_it() {
        let ast =
            flow_like_ast::parse("@secret\nconst apiKey: string = \"model-authored-credential\"\n")
                .expect("parse");

        let result = reconcile(&empty_board(), &ast);

        assert!(result.commands.is_empty(), "{:?}", result.commands);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("cannot take a non-empty FlowScript-authored default")
        }));
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.contains("model-authored-credential"))
        );
    }

    #[test]
    fn secret_to_secret_metadata_update_never_writes_or_clears_hidden_default() {
        let board = board_with_secret_variable();
        let ast = flow_like_ast::parse(
            r#"@description("Updated description")
@secret
const apiKey: string = ""   //@v:var_api
"#,
        )
        .expect("parse");

        let result = reconcile(&board, &ast);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::UpdateVariable {
                variable_id,
                default_value: None,
                clear_default_value: false,
                description: Some(description),
                secret: None,
                ..
            } if variable_id == "var_api" && description == "Updated description"
        ));
    }

    #[test]
    fn existing_secret_variable_rejects_nonempty_flowscript_default() {
        let board = board_with_secret_variable();
        let ast = flow_like_ast::parse(
            r#"@description("API key")
@secret
const apiKey: string = "model-authored-replacement"   //@v:var_api
"#,
        )
        .expect("parse");

        let result = reconcile(&board, &ast);

        assert!(result.commands.is_empty(), "{:?}", result.commands);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("cannot take a non-empty FlowScript-authored default")
        }));
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.contains("model-authored-replacement"))
        );
    }

    #[test]
    fn nonsecret_to_secret_update_preserves_existing_default_with_empty_placeholder() {
        let board = board_with_variable();
        let ast = flow_like_ast::parse(
            r#"@description("API key")
@secret
const apiKey: string = ""   //@v:var_api
"#,
        )
        .expect("parse");

        let result = reconcile(&board, &ast);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::UpdateVariable {
                variable_id,
                default_value: None,
                clear_default_value: false,
                secret: Some(true),
                ..
            } if variable_id == "var_api"
        ));
    }

    #[test]
    fn secret_declassification_atomically_clears_hidden_default() {
        let board = board_with_secret_variable();
        let ast = flow_like_ast::parse(
            r#"@description("API key")
const apiKey: string   //@v:var_api
"#,
        )
        .expect("parse");

        let result = reconcile(&board, &ast);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::UpdateVariable {
                variable_id,
                default_value: None,
                clear_default_value: true,
                secret: Some(false),
                ..
            } if variable_id == "var_api"
        ));
    }

    #[test]
    fn secret_declassification_with_model_default_is_rejected() {
        let board = board_with_secret_variable();
        let ast = flow_like_ast::parse(
            r#"@description("API key")
const apiKey: string = "model-authored-replacement"   //@v:var_api
"#,
        )
        .expect("parse");

        let result = reconcile(&board, &ast);

        assert!(result.commands.is_empty(), "{:?}", result.commands);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("cannot be declassified with a FlowScript-authored default")
        }));
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.contains("model-authored-replacement")),
            "security diagnostics must not echo an authored credential"
        );
    }

    #[test]
    fn secret_without_hidden_default_can_be_declassified_with_ordinary_default() {
        let mut board = board_with_secret_variable();
        board
            .variables
            .get_mut("var_api")
            .expect("secret variable")
            .default_value = None;
        let ast = flow_like_ast::parse(
            r#"@description("API key")
const apiKey: string = "ordinary-default"   //@v:var_api
"#,
        )
        .expect("parse");

        let result = reconcile(&board, &ast);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::UpdateVariable {
                variable_id,
                default_value: Some(flow_like_types::Value::String(value)),
                clear_default_value: false,
                secret: Some(false),
                ..
            } if variable_id == "var_api" && value == "ordinary-default"
        ));
    }

    #[test]
    fn secret_type_or_value_shape_edit_with_hidden_default_is_rejected() {
        for source in [
            "@description(\"API key\")\n@secret\nconst apiKey: int   //@v:var_api\n",
            "@description(\"API key\")\n@secret\nconst apiKey: string[]   //@v:var_api\n",
        ] {
            let board = board_with_secret_variable();
            let ast = flow_like_ast::parse(source).expect("parse");

            let result = reconcile(&board, &ast);

            assert!(result.commands.is_empty(), "{:?}", result.commands);
            assert!(result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("type/value shape/schema cannot be changed")
            }));
        }
    }

    #[test]
    fn secret_schema_edit_with_hidden_default_is_rejected() {
        let board = board_with_secret_struct_variable();
        let mut ast = super::super::lower_to_ast(&board);
        ast.variables[0].schema = Some(
            r#"{"type":"object","properties":{"token":{"type":"integer"}},"required":["token"]}"#
                .to_string(),
        );
        ast.interfaces = flow_like_ast::interfaces_for_variables(&ast.variables);

        let result = reconcile(&board, &ast);

        assert!(result.commands.is_empty(), "{:?}", result.commands);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("type/value shape/schema cannot be changed")
        }));
    }

    #[test]
    fn duplicate_raw_anchors_fail_closed_before_deriving_commands() {
        let board = empty_board();
        let mut ast =
            flow_like_ast::parse("const first: string = \"one\"\nconst second: string = \"two\"\n")
                .expect("parse");
        ast.variables[0].anchor = Some("same-variable".to_string());
        ast.variables[1].anchor = Some("same-variable".to_string());

        let result = reconcile(&board, &ast);

        assert!(result.commands.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert!(result.diagnostics[0].contains("duplicate FlowScript anchor"));
    }

    /// The lowerer renders an entry node with multiple exec outputs as its EventBlock header
    /// PLUS an immediate arm-routing branch carrying the same anchor. That pair is one entity
    /// and must not trip the duplicate-anchor preflight; the same anchor reappearing anywhere
    /// else stays a duplicate.
    #[test]
    fn entry_arm_routing_branch_shares_the_event_anchor_without_tripping_preflight() {
        let arm_routing = "eventsSimple() {   //@n:entrynode\n    if (eventsSimple()) { // exec_out   //@n:entrynode\n        logInfo({ message: \"hi\" })\n    }\n}\n";
        let ast = flow_like_ast::parse(arm_routing).expect("parse arm-routing form");
        assert!(
            duplicate_ast_anchors(&ast).is_empty(),
            "the entry's own arm-routing branch must not count as a duplicate anchor"
        );

        let elsewhere = "eventsSimple() {   //@n:entrynode\n    logInfo({ message: \"hi\" })   //@n:entrynode\n}\n";
        let ast = flow_like_ast::parse(elsewhere).expect("parse misuse form");
        assert_eq!(
            duplicate_ast_anchors(&ast),
            vec!["entrynode".to_string()],
            "a non-first-branch reuse of the entry anchor stays a duplicate"
        );
    }

    #[test]
    fn duplicate_variables_fail_closed_before_colliding_create_commands() {
        let ast = flow_like_ast::parse(
            "const ticket: string = \"first\"\nconst ticket: string = \"second\"\n",
        )
        .expect("parse");

        let result = reconcile(&empty_board(), &ast);

        assert!(result.commands.is_empty());
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("duplicate FlowScript variable declaration `ticket`")
        }));
    }

    #[test]
    fn duplicate_functions_fail_closed_without_orphaning_a_layer() {
        let ast = flow_like_ast::parse(
            r#"const untouched: string = "value"

function helper() {
}

function helper() {
}
"#,
        )
        .expect("parse");

        let result = reconcile_with_catalog(&empty_board(), &ast, &[]);

        assert!(
            result.commands.is_empty(),
            "the unrelated variable must not be created when declaration preflight fails"
        );
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("duplicate FlowScript function declaration `helper`")
        }));
    }

    #[test]
    fn duplicate_named_events_fail_closed_before_function_refs_become_ambiguous() {
        let ast = flow_like_ast::parse(
            r#"const untouched: string = "value"

eventsGeneric deleteAdventure(payload: Struct, adventureId: string) {
}

eventsWidgetAction deleteAdventure(widgetInstanceId: string, eventName: string, actionContext: Struct, inputValues: Struct) {
}

eventsSimple menuPageLoad() {
    a2uiInstantiateWidget({ widgetSelector: "adventure-card", instanceId: "card", fnRefs: [deleteAdventure] })
}
"#,
        )
        .expect("parse");

        let result = reconcile_with_catalog(&empty_board(), &ast, &[]);

        assert!(
            result.commands.is_empty(),
            "an ambiguous callable name must reject the whole document before command derivation: {:?}",
            result.commands
        );
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.contains("callable name `deleteAdventure` is ambiguous"))
            .expect("duplicate named-event diagnostic");
        assert!(diagnostic.contains("`eventsGeneric`"), "{diagnostic}");
        assert!(diagnostic.contains("`eventsWidgetAction`"), "{diagnostic}");
        assert!(
            diagnostic.contains("give each callable a unique name"),
            "{diagnostic}"
        );
    }

    #[test]
    fn duplicate_anchored_event_aliases_remain_valid_for_persisted_round_trips() {
        let ast = flow_like_ast::parse(
            r#"eventStart event() {   //@n:event_a
}

eventTimer event() {   //@n:event_b
}
"#,
        )
        .expect("parse");

        let diagnostics = duplicate_ast_declaration_diagnostics(&ast);
        assert!(
            diagnostics.is_empty(),
            "persisted entries may share a friendly alias when no new callable must resolve it: {diagnostics:?}"
        );
    }

    #[test]
    fn function_and_named_event_collision_fail_closed_in_the_shared_resolver_namespace() {
        let ast = flow_like_ast::parse(
            r#"function deleteAdventure() {
}

eventsGeneric deleteAdventure(payload: Struct) {
}
"#,
        )
        .expect("parse");

        let result = reconcile_with_catalog(&empty_board(), &ast, &[]);

        assert!(result.commands.is_empty());
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.contains("callable name `deleteAdventure` is ambiguous"))
            .expect("cross-category callable diagnostic");
        assert!(
            diagnostic.contains("a function and 1 named event"),
            "{diagnostic}"
        );
        assert!(
            diagnostic.contains("share the apply resolver namespace"),
            "{diagnostic}"
        );
    }

    #[test]
    fn unique_function_and_named_event_callables_pass_declaration_preflight() {
        let ast = flow_like_ast::parse(
            r#"function eraseAdventure() {
}

eventsGeneric deleteAdventure(payload: Struct) {
}

eventsWidgetAction deleteAdventureCard(widgetInstanceId: string) {
}
"#,
        )
        .expect("parse");

        let diagnostics = duplicate_ast_declaration_diagnostics(&ast);
        assert!(
            diagnostics.is_empty(),
            "unique callable names must remain valid: {diagnostics:?}"
        );
    }

    #[test]
    fn duplicate_interface_fields_and_parameters_are_rejected() {
        let ast = flow_like_ast::parse(
            r#"interface Ticket {
    id: string;
    id: string;
}

function helper(ticket: string, ticket: string): (ticket: string) {
}
"#,
        )
        .expect("parse");

        let result = reconcile(&empty_board(), &ast);

        assert!(result.commands.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("duplicate FlowScript interface field `id`"))
        );
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("duplicate FlowScript function parameter `ticket`")
        }));
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("function boundary name `ticket`")
                && diagnostic.contains("both a parameter and return")
        }));
    }

    #[test]
    fn normalized_function_boundary_name_collisions_are_rejected() {
        let ast = flow_like_ast::parse(
            r#"function helper(foo_bar: string, fooBar: string): (result_value: string, resultValue: string) {
}
"#,
        )
        .expect("parse");

        let result = reconcile(&empty_board(), &ast);
        assert!(result.commands.is_empty());
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("function parameter names")
                && diagnostic.contains("normalized boundary name `fooBar`")
        }));
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("function return names")
                && diagnostic.contains("normalized boundary name `resultValue`")
        }));
    }

    #[test]
    fn anchored_call_display_cannot_silently_reuse_a_different_node_type() {
        let mut board = empty_board();
        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let mut log = Node::new("log_info", "Log Info", "", "debug");
        log.id = "log".to_string();
        log.add_input_pin("exec_in", "In", "", VariableType::Execution);
        board.nodes.insert(log.id.clone(), log);

        let ast = flow_like_ast::parse(
            "simpleEvent() {   //@n:event\n    mailImapConnect()   //@n:log\n}\n",
        )
        .expect("parse");
        let result = reconcile_with_catalog(&board, &ast, &[]);

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("call `mailImapConnect` keeps anchor `log`")
        }));
    }

    #[test]
    fn anchored_event_display_cannot_silently_reuse_a_different_event_type() {
        let mut board = empty_board();
        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);
        let ast = flow_like_ast::parse("mailEvent() {   //@n:event\n}\n").expect("parse");

        let result = reconcile_with_catalog(&board, &ast, &[]);

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("event `mailEvent` keeps anchor `event`")
            })
        );
    }

    #[test]
    fn anchored_function_call_cannot_silently_keep_the_old_target() {
        let mut board = empty_board();
        for (id, name) in [("layer-a", "Helper A"), ("layer-b", "Helper B")] {
            let layer = Layer::new(id.to_string(), name.to_string(), LayerType::Function);
            board.layers.insert(layer.id.clone(), layer);
        }

        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let mut call = Node::new(CALL_FUNCTION_NODE_TYPE, "Call Function", "", "control");
        call.id = "call".to_string();
        call.add_input_pin("exec_in", "In", "", VariableType::Execution);
        call.add_input_pin(FUNCTION_LAYER_ID_PIN, "Function", "", VariableType::String)
            .default_value = Some(b"\"layer-a\"".to_vec());
        call.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(call.id.clone(), call);

        let ast = flow_like_ast::parse(
            r#"function helperA() {   //@l:layer-a
}

function helperB() {   //@l:layer-b
}

simpleEvent() {   //@n:event
    helperB()   //@n:call
}
"#,
        )
        .expect("parse");

        let result = reconcile_with_catalog(&board, &ast, &[]);

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("function call `helperB` keeps anchor `call`")
        }));
    }

    #[test]
    fn typed_call_identity_does_not_fall_back_to_a_matching_display() {
        let mut ast =
            flow_like_ast::parse("eventsSimple() {\n    logInfo({ message: \"hi\" })\n}\n")
                .expect("parse");
        let Stmt::Call { call, .. } = &mut ast.events[0].body.stmts[0] else {
            panic!("expected call")
        };
        call.node_type = "missing_exact_log_type".to_string();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log_info",
                "Log Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![],
            ),
        ];

        let result = reconcile_with_catalog(&empty_board(), &ast, &catalog);

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("exact node_type `missing_exact_log_type`")
            })
        );
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "log_info"
        )));
    }

    #[test]
    fn changed_variable_default_emits_update_variable() {
        let board = board_with_variable();
        let text = super::super::board_to_flowscript(
            &board,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
        .replace("\"old\"", "\"new\"");

        let result = reconcile_text(&board, &text);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::UpdateVariable {
                variable_id,
                default_value: Some(value),
                ..
            } if variable_id == "var_api"
                && value == &flow_like_types::Value::String("new".to_string())
        ));
    }

    #[test]
    fn removed_variable_emits_delete_variable() {
        let board = board_with_variable();
        let ast = BoardAst::default();

        let result = reconcile(&board, &ast);

        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::RemoveVariable { variable_id, .. } if variable_id == "var_api"
        ));
    }

    #[test]
    fn additive_reconcile_preserves_unmentioned_existing_nodes_and_variables() {
        let variable_board = board_with_variable();
        let variable_result = reconcile_with_catalog_mode(
            &variable_board,
            &BoardAst::default(),
            &[],
            ReconcileMode::Additive,
        );
        assert!(
            variable_result.commands.is_empty(),
            "additive typed documents must not delete omitted variables"
        );

        let node_board = board_with_log("hello");
        let node_result = reconcile_with_catalog_mode(
            &node_board,
            &BoardAst::default(),
            &[],
            ReconcileMode::Additive,
        );
        assert!(
            node_result
                .commands
                .iter()
                .all(|command| !matches!(command, BoardCommand::RemoveNode { .. })),
            "additive typed documents must not delete omitted nodes"
        );
    }

    #[test]
    fn additive_unanchored_variable_collision_cannot_mutate_existing_configuration() {
        let board = board_with_variable();
        let ast = flow_like_ast::parse("const apiKey: string = \"new\"\n").expect("parse");

        let result = reconcile_with_catalog_mode(&board, &ast, &[], ReconcileMode::Additive);

        assert!(result.commands.is_empty(), "got {:?}", result.commands);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("omits its exact anchor")),
            "got {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn interface_schema_roundtrip_emits_no_variable_update() {
        let mut board = empty_board();
        let mut variable = Variable::new("record", VariableType::Struct, ValueType::Normal);
        variable.id = "var_record".to_string();
        variable.schema = Some(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"Record","type":"object","properties":{"name":{"type":"string"},"tags":{"type":["array","null"],"items":{"type":"string"}}},"required":["name"]}"#
                .to_string(),
        );
        variable.set_default_value(flow_like_types::Value::Object(Default::default()));
        board.variables.insert(variable.id.clone(), variable);

        let text =
            super::super::board_to_flowscript(&board, &flow_like_ast::RenderOptions::default());
        let result = reconcile_text(&board, &text);

        assert!(
            result.commands.is_empty(),
            "interface schema text roundtrip must not rewrite the original board schema, got {:?}",
            result.commands
        );
    }

    #[test]
    fn missing_anchored_node_is_removed() {
        let board = board_with_log("hello");
        let mut ast = super::super::lower_to_ast(&board);
        // Drop the log statement from the event body to simulate a deletion in the text.
        for ev in &mut ast.events {
            ev.body.stmts.retain(
                |s| !matches!(s, Stmt::Call { call, .. } if call.anchor.as_deref() == Some("log")),
            );
        }

        let result = reconcile(&board, &ast);

        let removed: Vec<_> = result
            .commands
            .iter()
            .filter_map(|c| match c {
                BoardCommand::RemoveNode { node_id, .. } => Some(node_id.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(removed, vec!["log"], "absent text-visible node is removed");
    }

    #[test]
    fn parse_error_yields_diagnostic_no_commands() {
        let board = empty_board();
        let result = reconcile_text(&board, "this is not valid flowscript {{{");
        assert!(result.commands.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert!(result.diagnostics[0].contains("parse error"));
    }

    #[test]
    fn parsed_anchor_literal_edit_emits_update_pin() {
        let board = board_with_log("hello");
        let text = super::super::board_to_flowscript(
            &board,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
        .replace("\"hello\"", "\"world\"");

        let result = reconcile_text(&board, &text);

        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "log"
                    && pin_id == "text"
                    && value == &flow_like_types::Value::String("world".to_string())
        ));
    }

    fn pin_meta(name: &str, data_type: &str, _pin_type: PinType) -> PinMetadata {
        PinMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            data_type: data_type.to_string(),
            value_type: "Normal".to_string(),
            default_value: None,
            schema: None,
            is_generic: false,
            valid_values: None,
            enforce_schema: false,
        }
    }

    fn pin_meta_friendly(
        name: &str,
        friendly_name: &str,
        data_type: &str,
        value_type: &str,
        pin_type: PinType,
    ) -> PinMetadata {
        let mut pin = pin_meta(name, data_type, pin_type);
        pin.friendly_name = friendly_name.to_string();
        pin.value_type = value_type.to_string();
        pin
    }

    fn catalog_meta(
        name: &str,
        friendly_name: &str,
        inputs: Vec<PinMetadata>,
        outputs: Vec<PinMetadata>,
    ) -> NodeMetadata {
        NodeMetadata {
            name: name.to_string(),
            friendly_name: friendly_name.to_string(),
            description: String::new(),
            inputs,
            outputs,
            category: None,
            required_inputs: Vec::new(),
            companion_nodes: Vec::new(),
            capability_tags: Vec::new(),
        }
    }

    /// Board: event → makeData (struct output) → log, where log.text is fed through a
    /// sugared `struct_get` (renders as `makeData.data.report_id`).
    fn board_with_struct_member_chain() -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut producer = Node::new("make_data", "Make Data", "", "data");
        producer.id = "producer".to_string();
        let producer_in = producer
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let producer_out = producer
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let data_out = producer
            .add_output_pin("data", "Data", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(producer.id.clone(), producer);

        let mut getter = Node::new("struct_get", "Get Field", "", "structs");
        getter.id = "getter".to_string();
        let struct_in = getter
            .add_input_pin("struct", "Struct", "", VariableType::Struct)
            .id
            .clone();
        let field_pin = getter.add_input_pin("field", "Field", "", VariableType::String);
        field_pin.default_value = Some(b"\"report_id\"".to_vec());
        let value_out = getter
            .add_output_pin("value", "Value", "", VariableType::Generic)
            .id
            .clone();
        board.nodes.insert(getter.id.clone(), getter);

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let log_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let text_in = log
            .add_input_pin("text", "Text", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(log.id.clone(), log);

        connect(&mut board, "event", &event_out, "producer", &producer_in);
        connect(&mut board, "producer", &producer_out, "log", &log_in);
        connect(&mut board, "producer", &data_out, "getter", &struct_in);
        connect(&mut board, "getter", &value_out, "log", &text_in);
        board
    }

    fn member_chain_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "make_data",
                "Make Data",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("data", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "struct_get",
                "Get Field",
                vec![
                    pin_meta("struct", "Struct", PinType::Input),
                    pin_meta("field", "String", PinType::Input),
                ],
                vec![pin_meta("value", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    /// Deliberately OPEN, copied from the generated pin schema: the rejection must not depend on the
    /// schema closing itself, because `schemars` never does for FlowPath.
    const FLOW_PATH_PIN_SCHEMA: &str = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"FlowPath","type":"object","properties":{"path":{"type":"string"},"store_ref":{"type":"string"},"cache_store_ref":{"type":["string","null"]}},"required":["path","store_ref"]}"#;

    fn flow_path_catalog() -> Vec<NodeMetadata> {
        // Named `file`, not `path`: the real node's output pin IS `path`, which shadows the
        // same-named struct member and would resolve to the pin before member access is reached.
        let mut file = pin_meta("file", "Struct", PinType::Output);
        file.enforce_schema = true;
        file.schema = Some(FLOW_PATH_PIN_SCHEMA.to_string());
        let mut open_struct = pin_meta("record", "Struct", PinType::Output);
        open_struct.schema = Some(
            r#"{"title":"Mail","type":"object","properties":{"subject":{"type":"string"}}}"#
                .to_string(),
        );
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "path_from_upload_dir",
                "Upload Dir",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![pin_meta("exec_out", "Execution", PinType::Output), file],
            ),
            catalog_meta(
                "open_record",
                "Open Record",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    open_struct,
                ],
            ),
            catalog_meta(
                "struct_get",
                "Get Field",
                vec![
                    pin_meta("struct", "Struct", PinType::Input),
                    pin_meta("field", "String", PinType::Input),
                ],
                vec![pin_meta("value", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "variable_get",
                "Variable Get",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "Generic", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    fn struct_get_added(result: &ReconcileResult) -> bool {
        result.commands.iter().any(|command| {
            matches!(command, BoardCommand::AddNode { node_type, .. } if node_type == "struct_get")
        })
    }

    fn struct_get_field_literal(result: &ReconcileResult) -> Option<String> {
        result.commands.iter().find_map(|command| match command {
            BoardCommand::UpdateNodePin { pin_id, value, .. } if pin_id == "field" => {
                value.as_str().map(str::to_string)
            }
            _ => None,
        })
    }

    #[test]
    fn flow_path_member_access_is_rejected_with_accessor_hint() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const file = pathFromUploadDir({})
    log({ text: file.filename })
}
"#,
            &flow_path_catalog(),
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("`FlowPath`")
                    && diagnostic.contains("has no field `filename`")
                    && diagnostic.contains("filename({ path })")
            }),
            "{:?}",
            result.diagnostics
        );
        assert!(!struct_get_added(&result));
    }

    #[test]
    fn flow_path_declared_fields_still_resolve() {
        for member in ["path", "storeRef", "cacheStoreRef"] {
            let result = reconcile_text_with_catalog(
                &empty_board(),
                &format!(
                    r#"eventsSimple() {{
    const file = pathFromUploadDir({{}})
    log({{ text: file.{member} }})
}}
"#
                ),
                &flow_path_catalog(),
            );
            assert!(
                result.diagnostics.is_empty(),
                "{member}: {:?}",
                result.diagnostics
            );
            assert!(struct_get_added(&result), "{member}");
        }
    }

    #[test]
    fn flow_path_camel_member_lowers_to_the_declared_field_name() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const file = pathFromUploadDir({})
    log({ text: file.storeRef })
}
"#,
            &flow_path_catalog(),
        );

        // The runtime selects the JSON key verbatim, so `storeRef` has to be written as `store_ref`.
        assert_eq!(
            struct_get_field_literal(&result).as_deref(),
            Some("store_ref"),
            "{:?}",
            result.commands
        );
    }

    #[test]
    fn explicit_struct_get_on_a_flow_path_field_is_rejected() {
        let rejected = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const file = pathFromUploadDir({})
    log({ text: structGet({ struct: file, field: "extension" }) })
}
"#,
            &flow_path_catalog(),
        );
        assert!(
            rejected.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("`FlowPath`")
                    && diagnostic.contains("extension")
                    && diagnostic.contains("extension({ path })")
            }),
            "{:?}",
            rejected.diagnostics
        );

        let allowed = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const file = pathFromUploadDir({})
    log({ text: structGet({ struct: file, field: "path" }) })
}
"#,
            &flow_path_catalog(),
        );
        assert!(allowed.diagnostics.is_empty(), "{:?}", allowed.diagnostics);
    }

    /// KNOWN GAP: a FlowPath read from a board variable escapes the rejection. A FlowScript
    /// `const file: Struct` declaration supplies its own title-less `{"type":"object"}` schema
    /// (`packages/ast/src/schema.rs:386`), and that authored contract wins over the board
    /// variable's real schema in `variable_value_contract`, so there is no `FlowPath` title left to
    /// match. Fixing it means propagating pin schemas onto variable contracts — a separate change.
    /// Node-output sources, which is how file values normally flow, are covered.
    #[test]
    #[ignore = "variable contracts drop the FlowPath schema; needs schema propagation first"]
    fn flow_path_variable_member_access_is_rejected() {
        let mut board = empty_board();
        let mut variable = Variable::new("file", VariableType::Struct, ValueType::Normal);
        variable.id = "var_file".to_string();
        variable.schema = Some(FLOW_PATH_PIN_SCHEMA.to_string());
        board.variables.insert(variable.id.clone(), variable);

        let result = reconcile_text_with_catalog(
            &board,
            r#"const file: Struct = {}   //@v:var_file

eventsSimple() {
    log({ text: file.filename })
}
"#,
            &flow_path_catalog(),
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("`FlowPath`") && diagnostic.contains("filename")
            }),
            "{:?}",
            result.diagnostics
        );
    }

    #[test]
    fn open_schema_structs_other_than_flow_path_still_accept_members() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const record = openRecord({})
    log({ text: record.body })
}
"#,
            &flow_path_catalog(),
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(struct_get_added(&result));
    }

    fn schema_member_catalog() -> Vec<NodeMetadata> {
        let mut email = pin_meta("email", "Struct", PinType::Output);
        email.enforce_schema = true;
        email.schema = Some(
            r#"{"title":"Email","type":"object","properties":{"subject":{"type":"string"},"plain":{"type":"string"},"html":{"type":"string"}},"additionalProperties":false}"#
                .to_string(),
        );
        let mut emails = pin_meta_friendly("emails", "Emails", "Struct", "Array", PinType::Output);
        emails.schema = Some(
            r#"{"title":"EmailRef","type":"object","properties":{"uid":{"type":"integer"}}}"#
                .to_string(),
        );
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "email_imap_inbox_fetch_mail",
                "Fetch Mail",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![pin_meta("exec_out", "Execution", PinType::Output), email],
            ),
            catalog_meta(
                "mail_imap_list",
                "List Mails",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![pin_meta("exec_out", "Execution", PinType::Output), emails],
            ),
            catalog_meta(
                "struct_get",
                "Get Field",
                vec![
                    pin_meta("struct", "Struct", PinType::Input),
                    pin_meta("field", "String", PinType::Input),
                ],
                vec![pin_meta("value", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "array_length",
                "Array Length",
                vec![pin_meta_friendly(
                    "array",
                    "Array",
                    "Generic",
                    "Array",
                    PinType::Input,
                )],
                vec![pin_meta("length", "Integer", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "Generic", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    #[test]
    fn unchanged_struct_member_roundtrip_reuses_accessor() {
        // A sugared struct_get (`base.field` in text) must reuse the existing accessor on a
        // no-op apply — not rewire the consumer to the bare base value or add a duplicate.
        let board = board_with_struct_member_chain();
        let text = anchored_text(&board);
        let result = reconcile_text_with_catalog(&board, &text, &member_chain_catalog());
        assert!(
            result.commands.is_empty(),
            "no-op member-chain round-trip must be empty; got {:?} from text:\n{text}",
            result.commands
        );
    }

    #[test]
    fn unchanged_bracketed_struct_member_roundtrip_reuses_accessor() {
        // Non-identifier struct keys render with bracketed string syntax. Parsing that canonical
        // text must preserve `Member`, otherwise reconcile mistakes the struct for an array and
        // replaces this accessor with `array_get`.
        let mut board = board_with_struct_member_chain();
        let field_pin = board
            .nodes
            .get_mut("getter")
            .and_then(|node| {
                node.pins
                    .values_mut()
                    .find(|pin| pin.pin_type == PinType::Input && pin.name == "field")
            })
            .expect("struct_get field pin");
        field_pin.default_value = Some(b"\"row-rejection-reason\"".to_vec());

        let text = anchored_text(&board);
        assert!(
            text.contains("[\"row-rejection-reason\"]"),
            "canonical FlowScript should use bracketed member syntax:\n{text}"
        );
        let result = reconcile_text_with_catalog(&board, &text, &member_chain_catalog());
        assert!(
            result.diagnostics.is_empty(),
            "bracketed member round-trip must stay valid: {:?}",
            result.diagnostics
        );
        assert!(
            result.commands.is_empty(),
            "bracketed member round-trip must be a no-op; got {:?} from text:\n{text}",
            result.commands
        );
    }

    #[test]
    fn schema_member_fallback_rejects_unknown_email_field() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const mail = emailImapInboxFetchMail({})
    log({ text: mail.body })
}
"#,
            &schema_member_catalog(),
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("email_imap_inbox_fetch_mail.email")
                    && diagnostic.contains("declares no field `body`")
                    && diagnostic.contains("plain")
                    && diagnostic.contains("html")
            }),
            "{:?}",
            result.diagnostics
        );
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "struct_get"
        )));
    }

    #[test]
    fn schema_member_fallback_accepts_declared_and_schema_less_fields() {
        let declared = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const mail = emailImapInboxFetchMail({})
    log({ text: mail.plain })
}
"#,
            &schema_member_catalog(),
        );
        assert!(
            declared.diagnostics.is_empty(),
            "{:?}",
            declared.diagnostics
        );
        assert!(declared.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "struct_get"
        )));

        let mut open_catalog = schema_member_catalog();
        let fetch = open_catalog
            .iter_mut()
            .find(|meta| meta.name == "email_imap_inbox_fetch_mail")
            .expect("fetch metadata");
        fetch
            .outputs
            .iter_mut()
            .find(|pin| pin.name == "email")
            .expect("email output")
            .schema = None;
        let open = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const mail = emailImapInboxFetchMail({})
    log({ text: mail.body })
}
"#,
            &open_catalog,
        );
        assert!(open.diagnostics.is_empty(), "{:?}", open.diagnostics);
        assert!(open.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "struct_get"
        )));

        let mut enforced_but_open_catalog = schema_member_catalog();
        let fetch = enforced_but_open_catalog
            .iter_mut()
            .find(|meta| meta.name == "email_imap_inbox_fetch_mail")
            .expect("fetch metadata");
        let email = fetch
            .outputs
            .iter_mut()
            .find(|pin| pin.name == "email")
            .expect("email output");
        email.schema = Some(
            r#"{"title":"OpenEmail","type":"object","properties":{"plain":{"type":"string"}}}"#
                .to_string(),
        );
        assert!(email.enforce_schema);
        let enforced_but_open = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const mail = emailImapInboxFetchMail({})
    log({ text: mail.body })
}
"#,
            &enforced_but_open_catalog,
        );
        assert!(
            enforced_but_open.diagnostics.is_empty(),
            "pin schema enforcement must not override JSON Schema's open-object default: {:?}",
            enforced_but_open.diagnostics
        );

        let mut extensible_catalog = schema_member_catalog();
        let fetch = extensible_catalog
            .iter_mut()
            .find(|meta| meta.name == "email_imap_inbox_fetch_mail")
            .expect("fetch metadata");
        fetch
            .outputs
            .iter_mut()
            .find(|pin| pin.name == "email")
            .expect("email output")
            .schema = Some(
            r#"{"title":"ExtensibleEmail","type":"object","properties":{"plain":{"type":"string"}},"additionalProperties":true}"#
                .to_string(),
        );
        let extensible = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const mail = emailImapInboxFetchMail({})
    log({ text: mail.body })
}
"#,
            &extensible_catalog,
        );
        assert!(
            extensible.diagnostics.is_empty(),
            "{:?}",
            extensible.diagnostics
        );
        assert!(extensible.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "struct_get"
        )));
    }

    #[test]
    fn schema_member_fallback_rejects_fields_on_array_outputs_but_keeps_length() {
        let invalid = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const fetched = mailImapList({})
    log({ text: fetched.mails })
}
"#,
            &schema_member_catalog(),
        );
        assert!(
            invalid.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("mail_imap_list.emails")
                    && diagnostic.contains("collection type `Array`")
                    && diagnostic.contains("member `mails`")
            }),
            "{:?}",
            invalid.diagnostics
        );

        let length = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const fetched = mailImapList({})
    log({ text: fetched.length })
}
"#,
            &schema_member_catalog(),
        );
        assert!(length.diagnostics.is_empty(), "{:?}", length.diagnostics);
        assert!(length.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "array_length"
        )));
    }

    #[test]
    fn schema_less_collection_rejects_member_fallback_but_keeps_length() {
        let mut catalog = schema_member_catalog();
        catalog
            .iter_mut()
            .find(|meta| meta.name == "mail_imap_list")
            .expect("list metadata")
            .outputs
            .iter_mut()
            .find(|pin| pin.name == "emails")
            .expect("emails output")
            .schema = None;

        let invalid = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const fetched = mailImapList({})
    log({ text: fetched.mails })
}
"#,
            &catalog,
        );
        assert!(
            invalid.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("mail_imap_list.emails")
                    && diagnostic.contains("collection type `Array`")
                    && diagnostic.contains("member `mails`")
            }),
            "{:?}",
            invalid.diagnostics
        );
        assert!(!invalid.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "struct_get"
        )));

        let length = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const fetched = mailImapList({})
    log({ text: fetched.length })
}
"#,
            &catalog,
        );
        assert!(length.diagnostics.is_empty(), "{:?}", length.diagnostics);
        assert!(length.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "array_length"
        )));
    }

    #[test]
    fn member_access_uses_map_and_set_accessors_without_container_coercion() {
        let mut catalog = schema_member_catalog();
        catalog.extend([
            catalog_meta(
                "map_source",
                "Map Source",
                Vec::new(),
                vec![pin_meta_friendly(
                    "map",
                    "Map",
                    "Generic",
                    "HashMap",
                    PinType::Output,
                )],
            ),
            catalog_meta(
                "set_source",
                "Set Source",
                Vec::new(),
                vec![pin_meta_friendly(
                    "set",
                    "Set",
                    "Generic",
                    "HashSet",
                    PinType::Output,
                )],
            ),
            catalog_meta(
                "map_get",
                "Map Get",
                vec![
                    pin_meta_friendly("map_in", "Map", "Generic", "HashMap", PinType::Input),
                    pin_meta("key", "String", PinType::Input),
                ],
                vec![
                    pin_meta("value", "Generic", PinType::Output),
                    pin_meta("found", "Boolean", PinType::Output),
                ],
            ),
            catalog_meta(
                "map_size",
                "Map Size",
                vec![pin_meta_friendly(
                    "map_in",
                    "Map",
                    "Generic",
                    "HashMap",
                    PinType::Input,
                )],
                vec![pin_meta("size", "Integer", PinType::Output)],
            ),
            catalog_meta(
                "set_get_size",
                "Set Size",
                vec![pin_meta_friendly(
                    "set_in",
                    "Set",
                    "Generic",
                    "HashSet",
                    PinType::Input,
                )],
                vec![pin_meta("size", "Integer", PinType::Output)],
            ),
        ]);

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const values = mapSource({})
    log({ text: values.customer })
    log({ text: values.length })
    const members = setSource({})
    log({ text: members.length })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for expected in ["map_get", "map_size", "set_get_size"] {
            assert!(
                result.commands.iter().any(|command| matches!(
                    command,
                    BoardCommand::AddNode { node_type, .. } if node_type == expected
                )),
                "missing {expected}: {:?}",
                result.commands
            );
        }
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. }
                if matches!(node_type.as_str(), "struct_get" | "array_length")
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { to_pin, .. } if to_pin == "map_in"
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { to_pin, .. } if to_pin == "set_in"
        )));
    }

    #[test]
    fn concrete_scalar_member_access_is_rejected_before_struct_fallback() {
        let mut catalog = schema_member_catalog();
        catalog.push(catalog_meta(
            "string_source",
            "String Source",
            Vec::new(),
            vec![pin_meta("value", "String", PinType::Output)],
        ));

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const value = stringSource({})
    log({ text: value.unknown })
}
"#,
            &catalog,
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("string_source.value")
                    && diagnostic.contains("scalar type `String`")
                    && diagnostic.contains("member `unknown`")
            }),
            "{:?}",
            result.diagnostics
        );
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. } if node_type == "struct_get"
        )));
    }

    #[test]
    fn unchanged_variable_ref_roundtrip_reuses_variable_get() {
        // A bare variable ref in text lowers from a variable_get node; a no-op apply must
        // reuse that node instead of materializing a duplicate reader.
        let mut board = empty_board();
        let mut variable = Variable::new("greeting", VariableType::String, ValueType::Normal);
        variable.id = "var1".to_string();
        variable.set_default_value(flow_like_types::Value::String("hi".to_string()));
        board.variables.insert(variable.id.clone(), variable);

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut reader = Node::new("variable_get", "Get Variable", "", "variables");
        reader.id = "reader".to_string();
        let var_ref = reader.add_input_pin("var_ref", "Variable", "", VariableType::String);
        var_ref.default_value = Some(b"\"var1\"".to_vec());
        let value_out = reader
            .add_output_pin("value_ref", "Value", "", VariableType::Generic)
            .id
            .clone();
        board.nodes.insert(reader.id.clone(), reader);

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let log_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let text_in = log
            .add_input_pin("text", "Text", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(log.id.clone(), log);

        connect(&mut board, "event", &event_out, "log", &log_in);
        connect(&mut board, "reader", &value_out, "log", &text_in);

        let text = anchored_text(&board);
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];
        let result = reconcile_text_with_catalog(&board, &text, &catalog);
        assert!(
            result.commands.is_empty(),
            "no-op variable-ref round-trip must be empty; got {:?} from text:\n{text}",
            result.commands
        );

        let changed_text = text.replacen("greeting: string", "greeting: Date", 1);
        assert_ne!(
            changed_text, text,
            "the lowered variable type must be editable"
        );
        let changed = reconcile_text_with_catalog(&board, &changed_text, &catalog);
        assert!(
            changed.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("incompatible pin types or schemas")
                    && diagnostic.contains("Date/Normal")
                    && diagnostic.contains("String/Normal")
            }),
            "a retained variable_get edge must use the edited variable contract: {:?}",
            changed.diagnostics
        );
        assert!(!changed.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, to_node, .. }
                if from_node == "reader" && to_node == "log"
        )));
    }

    #[test]
    fn inserting_before_fan_in_only_splices_own_chain() {
        // Two events converge on one log node (exec fan-in). Inserting a new node into one
        // chain must splice only that chain's edge — the other event's wiring stays intact.
        let mut board = empty_board();

        for event_id in ["event_a", "event_b"] {
            let mut event = Node::new(
                if event_id == "event_a" {
                    "event_start"
                } else {
                    "event_timer"
                },
                "Event",
                "",
                "events",
            );
            event.id = event_id.to_string();
            event.set_start(true);
            let exec_out = event
                .add_output_pin("exec_out", "Out", "", VariableType::Execution)
                .id
                .clone();
            board.nodes.insert(event.id.clone(), event);
            let _ = exec_out;
        }

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let log_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let text_pin = log.add_input_pin("text", "Text", "", VariableType::String);
        text_pin.default_value = Some(b"\"hi\"".to_vec());
        board.nodes.insert(log.id.clone(), log);

        for event_id in ["event_a", "event_b"] {
            let exec_out = board.nodes[event_id]
                .pins
                .values()
                .find(|p| p.pin_type == PinType::Output)
                .map(|p| p.id.clone())
                .expect("event exec out");
            connect(&mut board, event_id, &exec_out, "log", &log_in);
        }

        let text = anchored_text(&board);
        let log_line = text
            .lines()
            .find(|line| line.contains("//@n:log"))
            .expect("log statement in rendered text")
            .to_string();
        let edited = text.replace(&log_line, &format!("    notify()\n{log_line}"));

        let catalog = vec![
            catalog_meta(
                "event_start",
                "Event Start",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "event_timer",
                "Event Timer",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "notify",
                "Notify",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];
        let result = reconcile_text_with_catalog(&board, &edited, &catalog);

        let disconnects: Vec<_> = result
            .commands
            .iter()
            .filter_map(|c| match c {
                BoardCommand::DisconnectPins {
                    from_node, to_node, ..
                } => Some((from_node.clone(), to_node.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            disconnects.len(),
            1,
            "only the edited chain's edge may be spliced; got {disconnects:?} from:\n{edited}"
        );
        assert_eq!(disconnects[0].1, "log");
        assert!(
            disconnects[0].0.starts_with("event_"),
            "splice must originate from the chain's own event; got {disconnects:?}"
        );
    }

    #[test]
    fn catalog_aware_reconcile_adds_unanchored_calls() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    log({ text: "hello" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 4);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::AddNode { node_type, ref_id, .. }
                if node_type == "log" && ref_id.as_deref() == Some("$1")
        ));
        assert!(matches!(
            &result.commands[1],
            BoardCommand::AddNode { node_type, ref_id, .. }
                if node_type == "events_simple" && ref_id.as_deref() == Some("$0")
        ));
        assert!(matches!(
            &result.commands[2],
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "$1"
                    && pin_id == "text"
                    && value == &flow_like_types::Value::String("hello".to_string())
        ));
        assert!(matches!(
            &result.commands[3],
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$0"
                    && from_pin == "exec_out"
                    && to_node == "$1"
                    && to_pin == "exec_in"
        ));
    }

    #[test]
    fn catalog_required_input_blocks_empty_unanchored_call() {
        let board = empty_board();
        let mut required_sink = catalog_meta(
            "required_sink",
            "Required Sink",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("payload", "String", PinType::Input),
            ],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        );
        required_sink.required_inputs = vec!["payload".to_string()];
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            required_sink,
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    requiredSink({})
}
"#,
            &catalog,
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| diagnostic
                == "node `requiredSink` is missing required inputs: payload"),
            "{:?}",
            result.diagnostics
        );
    }

    #[test]
    fn catalog_required_input_accepts_literal_connection_and_catalog_default() {
        fn required_catalog(default: Option<&str>) -> Vec<NodeMetadata> {
            let mut payload = pin_meta("payload", "String", PinType::Input);
            payload.default_value = default.map(str::to_string);
            let mut required_sink = catalog_meta(
                "required_sink",
                "Required Sink",
                vec![pin_meta("exec_in", "Execution", PinType::Input), payload],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            );
            // Keep this explicitly required even in the defaulted case: the guard must recognize
            // that the catalog value itself satisfies the requirement.
            required_sink.required_inputs = vec!["payload".to_string()];
            vec![
                catalog_meta(
                    "events_simple",
                    "Simple Event",
                    Vec::new(),
                    vec![pin_meta("exec_out", "Execution", PinType::Output)],
                ),
                catalog_meta(
                    "make_value",
                    "Make Value",
                    Vec::new(),
                    vec![pin_meta("value", "String", PinType::Output)],
                ),
                required_sink,
            ]
        }

        let cases = [
            (
                r#"eventsSimple() {
    requiredSink({ payload: "literal" })
}
"#,
                required_catalog(None),
            ),
            (
                r#"eventsSimple() {
    const made = makeValue({})
    requiredSink({ payload: made.value })
}
"#,
                required_catalog(None),
            ),
            (
                r#"eventsSimple() {
    requiredSink({})
}
"#,
                required_catalog(Some("\"from catalog\"")),
            ),
        ];

        for (source, catalog) in cases {
            let result = reconcile_text_with_catalog(&empty_board(), source, &catalog);
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        }
    }

    #[test]
    fn catalog_required_inputs_preserve_repeated_pin_occurrences() {
        let board = empty_board();
        let mut repeated_sink = catalog_meta(
            "repeated_sink",
            "Repeated Sink",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("payload", "String", PinType::Input),
                pin_meta("payload", "String", PinType::Input),
            ],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        );
        repeated_sink.required_inputs = vec!["payload".to_string(), "payload".to_string()];
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            repeated_sink,
        ];

        let missing = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    repeatedSink({ payload: "first" })
}
"#,
            &catalog,
        );
        assert!(
            missing
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.ends_with("payload[#2]")),
            "{:?}",
            missing.diagnostics
        );

        let satisfied = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    repeatedSink({ payload: "first", payload: "second" })
}
"#,
            &catalog,
        );
        assert!(
            satisfied.diagnostics.is_empty(),
            "{:?}",
            satisfied.diagnostics
        );
    }

    fn anchored_required_sink_catalog() -> Vec<NodeMetadata> {
        let mut required_sink = catalog_meta(
            "required_sink",
            "Required Sink",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("payload", "String", PinType::Input),
            ],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        );
        required_sink.required_inputs = vec!["payload".to_string()];
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "make_value",
                "Make Value",
                Vec::new(),
                vec![pin_meta("value", "String", PinType::Output)],
            ),
            required_sink,
        ]
    }

    fn board_with_anchored_required_sink(
        payload_default: Option<&str>,
        connect_payload: bool,
    ) -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut sink = Node::new("required_sink", "Required Sink", "", "test");
        sink.id = "sink".to_string();
        let sink_exec = sink
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let payload = sink.add_input_pin("payload", "Payload", "", VariableType::String);
        payload.default_value = payload_default.map(|value| value.as_bytes().to_vec());
        let payload_id = payload.id.clone();
        sink.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(sink.id.clone(), sink);
        connect(&mut board, "event", &event_out, "sink", &sink_exec);

        if connect_payload {
            let mut source = Node::new("make_value", "Make Value", "", "test");
            source.id = "source".to_string();
            let value = source
                .add_output_pin("value", "Value", "", VariableType::String)
                .id
                .clone();
            board.nodes.insert(source.id.clone(), source);
            connect(&mut board, "source", &value, "sink", &payload_id);
        }

        board
    }

    /// A required pin that was ALREADY unset on the live anchored node is the board's status
    /// quo: re-anchoring the same call (the unavoidable lowered form of that board) must not
    /// fail, or every unrelated edit to the document is blocked. New unanchored calls keep full
    /// enforcement (see `catalog_required_input_blocks_empty_unanchored_call`).
    #[test]
    fn catalog_required_input_grandfathers_already_unset_anchored_pin() {
        let board = board_with_anchored_required_sink(None, false);
        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {   //@n:event
    requiredSink({})   //@n:sink
}
"#,
            &anchored_required_sink_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.is_empty(), "{:?}", result.commands);
    }

    #[test]
    fn catalog_required_input_accepts_anchored_literal_from_outer_diff() {
        let board = board_with_anchored_required_sink(None, false);
        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {   //@n:event
    requiredSink({ payload: "configured" })   //@n:sink
}
"#,
            &anchored_required_sink_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "sink"
                    && pin_id == "payload"
                    && value == &flow_like_types::Value::String("configured".to_string())
        )));
    }

    #[test]
    fn catalog_required_input_accepts_retained_anchored_connection() {
        let board = board_with_anchored_required_sink(None, true);
        let text = anchored_text(&board);
        let result = reconcile_text_with_catalog(&board, &text, &anchored_required_sink_catalog());

        assert!(
            result.diagnostics.is_empty(),
            "{:?}\nFlowScript:\n{text}",
            result.diagnostics
        );
    }

    #[test]
    fn catalog_required_input_accepts_live_default_on_anchored_node() {
        let board = board_with_anchored_required_sink(Some("\"configured\""), false);
        // Omit the argument deliberately: the explicit catalog requirement must be merged into
        // the live metadata, then recognized as satisfied by the instance's retained default.
        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {   //@n:event
    requiredSink({})   //@n:sink
}
"#,
            &anchored_required_sink_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }

    #[test]
    fn typed_event_node_type_is_authoritative_over_handler_name() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_generic",
                "Generic Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "notify",
                "Notify",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];
        let ast = BoardAst {
            events: vec![EventBlock {
                // Deliberately disagrees with node_type: this is the authored handler alias, not
                // a catalog lookup key on the typed path.
                name: "eventsSimple".to_string(),
                node_type: "events_generic".to_string(),
                event_name: None,
                params: Vec::new(),
                body: Block {
                    stmts: vec![Stmt::Call {
                        call: Call {
                            node_type: "notify".to_string(),
                            display: "notify".to_string(),
                            args: Vec::new(),
                            anchor: None,
                        },
                        anchor: None,
                    }],
                },
                anchor: None,
            }],
            ..BoardAst::default()
        };

        let result = reconcile_with_catalog(&board, &ast, &catalog);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "events_simple"
            )
        }));
        let entry_index = result
            .commands
            .iter()
            .position(|command| {
                matches!(
                    command,
                    BoardCommand::AddNode {
                        node_type,
                        friendly_name: Some(alias),
                        ..
                    } if node_type == "events_generic" && alias == "eventsSimple"
                )
            })
            .expect("exact Generic Event entry with the authored handler alias");
        let notify_index = result
            .commands
            .iter()
            .position(|command| {
                matches!(
                    command,
                    BoardCommand::AddNode { node_type, .. } if node_type == "notify"
                )
            })
            .expect("workflow body node");
        assert!(
            entry_index > notify_index,
            "custom event registrations must still be emitted after workflow setup"
        );
    }

    #[test]
    fn typed_event_rejects_exact_node_type_that_cannot_start_a_chain() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "log",
                "Log",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "notify",
                "Notify",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];
        let ast = BoardAst {
            events: vec![EventBlock {
                name: "start".to_string(),
                node_type: "log".to_string(),
                event_name: None,
                params: Vec::new(),
                body: Block {
                    stmts: vec![Stmt::Call {
                        call: Call {
                            node_type: "notify".to_string(),
                            display: "notify".to_string(),
                            args: Vec::new(),
                            anchor: None,
                        },
                        anchor: None,
                    }],
                },
                anchor: None,
            }],
            ..BoardAst::default()
        };

        let result = reconcile_with_catalog(&board, &ast, &catalog);

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("node_type `log`")
                && diagnostic.contains("cannot be used as an event entry")
                && diagnostic.contains("Execution input")
        }));
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "log"
            )
        }));
    }

    #[test]
    fn new_generic_event_parameters_become_additional_output_pins() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_generic",
                "Generic Event",
                Vec::new(),
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("payload", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsGeneric(payload: Struct, ticketId: string) {
    log({ text: ticketId })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let additional_pins = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::AddNode {
                    node_type,
                    additional_pins,
                    ..
                } if node_type == "events_generic" => additional_pins.as_ref(),
                _ => None,
            })
            .expect("generic event carries additional pins");
        assert_eq!(additional_pins.len(), 1, "payload is a catalog pin");
        assert_eq!(additional_pins[0].name, "ticketId");
        assert_eq!(additional_pins[0].pin_type, "Output");
        assert_eq!(additional_pins[0].data_type, "String");
        assert_eq!(additional_pins[0].value_type.as_deref(), Some("Normal"));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$0"
                        && from_pin == "ticketId"
                        && to_node == "$1"
                        && to_pin == "text"
            )
        }));
    }

    #[test]
    fn named_generic_handler_fallback_is_actionable_and_keeps_tool_alias() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_generic",
                "Generic Event",
                Vec::new(),
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("payload", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "log_info",
                "Log Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "agent_from_model",
                "Agent From Model",
                Vec::new(),
                vec![pin_meta("agent_out", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "agent_register_function_tools",
                "Register Function Tools",
                vec![pin_meta("agent_in", "Struct", PinType::Input)],
                vec![pin_meta("agent_out", "Struct", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const agent = agentRegisterFunctionTools({ agentIn: agentFromModel({}), tools: [fetchPage] })
    fetchPage(payload: Struct) {
        logInfo({ message: "fetching" })
    }
}
"#,
            &catalog,
        );

        assert!(
            result.diagnostics.is_empty(),
            "a successful Generic handler fallback must remain atomically applicable: {:?}",
            result.diagnostics
        );
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode {
                    node_type,
                    friendly_name: Some(friendly_name),
                    ..
                } if node_type == "events_generic" && friendly_name == "fetchPage"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::SetNodeFunctionRefs { fn_refs, .. }
                    if fn_refs == &vec!["fetchPage".to_string()]
            )
        }));
    }

    /// A root event can register a sibling root entry as an agent tool. The graph-level handler
    /// still belongs to the registering event's lexical FlowScript scope: its body may read that
    /// event's payload outputs (for example the current chat attachments). Lowering must nest the
    /// handler under the owner event so those exact cross-entry pin edges remain resolvable when
    /// the anchored text is reconciled against the same board.
    #[test]
    fn root_agent_tool_handler_capture_roundtrips_as_noop() {
        let mut board = empty_board();

        let mut event = Node::new("simple_agent", "Simple Agent", "", "events");
        event.id = "simple-agent".to_string();
        event.set_start(true);
        let event_exec = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let attachments = event
            .add_output_pin("attachments", "Attachments", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut register = Node::new(
            "agent_register_function_tools",
            "Register Function Tools",
            "",
            "agent",
        );
        register.id = "register-tools".to_string();
        register.set_can_reference_fns(true);
        register
            .fn_refs
            .as_mut()
            .expect("function references enabled")
            .fn_refs
            .push("attachment-handler".to_string());
        let registered_agent = register
            .add_output_pin("agent", "Agent", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(register.id.clone(), register);

        let mut invoke = Node::new("agent_invoke", "Invoke Agent", "", "agent");
        invoke.id = "invoke-agent".to_string();
        let invoke_exec = invoke
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let invoke_agent = invoke
            .add_input_pin("agent", "Agent", "", VariableType::Struct)
            .id
            .clone();
        invoke.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(invoke.id.clone(), invoke);

        let mut handler = Node::new("events_generic", "Extract Current Attachment", "", "events");
        handler.id = "attachment-handler".to_string();
        handler.set_start(true);
        handler.set_can_be_referenced_by_fns(true);
        let handler_exec = handler
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        handler.add_output_pin("payload", "Payload", "", VariableType::Struct);
        handler.add_output_pin("filename", "Filename", "", VariableType::String);
        board.nodes.insert(handler.id.clone(), handler);

        let mut extract = Node::new(
            "extract_attachment_pages",
            "Extract Attachment Pages",
            "",
            "files",
        );
        extract.id = "extract-pages".to_string();
        let extract_exec = extract
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let current_attachments = extract
            .add_input_pin(
                "current_attachments",
                "Current Attachments",
                "",
                VariableType::Struct,
            )
            .id
            .clone();
        extract.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        extract.add_output_pin("result", "Result", "", VariableType::String);
        board.nodes.insert(extract.id.clone(), extract);

        connect(
            &mut board,
            "simple-agent",
            &event_exec,
            "invoke-agent",
            &invoke_exec,
        );
        connect(
            &mut board,
            "register-tools",
            &registered_agent,
            "invoke-agent",
            &invoke_agent,
        );
        connect(
            &mut board,
            "attachment-handler",
            &handler_exec,
            "extract-pages",
            &extract_exec,
        );
        connect(
            &mut board,
            "simple-agent",
            &attachments,
            "extract-pages",
            &current_attachments,
        );

        let ast = super::super::lower_to_ast(&board);
        assert_eq!(
            ast.events.len(),
            1,
            "the referenced tool entry must not be rendered as a sibling root event"
        );
        assert!(
            ast.events[0].body.stmts.iter().any(|statement| matches!(
                statement,
                Stmt::Handler(handler)
                    if handler.anchor.as_deref() == Some("attachment-handler")
            )),
            "the referenced tool entry must be nested in its unique registering event"
        );

        let text = anchored_text(&board);
        let catalog = board
            .nodes
            .values()
            .map(node_to_metadata)
            .collect::<Vec<_>>();
        let result = reconcile_text_with_catalog(&board, &text, &catalog);

        assert!(
            result.diagnostics.is_empty(),
            "the owner event's attachments parameter must remain visible to its tool handler:\n{text}\n{:?}",
            result.diagnostics
        );
        assert!(
            result.commands.is_empty(),
            "an unchanged anchored owner/handler graph must be a no-op:\n{text}\n{:?}",
            result.commands
        );
    }

    #[test]
    fn catalog_aware_reconcile_skips_synthetic_tools_arg_without_diagnostic() {
        // `agentRegisterFunctionTools` has no `tools` input pin — its function references are a
        // synthetic `tools:` argument the decompiler emits. Reconcile must skip it rather than
        // report a missing pin (which surfaced to users as a false "FlowScript apply blocked").
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "agent_from_model",
                "Agent From Model",
                Vec::new(),
                vec![pin_meta("agent_out", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "agent_register_function_tools",
                "Register Function Tools",
                vec![pin_meta("agent_in", "Struct", PinType::Input)],
                vec![pin_meta("agent_out", "Struct", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const agent = agentRegisterFunctionTools({ agentIn: agentFromModel({}), tools: [fetchPage] })
}
"#,
            &catalog,
        );

        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.contains("no input pin named `tools`")),
            "synthetic tools arg must not produce a missing-pin diagnostic: {:?}",
            result.diagnostics
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { node_type, .. }
                    if node_type == "agent_register_function_tools"
            )),
            "agent node should still be added: {:?}",
            result.commands
        );
        // The synthetic `tools:` arg on a NEW node materializes as a SetNodeFunctionRefs command
        // carrying the referenced target names for the applier to resolve.
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::SetNodeFunctionRefs { fn_refs, .. }
                    if fn_refs == &vec!["fetchPage".to_string()]
            )),
            "expected SetNodeFunctionRefs with [fetchPage]: {:?}",
            result.commands
        );
    }

    /// A top-level `function` used as an agent tool materializes as `CreateLayer` + boundary pins
    /// and nothing else, while apply requires each `SetNodeFunctionRefs` target to resolve to a
    /// node carrying `fn_refs.can_be_referenced_by_fns`. Without a reconcile-side check, `check`
    /// says `valid`, `commit` says `queued` with zero diagnostics, and the apply dies with
    /// "Function layer `X` has no referenceable event/handler entry", rolling the whole document
    /// back — a failure the model can neither see nor attribute.
    ///
    /// Minting an `events_generic` entry to make it apply is NOT the remedy: that entry exposes
    /// only `payload`, so the tool advertises a `{payload}` schema the model's named arguments
    /// never bind to, and `execute_tool_call` reads the `set_result` that only
    /// `events_generic_return_result` writes — a function's `return` wires to layer boundary pins,
    /// which the tool path never reads. It would apply clean and return the literal
    /// "Tool executed successfully".
    #[test]
    fn function_used_as_agent_tool_is_diagnosed_by_name() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log_info",
                "Log Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "agent_register_function_tools",
                "Register Function Tools",
                vec![pin_meta("agent_in", "Struct", PinType::Input)],
                vec![pin_meta("agent_out", "Struct", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function summarizeTicket(subject: string): (headline: string) {
    logInfo({ message: subject })
    return "summarized"
}

eventsSimple() {
    const agent = agentRegisterFunctionTools({ agentIn: "{}", tools: [summarizeTicket] })
    logInfo({ message: agent.agentOut })
}
"#,
            &catalog,
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("summarizeTicket") && d.contains("no event/handler entry")),
            "a `function` used as an agent tool must be diagnosed BY NAME at check time: {:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn catalog_aware_reconcile_uses_metadata_enricher_for_dynamic_pins() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            // Static metadata only knows the `shape` driver pin; a/b inputs and the `out` output are
            // "dynamic" and supplied by the enricher based on the shape literal.
            catalog_meta(
                "dynamic_node",
                "Dynamic Node",
                vec![pin_meta("shape", "String", PinType::Input)],
                Vec::new(),
            ),
            catalog_meta(
                "text_source",
                "Text Source",
                Vec::new(),
                vec![pin_meta("text", "String", PinType::Output)],
            ),
            catalog_meta(
                "sink",
                "Sink",
                vec![pin_meta("input", "Generic", PinType::Input)],
                Vec::new(),
            ),
        ];

        let enricher: MetadataEnricher = Box::new(
            |meta: &NodeMetadata,
             args: &[(String, flow_like_types::Value)],
             _board: &Board|
             -> Option<NodeMetadata> {
                if meta.name != "dynamic_node" {
                    return None;
                }
                let shape = args
                    .iter()
                    .find(|(name, _)| name == "shape")
                    .and_then(|(_, value)| value.as_str());
                if shape != Some("ab") {
                    return None;
                }
                let mut enriched = meta.clone();
                enriched
                    .inputs
                    .push(pin_meta("a", "Generic", PinType::Input));
                enriched
                    .inputs
                    .push(pin_meta("b", "Generic", PinType::Input));
                enriched
                    .outputs
                    .push(pin_meta("out", "Generic", PinType::Output));
                Some(enriched)
            },
        );

        let result = reconcile_text_with_catalog_enriched(
            &board,
            r#"eventsSimple() {
    const d = dynamicNode({ shape: "ab", a: textSource().text, b: textSource().text })
    sink({ input: d.out })
}
"#,
            &catalog,
            &enricher,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // Enricher-provided INPUT pins a and b are recognized and wired.
        for placeholder in ["a", "b"] {
            assert!(
                result.commands.iter().any(|command| {
                    matches!(
                        command,
                        BoardCommand::ConnectPins { from_node, to_node, to_pin, .. }
                            if command_node_type(&result.commands, from_node).as_deref()
                                == Some("text_source")
                                && command_node_type(&result.commands, to_node).as_deref()
                                    == Some("dynamic_node")
                                && to_pin == placeholder
                    )
                }),
                "dynamic input `{placeholder}` not wired; commands: {:?}",
                result.commands
            );
        }
        // Enricher-provided OUTPUT pin `out` resolves for `d.out` and wires into the sink.
        assert!(
            result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                        if command_node_type(&result.commands, from_node).as_deref()
                            == Some("dynamic_node")
                            && from_pin == "out"
                            && command_node_type(&result.commands, to_node).as_deref() == Some("sink")
                            && to_pin == "input"
                )
            }),
            "dynamic output `out` not wired into sink; commands: {:?}",
            result.commands
        );
    }

    #[test]
    fn catalog_aware_reconcile_lowers_event_return_to_result_node() {
        // A `return` inside a keyword-less event/tool entry must NOT be rejected — it reverses the
        // `events_generic_return_result` sugar so agent tools and event handlers can return a value.
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_generic_return_result",
                "Return Generic Result",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("response", "Generic", PinType::Input),
                ],
                Vec::new(),
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    return "done"
}
"#,
            &catalog,
        );

        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.contains("only supported inside FlowScript functions")),
            "event return must not be rejected: {:?}",
            result.diagnostics
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { node_type, .. }
                    if node_type == "events_generic_return_result"
            )),
            "return should add a result node: {:?}",
            result.commands
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { pin_id, value, .. }
                    if pin_id == "response"
                        && value == &flow_like_types::Value::String("done".to_string())
            )),
            "return value should feed the response pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn catalog_aware_reconcile_chains_exec_through_and_after_loops() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("array", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("loop", "Execution", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                    pin_meta("item", "Generic", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    for (const item of forEach()) {
        log({ text: "inner" })
    }
    log({ text: "after" })
    log({ text: "last" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let connects: Vec<(String, String, String, String)> = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                    ..
                } => Some((
                    from_node.clone(),
                    from_pin.clone(),
                    to_node.clone(),
                    to_pin.clone(),
                )),
                _ => None,
            })
            .collect();

        // $0 event, $1 for_each, $2 log(inner), $3 log(after), $4 log(last)
        let expected = [
            ("$0", "exec_out", "$1", "exec_in"),
            ("$1", "loop", "$2", "exec_in"),
            ("$1", "done", "$3", "exec_in"),
            // The regression: a statement AFTER a loop must chain onward from ITS exec output.
            // Gating the cursor advance on the cursor's own (non-default) pin instead of the
            // streaming-preferred output left every statement after the loop dangling.
            ("$3", "exec_out", "$4", "exec_in"),
        ];
        for (from_node, from_pin, to_node, to_pin) in expected {
            assert!(
                connects.iter().any(|(fnode, fpin, tnode, tpin)| {
                    fnode == from_node && fpin == from_pin && tnode == to_node && tpin == to_pin
                }),
                "missing exec connection {from_node}.{from_pin} -> {to_node}.{to_pin}; got {connects:?}"
            );
        }
        assert!(
            !connects
                .iter()
                .any(|(fnode, fpin, tnode, _)| fnode == "$1" && fpin == "done" && tnode == "$4"),
            "log(last) must chain from log(after), not from the loop's done pin again; got {connects:?}"
        );
    }

    #[test]
    fn catalog_aware_reconcile_splices_impure_argument_calls_into_exec_chain() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            // Pure: no exec pins — must stay data-only.
            catalog_meta(
                "make_struct",
                "Make Struct",
                Vec::new(),
                vec![pin_meta("struct", "Struct", PinType::Output)],
            ),
            // Impure: has exec pins — must be spliced into the exec chain.
            catalog_meta(
                "faker_full_name",
                "Fake Full Name",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("full_name", "String", PinType::Output),
                ],
            ),
            catalog_meta(
                "set_field",
                "Set Field",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("struct", "Struct", PinType::Input),
                    pin_meta("value", "String", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("result", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    setField({ struct: makeStruct(), value: fakerFullName() })
    log({ text: "done" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let connects: Vec<(String, String, String, String)> = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                    ..
                } => Some((
                    from_node.clone(),
                    from_pin.clone(),
                    to_node.clone(),
                    to_pin.clone(),
                )),
                _ => None,
            })
            .collect();

        // $0 event, $1 set_field, $2 make_struct (pure), $3 faker (impure), $4 log
        let expected = [
            // The impure argument call is spliced into the chain ahead of its consumer…
            ("$0", "exec_out", "$3", "exec_in"),
            ("$3", "exec_out", "$1", "exec_in"),
            // …and the chain continues normally after the consuming statement.
            ("$1", "exec_out", "$4", "exec_in"),
            // Data wiring stays intact.
            ("$3", "full_name", "$1", "value"),
            ("$2", "struct", "$1", "struct"),
        ];
        for (from_node, from_pin, to_node, to_pin) in expected {
            assert!(
                connects.iter().any(|(fnode, fpin, tnode, tpin)| {
                    fnode == from_node && fpin == from_pin && tnode == to_node && tpin == to_pin
                }),
                "missing connection {from_node}.{from_pin} -> {to_node}.{to_pin}; got {connects:?}"
            );
        }
        // The pure helper must not receive exec wiring.
        assert!(
            !connects.iter().any(|(fnode, fpin, tnode, tpin)| {
                (fnode == "$2" && fpin.contains("exec")) || (tnode == "$2" && tpin.contains("exec"))
            }),
            "make_struct is pure and must stay data-only; got {connects:?}"
        );
    }

    #[test]
    fn catalog_aware_reconcile_resolves_local_literal_aliases() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    let text = "hello"
    log({ text: text })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "$1"
                        && pin_id == "text"
                        && value == &flow_like_types::Value::String("hello".to_string())
            )
        }));
    }

    #[test]
    fn legacy_function_layer_names_compare_in_flowscript_form() {
        let mut board = empty_board();
        let layer = Layer::new(
            "legacy-function".to_string(),
            "Handle Ticket".to_string(),
            LayerType::Function,
        );
        board.layers.insert(layer.id.clone(), layer);
        let ast = super::super::lower_to_ast(&board);

        assert_eq!(ast.functions[0].name, "handleTicket");
        let result = reconcile_with_catalog(&board, &ast, &[]);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.is_empty(), "{:?}", result.commands);
    }

    #[test]
    fn function_cache_decorator_updates_and_removes_existing_layer_cache() {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "cached-function".to_string(),
            "Cached Lookup".to_string(),
            LayerType::Function,
        );
        layer.cache = Some(LayerCache {
            enabled: true,
            prefix: "old".to_string(),
            ttl_seconds: Some(60),
            scope: LayerCacheScope::App,
        });
        board.layers.insert(layer.id.clone(), layer);

        let updated = flow_like_ast::parse(
            "@cache({ namespace: \"pricing\", ttlSeconds: 3600, scope: \"user\" })\nfunction cachedLookup() {   //@l:cached-function\n}\n",
        )
        .expect("parse cached function");
        let result = reconcile(&board, &updated);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateLayerCache {
                layer_id,
                cache: Some(LayerCache {
                    enabled: true,
                    prefix,
                    ttl_seconds: Some(3600),
                    scope: LayerCacheScope::User,
                }),
                ..
            } if layer_id == "cached-function" && prefix == "pricing"
        )));

        let uncached =
            flow_like_ast::parse("function cachedLookup() {   //@l:cached-function\n}\n")
                .expect("parse uncached function");
        let result = reconcile(&board, &uncached);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateLayerCache {
                layer_id,
                cache: None,
                ..
            } if layer_id == "cached-function"
        )));
    }

    #[test]
    fn permanent_cached_function_board_roundtrips_without_commands() {
        for (index, persisted_ttl) in [None, Some(0)].into_iter().enumerate() {
            let mut board = empty_board();
            let mut layer = Layer::new(
                format!("cached-function-{index}"),
                "Cached Lookup".to_string(),
                LayerType::Function,
            );
            layer.cache = Some(LayerCache {
                enabled: true,
                prefix: "pricing".to_string(),
                ttl_seconds: persisted_ttl,
                scope: LayerCacheScope::User,
            });
            board.layers.insert(layer.id.clone(), layer);

            let ast = crate::flow::ast::lower_to_ast(&board);
            let source = flow_like_ast::render(
                &ast,
                &flow_like_ast::RenderOptions {
                    anchors: true,
                    ..Default::default()
                },
            );
            assert!(
                source
                    .contains("@cache({ namespace: \"pricing\", ttlSeconds: 0, scope: \"user\" })")
            );
            let parsed =
                flow_like_ast::parse(&source).expect("canonical cached function should parse");
            let result = reconcile(&board, &parsed);
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            assert!(
                result.commands.is_empty(),
                "permanent cache form {persisted_ttl:?} must round-trip as a no-op: {:?}",
                result.commands
            );
        }
    }

    #[test]
    fn default_cached_function_roundtrips_as_bare_decorator_without_commands() {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "cached-function".to_string(),
            "Cached Lookup".to_string(),
            LayerType::Function,
        );
        layer.cache = Some(LayerCache {
            enabled: true,
            prefix: "global".to_string(),
            ttl_seconds: Some(300),
            scope: LayerCacheScope::App,
        });
        board.layers.insert(layer.id.clone(), layer);

        let ast = crate::flow::ast::lower_to_ast(&board);
        let source = flow_like_ast::render(
            &ast,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );
        assert!(source.starts_with("@cache\nfunction cachedLookup"));

        let parsed = flow_like_ast::parse(&source).expect("default cache should parse");
        let result = reconcile(&board, &parsed);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.is_empty(),
            "semantic cache defaults must round-trip as a no-op: {:?}",
            result.commands
        );
    }

    #[test]
    fn bare_cache_creates_a_function_layer_with_semantic_defaults() {
        let catalog = vec![catalog_meta(
            "log_info",
            "Log Info",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("message", "String", PinType::Input),
            ],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        )];
        let ast = flow_like_ast::parse(
            "@cache\nfunction cachedLookup() {\n    logInfo({ message: \"lookup\" })\n}\n",
        )
        .expect("bare cache should parse");
        let result = reconcile_with_catalog(&empty_board(), &ast, &catalog);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::CreateLayer {
                layer_type: Some(layer_type),
                cache: Some(LayerCache {
                    enabled: true,
                    prefix,
                    ttl_seconds: Some(300),
                    scope: LayerCacheScope::App,
                }),
                ..
            } if layer_type == "Function" && prefix == "global"
        )));
    }

    #[test]
    fn new_cached_function_carries_cache_on_create_layer() {
        let catalog = vec![catalog_meta(
            "log_info",
            "Log Info",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("message", "String", PinType::Input),
            ],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        )];
        let result = reconcile_text_with_catalog(
            &empty_board(),
            "@cache({ namespace: \"pricing\", ttlSeconds: 15, scope: \"user\" })\nfunction cachedLookup() {\n    logInfo({ message: \"lookup\" })\n}\n",
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::CreateLayer {
                layer_type: Some(layer_type),
                cache: Some(LayerCache {
                    enabled: true,
                    prefix,
                    ttl_seconds: Some(15),
                    scope: LayerCacheScope::User,
                }),
                ..
            } if layer_type == "Function" && prefix == "pricing"
        )));
    }

    #[test]
    fn catalog_aware_reconcile_creates_function_layers_from_functions() {
        let board = empty_board();
        let catalog = vec![catalog_meta(
            "string_format",
            "String Format",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("format_string", "String", PinType::Input),
                pin_meta("name", "String", PinType::Input),
            ],
            vec![
                pin_meta("exec_out", "Execution", PinType::Output),
                pin_meta("value", "String", PinType::Output),
            ],
        )];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(name: string): (message: string) {
    const formatted = stringFormat({ formatString: "Hello {name}", name: name })
    return formatted.value
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::CreateLayer {
                name,
                ref_id,
                layer_type,
                pins: Some(pins),
                ..
            } if name == "greet"
                && ref_id.as_deref() == Some("$0")
                && layer_type.as_deref() == Some("Function")
                && pins.iter().any(|pin| pin.name == "name" && pin.pin_type == "Input")
                && pins.iter().any(|pin| pin.name == "message" && pin.pin_type == "Output")
        ));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, target_layer, .. }
                    if node_type == "string_format" && target_layer.as_deref() == Some("$0")
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$0"
                        && from_pin == "name"
                        && to_node == "$1"
                        && to_pin == "name"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "value"
                        && to_node == "$0"
                        && to_pin == "message"
            )
        }));
    }

    #[test]
    fn impure_function_gets_exec_boundary_pins_and_body_wiring() {
        let board = empty_board();
        let catalog = vec![catalog_meta(
            "log",
            "Log",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("message", "String", PinType::Input),
            ],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        )];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function notify(message: string) {
    log({ message: message })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::CreateLayer { name, pins: Some(pins), .. }
                if name == "notify"
                    && pins.iter().any(|pin| pin.name == "exec_in"
                        && pin.pin_type == "Input"
                        && pin.data_type == "Execution")
                    && pins.iter().any(|pin| pin.name == "exec_out"
                        && pin.pin_type == "Output"
                        && pin.data_type == "Execution")
        ));
        // Body entry: layer exec_in → log exec_in; exit: log exec_out → layer exec_out.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$0" && from_pin == "exec_in" && to_node == "$1" && to_pin == "exec_in"
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1" && from_pin == "exec_out" && to_node == "$0" && to_pin == "exec_out"
        )));
    }

    #[test]
    fn pure_function_layer_stays_free_of_exec_pins() {
        let board = empty_board();
        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(name: string): (message: string) {
    const formatted = stringFormat({ formatString: "Hello {name}", name: name })
    return formatted.value
}
"#,
            &string_format_dynamic_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::CreateLayer { pins: Some(pins), .. }
                if pins.iter().all(|pin| pin.data_type != "Execution")
        ));
    }

    #[test]
    fn calling_a_declared_function_creates_call_function_node() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_call_function",
                "Call Function",
                vec![pin_meta("function_layer_id", "String", PinType::Input)],
                vec![],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"run() {
    notify({ message: "hi" })
}

function notify(message: string) {
    log({ message: message })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // Refs follow planning order: layer $0, event $1, call node $2, body log node $3.
        // Command application order intentionally differs: workflow/layer nodes are added first
        // and the Event entry is the final AddNode registration target.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, ref_id, target_layer, .. }
                if node_type == "control_call_function"
                    && ref_id.as_deref() == Some("$2")
                    && target_layer.is_none()
        )));
        let event_add_index = result
            .commands
            .iter()
            .position(|command| {
                matches!(
                    command,
                    BoardCommand::AddNode { node_type, .. } if node_type == "events_simple"
                )
            })
            .expect("event entry add command");
        let last_logic_add_index = result
            .commands
            .iter()
            .enumerate()
            .filter(|(_, command)| {
                matches!(
                    command,
                    BoardCommand::AddNode { node_type, .. } if node_type != "events_simple"
                )
            })
            .map(|(index, _)| index)
            .max()
            .expect("workflow add commands");
        assert!(
            event_add_index > last_logic_add_index,
            "event entry must be added after workflow nodes: {:?}",
            result.commands
        );
        // The call node targets the new function layer by its ref.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "$2"
                    && pin_id == "function_layer_id"
                    && value == &flow_like_types::Value::String("$0".to_string())
        )));
        // The call's literal argument lands on the mirrored `message` pin.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "$2"
                    && pin_id == "message"
                    && value == &flow_like_types::Value::String("hi".to_string())
        )));
        // The call node joins the event's execution chain.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1" && from_pin == "exec_out" && to_node == "$2" && to_pin == "exec_in"
        )));
        // And the function body chains from the layer's exec boundary.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$0" && from_pin == "exec_in" && to_node == "$3" && to_pin == "exec_in"
        )));
        // The final body node closes the function's execution chain at its layer boundary.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$3" && from_pin == "exec_out" && to_node == "$0" && to_pin == "exec_out"
        )));
    }

    #[test]
    fn focused_helpers_can_feed_separate_event_entries_registered_last() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_generic",
                "Generic Event",
                vec![],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("payload", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "control_call_function",
                "Call Function",
                vec![pin_meta("function_layer_id", "String", PinType::Input)],
                vec![],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function pollInbox() {
    log({ message: "poll" })
}

function processApproval(payload: Struct) {
    log({ message: "approval" })
}

eventsSimple() {
    pollInbox()
}

eventsGeneric(payload: Struct) {
    processApproval({ payload: payload })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            result
                .commands
                .iter()
                .filter(|command| matches!(command, BoardCommand::CreateLayer { .. }))
                .count(),
            2,
            "each focused helper must become its own function layer"
        );
        assert_eq!(
            result
                .commands
                .iter()
                .filter(|command| matches!(
                    command,
                    BoardCommand::AddNode { node_type, .. }
                        if node_type == "control_call_function"
                ))
                .count(),
            2,
            "each Event must invoke its own helper through a call node"
        );

        let event_indices = result
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| match command {
                BoardCommand::AddNode { node_type, .. }
                    if matches!(node_type.as_str(), "events_simple" | "events_generic") =>
                {
                    Some(index)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(event_indices.len(), 2, "both Event roots must be retained");
        let last_logic_index = result
            .commands
            .iter()
            .enumerate()
            .filter(|(_, command)| {
                matches!(command, BoardCommand::CreateLayer { .. })
                    || matches!(
                        command,
                        BoardCommand::AddNode { node_type, .. }
                            if !matches!(node_type.as_str(), "events_simple" | "events_generic")
                    )
            })
            .map(|(index, _)| index)
            .max()
            .expect("helper logic commands");
        assert!(
            event_indices
                .iter()
                .all(|event_index| *event_index > last_logic_index),
            "Event registration must happen after all helper layers and call nodes: {:?}",
            result.commands
        );
    }

    #[test]
    fn rejects_concrete_date_output_for_string_helper_parameter() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_call_function",
                "Call Function",
                vec![pin_meta("function_layer_id", "String", PinType::Input)],
                vec![],
            ),
            catalog_meta(
                "utils_datetime_now",
                "Now",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("date", "Date", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function saveTicket(updatedAt: string) {
    log({ message: updatedAt })
}

eventsSimple() {
    const now = utilsDatetimeNow()
    saveTicket({ updatedAt: now.date })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("argument `updatedAt` on `saveTicket`")
                && diagnostic.contains("`Date/Normal`")
                && diagnostic.contains("`String/Normal`")
                && diagnostic.contains("catalog-declared conversion")
        }));
        assert!(
            !result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { to_pin, .. } if to_pin == "updatedAt"
            )),
            "an incompatible helper argument must not be wired: {:?}",
            result.commands
        );
    }

    #[test]
    fn planned_output_type_requires_concrete_container_match_but_keeps_generic_permissive() {
        let scalar_struct = pin_meta("row", "Struct", PinType::Input);
        let mut array_struct = pin_meta("rows", "Struct", PinType::Input);
        array_struct.value_type = "Array".to_string();

        let scalar_output = PlannedOutputType {
            source: "source.row".to_string(),
            pin_name: "row".to_string(),
            data_type: "Struct".to_string(),
            value_type: "Normal".to_string(),
            is_generic: false,
            schema: None,
            enforce_schema: false,
        };
        let array_output = PlannedOutputType {
            source: "source.rows".to_string(),
            pin_name: "rows".to_string(),
            data_type: "Struct".to_string(),
            value_type: "Array".to_string(),
            is_generic: false,
            schema: None,
            enforce_schema: false,
        };

        let refs = HashMap::new();
        assert!(planned_output_is_compatible(
            &scalar_struct,
            &scalar_output,
            &refs
        ));
        assert!(!planned_output_is_compatible(
            &scalar_struct,
            &array_output,
            &refs
        ));
        assert!(!planned_output_is_compatible(
            &array_struct,
            &scalar_output,
            &refs
        ));

        let generic_input = pin_meta("value", "Generic", PinType::Input);
        assert!(planned_output_is_compatible(
            &generic_input,
            &array_output,
            &refs
        ));
        let generic_output = PlannedOutputType {
            source: "source.value".to_string(),
            pin_name: "value".to_string(),
            data_type: "Generic".to_string(),
            value_type: "Normal".to_string(),
            is_generic: true,
            schema: None,
            enforce_schema: false,
        };
        assert!(planned_output_is_compatible(
            &array_struct,
            &generic_output,
            &refs
        ));

        let mut generic_array_input = pin_meta("values", "Generic", PinType::Input);
        generic_array_input.value_type = "Array".to_string();
        let scalar_struct_output = pin_meta("row", "Struct", PinType::Output);
        assert!(!planned_output_is_compatible(
            &generic_array_input,
            &scalar_output,
            &refs
        ));
        assert!(!metadata_pins_are_compatible(
            &generic_array_input,
            &scalar_struct_output,
            &refs
        ));
        let generic_array_output = PlannedOutputType {
            source: "source.values".to_string(),
            pin_name: "values".to_string(),
            data_type: "Generic".to_string(),
            value_type: "Array".to_string(),
            is_generic: true,
            schema: None,
            enforce_schema: false,
        };
        assert!(!planned_output_is_compatible(
            &scalar_struct,
            &generic_array_output,
            &refs
        ));
        assert!(planned_output_is_compatible(
            &generic_input,
            &generic_array_output,
            &refs
        ));
        let generic_dynamic_output = pin_meta("value", "Generic", PinType::Output);
        assert!(metadata_pins_are_compatible(
            &generic_array_input,
            &generic_dynamic_output,
            &refs
        ));
    }

    #[test]
    fn new_connections_require_compatible_enforced_struct_schemas() {
        let mut input = pin_meta("payload", "Struct", PinType::Input);
        input.schema =
            Some(r#"{"type":"object","properties":{"subject":{"type":"string"}}}"#.to_string());
        input.enforce_schema = true;
        let mut output = pin_meta("message", "Struct", PinType::Output);
        output.schema =
            Some(r#"{"properties":{"uid":{"type":"integer"}},"type":"object"}"#.to_string());
        output.enforce_schema = true;
        let refs = HashMap::new();

        assert!(!metadata_pins_are_compatible(&input, &output, &refs));
        let planned = PlannedOutputType {
            source: "mail.message".to_string(),
            pin_name: output.name.clone(),
            data_type: output.data_type.clone(),
            value_type: output.value_type.clone(),
            is_generic: false,
            schema: output.schema.clone(),
            enforce_schema: true,
        };
        assert!(!planned_output_is_compatible(&input, &planned, &refs));

        output.schema = Some(
            r#"{ "properties": { "subject": { "type": "string" } }, "type": "object" }"#
                .to_string(),
        );
        assert!(
            metadata_pins_are_compatible(&input, &output, &refs),
            "schema comparison must ignore JSON whitespace and key order"
        );

        output.schema =
            Some(r#"{"type":"object","properties":{"uid":{"type":"integer"}}}"#.to_string());
        input.name = "struct_in".to_string();
        assert!(
            metadata_pins_are_compatible(&input, &output, &refs),
            "schema-adopting struct nodes must accept the connected Struct schema"
        );
        input.name = "payload".to_string();
        assert!(
            !metadata_pins_are_compatible(&input, &output, &refs),
            "ordinary Struct pins must still reject mismatched schemas"
        );
        input.enforce_schema = false;
        output.enforce_schema = false;
        assert!(
            metadata_pins_are_compatible(&input, &output, &refs),
            "differing descriptive schemas must remain connectable when neither side enforces"
        );
    }

    #[test]
    fn new_edge_from_existing_node_is_checked_but_identical_legacy_edge_is_retained() {
        let mut board = empty_board();
        let mut source_node = Node::new("string_source", "String Source", "", "test");
        source_node.id = "source".to_string();
        let source_pin_id = source_node
            .add_output_pin("value", "Value", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(source_node.id.clone(), source_node);

        let mut legacy_sink = Node::new("date_sink", "Date Sink", "", "test");
        legacy_sink.id = "legacy_sink".to_string();
        let legacy_input_id = legacy_sink
            .add_input_pin("date", "Date", "", VariableType::Date)
            .id
            .clone();
        board.nodes.insert(legacy_sink.id.clone(), legacy_sink);
        // Deliberately retain an old invalid edge: reconcile must not make unchanged boards
        // uneditable, but it must not use this grandfathering for any newly authored edge.
        connect(
            &mut board,
            "source",
            &source_pin_id,
            "legacy_sink",
            &legacy_input_id,
        );

        let source = ValueSource {
            node: NodeEntity::Existing("source".to_string()),
            output_pin: Some("value".to_string()),
        };
        let call = Call {
            node_type: "date_sink".to_string(),
            display: "dateSink".to_string(),
            args: vec![Arg {
                name: "date".to_string(),
                value: Expr::Ref("existingValue".to_string()),
            }],
            anchor: None,
        };

        let legacy_meta = node_to_metadata(&board.nodes["legacy_sink"]);
        let mut retained = StructuralPlanner::new(&board, &[], None);
        retained.push_scope();
        retained.symbols.last_mut().unwrap().insert(
            "existingValue".to_string(),
            SymbolValue::Source(source.clone()),
        );
        retained.plan_call_arguments(
            &call,
            &NodeEntity::Existing("legacy_sink".to_string()),
            &legacy_meta,
            None,
            true,
        );
        assert!(retained.result.diagnostics.is_empty());
        assert!(retained.connect_commands.is_empty());

        let new_meta = catalog_meta(
            "date_sink",
            "Date Sink",
            vec![pin_meta("date", "Date", PinType::Input)],
            Vec::new(),
        );
        let mut newly_authored = StructuralPlanner::new(&board, &[], None);
        newly_authored.push_scope();
        newly_authored
            .symbols
            .last_mut()
            .unwrap()
            .insert("existingValue".to_string(), SymbolValue::Source(source));
        newly_authored.plan_call_arguments(
            &call,
            &NodeEntity::New {
                ref_id: "$new_sink".to_string(),
                meta: new_meta.clone(),
            },
            &new_meta,
            None,
            true,
        );
        assert!(newly_authored.result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("incompatible pin types or schemas")
                && diagnostic.contains("String/Normal")
                && diagnostic.contains("Date/Normal")
        }));
        assert!(newly_authored.connect_commands.is_empty());
    }

    #[test]
    fn function_return_rejects_a_new_incompatible_data_edge() {
        let catalog = vec![catalog_meta(
            "date_source",
            "Date Source",
            Vec::new(),
            vec![pin_meta("date", "Date", PinType::Output)],
        )];
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function makeMessage(): (message: string) {
    const value = dateSource()
    return value.date
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("return value 1")
                && diagnostic.contains("incompatible pin types or schemas")
                && diagnostic.contains("Date/Normal")
                && diagnostic.contains("String/Normal")
        }));
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { to_pin, .. } if to_pin == "message"
        )));
    }

    #[test]
    fn function_literal_returns_materialize_typed_variable_sources() {
        let catalog = vec![catalog_meta(
            "variable_get",
            "Get Variable",
            vec![pin_meta("var_ref", "String", PinType::Input)],
            vec![pin_meta("value_ref", "Generic", PinType::Output)],
        )];
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function constantTag(): (tag: string) {
    return "final"
}

function constantCount(): (count: int) {
    return 42
}

function constantFlag(): (flag: bool) {
    return true
}
"#,
            &catalog,
        );

        // Literal-only functions must reconcile cleanly: no FS_FUNCTION_RETURN_MISMATCH and no
        // FS_HELPER_EMPTY cascade (the materialized variable_get IS a body node).
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        for (function, return_pin, data_type, default_value) in [
            (
                "constantTag",
                "tag",
                "String",
                flow_like_types::Value::from("final"),
            ),
            (
                "constantCount",
                "count",
                "Integer",
                flow_like_types::Value::from(42),
            ),
            (
                "constantFlag",
                "flag",
                "Boolean",
                flow_like_types::Value::from(true),
            ),
        ] {
            let layer_ref = result
                .commands
                .iter()
                .find_map(|command| match command {
                    BoardCommand::CreateLayer {
                        name,
                        ref_id: Some(ref_id),
                        ..
                    } if name == function => Some(ref_id.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing function layer for {function}"));
            let variable_id = result
                .commands
                .iter()
                .find_map(|command| match command {
                    BoardCommand::CreateVariable {
                        variable_id: Some(id),
                        name,
                        data_type: created_type,
                        default_value: Some(default),
                        target_layer: Some(target),
                        ..
                    } if name == &format!("{function}_{return_pin}")
                        && created_type == data_type
                        && default == &default_value
                        && target == &layer_ref =>
                    {
                        Some(id.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    panic!(
                        "missing typed literal variable for {function}: {:?}",
                        result.commands
                    )
                });
            let getter_ref = result
                .commands
                .iter()
                .find_map(|command| match command {
                    BoardCommand::AddNode {
                        node_type,
                        ref_id: Some(ref_id),
                        target_layer: Some(target),
                        ..
                    } if node_type == "variable_get" && target == &layer_ref => {
                        Some(ref_id.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing variable_get in {function} layer"));
            assert!(
                result.commands.iter().any(|command| matches!(
                    command,
                    BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                        if node_id == &getter_ref
                            && pin_id == "var_ref"
                            && value == &flow_like_types::Value::String(variable_id.clone())
                )),
                "variable_get for {function} must select the literal variable"
            );
            assert!(
                result.commands.iter().any(|command| matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                        if from_node == &getter_ref
                            && from_pin == "value_ref"
                            && to_node == &layer_ref
                            && to_pin == return_pin
                )),
                "missing boundary return connection for {function}: {:?}",
                result.commands
            );
        }
    }

    /// Board mirroring what applying `function constantTag(): (tag: string) { return "final" }`
    /// produces: a Function layer whose boundary return pin is fed by a `variable_get` reading
    /// the materialized layer-local literal variable.
    fn board_with_materialized_literal_return() -> Board {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "fn-layer".to_string(),
            "constantTag".to_string(),
            LayerType::Function,
        );

        let mut template = Node::new("boundary", "Boundary", "", "test");
        let return_pin = template
            .add_output_pin("tag", "Tag", "", VariableType::String)
            .clone();
        layer.pins.insert(return_pin.id.clone(), return_pin.clone());

        let mut variable =
            Variable::new("constantTag_tag", VariableType::String, ValueType::Normal);
        variable.id = "var_constantTag_tag".to_string();
        variable.set_default_value(flow_like_types::Value::String("final".to_string()));
        layer.variables.insert(variable.id.clone(), variable);

        let mut getter = Node::new("variable_get", "Get Variable", "", "variables");
        getter.id = "getter".to_string();
        getter.layer = Some(layer.id.clone());
        getter
            .add_input_pin("var_ref", "Variable", "", VariableType::String)
            .default_value = Some(b"\"var_constantTag_tag\"".to_vec());
        let value_ref = getter.add_output_pin("value_ref", "Value", "", VariableType::String);
        value_ref.connected_to.insert(return_pin.id.clone());
        let value_ref_id = value_ref.id.clone();
        board.nodes.insert(getter.id.clone(), getter);

        layer
            .pins
            .get_mut(&return_pin.id)
            .expect("boundary return pin")
            .depends_on
            .insert(value_ref_id);
        board.layers.insert(layer.id.clone(), layer);
        board
    }

    #[test]
    fn lowered_literal_return_folds_decl_and_roundtrips_cleanly() {
        let board = board_with_materialized_literal_return();
        let text = anchored_text(&board);

        assert!(
            text.contains("return \"final\""),
            "lowering must preserve the literal return statement:\n{text}"
        );
        assert!(
            !text.contains("constantTag_tag") && !text.contains("constantTagTag"),
            "the materialized variable's inert local decl must be folded into the return:\n{text}"
        );

        let result = reconcile_text_with_catalog(&board, &text, &[]);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.is_empty(),
            "literal-return roundtrip must be a no-op: {:?}",
            result.commands
        );
    }

    #[test]
    fn changed_literal_return_updates_the_materialized_variable_in_place() {
        let board = board_with_materialized_literal_return();
        let text = anchored_text(&board).replace("return \"final\"", "return \"changed\"");

        let result = reconcile_text_with_catalog(&board, &text, &[]);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            result.commands.len(),
            1,
            "a changed literal must reuse the materialized variable: {:?}",
            result.commands
        );
        assert!(matches!(
            &result.commands[0],
            BoardCommand::UpdateVariable {
                variable_id,
                default_value: Some(flow_like_types::Value::String(value)),
                ..
            } if variable_id == "var_constantTag_tag" && value == "changed"
        ));
    }

    #[tokio::test]
    async fn applied_loop_body_return_roundtrip_is_idempotent() {
        use crate::state::{FlowLikeConfig, FlowLikeState};
        use crate::utils::http::HTTPClient;
        use std::sync::Arc;

        let mut for_each = Node::new("control_for_each", "For Each", "", "control");
        for_each.add_input_pin("exec_in", "In", "", VariableType::Execution);
        for_each
            .add_input_pin("array", "Array", "", VariableType::Generic)
            .value_type = ValueType::Array;
        for_each.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        for_each.add_output_pin("value", "Value", "", VariableType::Generic);
        for_each.add_output_pin("index", "Index", "", VariableType::Integer);
        for_each.add_output_pin("done", "Done", "", VariableType::Execution);

        let mut push = Node::new("array_push", "Push", "", "array");
        push.add_input_pin("exec_in", "In", "", VariableType::Execution);
        push.add_input_pin("array_in", "Array", "", VariableType::Generic)
            .value_type = ValueType::Array;
        push.add_input_pin("value", "Value", "", VariableType::Generic);
        push.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        push.add_output_pin("array_out", "Array", "", VariableType::Generic)
            .value_type = ValueType::Array;

        let catalog_nodes = vec![for_each, push];
        let mut board = empty_board();
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let script = r#"function parseRssXml(items: Generic[]): (rows: Generic[]) {
    for (const item of controlForEach({ array: items })) {
        const batchPush = arrayPush({ arrayIn: items, value: item.value })
    }
    return batchPush.arrayOut
}
"#;
        let applied = super::super::apply_flowscript_to_board(
            &mut board,
            script,
            &catalog_nodes,
            state,
            None,
            false,
        )
        .await
        .expect("loop accumulator script applies");
        assert!(applied.diagnostics.is_empty(), "{:?}", applied.diagnostics);

        // The lowerer names bindings after the node, not after the source that created it, so
        // find the accumulator's rendered name instead of assuming the authored one survived.
        let text = anchored_text(&board);
        let binding = text
            .lines()
            .find_map(|line| {
                let (name, call) = line.trim().strip_prefix("const ")?.split_once(" = ")?;
                call.starts_with("arrayPush(").then(|| name.to_string())
            })
            .expect("the accumulator must lower as a binding inside the loop body");
        assert!(
            text.contains(&format!("return {binding}.arrayOut")),
            "the lowered board must still read the loop-local accumulator:\n{text}"
        );

        let catalog: Vec<NodeMetadata> = catalog_nodes.iter().map(node_to_metadata).collect();
        let result = reconcile_text_with_catalog(&board, &text, &catalog);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.is_empty(),
            "re-applying the board's own lowered script must be a no-op: {:?}",
            result.commands
        );
    }

    #[tokio::test]
    async fn applied_literal_return_roundtrip_is_idempotent() {
        use crate::state::{FlowLikeConfig, FlowLikeState};
        use crate::utils::http::HTTPClient;
        use std::sync::Arc;

        let mut variable_get = Node::new("variable_get", "Get Variable", "", "variables");
        variable_get.add_input_pin("var_ref", "Variable", "", VariableType::String);
        variable_get.add_output_pin("value_ref", "Value", "", VariableType::Generic);
        let catalog_nodes = vec![variable_get];

        let mut board = empty_board();
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let script = r#"function constantTag(): (tag: string) {
    return "final"
}
"#;
        let applied = super::super::apply_flowscript_to_board(
            &mut board,
            script,
            &catalog_nodes,
            state,
            None,
            false,
        )
        .await
        .expect("literal return script applies");
        assert!(applied.diagnostics.is_empty(), "{:?}", applied.diagnostics);
        assert_eq!(
            board
                .layers
                .values()
                .map(|layer| layer.variables.len())
                .sum::<usize>(),
            1,
            "apply materializes exactly one literal variable"
        );

        let text = anchored_text(&board);
        assert!(
            text.contains("return \"final\""),
            "lowered board must keep the return statement:\n{text}"
        );

        let catalog: Vec<NodeMetadata> = catalog_nodes.iter().map(node_to_metadata).collect();
        let result = reconcile_text_with_catalog(&board, &text, &catalog);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.is_empty(),
            "re-reconciling the board's own lowered script must be a no-op: {:?}",
            result.commands
        );
    }

    #[test]
    fn branch_arm_literal_returns_share_one_materialized_variable() {
        let catalog = vec![
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "control_branch",
                "Branch",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("condition", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("true", "Execution", PinType::Output),
                    pin_meta("false", "Execution", PinType::Output),
                ],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function pickTag(): (tag: string) {
    if (true) {
        return "a"
    } else {
        return "b"
    }
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let created_variables = result
            .commands
            .iter()
            .filter(|command| matches!(command, BoardCommand::CreateVariable { .. }))
            .count();
        assert_eq!(
            created_variables, 1,
            "both arms must share one materialized variable: {:?}",
            result.commands
        );
        let getters = result
            .commands
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    BoardCommand::AddNode { node_type, .. } if node_type == "variable_get"
                )
            })
            .count();
        assert_eq!(getters, 1, "{:?}", result.commands);
        let return_edges = result
            .commands
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { to_pin, .. } if to_pin == "tag"
                )
            })
            .count();
        assert_eq!(return_edges, 1, "{:?}", result.commands);
    }

    #[tokio::test]
    async fn promoted_local_roundtrip_reapplies_cleanly() {
        use crate::state::{FlowLikeConfig, FlowLikeState};
        use crate::utils::http::HTTPClient;
        use std::sync::Arc;

        let mut variable_get = Node::new("variable_get", "Get Variable", "", "variables");
        variable_get.add_input_pin("var_ref", "Variable", "", VariableType::String);
        variable_get.add_output_pin("value_ref", "Value", "", VariableType::Generic);

        let mut variable_set = Node::new("variable_set", "Set Variable", "", "variables");
        variable_set.add_input_pin("exec_in", "In", "", VariableType::Execution);
        variable_set.add_input_pin("var_ref", "Variable", "", VariableType::String);
        variable_set.add_input_pin("value_in", "Value", "", VariableType::Generic);
        variable_set.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        variable_set.add_output_pin("value_ref", "Value", "", VariableType::Generic);

        let mut string_trim = Node::new("string_trim", "String Trim", "", "strings");
        string_trim.add_input_pin("string", "String", "", VariableType::String);
        string_trim.add_output_pin("trimmed", "Trimmed", "", VariableType::String);

        let mut control_branch = Node::new("control_branch", "Branch", "", "control");
        control_branch.add_input_pin("exec_in", "In", "", VariableType::Execution);
        control_branch.add_input_pin("condition", "Condition", "", VariableType::Boolean);
        control_branch.add_output_pin("true", "True", "", VariableType::Execution);
        control_branch.add_output_pin("false", "False", "", VariableType::Execution);

        let catalog_nodes = vec![variable_get, variable_set, string_trim, control_branch];

        let mut board = empty_board();
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let script = r#"function makeTag(): (tag: string) {
    let label = stringTrim({ string: " raw " })
    if (true) {
        label = stringTrim({ string: " hot " })
    }
    return label
}
"#;
        let applied = super::super::apply_flowscript_to_board(
            &mut board,
            script,
            &catalog_nodes,
            state,
            None,
            false,
        )
        .await
        .expect("promoted local script applies");
        assert!(applied.diagnostics.is_empty(), "{:?}", applied.diagnostics);

        let text = anchored_text(&board);
        assert!(
            text.contains("let label:"),
            "the promoted local must keep its declaration:\n{text}"
        );
        assert!(
            text.contains("return label"),
            "the variable return must survive lowering:\n{text}"
        );

        let catalog: Vec<NodeMetadata> = catalog_nodes.iter().map(node_to_metadata).collect();
        let result = reconcile_text_with_catalog(&board, &text, &catalog);
        assert!(
            result.diagnostics.is_empty(),
            "the board's own lowered script must keep applying: {:?}",
            result.diagnostics
        );
        assert!(
            result.commands.is_empty(),
            "promoted-local roundtrip must be a no-op: {:?}",
            result.commands
        );
    }

    #[test]
    fn same_named_promoted_locals_in_two_functions_get_distinct_variable_ids() {
        let catalog = vec![
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "variable_set",
                "Set Variable",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("var_ref", "String", PinType::Input),
                    pin_meta("value_in", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value_ref", "Generic", PinType::Output),
                ],
            ),
            catalog_meta(
                "control_branch",
                "Branch",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("condition", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("true", "Execution", PinType::Output),
                    pin_meta("false", "Execution", PinType::Output),
                ],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function first(): (result: int) {
    let count = 1
    if (true) {
        count = 2
    }
    return count
}

function second(): (result: int) {
    let count = 3
    if (true) {
        count = 4
    }
    return count
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let created: Vec<(String, String)> = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::CreateVariable {
                    variable_id: Some(id),
                    name,
                    target_layer: Some(target),
                    ..
                } if name == "count" => Some((id.clone(), target.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            created.len(),
            2,
            "each function promotes its own local: {:?}",
            result.commands
        );
        assert_ne!(
            created[0].0, created[1].0,
            "same-named locals in different layers must not share a variable id"
        );
        assert_ne!(
            created[0].1, created[1].1,
            "one variable per function layer"
        );

        // Every var_ref selection must point at the variable created for ITS node's layer.
        let variable_by_layer: HashMap<&str, &str> = created
            .iter()
            .map(|(id, layer)| (layer.as_str(), id.as_str()))
            .collect();
        let layer_by_node: HashMap<String, String> = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::AddNode {
                    ref_id: Some(ref_id),
                    target_layer: Some(target),
                    ..
                } => Some((ref_id.clone(), target.clone())),
                _ => None,
            })
            .collect();
        let mut var_ref_updates = 0;
        for command in &result.commands {
            let BoardCommand::UpdateNodePin {
                node_id,
                pin_id,
                value: flow_like_types::Value::String(value),
                ..
            } = command
            else {
                continue;
            };
            if pin_id != "var_ref" {
                continue;
            }
            var_ref_updates += 1;
            let layer = layer_by_node
                .get(node_id)
                .unwrap_or_else(|| panic!("var_ref update on unknown node {node_id}"));
            assert_eq!(
                Some(value.as_str()),
                variable_by_layer.get(layer.as_str()).copied(),
                "variable node in layer {layer} must reference that layer's variable: {:?}",
                result.commands
            );
        }
        assert!(
            var_ref_updates >= 4,
            "expected setter+getter selections per function: {:?}",
            result.commands
        );
    }

    #[test]
    fn promoted_mutable_let_seeds_non_literal_initializer_into_exec_chain() {
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "variable_set",
                "Set Variable",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("var_ref", "String", PinType::Input),
                    pin_meta("value_in", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value_ref", "Generic", PinType::Output),
                ],
            ),
            // Pure: no exec pins.
            catalog_meta(
                "int_add",
                "Add Integers",
                vec![
                    pin_meta("a", "Integer", PinType::Input),
                    pin_meta("b", "Integer", PinType::Input),
                ],
                vec![pin_meta("result", "Integer", PinType::Output)],
            ),
            catalog_meta(
                "control_branch",
                "Branch",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("condition", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("true", "Execution", PinType::Output),
                    pin_meta("false", "Execution", PinType::Output),
                ],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"run() {
    let out = intAdd({ a: 1, b: 2 })
    if (true) {
        out = 7
    }
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        // The promoted variable must carry the initializer's output type, not Generic.
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::CreateVariable { name, data_type, .. }
                    if name == "out" && data_type == "Integer"
            )),
            "promoted `out` must be typed Integer: {:?}",
            result.commands
        );

        let adder_ref = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::AddNode {
                    node_type,
                    ref_id: Some(ref_id),
                    ..
                } if node_type == "int_add" => Some(ref_id.clone()),
                _ => None,
            })
            .expect("the int_add initializer must materialize");

        // The seeding variable_set is the one data-wired from the initializer call.
        let seed_ref = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                    ..
                } if from_node == &adder_ref && from_pin == "result" && to_pin == "value_in" => {
                    Some(to_node.clone())
                }
                _ => None,
            })
            .expect("the initializer must seed a variable_set");
        assert_eq!(
            command_node_type(&result.commands, &seed_ref).as_deref(),
            Some("variable_set")
        );

        // The seed joins the exec chain: event -> seed set -> branch.
        let branch_ref = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::AddNode {
                    node_type,
                    ref_id: Some(ref_id),
                    ..
                } if node_type == "control_branch" => Some(ref_id.clone()),
                _ => None,
            })
            .expect("control_branch node");
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_pin, to_node, to_pin, .. }
                    if from_pin == "exec_out" && to_node == &seed_ref && to_pin == "exec_in"
            )),
            "seed variable_set must be exec-chained after the event: {:?}",
            result.commands
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == &seed_ref
                        && from_pin == "exec_out"
                        && to_node == &branch_ref
                        && to_pin == "exec_in"
            )),
            "branch must chain after the seed variable_set: {:?}",
            result.commands
        );

        // The arm reassignment stays a separate literal variable_set wired from the true arm.
        let arm_set_ref = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::UpdateNodePin {
                    node_id,
                    pin_id,
                    value,
                    ..
                } if node_id != &seed_ref
                    && pin_id == "value_in"
                    && value == &flow_like_types::Value::from(7) =>
                {
                    Some(node_id.clone())
                }
                _ => None,
            })
            .expect("arm reassignment variable_set");
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == &branch_ref
                    && from_pin == "true"
                    && to_node == &arm_set_ref
                    && to_pin == "exec_in"
        )));
    }

    #[test]
    fn const_call_rebound_in_branch_arm_is_diagnosed() {
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "make_ticket",
                "Make Ticket",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("ticket", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "control_branch",
                "Branch",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("condition", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("true", "Execution", PinType::Output),
                    pin_meta("false", "Execution", PinType::Output),
                ],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"run() {
    const ticket = makeTicket()
    if (true) {
        ticket = makeTicket()
    }
}
"#,
            &catalog,
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("rebinds an outer-scope binding")
                    && diagnostic.contains("function parameter or `const` node output")
                    && diagnostic.contains("`ticket`")
            }),
            "nested const rebinding must be diagnosed: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn event_level_multi_value_return_is_diagnosed() {
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "make_ticket",
                "Make Ticket",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("ticket", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "events_generic_return_result",
                "Return Result",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("response", "Generic", PinType::Input),
                ],
                Vec::new(),
            ),
        ];

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"run() {
    const value = makeTicket()
    return value.ticket, value.ticket
}
"#,
            &catalog,
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| diagnostic
                .contains("event returns accept a single value; got 2")),
            "multi-value event return must be diagnosed: {:?}",
            result.diagnostics
        );
    }

    fn stale_event_entry(
        id: &str,
        friendly_name: &str,
        layer: Option<&str>,
        query_type: Option<VariableType>,
    ) -> Node {
        let mut event = Node::new("events_generic", friendly_name, "", "events");
        event.id = id.to_string();
        event.layer = layer.map(str::to_string);
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        if let Some(query_type) = query_type {
            event.add_output_pin("query", "Query", "", query_type);
        }
        event
    }

    fn stale_event_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_generic",
                "Generic Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "notify",
                "Notify",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    #[test]
    fn stale_alias_event_anchor_rebinds_unique_compatible_live_entry() {
        let mut board = empty_board();
        board.nodes.insert(
            "live-event".to_string(),
            stale_event_entry(
                "live-event",
                "Wiki Explorer Load",
                None,
                Some(VariableType::String),
            ),
        );
        // Same identity and scope, but a different payload contract. It must not make the
        // compatible String event ambiguous or receive either newly planned edge.
        board.nodes.insert(
            "incompatible-event".to_string(),
            stale_event_entry(
                "incompatible-event",
                "Wiki Explorer Load",
                None,
                Some(VariableType::Integer),
            ),
        );

        let result = reconcile_text_with_catalog(
            &board,
            r#"wikiExplorerLoad(query: string) {   //@n:gone-event
    notify({ message: query })
}
"#,
            &stale_event_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.corrections.iter().any(|correction| {
            correction.contains("gone-event") && correction.contains("live-event")
        }));
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. }
                if matches!(node_type.as_str(), "events_simple" | "events_generic")
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, ref_id: Some(ref_id), .. }
                if node_type == "notify" && ref_id == "$0"
        )));
        for (from_pin, to_pin) in [("exec_out", "exec_in"), ("query", "message")] {
            assert!(
                result.commands.iter().any(|command| matches!(
                    command,
                    BoardCommand::ConnectPins {
                        from_node,
                        from_pin: actual_from_pin,
                        to_node,
                        to_pin: actual_to_pin,
                        ..
                    } if from_node == "live-event"
                        && actual_from_pin == from_pin
                        && to_node == "$0"
                        && actual_to_pin == to_pin
                )),
                "missing recovered event edge {from_pin} -> {to_pin}: {:?}",
                result.commands
            );
        }
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, .. } if from_node == "incompatible-event"
        )));
    }

    #[test]
    fn stale_typed_event_anchor_recreates_exact_catalog_entry_when_no_live_match() {
        let ast = BoardAst {
            events: vec![EventBlock {
                name: "wikiExplorerLoad".to_string(),
                node_type: "events_generic".to_string(),
                event_name: None,
                params: Vec::new(),
                body: Block {
                    stmts: vec![Stmt::Call {
                        call: Call {
                            node_type: "notify".to_string(),
                            display: "notify".to_string(),
                            args: Vec::new(),
                            anchor: None,
                        },
                        anchor: None,
                    }],
                },
                anchor: Some("gone-event".to_string()),
            }],
            ..BoardAst::default()
        };

        let result = reconcile_with_catalog(&empty_board(), &ast, &stale_event_catalog());

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result
                .corrections
                .iter()
                .any(|correction| correction.contains("Recreated event")
                    && correction.contains("gone-event"))
        );
        let notify_index = result
            .commands
            .iter()
            .position(|command| {
                matches!(
                    command,
                    BoardCommand::AddNode { node_type, ref_id: Some(ref_id), .. }
                        if node_type == "notify" && ref_id == "$1"
                )
            })
            .expect("notify body node");
        let event_index = result
            .commands
            .iter()
            .position(|command| {
                matches!(
                    command,
                    BoardCommand::AddNode {
                        node_type,
                        ref_id: Some(ref_id),
                        friendly_name: Some(friendly_name),
                        target_layer: None,
                        ..
                    } if node_type == "events_generic"
                        && ref_id == "$0"
                        && friendly_name == "wikiExplorerLoad"
                )
            })
            .expect("recreated exact event entry");
        assert!(
            event_index > notify_index,
            "event registration stays last: {:?}",
            result.commands
        );
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$0"
                    && from_pin == "exec_out"
                    && to_node == "$1"
                    && to_pin == "exec_in"
        )));
        assert!(!result.commands.iter().any(|command| match command {
            BoardCommand::ConnectPins {
                from_node, to_node, ..
            } => {
                from_node == "gone-event" || to_node == "gone-event"
            }
            BoardCommand::UpdateNodePin { node_id, .. }
            | BoardCommand::RemoveNode { node_id, .. }
            | BoardCommand::RenameNode { node_id, .. } => node_id == "gone-event",
            _ => false,
        }));
    }

    #[test]
    fn stale_event_with_unavailable_explicit_type_does_not_rebind_by_alias() {
        let mut board = empty_board();
        board.nodes.insert(
            "live-event".to_string(),
            stale_event_entry("live-event", "Wiki Explorer Load", None, None),
        );
        let ast = BoardAst {
            events: vec![EventBlock {
                name: "wikiExplorerLoad".to_string(),
                node_type: "events_unavailable".to_string(),
                event_name: None,
                params: Vec::new(),
                body: Block {
                    stmts: vec![Stmt::Call {
                        call: Call {
                            node_type: "notify".to_string(),
                            display: "notify".to_string(),
                            args: Vec::new(),
                            anchor: None,
                        },
                        anchor: None,
                    }],
                },
                anchor: Some("gone-event".to_string()),
            }],
            ..BoardAst::default()
        };

        let result = reconcile_with_catalog(&board, &ast, &stale_event_catalog());

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("gone-event")
                && diagnostic.contains("exact node_type `events_unavailable`")
                && diagnostic.contains("not available in the catalog")
        }));
        assert!(result.corrections.is_empty());
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, .. } if from_node == "live-event"
        )));
    }

    #[test]
    fn stale_canonical_event_header_recreates_exact_catalog_entry() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsGeneric wikiExplorerLoad() {   //@n:gone-event
    notify({})
}
"#,
            &stale_event_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode {
                node_type,
                friendly_name: Some(friendly_name),
                ..
            } if node_type == "events_generic" && friendly_name == "wikiExplorerLoad"
        )));
        assert!(result.corrections.iter().any(|correction| {
            correction.contains("Recreated event") && correction.contains("gone-event")
        }));
    }

    #[test]
    fn stale_event_anchor_does_not_guess_between_compatible_live_entries() {
        let mut board = empty_board();
        for id in ["event-a", "event-b"] {
            board.nodes.insert(
                id.to_string(),
                stale_event_entry(id, "Wiki Explorer Load", None, None),
            );
        }

        let result = reconcile_text_with_catalog(
            &board,
            r#"wikiExplorerLoad() {   //@n:gone-event
    notify({})
}
"#,
            &stale_event_catalog(),
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("gone-event")
                    && diagnostic.contains("ambiguous")
                    && diagnostic.contains("event-a")
                    && diagnostic.contains("event-b")
            }),
            "{:?}",
            result.diagnostics
        );
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. }
                if matches!(node_type.as_str(), "events_simple" | "events_generic")
        )));
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, .. }
                if matches!(from_node.as_str(), "event-a" | "event-b")
        )));
    }

    #[test]
    fn stale_alias_event_anchor_without_live_match_remains_blocking() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"wikiExplorerLoad() {   //@n:gone-event
    notify({})
}
"#,
            &stale_event_catalog(),
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("gone-event")
                    && diagnostic.contains("alias-only")
                    && diagnostic.contains("exact event type")
            }),
            "{:?}",
            result.diagnostics
        );
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, .. }
                if matches!(node_type.as_str(), "events_simple" | "events_generic")
        )));
    }

    #[test]
    fn nested_stale_event_rebinds_only_within_function_layer() {
        let mut board = empty_board();
        let layer = Layer::new(
            "helper-layer".to_string(),
            "Helper".to_string(),
            LayerType::Function,
        );
        board.layers.insert(layer.id.clone(), layer);
        board.nodes.insert(
            "root-event".to_string(),
            stale_event_entry("root-event", "Wiki Explorer Load", None, None),
        );
        board.nodes.insert(
            "nested-event".to_string(),
            stale_event_entry(
                "nested-event",
                "Wiki Explorer Load",
                Some("helper-layer"),
                None,
            ),
        );

        let result = reconcile_text_with_catalog(
            &board,
            r#"function helper() {   //@l:helper-layer
    wikiExplorerLoad() {   //@n:gone-event
        notify({})
    }
}
"#,
            &stale_event_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode {
                node_type,
                ref_id: Some(ref_id),
                target_layer: Some(target_layer),
                ..
            } if node_type == "notify"
                && ref_id == "$0"
                && target_layer == "helper-layer"
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "nested-event"
                    && from_pin == "exec_out"
                    && to_node == "$0"
                    && to_pin == "exec_in"
        )));
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, .. } if from_node == "root-event"
        )));
    }

    #[test]
    fn named_event_creates_entry_with_friendly_name() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple dashboardLoad() {
    log({ text: "hello" })
}
"#,
            &member_chain_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { node_type, friendly_name: Some(friendly_name), .. }
                    if node_type == "events_simple" && friendly_name == "dashboardLoad"
            )),
            "named event must create the typed entry with its given name: {:?}",
            result.commands
        );
    }

    #[test]
    fn unnamed_event_keeps_catalog_default_name() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    log({ text: "hello" })
}
"#,
            &member_chain_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { node_type, friendly_name: None, .. }
                    if node_type == "events_simple"
            )),
            "unnamed event must keep the catalog default name: {:?}",
            result.commands
        );
    }

    #[test]
    fn anchored_event_rename_emits_single_rename_command() {
        let board = board_with_log("hello");
        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple dashboardLoad() {   //@n:event
    log({ text: "hello" })   //@n:log
}
"#,
            &member_chain_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            result.commands.len(),
            1,
            "a name-only change must be exactly one command: {:?}",
            result.commands
        );
        assert!(matches!(
            &result.commands[0],
            BoardCommand::RenameNode { node_id, friendly_name, .. }
                if node_id == "event" && friendly_name == "dashboardLoad"
        ));
    }

    #[test]
    fn anchored_event_matching_name_is_a_noop() {
        let board = board_with_log("hello");
        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple start() {   //@n:event
    log({ text: "hello" })   //@n:log
}
"#,
            &member_chain_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.is_empty(),
            "an unchanged given name must not emit commands: {:?}",
            result.commands
        );
    }

    /// Applying a named event, lowering the board, and re-reconciling must preserve the name and
    /// be a no-op — the full authoring round-trip for `eventsSimple dashboardLoad() { }`.
    #[tokio::test]
    async fn applied_named_event_roundtrip_preserves_name() {
        use crate::state::{FlowLikeConfig, FlowLikeState};
        use crate::utils::http::HTTPClient;
        use std::sync::Arc;

        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let mut log = Node::new("log", "Log", "", "debug");
        log.add_input_pin("exec_in", "In", "", VariableType::Execution);
        log.add_input_pin("text", "Text", "", VariableType::String);
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let catalog_nodes = vec![event, log];

        let mut board = empty_board();
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let applied = super::super::apply_flowscript_to_board(
            &mut board,
            r#"eventsSimple dashboardLoad() {
    log({ text: "hello" })
}
"#,
            &catalog_nodes,
            state,
            None,
            false,
        )
        .await
        .expect("named event script applies");
        assert!(applied.diagnostics.is_empty(), "{:?}", applied.diagnostics);

        let entry = board
            .nodes
            .values()
            .find(|node| node.name == "events_simple")
            .expect("entry node exists");
        assert_eq!(entry.friendly_name, "dashboardLoad");

        let text = anchored_text(&board);
        assert!(
            text.contains("dashboardLoad() {"),
            "lowered output must preserve the given event name:\n{text}"
        );

        let catalog: Vec<NodeMetadata> = catalog_nodes.iter().map(node_to_metadata).collect();
        let result = reconcile_text_with_catalog(&board, &text, &catalog);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.is_empty(),
            "re-reconciling the lowered named event must be a no-op:\n{text}\n{:?}",
            result.commands
        );
    }

    #[test]
    fn function_return_diagnostics_name_the_function_and_expression() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function brokenOne(): (tag: string) {
    return missingRef
}

function brokenTwo(): (tag: string) {
    return otherMissing
}
"#,
            &[],
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("`brokenOne`")
                    && diagnostic.contains("`missingRef`")
                    && diagnostic.contains("is not a resolvable FlowScript value")
            }),
            "{:?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("`brokenTwo`")
                    && diagnostic.contains("`otherMissing`")
                    && diagnostic.contains("is not a resolvable FlowScript value")
            }),
            "{:?}",
            result.diagnostics
        );
    }

    fn multi_output_return_catalog() -> Vec<NodeMetadata> {
        let mut user_context = pin_meta("user_context", "Struct", PinType::Output);
        user_context.friendly_name = "User Context".to_string();
        user_context.schema = Some(
            r#"{"type":"object","properties":{"sub":{"type":"string"}},"additionalProperties":false}"#
                .to_string(),
        );
        user_context.enforce_schema = true;
        vec![
            catalog_meta(
                "utils_user_get_executing_user",
                "Get Executing User",
                Vec::new(),
                vec![
                    user_context,
                    pin_meta("has_user", "Boolean", PinType::Output),
                ],
            ),
            catalog_meta(
                "val_to_string",
                "To String",
                vec![
                    pin_meta("value", "Generic", PinType::Input),
                    pin_meta("pretty", "Boolean", PinType::Input),
                ],
                vec![pin_meta("string", "String", PinType::Output)],
            ),
            catalog_meta(
                "utils_hash_sha256",
                "SHA256 Hash",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("input", "String", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("hash", "String", PinType::Output),
                ],
            ),
            catalog_meta(
                "struct_get",
                "Get Field",
                vec![
                    pin_meta("struct", "Struct", PinType::Input),
                    pin_meta("field", "String", PinType::Input),
                ],
                vec![pin_meta("value", "Generic", PinType::Output)],
            ),
        ]
    }

    const MULTI_OUTPUT_RETURN_SOURCE: &str = r#"function getOwnerIdentity(): (ownerSub: string, ownerKey: string, hasUser: bool) {
    const user = utilsUserGetExecutingUser()
    const asText = valToString({ value: user.userContext.sub, pretty: false })
    const hashed = utilsHashSha256({ input: asText.string })
    return hashed.hash, asText.string, user.hasUser
}
"#;

    /// The uptime-monitor regression: a function whose return values come from a multi-output
    /// catalog node through member-access chains must wire every declared boundary return pin.
    #[test]
    fn multi_output_member_chain_returns_wire_every_boundary_pin() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            MULTI_OUTPUT_RETURN_SOURCE,
            &multi_output_return_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let layer_ref = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::CreateLayer {
                    name,
                    ref_id: Some(ref_id),
                    ..
                } if name == "getOwnerIdentity" => Some(ref_id.clone()),
                _ => None,
            })
            .expect("missing getOwnerIdentity layer");
        for return_pin in ["ownerSub", "ownerKey", "hasUser"] {
            assert!(
                result.commands.iter().any(|command| matches!(
                    command,
                    BoardCommand::ConnectPins { to_node, to_pin, .. }
                        if to_node == &layer_ref && to_pin == return_pin
                )),
                "missing boundary return connection for `{return_pin}`: {:?}",
                result.commands
            );
        }
    }

    fn multi_output_return_catalog_nodes() -> Vec<Node> {
        let mut user = Node::new(
            "utils_user_get_executing_user",
            "Get Executing User",
            "",
            "utils",
        );
        let context_pin =
            user.add_output_pin("user_context", "User Context", "", VariableType::Struct);
        context_pin.schema = Some(
            r#"{"type":"object","properties":{"sub":{"type":"string"}},"additionalProperties":false}"#
                .to_string(),
        );
        context_pin.options = Some(PinOptions::new().set_enforce_schema(true).build());
        user.add_output_pin("has_user", "Has User", "", VariableType::Boolean);

        let mut to_string = Node::new("val_to_string", "To String", "", "utils");
        to_string.add_input_pin("value", "Value", "", VariableType::Generic);
        to_string.add_input_pin("pretty", "Pretty", "", VariableType::Boolean);
        to_string.add_output_pin("string", "String", "", VariableType::String);

        let mut sha256 = Node::new("utils_hash_sha256", "SHA256 Hash", "", "utils");
        sha256.add_input_pin("exec_in", "Execute", "", VariableType::Execution);
        sha256.add_input_pin("input", "Input", "", VariableType::String);
        sha256.add_output_pin("exec_out", "Done", "", VariableType::Execution);
        sha256.add_output_pin("hash", "Hash", "", VariableType::String);

        let mut struct_get = Node::new("struct_get", "Get Field", "", "structs");
        struct_get.add_input_pin("struct", "Struct", "", VariableType::Struct);
        struct_get.add_input_pin("field", "Field", "", VariableType::String);
        struct_get.add_output_pin("value", "Value", "", VariableType::Generic);

        vec![user, to_string, sha256, struct_get]
    }

    /// Applying the multi-output-return function, lowering the board, and re-reconciling the
    /// lowered text must be a no-op: no duplicated member-access chains, no lost return wiring.
    #[tokio::test]
    async fn applied_multi_output_return_roundtrip_is_idempotent() {
        use crate::state::{FlowLikeConfig, FlowLikeState};
        use crate::utils::http::HTTPClient;
        use std::sync::Arc;

        let catalog_nodes = multi_output_return_catalog_nodes();
        let mut board = empty_board();
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let applied = super::super::apply_flowscript_to_board(
            &mut board,
            MULTI_OUTPUT_RETURN_SOURCE,
            &catalog_nodes,
            state,
            None,
            false,
        )
        .await
        .expect("multi-output return script applies");
        assert!(applied.diagnostics.is_empty(), "{:?}", applied.diagnostics);

        let layer = board
            .layers
            .values()
            .find(|layer| layer.name == "getOwnerIdentity")
            .expect("function layer exists");
        for return_pin in ["ownerSub", "ownerKey", "hasUser"] {
            let boundary = layer
                .pins
                .values()
                .find(|pin| pin.name == *return_pin)
                .unwrap_or_else(|| panic!("missing boundary pin {return_pin}"));
            assert!(
                !boundary.depends_on.is_empty(),
                "boundary return pin `{return_pin}` must have an incoming edge"
            );
        }

        let text = anchored_text(&board);
        assert!(
            text.contains("return "),
            "lowered board must keep the return statement:\n{text}"
        );

        let catalog: Vec<NodeMetadata> = catalog_nodes.iter().map(node_to_metadata).collect();
        let result = reconcile_text_with_catalog(&board, &text, &catalog);
        assert!(
            result.diagnostics.is_empty(),
            "roundtrip diagnostics:\n{text}\n{:?}",
            result.diagnostics
        );
        assert!(
            result.commands.is_empty(),
            "re-reconciling the board's own lowered script must be a no-op:\n{text}\n{:?}",
            result.commands
        );
    }

    /// A bare reference to a multi-output node in return position has no explicit output pin;
    /// the declared return pin's type must disambiguate (here: the one Boolean output).
    #[test]
    fn declared_return_pin_type_disambiguates_multi_output_source() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function currentUserFlag(): (hasUser: bool) {
    const user = utilsUserGetExecutingUser()
    return user
}
"#,
            &multi_output_return_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_pin, to_pin, .. }
                    if from_pin == "has_user" && to_pin == "hasUser"
            )),
            "return pin type must select the one Boolean output: {:?}",
            result.commands
        );
    }

    /// The repair-loop shape: the function already exists on the board (anchored body, unfed
    /// boundary pin) and the model adds `return probe` referencing the anchored binding of a live
    /// multi-output node. The declared return pin's type must disambiguate the output.
    #[tokio::test]
    async fn declared_return_pin_type_disambiguates_existing_multi_output_source() {
        use crate::state::{FlowLikeConfig, FlowLikeState};
        use crate::utils::http::HTTPClient;
        use std::sync::Arc;

        let mut probe = Node::new("http_probe", "HTTP Probe", "", "web");
        probe.add_input_pin("exec_in", "Execute", "", VariableType::Execution);
        probe.add_output_pin("exec_out", "Done", "", VariableType::Execution);
        probe.add_output_pin("response", "Response", "", VariableType::Struct);
        probe.add_output_pin("ok", "Ok", "", VariableType::Boolean);
        let catalog_nodes = vec![probe];

        let mut board = empty_board();
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let applied = super::super::apply_flowscript_to_board(
            &mut board,
            r#"function probeFlag(): (succeeded: bool) {
    const probe = httpProbe()
}
"#,
            &catalog_nodes,
            state,
            None,
            false,
        )
        .await
        .expect("function without return applies");
        assert!(applied.diagnostics.is_empty(), "{:?}", applied.diagnostics);

        // The model binds the anchored call and returns the binding — the anchor keeps the
        // statement resolving to the LIVE node, so the return source is an Existing entity.
        let text = anchored_text(&board);
        assert!(text.contains("    httpProbe()   //@n:"), "{text}");
        let rebound = text.replace(
            "    httpProbe()   //@n:",
            "    const probe = httpProbe()   //@n:",
        );
        let insert_at = rebound.rfind("\n}").expect("function closing brace");
        let with_return = format!(
            "{}\n    return probe{}",
            &rebound[..insert_at],
            &rebound[insert_at..]
        );

        let catalog: Vec<NodeMetadata> = catalog_nodes.iter().map(node_to_metadata).collect();
        let result = reconcile_text_with_catalog(&board, &with_return, &catalog);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let layer_id = board
            .layers
            .values()
            .find(|layer| layer.name == "probeFlag")
            .map(|layer| layer.id.clone())
            .expect("function layer exists");
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_pin, to_node, to_pin, .. }
                    if from_pin == "ok" && to_node == &layer_id && to_pin == "succeeded"
            )),
            "the bool return pin must select the live node's Boolean output: {:?}",
            result.commands
        );
    }

    /// A return value whose source output pin stays ambiguous (two same-typed outputs) must be a
    /// diagnostic that names the function and the expression — never a silently unfed return pin.
    #[test]
    fn ambiguous_multi_output_return_is_diagnosed_with_function_and_value() {
        let catalog = vec![catalog_meta(
            "make_pair",
            "Make Pair",
            Vec::new(),
            vec![
                pin_meta("first", "String", PinType::Output),
                pin_meta("second", "String", PinType::Output),
            ],
        )];
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function pickOne(): (chosen: string) {
    const pair = makePair()
    return pair
}
"#,
            &catalog,
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("could not choose output pin")
                    && diagnostic.contains("`pickOne`")
                    && diagnostic.contains("`pair`")
            }),
            "{:?}",
            result.diagnostics
        );
        assert!(
            !result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { to_pin, .. } if to_pin == "chosen"
            )),
            "an ambiguous return must not guess a connection: {:?}",
            result.commands
        );
    }

    #[test]
    fn anchored_function_signature_drift_is_rejected_before_return_wiring() {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "function-layer".to_string(),
            "make message".to_string(),
            LayerType::Function,
        );
        let mut template = Node::new("boundary", "Boundary", "", "test");
        let return_pin = template
            .add_output_pin("message", "Message", "", VariableType::String)
            .clone();
        layer.pins.insert(return_pin.id.clone(), return_pin);
        board.layers.insert(layer.id.clone(), layer);

        let catalog = vec![catalog_meta(
            "date_source",
            "Date Source",
            Vec::new(),
            vec![pin_meta("date", "Date", PinType::Output)],
        )];
        let result = reconcile_text_with_catalog(
            &board,
            r#"function makeMessage(): (message: Date) {   //@l:function-layer
    const value = dateSource()
    return value.date
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("changes the data-boundary contract")
                && diagnostic.contains("function-layer")
        }));
        assert!(
            result.commands.is_empty(),
            "signature drift must not create a producer or return edge: {:?}",
            result.commands
        );
    }

    #[test]
    fn named_interface_function_boundaries_keep_and_validate_their_schema() {
        let source = r#"interface Ticket {
    id: string;
}

function makeTicket(): (result: Ticket) {
    const value = ticketSource()
    return value.ticket
}
"#;
        let ast = flow_like_ast::parse(source).expect("parse");
        let schema = interface_schema_map(&ast)
            .remove("Ticket")
            .expect("generated Ticket schema");
        let mut output = pin_meta("ticket", "Struct", PinType::Output);
        output.schema = Some(schema.clone());
        output.enforce_schema = true;
        let catalog = vec![catalog_meta(
            "ticket_source",
            "Ticket Source",
            Vec::new(),
            vec![output],
        )];

        let result = reconcile_with_catalog(&empty_board(), &ast, &catalog);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::CreateLayer { pins: Some(pins), .. }
                if pins.iter().any(|pin| pin.name == "result"
                    && pin.schema.as_deref() == Some(schema.as_str())
                    && pin.enforce_schema)
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_pin, to_pin, .. }
                if from_pin == "ticket" && to_pin == "result"
        )));
    }

    #[test]
    fn schema_bearing_function_boundary_lowers_and_reconciles_nominally() {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "typed-function".to_string(),
            "make ticket".to_string(),
            LayerType::Function,
        );
        let mut template = Node::new("boundary", "Boundary", "", "test");
        let result_pin = template.add_output_pin("result", "Result", "", VariableType::Struct);
        result_pin.schema = Some(
            r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}"#
                .to_string(),
        );
        result_pin.options = Some(PinOptions {
            enforce_schema: Some(true),
            ..PinOptions::default()
        });
        let result_pin = result_pin.clone();
        let result_pin_id = result_pin.id.clone();
        layer.pins.insert(result_pin.id.clone(), result_pin);
        board.layers.insert(layer.id.clone(), layer);

        let text = anchored_text(&board);
        assert!(text.contains("interface MakeTicketResult"), "{text}");
        assert!(
            text.contains("(result: MakeTicketResult)"),
            "function return must retain its nominal interface: {text}"
        );

        let result = reconcile_text_with_catalog(&board, &text, &[]);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.is_empty(),
            "create→lower→reconcile must be a no-op: {:?}",
            result.commands
        );

        let erased_nominal_type = text.replacen("result: MakeTicketResult", "result: Struct", 1);
        let erased_result = reconcile_text_with_catalog(&board, &erased_nominal_type, &[]);
        assert!(
            erased_result
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.contains("changes the data-boundary contract") })
        );

        board.cleanup();
        let compacted_schema = board.layers["typed-function"].pins[&result_pin_id]
            .schema
            .as_deref()
            .unwrap();
        assert!(
            board.refs.contains_key(compacted_schema),
            "normal cleanup must exercise the schema-ref round-trip"
        );
        let compacted_text = anchored_text(&board);
        assert!(compacted_text.contains("interface MakeTicketResult"));
        let compacted_result = reconcile_text_with_catalog(&board, &compacted_text, &[]);
        assert!(
            compacted_result.diagnostics.is_empty(),
            "schema refs must be expanded before boundary comparison: {:?}",
            compacted_result.diagnostics
        );
        assert!(compacted_result.commands.is_empty());

        board
            .layers
            .get_mut("typed-function")
            .unwrap()
            .pins
            .get_mut(&result_pin_id)
            .unwrap()
            .options
            .as_mut()
            .unwrap()
            .enforce_schema = Some(false);
        let legacy_text = anchored_text(&board);
        assert!(!legacy_text.contains("interface "), "{legacy_text}");
        assert!(legacy_text.contains("(result: Struct)"), "{legacy_text}");
        let legacy_result = reconcile_text_with_catalog(&board, &legacy_text, &[]);
        assert!(
            legacy_result.diagnostics.is_empty(),
            "non-enforced descriptive schemas must remain a no-op: {:?}",
            legacy_result.diagnostics
        );
        assert!(legacy_result.commands.is_empty());
    }

    #[test]
    fn anchored_event_parameter_schema_roundtrips_and_rejects_nominal_drift() {
        let mut board = empty_board();
        let mut event = Node::new("events_chat", "Chat Event", "", "events");
        event.id = "chat-event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let history = event.add_output_pin("history", "History", "", VariableType::Struct);
        history.schema = Some(
            r#"{"type":"object","properties":{"messages":{"type":"array","items":{"type":"string"}}},"required":["messages"],"additionalProperties":false}"#
                .to_string(),
        );
        history.options = Some(PinOptions {
            enforce_schema: Some(true),
            ..PinOptions::default()
        });
        board.nodes.insert(event.id.clone(), event);
        board.cleanup();

        let text = anchored_text(&board);
        assert!(text.contains("interface EventsChatHistory"), "{text}");
        assert!(text.contains("history: EventsChatHistory"), "{text}");
        let unchanged = reconcile_text_with_catalog(&board, &text, &[]);
        assert!(
            unchanged.diagnostics.is_empty(),
            "cleaned event schema must round-trip: {:?}",
            unchanged.diagnostics
        );
        assert!(unchanged.commands.is_empty());

        let changed = text.replacen("history: EventsChatHistory", "history: Struct", 1);
        let drift = reconcile_text_with_catalog(&board, &changed, &[]);
        assert!(drift.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("changes the parameter contract")
                && diagnostic.contains("chat-event")
        }));
        assert!(drift.commands.is_empty());
    }

    #[test]
    fn variable_assignment_rejects_a_new_incompatible_data_edge() {
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "date_source",
                "Date Source",
                Vec::new(),
                vec![pin_meta("date", "Date", PinType::Output)],
            ),
            catalog_meta(
                "variable_set",
                "Set Variable",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("var_ref", "String", PinType::Input),
                    pin_meta("value_in", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value_ref", "Generic", PinType::Output),
                ],
            ),
        ];
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"const message: string = ""

eventsSimple() {
    const value = dateSource()
    message = value.date
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("assignment to variable `var_message`")
                && diagnostic.contains("incompatible pin types or schemas")
                && diagnostic.contains("Date/Normal")
                && diagnostic.contains("String/Normal")
        }));
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { to_node, to_pin, .. }
                if to_pin == "value_in"
                    && command_node_type(&result.commands, to_node).as_deref()
                        == Some("variable_set")
        )));
    }

    #[test]
    fn retained_variable_edge_is_revalidated_after_contract_change() {
        let mut board = empty_board();
        let mut source_node = Node::new("string_source", "String Source", "", "test");
        source_node.id = "source".to_string();
        let source_pin = source_node
            .add_output_pin("value", "Value", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(source_node.id.clone(), source_node);

        let mut setter = Node::new("variable_set", "Set Variable", "", "variable");
        setter.id = "setter".to_string();
        let input_pin = setter
            .add_input_pin("value_in", "Value", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(setter.id.clone(), setter);
        connect(&mut board, "source", &source_pin, "setter", &input_pin);

        let mut planner = StructuralPlanner::new(&board, &[], None);
        let authored_date_contract = pin_meta("value_in", "Date", PinType::Input);
        let queued = planner.queue_validated_data_connection(
            &ValueSource {
                node: NodeEntity::Existing("source".to_string()),
                output_pin: Some("value".to_string()),
            },
            "value".to_string(),
            &NodeEntity::Existing("setter".to_string()),
            &authored_date_contract,
            "value_in",
            "Retain variable assignment".to_string(),
            "assignment after variable type change",
            true,
        );

        assert!(!queued);
        assert!(planner.result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("assignment after variable type change")
                && diagnostic.contains("String/Normal")
                && diagnostic.contains("Date/Normal")
        }));
        assert!(planner.connect_commands.is_empty());
    }

    #[test]
    fn cleaned_schema_refs_match_inline_authored_edge_and_variable_contracts() {
        let schema = r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}"#
            .to_string();
        let enforced = || {
            Some(PinOptions {
                enforce_schema: Some(true),
                ..PinOptions::default()
            })
        };
        let mut board = empty_board();
        let mut source = Node::new("struct_source", "Struct Source", "", "test");
        source.id = "source".to_string();
        let source_pin = source.add_output_pin("value", "Value", "", VariableType::Struct);
        source_pin.schema = Some(schema.clone());
        source_pin.options = enforced();
        let source_pin_id = source_pin.id.clone();
        board.nodes.insert(source.id.clone(), source);

        let mut target = Node::new("struct_sink", "Struct Sink", "", "test");
        target.id = "target".to_string();
        let target_pin = target.add_input_pin("value", "Value", "", VariableType::Struct);
        target_pin.schema = Some(schema.clone());
        target_pin.options = enforced();
        let target_pin_id = target_pin.id.clone();
        board.nodes.insert(target.id.clone(), target);
        connect(
            &mut board,
            "source",
            &source_pin_id,
            "target",
            &target_pin_id,
        );

        let mut variable = Variable::new("payload", VariableType::Struct, ValueType::Normal);
        variable.id = "payload-variable".to_string();
        variable.schema = Some(schema.clone());
        board.variables.insert(variable.id.clone(), variable);
        board.cleanup();

        let mut planner = StructuralPlanner::new(&board, &[], None);
        let mut authored_input = pin_meta("value", "Struct", PinType::Input);
        authored_input.schema = Some(schema.clone());
        authored_input.enforce_schema = true;
        let retained = planner.queue_validated_data_connection(
            &ValueSource {
                node: NodeEntity::Existing("source".to_string()),
                output_pin: Some("value".to_string()),
            },
            "value".to_string(),
            &NodeEntity::Existing("target".to_string()),
            &authored_input,
            "value",
            "Retain cleaned schema edge".to_string(),
            "cleaned schema edge",
            false,
        );
        assert!(retained, "{:?}", planner.result.diagnostics);
        assert!(planner.connect_commands.is_empty());

        let mut authored_variable = pin_meta("value_ref", "Struct", PinType::Output);
        authored_variable.schema = Some(schema);
        planner
            .variable_value_contracts
            .insert("payload-variable".to_string(), authored_variable);
        assert!(!planner.variable_contract_changed_from_board("payload-variable"));
    }

    #[test]
    fn duplicate_catalog_types_are_order_independent_and_conflicts_fail_closed() {
        let string_source = catalog_meta(
            "shared_source",
            "Z Source",
            Vec::new(),
            vec![pin_meta("value", "String", PinType::Output)],
        );
        let mut identical_source = string_source.clone();
        identical_source.friendly_name = "A Source".to_string();
        let call = Call {
            node_type: "shared_source".to_string(),
            display: "sharedSource".to_string(),
            args: Vec::new(),
            anchor: None,
        };

        let forward = CatalogIndex::new(&[string_source.clone(), identical_source.clone()])
            .resolve_call(&call)
            .expect("identical executable contracts should collapse");
        let reversed = CatalogIndex::new(&[identical_source.clone(), string_source.clone()])
            .resolve_call(&call)
            .expect("catalog order must not affect the selected declaration");
        assert_eq!(forward.friendly_name, "A Source");
        assert_eq!(forward.friendly_name, reversed.friendly_name);

        let mut schema_source = catalog_meta(
            "schema_source",
            "Schema Source Z",
            Vec::new(),
            vec![pin_meta("value", "Struct", PinType::Output)],
        );
        schema_source.outputs[0].schema =
            Some(r#"{"type":"object","properties":{"id":{"type":"string"}}}"#.to_string());
        schema_source.outputs[0].enforce_schema = true;
        let mut reordered_schema_source = schema_source.clone();
        reordered_schema_source.friendly_name = "Schema Source A".to_string();
        reordered_schema_source.outputs[0].schema = Some(
            r#"{ "properties": { "id": { "type": "string" } }, "type": "object" }"#.to_string(),
        );
        let schema_call = Call {
            node_type: "schema_source".to_string(),
            display: "schemaSource".to_string(),
            args: Vec::new(),
            anchor: None,
        };
        CatalogIndex::new(&[schema_source, reordered_schema_source])
            .resolve_call(&schema_call)
            .expect("canonical JSON-equivalent schemas are one executable contract");

        let mut conflicting_source = identical_source;
        conflicting_source.outputs[0].data_type = "Date".to_string();
        for catalog in [
            vec![string_source.clone(), conflicting_source.clone()],
            vec![conflicting_source.clone(), string_source.clone()],
        ] {
            let error = CatalogIndex::new(&catalog)
                .resolve_call(&call)
                .expect_err("conflicting same-name declarations must not be selected");
            assert!(
                error.contains("conflicting catalog declarations"),
                "{error}"
            );
        }
    }

    #[test]
    fn repeated_existing_inputs_grandfather_only_the_exact_occurrence() {
        let mut board = empty_board();
        let mut source_node = Node::new("bool_source", "Bool Source", "", "test");
        source_node.id = "source".to_string();
        let source_pin_id = source_node
            .add_output_pin("value", "Value", "", VariableType::Boolean)
            .id
            .clone();
        board.nodes.insert(source_node.id.clone(), source_node);

        let mut target = Node::new("bool_or", "Boolean Or", "", "test");
        target.id = "target".to_string();
        let first_input_id = target
            .add_input_pin("boolean", "Boolean", "", VariableType::Boolean)
            .id
            .clone();
        target.add_input_pin("boolean", "Boolean", "", VariableType::Boolean);
        board.nodes.insert(target.id.clone(), target);
        connect(
            &mut board,
            "source",
            &source_pin_id,
            "target",
            &first_input_id,
        );

        let source = ValueSource {
            node: NodeEntity::Existing("source".to_string()),
            output_pin: Some("value".to_string()),
        };
        let target_meta = node_to_metadata(&board.nodes["target"]);
        let call = Call {
            node_type: "bool_or".to_string(),
            display: "boolOr".to_string(),
            args: vec![
                Arg {
                    name: "boolean".to_string(),
                    value: Expr::Ref("value".to_string()),
                },
                Arg {
                    name: "boolean".to_string(),
                    value: Expr::Ref("value".to_string()),
                },
            ],
            anchor: None,
        };
        let mut planner = StructuralPlanner::new(&board, &[], None);
        planner.push_scope();
        planner
            .symbols
            .last_mut()
            .unwrap()
            .insert("value".to_string(), SymbolValue::Source(source));
        planner.plan_call_arguments(
            &call,
            &NodeEntity::Existing("target".to_string()),
            &target_meta,
            None,
            true,
        );

        assert!(planner.result.diagnostics.is_empty());
        assert_eq!(planner.connect_commands.len(), 1);
        assert!(matches!(
            &planner.connect_commands[0],
            BoardCommand::ConnectPins { to_node, to_pin, .. }
                if to_node == "target" && to_pin == "boolean[#2]"
        ));
    }

    #[test]
    fn malformed_multi_source_input_grandfathers_the_matching_endpoint_not_the_first() {
        let mut board = empty_board();
        let mut source_ids = Vec::new();
        for id in ["source-a", "source-b"] {
            let mut source = Node::new("bool_source", "Bool Source", "", "test");
            source.id = id.to_string();
            let pin_id = source
                .add_output_pin("value", "Value", "", VariableType::Boolean)
                .id
                .clone();
            source_ids.push((id.to_string(), pin_id));
            board.nodes.insert(source.id.clone(), source);
        }
        source_ids.sort_by(|left, right| left.1.cmp(&right.1));

        let mut target = Node::new("bool_sink", "Bool Sink", "", "test");
        target.id = "target".to_string();
        let target_pin_id = target
            .add_input_pin("value", "Value", "", VariableType::Boolean)
            .id
            .clone();
        // Simulate a malformed legacy/fan-in board without relying on the ordinary DATA connect
        // command, which correctly replaces the prior source.
        target
            .pins
            .get_mut(&target_pin_id)
            .unwrap()
            .depends_on
            .extend(source_ids.iter().map(|(_, pin_id)| pin_id.clone()));
        for (_, pin_id) in &source_ids {
            let source = board
                .nodes
                .values_mut()
                .find(|node| node.pins.contains_key(pin_id))
                .unwrap();
            source
                .pins
                .get_mut(pin_id)
                .unwrap()
                .connected_to
                .insert(target_pin_id.clone());
        }
        board.nodes.insert(target.id.clone(), target);

        let requested_source = source_ids[1].0.clone();
        let mut planner = StructuralPlanner::new(&board, &[], None);
        let queued = planner.queue_validated_data_connection(
            &ValueSource {
                node: NodeEntity::Existing(requested_source),
                output_pin: Some("value".to_string()),
            },
            "value".to_string(),
            &NodeEntity::Existing("target".to_string()),
            &pin_meta("value", "Boolean", PinType::Input),
            "value",
            "Retain matching edge".to_string(),
            "malformed fan-in edge",
            false,
        );

        assert!(queued);
        assert!(planner.result.diagnostics.is_empty());
        assert!(planner.connect_commands.is_empty());
    }

    #[test]
    fn malformed_multi_source_resolution_reuses_the_matching_variable_get() {
        let mut board = empty_board();
        let mut source_pins = Vec::new();
        for (node_id, variable_id, variable_name) in [
            ("reader-a", "var-a", "alpha"),
            ("reader-b", "var-b", "beta"),
        ] {
            let mut variable =
                Variable::new(variable_name, VariableType::String, ValueType::Normal);
            variable.id = variable_id.to_string();
            board.variables.insert(variable.id.clone(), variable);

            let mut reader = Node::new("variable_get", "Get Variable", "", "variables");
            reader.id = node_id.to_string();
            reader
                .add_input_pin("var_ref", "Variable", "", VariableType::String)
                .default_value = Some(format!("\"{variable_id}\"").into_bytes());
            let output_id = reader
                .add_output_pin("value_ref", "Value", "", VariableType::Generic)
                .id
                .clone();
            source_pins.push(output_id);
            board.nodes.insert(reader.id.clone(), reader);
        }

        let mut target = Node::new("string_sink", "String Sink", "", "test");
        target.id = "target".to_string();
        let target_pin_id = target
            .add_input_pin("value", "Value", "", VariableType::String)
            .id
            .clone();
        target
            .pins
            .get_mut(&target_pin_id)
            .unwrap()
            .depends_on
            .extend(source_pins.iter().cloned());
        for source_pin_id in &source_pins {
            let source = board
                .nodes
                .values_mut()
                .find(|node| node.pins.contains_key(source_pin_id))
                .unwrap();
            source
                .pins
                .get_mut(source_pin_id)
                .unwrap()
                .connected_to
                .insert(target_pin_id.clone());
        }
        board.nodes.insert(target.id.clone(), target);

        let mut planner = StructuralPlanner::new(&board, &[], None);
        let sources = planner
            .existing_sources_for_input_ref(&NodeEntity::Existing("target".to_string()), "value");
        assert_eq!(sources.len(), 2);
        let expected = sources[1].clone();
        let expected_node = find_board_node(&board, &expected.node.node_ref()).unwrap();
        let expected_variable_id = node_pin_literal_string(expected_node, "var_ref").unwrap();
        let expected_variable_name = board.variables[&expected_variable_id].name.clone();

        planner.push_scope();
        planner.insert_symbol(
            expected_variable_name.clone(),
            SymbolValue::VariableRef {
                variable_id: expected_variable_id,
            },
        );
        let resolved = planner.resolve_expr_for_argument(
            &Expr::Ref(expected_variable_name),
            &NodeEntity::Existing("target".to_string()),
            "value",
            None,
        );

        assert!(matches!(
            resolved,
            Some(SymbolValue::Source(source))
                if source.node.node_ref() == expected.node.node_ref()
                    && source.output_pin == expected.output_pin
        ));
        assert!(planner.add_commands.is_empty());
        assert!(planner.result.diagnostics.is_empty());
    }

    #[test]
    fn implicit_output_selection_does_not_coerce_array_to_scalar() {
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "multi_output_source",
                "Multi Output Source",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta_friendly("result", "Rows", "Struct", "Array", PinType::Output),
                    pin_meta("count", "Integer", PinType::Output),
                ],
            ),
            catalog_meta(
                "consume_struct",
                "Consume Struct",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("row", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    const produced = multiOutputSource({})
    consumeStruct({ row: produced })
}
"#,
            &catalog,
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("could not choose an output pin")
                    && diagnostic.contains("argument `row`")
                    && diagnostic.contains("consumeStruct")
            }),
            "{:?}",
            result.diagnostics
        );
        assert!(
            !result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { to_pin, .. } if to_pin == "row"
            )),
            "an array output must not be wired to a scalar input: {:?}",
            result.commands
        );
    }

    #[test]
    fn rejects_declared_struct_parameter_connected_directly_to_string_pin() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_call_function",
                "Call Function",
                vec![pin_meta("function_layer_id", "String", PinType::Input)],
                vec![],
            ),
            catalog_meta(
                "struct_source",
                "Struct Source",
                vec![],
                vec![pin_meta("record", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function processMail(mail: Struct) {
    log({ message: mail })
}

eventsSimple() {
    const row = structSource()
    processMail({ mail: row.record })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("argument `message` on `log`")
                && diagnostic.contains("`Struct/Normal`")
                && diagnostic.contains("`String/Normal`")
        }));
        assert!(
            !result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { to_pin, .. } if to_pin == "message"
            )),
            "a declared Struct parameter must not wire directly into a String pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn keeps_schema_less_struct_and_generic_helper_connections_permissive() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_call_function",
                "Call Function",
                vec![pin_meta("function_layer_id", "String", PinType::Input)],
                vec![],
            ),
            catalog_meta(
                "struct_source",
                "Struct Source",
                vec![],
                vec![pin_meta("record", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "Generic", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function processMail(mail: Struct) {
    log({ message: mail })
}

eventsSimple() {
    const row = structSource()
    processMail({ mail: row.record })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { to_pin, .. } if to_pin == "mail"
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { to_pin, .. } if to_pin == "message"
        )));
    }

    #[test]
    fn new_empty_event_body_is_rejected() {
        let board = empty_board();
        let catalog = vec![catalog_meta(
            "events_simple",
            "Simple Event",
            vec![],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        )];

        let result = reconcile_text_with_catalog(&board, "eventsSimple() {\n}\n", &catalog);

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("new event") && diagnostic.contains("no executable body nodes")
        }));
    }

    #[test]
    fn new_empty_function_body_is_rejected() {
        let board = empty_board();

        let result =
            reconcile_text_with_catalog(&board, "function prepareSupportTable() {\n}\n", &[]);

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("new function") && diagnostic.contains("no executable body nodes")
        }));
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("no materialized body nodes")
                && diagnostic.contains("runtime-empty helper")
        }));
    }

    #[test]
    fn new_pure_function_without_returns_is_rejected_as_runtime_empty() {
        let board = empty_board();
        let catalog = vec![catalog_meta(
            "string_format",
            "String Format",
            vec![pin_meta("format_string", "String", PinType::Input)],
            vec![pin_meta("value", "String", PinType::Output)],
        )];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function prepareSupportTicket() {
    stringFormat({ formatString: "support" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.contains("new function `prepareSupportTicket` is pure")
                && diagnostic.contains("no observable runtime effect")
                && diagnostic.contains("cannot be reached through execution wiring")
                && diagnostic.contains("Declare and return a value")
        }));
    }

    #[test]
    fn unresolved_side_effect_call_does_not_cascade_into_pure_function_warnings() {
        let board = empty_board();
        let catalog = vec![catalog_meta(
            "string_format",
            "String Format",
            vec![pin_meta("format_string", "String", PinType::Input)],
            vec![pin_meta("value", "String", PinType::Output)],
        )];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function sendApprovalRequest(smtp: Struct) {
    const subject = stringFormat({ formatString: "Approval" })
    missingSmtpSend({ connection: smtp, to: "approver@example.com", subject: subject.value })
}

function unrelatedPureHelper() {
    stringFormat({ formatString: "still pure" })
}
"#,
            &catalog,
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("missingSmtpSend")
                    && diagnostic.contains("does not match a catalog declaration")
            }),
            "the exact call repair must remain visible: {:?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().all(|diagnostic| {
                !diagnostic.contains("sendApprovalRequest` is pure")
                    && !diagnostic.contains("runtime-empty helper")
                    && !diagnostic.contains("materialized body tail")
            }),
            "unresolved-call fallout must not masquerade as independent function defects: {:?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("unrelatedPureHelper` is pure")
                    && diagnostic.contains("no observable runtime effect")
            }),
            "suppression must remain scoped to the function with the unresolved call: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn unsafe_resolved_call_shape_does_not_cascade_into_function_structure_warnings() {
        let board = empty_board();
        let catalog = vec![catalog_meta(
            "email_imap_inbox_fetch_mail",
            "Fetch Mail",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("email_ref", "Struct", PinType::Input),
            ],
            vec![
                pin_meta("exec_out", "Execution", PinType::Output),
                pin_meta("email", "Struct", PinType::Output),
            ],
        )];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function fetchInboxWrong(connection: Struct) {
    emailImapInboxFetchMail({ email: connection, unseenOnly: true, markSeen: true })
}
"#,
            &catalog,
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("mailbox-fetch shape cannot be auto-corrected")
                    && diagnostic.contains("accepts exactly `emailRef`")
            }),
            "the actionable call-shape diagnostic must remain: {:?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().all(|diagnostic| {
                !diagnostic.contains("runtime-empty helper")
                    && !diagnostic.contains("no materialized body nodes")
                    && !diagnostic.contains("no Function exec_in connection")
                    && !diagnostic.contains("no materialized body tail")
            }),
            "unsafe-call fallout must not become independent function diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn batch_upsert_success_closes_impure_function_body() {
        let board = empty_board();
        let catalog = vec![catalog_meta(
            "batch_upsert_local_db",
            "Batch Upsert",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("database", "Struct", PinType::Input),
                pin_meta("id_row", "String", PinType::Input),
                pin_meta_friendly("value", "Value", "Struct", "Array", PinType::Input),
            ],
            vec![
                pin_meta("exec_out", "Execution", PinType::Output),
                pin_meta("error", "Execution", PinType::Output),
                pin_meta("error_message", "String", PinType::Output),
            ],
        )];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function saveTicket() {
    batchUpsertLocalDb({ database: {}, idRow: "ticket_id", value: [] })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1"
                    && from_pin == "exec_out"
                    && to_node == "$0"
                    && to_pin == "exec_out"
        )));
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, .. }
                if from_node == "$1" && from_pin == "error" && to_node == "$0"
        )));
    }

    #[test]
    fn unknown_multi_exec_function_tail_keeps_actionable_branch_hint() {
        let board = empty_board();
        let catalog = vec![catalog_meta(
            "custom_terminal",
            "Custom Terminal",
            vec![pin_meta("exec_in", "Execution", PinType::Input)],
            vec![
                pin_meta("success", "Execution", PinType::Output),
                pin_meta("error", "Execution", PinType::Output),
            ],
        )];

        let result = reconcile_text_with_catalog(
            &board,
            "function runTerminal() {\n    customTerminal()\n}\n",
            &catalog,
        );

        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.contains("runTerminal")
                    && diagnostic.contains("no materialized body tail")
                    && diagnostic.contains("explicit labelled arms")
                    && diagnostic.contains("continuation policy")
            }),
            "{:?}",
            result.diagnostics
        );
    }

    #[test]
    fn existing_flat_layer_members_drive_layer_local_placement_only() {
        let mut board = empty_board();
        let layer = Layer::new(
            "function-layer".to_string(),
            "helper".to_string(),
            LayerType::Function,
        );
        board.layers.insert(layer.id.clone(), layer);

        let mut root = Node::new("root", "Root", "", "test");
        root.id = "root-node".to_string();
        root.coordinates = Some((100.0, 50.0, 0.0));
        board.nodes.insert(root.id.clone(), root);

        let mut body = Node::new("body", "Body", "", "test");
        body.id = "body-node".to_string();
        body.layer = Some("function-layer".to_string());
        body.coordinates = Some((1_000.0, 300.0, 0.0));
        board.nodes.insert(body.id.clone(), body);

        let planner = StructuralPlanner::new(&board, &[], None);
        assert_eq!(
            planner.rightmost_existing_position(Some("function-layer")),
            (1_000.0, 300.0),
            "canonical flat members must offset edits inside their Function"
        );
        assert_eq!(
            planner.rightmost_existing_position(None),
            (100.0, 50.0),
            "Function members must not push root/event placement to the right"
        );
    }

    #[test]
    fn new_function_bodies_use_compact_layer_local_positions() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_call_function",
                "Call Function",
                vec![pin_meta("function_layer_id", "String", PinType::Input)],
                vec![],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function first() {
    log({ message: "1" })
    log({ message: "2" })
    log({ message: "3" })
    log({ message: "4" })
    log({ message: "5" })
}

function second() {
    log({ message: "a" })
    log({ message: "b" })
}

eventsSimple() {
    first()
    second()
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let positions_for = |layer: &str| {
            result
                .commands
                .iter()
                .filter_map(|command| match command {
                    BoardCommand::AddNode {
                        target_layer: Some(target),
                        position: Some(position),
                        ..
                    } if target == layer => Some((position.x, position.y)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let first = positions_for("$0");
        let second = positions_for("$1");
        assert_eq!(first.len(), 5);
        assert_eq!(second.len(), 2);
        assert_eq!(first[0], (260.0, 200.0));
        assert_eq!(second[0], (260.0, 200.0));
        assert!(first.iter().all(|(x, _)| *x <= 1040.0), "{first:?}");
        assert!(second.iter().all(|(x, _)| *x <= 1040.0), "{second:?}");
        assert!(
            first.iter().any(|(_, y)| *y > 200.0),
            "the fifth node should wrap to a second row: {first:?}"
        );
    }

    #[test]
    fn support_workflow_helpers_stay_compact_and_event_entry_is_created_last() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_call_function",
                "Call Function",
                vec![pin_meta("function_layer_id", "String", PinType::Input)],
                vec![],
            ),
            catalog_meta(
                "list_unread",
                "List Unread",
                vec![],
                vec![pin_meta_friendly(
                    "emails",
                    "Emails",
                    "Generic",
                    "Array",
                    PinType::Output,
                )],
            ),
            catalog_meta(
                "for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array", "Array", "Generic", "Array", PinType::Input),
                ],
                vec![
                    pin_meta("loop", "Execution", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                    pin_meta("item", "Generic", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function ensureSupportTicketsTable() {
    log({ message: "ensure table" })
}

function pollSupportInbox() {
    const unreadRefs = listUnread()
    for (const item of forEach({ array: unreadRefs.emails })) {
        log({ message: "process mail" })
    }
}

eventsSimple() {
    ensureSupportTicketsTable()
    pollSupportInbox()
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| { !diagnostic.contains("no incoming execution connection") })
        );

        for layer in ["$0", "$1"] {
            let layer_positions = result
                .commands
                .iter()
                .filter_map(|command| match command {
                    BoardCommand::AddNode {
                        target_layer: Some(target),
                        position: Some(position),
                        ..
                    } if target == layer => Some((position.x, position.y)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                !layer_positions.is_empty(),
                "missing body nodes for {layer}"
            );
            assert!(
                layer_positions.iter().all(|(x, _)| *x <= 1040.0),
                "{layer} body escaped its compact layer-local grid: {layer_positions:?}"
            );
        }

        let event_index = result
            .commands
            .iter()
            .position(|command| {
                matches!(
                    command,
                    BoardCommand::AddNode { node_type, .. } if node_type == "events_simple"
                )
            })
            .expect("event entry command");
        let last_setup_index = result
            .commands
            .iter()
            .enumerate()
            .filter(|(_, command)| {
                matches!(
                    command,
                    BoardCommand::AddNode { .. } | BoardCommand::CreateLayer { .. }
                )
            })
            .map(|(index, _)| index)
            .max()
            .expect("setup commands");
        assert_eq!(
            event_index, last_setup_index,
            "event entry must be created only after helper layers and their workflow nodes"
        );
    }

    #[test]
    fn new_if_else_sugar_synthesizes_control_branch_with_labeled_arms() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_branch",
                "Branch",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("condition", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("true", "Execution", PinType::Output),
                    pin_meta("false", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"run() {
    if (true) {
        log({ message: "yes" })
    } else {
        log({ message: "no" })
    }
    log({ message: "after" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // Event $0, branch $1, then-arm log $2, else-arm log $3, after log $4.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, ref_id, .. }
                if node_type == "control_branch" && ref_id.as_deref() == Some("$1")
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "$1"
                    && pin_id == "condition"
                    && value == &flow_like_types::Value::Bool(true)
        )));
        // Each arm wires from ITS labeled exec pin.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1" && from_pin == "true" && to_node == "$2" && to_pin == "exec_in"
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1" && from_pin == "false" && to_node == "$3" && to_pin == "exec_in"
        )));
        // Fully-armed branch: the statement after the if/else must NOT steal an arm pin.
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, to_node, .. }
                if from_node == "$1" && to_node == "$4"
        )));
    }

    fn binary_condition_catalog(comparison: NodeMetadata) -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_branch",
                "Branch",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("condition", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("true", "Execution", PinType::Output),
                    pin_meta("false", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            comparison,
        ]
    }

    #[test]
    fn string_equality_condition_lowers_to_comparator_output() {
        let catalog = binary_condition_catalog(catalog_meta(
            "equal_string",
            "Equal String",
            vec![
                pin_meta("string", "String", PinType::Input),
                pin_meta("string", "String", PinType::Input),
            ],
            vec![pin_meta("equal", "Boolean", PinType::Output)],
        ));
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"run() {
    if ("sender@example.com" == "example@example.com") {
        log({ message: "approved" })
    }
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let comparator = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::AddNode {
                    node_type,
                    ref_id: Some(ref_id),
                    ..
                } if node_type == "equal_string" => Some(ref_id.as_str()),
                _ => None,
            })
            .expect("equal_string node");
        let branch = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::AddNode {
                    node_type,
                    ref_id: Some(ref_id),
                    ..
                } if node_type == "control_branch" => Some(ref_id.as_str()),
                _ => None,
            })
            .expect("control_branch node");
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == comparator
                    && from_pin == "equal"
                    && to_node == branch
                    && to_pin == "condition"
        )));
    }

    #[test]
    fn integer_ordering_condition_lowers_to_typed_comparator() {
        let catalog = binary_condition_catalog(catalog_meta(
            "int_greater_than",
            "Integer Greater Than",
            vec![
                pin_meta("integer1", "Integer", PinType::Input),
                pin_meta("integer2", "Integer", PinType::Input),
            ],
            vec![pin_meta("greater_than", "Boolean", PinType::Output)],
        ));
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"run() {
    if (3 > 2) {
        log({ message: "ordered" })
    }
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let comparator = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::AddNode {
                    node_type,
                    ref_id: Some(ref_id),
                    ..
                } if node_type == "int_greater_than" => Some(ref_id.as_str()),
                _ => None,
            })
            .expect("int_greater_than node");
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == comparator
                    && pin_id == "integer1"
                    && value == &flow_like_types::Value::from(3)
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == comparator
                    && pin_id == "integer2"
                    && value == &flow_like_types::Value::from(2)
        )));
    }

    #[test]
    fn arithmetic_and_boolean_operators_round_trip_through_catalog_nodes() {
        let mut catalog = binary_condition_catalog(catalog_meta(
            "int_greater_than",
            "Integer Greater Than",
            vec![
                pin_meta("integer1", "Integer", PinType::Input),
                pin_meta("integer2", "Integer", PinType::Input),
            ],
            vec![pin_meta("greater_than", "Boolean", PinType::Output)],
        ));
        catalog.extend([
            catalog_meta(
                "int_add",
                "Integer Add",
                vec![
                    pin_meta("integer1", "Integer", PinType::Input),
                    pin_meta("integer2", "Integer", PinType::Input),
                ],
                vec![pin_meta("sum", "Integer", PinType::Output)],
            ),
            catalog_meta(
                "bool_and",
                "Boolean And",
                vec![
                    pin_meta("boolean", "Boolean", PinType::Input),
                    pin_meta("boolean", "Boolean", PinType::Input),
                ],
                vec![pin_meta("result", "Boolean", PinType::Output)],
            ),
            catalog_meta(
                "bool_or",
                "Boolean Or",
                vec![
                    pin_meta("boolean", "Boolean", PinType::Input),
                    pin_meta("boolean", "Boolean", PinType::Input),
                ],
                vec![pin_meta("result", "Boolean", PinType::Output)],
            ),
        ]);

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"run() {
    if (1 + 2 > 2 && true || false) {
        log({ message: "approved" })
    }
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for expected in ["int_add", "int_greater_than", "bool_and", "bool_or"] {
            assert!(
                result.commands.iter().any(|command| matches!(
                    command,
                    BoardCommand::AddNode { node_type, .. } if node_type == expected
                )),
                "missing {expected}: {:?}",
                result.commands
            );
        }
        let bool_or = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::AddNode {
                    node_type,
                    ref_id: Some(ref_id),
                    ..
                } if node_type == "bool_or" => Some(ref_id.as_str()),
                _ => None,
            })
            .expect("bool_or node");
        let branch = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::AddNode {
                    node_type,
                    ref_id: Some(ref_id),
                    ..
                } if node_type == "control_branch" => Some(ref_id.as_str()),
                _ => None,
            })
            .expect("control_branch node");
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == bool_or
                    && from_pin == "result"
                    && to_node == branch
                    && to_pin == "condition"
        )));
    }

    #[test]
    fn editing_existing_binary_condition_reuses_comparator() {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut comparator = Node::new("equal_string", "Equal", "", "string");
        comparator.id = "comparator".to_string();
        let left = comparator.add_input_pin("string", "String", "", VariableType::String);
        left.default_value = Some(b"\"sender@example.com\"".to_vec());
        let right = comparator.add_input_pin("string", "String", "", VariableType::String);
        right.default_value = Some(b"\"old@example.com\"".to_vec());
        let equal = comparator
            .add_output_pin("equal", "Equal", "", VariableType::Boolean)
            .id
            .clone();
        board.nodes.insert(comparator.id.clone(), comparator);

        let mut branch = Node::new("control_branch", "Branch", "", "control");
        branch.id = "branch".to_string();
        let branch_in = branch
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let condition = branch
            .add_input_pin("condition", "Condition", "", VariableType::Boolean)
            .id
            .clone();
        let branch_true = branch
            .add_output_pin("true", "True", "", VariableType::Execution)
            .id
            .clone();
        branch.add_output_pin("false", "False", "", VariableType::Execution);
        board.nodes.insert(branch.id.clone(), branch);

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let log_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let message = log.add_input_pin("message", "Message", "", VariableType::String);
        message.default_value = Some(b"\"matched\"".to_vec());
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(log.id.clone(), log);

        connect(&mut board, "event", &event_out, "branch", &branch_in);
        connect(&mut board, "comparator", &equal, "branch", &condition);
        connect(&mut board, "branch", &branch_true, "log", &log_in);

        let text = anchored_text(&board).replace("\"old@example.com\"", "\"new@example.com\"");
        let result = reconcile_text_with_catalog(&board, &text, &[]);

        assert!(
            result.diagnostics.is_empty(),
            "{:?}\nFlowScript:\n{text}",
            result.diagnostics
        );
        assert_eq!(
            result
                .commands
                .iter()
                .filter(|command| matches!(command, BoardCommand::AddNode { .. }))
                .count(),
            0,
            "an edited binary condition must reuse its existing comparator"
        );
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "comparator"
                    && pin_id == "string[#2]"
                    && value == &flow_like_types::Value::String("new@example.com".to_string())
        )));
    }

    #[test]
    fn lone_if_continues_following_statement_from_false_pin() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_branch",
                "Branch",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("condition", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("true", "Execution", PinType::Output),
                    pin_meta("false", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"run() {
    if (true) {
        log({ message: "yes" })
    }
    log({ message: "after" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // Event $0, branch $1, then-arm log $2, after log $3: the statement after a lone `if`
        // continues from the unclaimed `false` pin.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1" && from_pin == "false" && to_node == "$3" && to_pin == "exec_in"
        )));
    }

    /// Board: event → log_mid → log_tail (execution chain with anchored nodes).
    fn board_with_exec_chain() -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut mid = Node::new("log", "Log", "", "debug");
        mid.id = "log_mid".to_string();
        let mid_in = mid
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let mid_out = mid
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        mid.add_input_pin("text", "Text", "", VariableType::String)
            .default_value = Some(b"\"mid\"".to_vec());
        board.nodes.insert(mid.id.clone(), mid);

        let mut tail = Node::new("log", "Log", "", "debug");
        tail.id = "log_tail".to_string();
        let tail_in = tail
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        tail.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        tail.add_input_pin("text", "Text", "", VariableType::String)
            .default_value = Some(b"\"tail\"".to_vec());
        board.nodes.insert(tail.id.clone(), tail);

        connect(&mut board, "event", &event_out, "log_mid", &mid_in);
        connect(&mut board, "log_mid", &mid_out, "log_tail", &tail_in);
        board
    }

    fn exec_chain_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    #[test]
    fn deleting_mid_chain_statement_bridges_execution() {
        let board = board_with_exec_chain();
        let text = crate::flow::ast::board_to_flowscript(
            &board,
            &crate::flow::ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );
        let edited: String = text
            .lines()
            .filter(|line| !line.contains("//@n:log_mid"))
            .collect::<Vec<_>>()
            .join("\n");

        let result = reconcile_text_with_catalog(&board, &edited, &exec_chain_catalog());
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::RemoveNode { node_id, .. } if node_id == "log_mid"
        )));
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "event"
                        && from_pin == "exec_out"
                        && to_node == "log_tail"
                        && to_pin == "exec_in"
            )),
            "deletion must re-join the severed chain: {:?}",
            result.commands
        );
    }

    #[test]
    fn deletion_bridge_prefers_flat_pin_metadata_over_stale_mirror() {
        let mut board = board_with_exec_chain();
        let mut stale_event = board.nodes.get("event").expect("event").clone();
        let event_output_id = find_output_pin(&stale_event, "exec_out")
            .expect("event output")
            .id
            .clone();
        stale_event
            .pins
            .get_mut(&event_output_id)
            .expect("stale event output")
            .name = "stale_exec_out".to_string();

        let mut layer = Layer::new(
            "legacy-layer".to_string(),
            "Legacy Layer".to_string(),
            LayerType::Function,
        );
        layer.nodes.insert(stale_event.id.clone(), stale_event);
        board.layers.insert(layer.id.clone(), layer);

        let removed = HashSet::from(["log_mid".to_string()]);
        let occupied_sources = HashSet::new();
        let mut commands = Vec::new();
        let mut diagnostics = Vec::new();
        bridge_removed_exec_chains(
            &board,
            &removed,
            &occupied_sources,
            &mut commands,
            &mut diagnostics,
        );

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(
            commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "event"
                        && from_pin == "exec_out"
                        && to_node == "log_tail"
                        && to_pin == "exec_in"
            )),
            "bridge must use canonical flat pin metadata: {commands:?}"
        );
        assert!(!commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_pin, .. } if from_pin == "stale_exec_out"
        )));
    }

    #[test]
    fn replacing_mid_chain_statement_does_not_bridge_over_the_replacement() {
        let board = board_with_exec_chain();
        let text = crate::flow::ast::board_to_flowscript(
            &board,
            &crate::flow::ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );
        let edited: String = text
            .lines()
            .map(|line| {
                if line.contains("//@n:log_mid") {
                    "    log({ text: \"replacement\" })".to_string()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let result = reconcile_text_with_catalog(&board, &edited, &exec_chain_catalog());
        // The replacement node takes over the chain...
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, .. }
                if from_node == "event" && from_pin == "exec_out" && to_node.starts_with('$')
        )));
        // ...so no bridge may steal the event's exec output back to the old successor
        // (exec outputs are single-target: a later bridge would orphan the replacement).
        assert!(
            !result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, .. }
                    if from_node == "event" && from_pin == "exec_out" && to_node == "log_tail"
            )),
            "bridge must skip sources the plan already drives: {:?}",
            result.commands
        );
    }

    fn node_limit_add_command(ref_id: &str, target_layer: Option<&str>) -> BoardCommand {
        BoardCommand::AddNode {
            node_type: "log".to_string(),
            ref_id: Some(ref_id.to_string()),
            position: None,
            friendly_name: None,
            additional_pins: None,
            target_layer: target_layer.map(str::to_string),
            summary: None,
        }
    }

    fn board_with_mirrored_layer_nodes(count: usize) -> Board {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "function-layer".to_string(),
            "helper".to_string(),
            LayerType::Function,
        );
        for index in 0..count {
            let mut node = Node::new("log", "Log", "", "debug");
            node.id = format!("body-{index}");
            node.layer = Some(layer.id.clone());
            board.nodes.insert(node.id.clone(), node.clone());
            layer.nodes.insert(node.id.clone(), node);
        }
        board.layers.insert(layer.id.clone(), layer);
        board
    }

    #[test]
    fn layer_node_limit_counts_mirrored_canonical_nodes_once() {
        let board = board_with_mirrored_layer_nodes(MAX_NODES_PER_LAYER);
        let unrelated_root_add = node_limit_add_command("$root", None);

        assert!(
            layer_node_limit_violations(&board, &[unrelated_root_add]).is_none(),
            "the flat and layer-local maps contain the same identities and must not consume the Function budget twice"
        );
    }

    #[test]
    fn layer_node_limit_reports_unique_population_after_deduplication() {
        let board = board_with_mirrored_layer_nodes(MAX_NODES_PER_LAYER);
        let layer_add = node_limit_add_command("$body", Some("function-layer"));

        let diagnostics = layer_node_limit_violations(&board, &[layer_add])
            .expect("one genuinely new Function node must exceed the limit");
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].contains(&format!("{} nodes", MAX_NODES_PER_LAYER + 1)),
            "the diagnostic must report the unique post-edit population: {diagnostics:?}"
        );
        assert!(!diagnostics[0].contains(&format!("{} nodes", MAX_NODES_PER_LAYER * 2 + 1)));
    }

    #[test]
    fn layer_node_limit_treats_empty_layer_ids_as_root() {
        let mut board = empty_board();
        for index in 0..(MAX_NODES_PER_LAYER - 1) {
            let mut node = Node::new("log", "Log", "", "debug");
            node.id = format!("root-{index}");
            node.layer = Some(String::new());
            board.nodes.insert(node.id.clone(), node);
        }
        let root_adds = [
            node_limit_add_command("$root-1", None),
            node_limit_add_command("$root-2", None),
        ];

        let diagnostics = layer_node_limit_violations(&board, &root_adds)
            .expect("empty layer ids and explicit root additions share one budget");
        assert!(diagnostics[0].contains(&format!(
            "the root layer with {} nodes",
            MAX_NODES_PER_LAYER + 1
        )));
    }

    #[test]
    fn layer_node_limit_rejects_oversized_edit() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let mut flowscript = String::from("run() {\n");
        for index in 0..MAX_NODES_PER_LAYER {
            flowscript.push_str(&format!("    log({{ message: \"{index}\" }})\n"));
        }
        flowscript.push_str("}\n");

        let result = reconcile_text_with_catalog(&board, &flowscript, &catalog);
        assert!(
            result.commands.is_empty(),
            "oversized edit must queue nothing"
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains(&format!("max {MAX_NODES_PER_LAYER}"))),
            "{:?}",
            result.diagnostics
        );
    }

    /// Catalog for a `string_format` node exactly as its `get_node()` declares it: a `format_string`
    /// input and a `value` output, with NO placeholder pins (those are minted by `on_update`).
    fn string_format_dynamic_catalog() -> Vec<NodeMetadata> {
        vec![catalog_meta(
            "string_format",
            "String Format",
            vec![pin_meta("format_string", "String", PinType::Input)],
            vec![pin_meta("value", "String", PinType::Output)],
        )]
    }

    #[test]
    fn repeated_string_format_placeholders_are_unique_and_ordered() {
        let query = "SELECT id, parent_id, title, path, updated_at FROM wiki_pages \
            WHERE lower(title) LIKE lower('%{query}%') \
            OR lower(path) LIKE lower('%{query}%') \
            ORDER BY path LIMIT 50 OFFSET {offset};";

        assert_eq!(
            format_string_placeholders(query),
            vec!["query".to_string(), "offset".to_string()]
        );
    }

    fn render_template_dynamic_catalog() -> Vec<NodeMetadata> {
        vec![catalog_meta(
            "string_render_template",
            "Render Template",
            vec![pin_meta("template", "String", PinType::Input)],
            vec![pin_meta("rendered", "String", PinType::Output)],
        )]
    }

    #[test]
    fn string_format_wired_placeholder_pin_connects_without_diagnostic() {
        let board = empty_board();
        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(name: string): (message: string) {
    const formatted = stringFormat({ formatString: "Hello {name}", name: name })
    return formatted.value
}
"#,
            &string_format_dynamic_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$0"
                        && from_pin == "name"
                        && to_node == "$1"
                        && to_pin == "name"
            )),
            "placeholder `{{name}}` should wire the function param into the synthesized `name` pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn string_format_literal_placeholder_pin_emits_update_without_diagnostic() {
        let board = empty_board();
        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(): (message: string) {
    const formatted = stringFormat({ formatString: "Count {idx}", idx: "five" })
    return formatted.value
}
"#,
            &string_format_dynamic_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { pin_id, value, .. }
                    if pin_id == "idx"
                        && value == &flow_like_types::Value::String("five".to_string())
            )),
            "literal placeholder value should set the synthesized `idx` pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn string_format_non_placeholder_arg_still_diagnoses() {
        let board = empty_board();
        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(): (message: string) {
    const formatted = stringFormat({ formatString: "Count {idx}", jdx: "x" })
    return formatted.value
}
"#,
            &string_format_dynamic_catalog(),
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("no input pin named `jdx`")),
            "an arg that is not a template placeholder must not be synthesized: {:?}",
            result.diagnostics
        );
    }

    fn sql_query_dynamic_catalog() -> Vec<NodeMetadata> {
        vec![catalog_meta(
            "df_sql_query",
            "SQL Query",
            vec![
                pin_meta("session", "Struct", PinType::Input),
                pin_meta("query", "String", PinType::Input),
                pin_meta("params", "Struct", PinType::Input),
            ],
            vec![pin_meta("rows", "Struct", PinType::Output)],
        )]
    }

    #[test]
    fn sql_placeholder_pins_are_predicted_from_the_query_literal() {
        // `paramOrgId` exists only after `on_update` reads the query, so without prediction the
        // whole batch is rejected and the only way to author this is string concatenation.
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function load(orgId: string): (rows: struct) {
    const result = dfSqlQuery({ query: "SELECT * FROM users WHERE org = $org_id", paramOrgId: orgId })
    return result.rows
}
"#,
            &sql_query_dynamic_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_pin, to_pin, .. }
                    if from_pin == "orgId" && to_pin == "paramOrgId"
            )),
            "the parameter value should wire into the predicted pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn sql_numbered_placeholder_pin_is_predicted() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function load(): (rows: struct) {
    const result = dfSqlQuery({ query: "SELECT * FROM users WHERE id = $1", param1: 7 })
    return result.rows
}
"#,
            &sql_query_dynamic_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }

    #[test]
    fn sql_arg_without_a_matching_placeholder_still_diagnoses() {
        // The query declares `$org_id`, so `paramTeamId` is a typo, not a parameter. Catching
        // it here is the whole reason prediction is driven by the literal.
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function load(): (rows: struct) {
    const result = dfSqlQuery({ query: "SELECT * FROM users WHERE org = $org_id", paramTeamId: "x" })
    return result.rows
}
"#,
            &sql_query_dynamic_catalog(),
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("no input pin named `paramTeamId`")),
            "unexpected diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn sql_placeholder_inside_a_string_literal_is_not_a_parameter() {
        // `'$5.00'` is data. Predicting `param5` here would accept an argument the node never
        // mints, turning a check-time typo into an apply-time rollback.
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function load(): (rows: struct) {
    const result = dfSqlQuery({ query: "SELECT * FROM t WHERE price = '$5.00'", param5: 1 })
    return result.rows
}
"#,
            &sql_query_dynamic_catalog(),
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("no input pin named `param5`")),
            "unexpected diagnostics: {:?}",
            result.diagnostics
        );
    }

    fn instantiate_widget_catalog() -> Vec<NodeMetadata> {
        vec![catalog_meta(
            "a2ui_instantiate_widget",
            "Instantiate Widget",
            vec![
                pin_meta("widget_selector", "String", PinType::Input),
                pin_meta("instance_id", "String", PinType::Input),
            ],
            vec![pin_meta("element_ref", "Struct", PinType::Output)],
        )]
    }

    #[test]
    fn widget_binding_pins_are_accepted_on_a_new_node() {
        // These pins come from the persisted widget, which reconcile cannot read. Refusing to
        // plan the command used to fail the entire board build for a *correct* binding, even
        // though apply resolves it fine once `on_update` has run.
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function render(title: string): (element: struct) {
    const instance = a2uiInstantiateWidget({ widgetSelector: "Article", instanceId: "a1", dynPathTitle: title, dynPropTone: "muted", dynCustDensity: "compact", dynInHeading: title })
    return instance.elementRef
}
"#,
            &instantiate_widget_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { to_pin, .. } if to_pin == "dynPathTitle"
            )),
            "the wired binding should be planned: {:?}",
            result.commands
        );
    }

    #[test]
    fn non_binding_args_on_a_widget_node_still_diagnose() {
        // Permissive prediction is scoped to the `dyn*` prefixes; an ordinary typo on the same
        // node must still be caught at check time.
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function render(): (element: struct) {
    const instance = a2uiInstantiateWidget({ widgetSelector: "Article", instanceId: "a1", widgetSelektor: "typo" })
    return instance.elementRef
}
"#,
            &instantiate_widget_catalog(),
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("widgetSelektor")),
            "unexpected diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn connection_derived_widget_bindings_diagnose_with_their_real_cause() {
        // `a2ui_widget_update_inputs` derives its `dyn_in_*` pins from a connected
        // `element_ref`, and connections are the last commands apply runs — so the pin cannot
        // exist in time no matter what reconcile predicts. Failing here with the cause named
        // beats planning a command that can only end in an apply-time rollback.
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function patch(title: string): (done: bool) {
    a2uiWidgetUpdateInputs({ dynInHeading: title })
    return true
}
"#,
            &[catalog_meta(
                "a2ui_widget_update_inputs",
                "Update Widget Inputs",
                vec![pin_meta("element_ref", "Struct", PinType::Input)],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            )],
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("`elementRef` input is connected")),
            "unexpected diagnostics: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn widget_binding_prefixes_cover_every_kind_the_nodes_mint() {
        for name in [
            "dyn_path_title",
            "dynPathTitle",
            "dyn_prop_tone",
            "dynPropTone",
            "dyn_cust_density",
            "dynCustDensity",
            "dyn_in_heading",
            "dynInHeading",
            "dyn_arg_limit",
            "dynArgLimit",
        ] {
            assert!(
                is_widget_dynamic_binding_arg(name),
                "{name} should be recognized as a widget binding"
            );
        }
        for name in ["widget_selector", "instance_id", "dynamic", "dyn"] {
            assert!(
                !is_widget_dynamic_binding_arg(name),
                "{name} must not be treated as a widget binding"
            );
        }
    }

    #[test]
    fn jinja_template_placeholder_pin_is_predicted_without_runtime_enricher() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function greet(name: string): (message: string) {
    const rendered = stringRenderTemplate({ template: "Hello {{ name }}", name: name })
    return rendered.rendered
}
"#,
            &render_template_dynamic_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$0"
                        && from_pin == "name"
                        && to_node == "$1"
                        && to_pin == "name"
            )),
            "Jinja's undeclared `name` variable should become a dynamic input pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn csv_chart_mode_pins_are_predicted_without_runtime_enricher() {
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "a2ui_push_csv_to_chart",
                "Push Data to Chart",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("element_ref", "Struct", PinType::Input),
                    pin_meta("library", "String", PinType::Input),
                    pin_meta("format", "String", PinType::Input),
                    pin_meta("data", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"eventsSimple() {
    a2uiPushCsvToChart({ elementRef: {}, library: "Nivo", format: "CSV", csv: "x,y\\n1,2", chartType: "Bar", delimiter: "," })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for expected in ["csv", "chart_type", "delimiter"] {
            assert!(
                result.commands.iter().any(|command| matches!(
                    command,
                    BoardCommand::UpdateNodePin { pin_id, .. } if pin_id == expected
                )),
                "missing dynamic CSV pin `{expected}`: {:?}",
                result.commands
            );
        }
    }

    #[test]
    fn config_edit_defers_literal_on_new_string_format_placeholder() {
        // Existing anchored `string_format` with format "Hi {name}" — it has a live `name` pin but
        // no `age` pin (that placeholder does not exist yet).
        let mut board = empty_board();
        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let mut fmt = Node::new("string_format", "String Format", "", "std");
        fmt.id = "fmt".to_string();
        fmt.add_input_pin("format_string", "Format", "", VariableType::String)
            .default_value = Some("\"Hi {name}\"".to_string().into_bytes());
        fmt.add_input_pin("name", "Name", "", VariableType::Generic);
        fmt.add_output_pin("value", "Value", "", VariableType::String);
        board.nodes.insert(fmt.id.clone(), fmt);

        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "string_format",
                "String Format",
                vec![pin_meta("format_string", "String", PinType::Input)],
                vec![pin_meta("value", "String", PinType::Output)],
            ),
        ];

        // The re-submitted text adds a NEW `{age}` placeholder plus its literal.
        let result = reconcile_text_with_catalog(
            &board,
            "eventsSimple() {   //@n:event\n    stringFormat({ formatString: \"Hi {name} {age}\", name: \"World\", age: \"5\" })   //@n:fmt\n}\n",
            &catalog,
        );

        // The not-yet-minted `age` placeholder must NOT be reported as a missing pin...
        assert!(
            !result.diagnostics.iter().any(|d| d.contains("age")),
            "new placeholder should not diagnose as missing: {:?}",
            result.diagnostics
        );
        // ...its literal is deferred as an UpdateNodePin (apply mints the pin via on_update first).
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "fmt"
                        && pin_id == "age"
                        && value == &flow_like_types::Value::String("5".to_string())
            )),
            "literal for the new `age` placeholder should be deferred as an UpdateNodePin: {:?}",
            result.commands
        );
    }

    fn accumulator_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "variable_set",
                "Set Variable",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("var_ref", "String", PinType::Input),
                    pin_meta("value_in", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value_ref", "Generic", PinType::Output),
                ],
            ),
            catalog_meta(
                "array_push",
                "Push",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array_in", "Array", "Generic", "Array", PinType::Input),
                    pin_meta("value", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta_friendly("array_out", "Array", "Generic", "Array", PinType::Output),
                ],
            ),
            catalog_meta(
                "control_for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array", "Array", "Generic", "Array", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value", "Generic", PinType::Output),
                    pin_meta("index", "Integer", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "batch_insert_local_db",
                "Batch Insert Local DB",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("database", "Struct", PinType::Input),
                    pin_meta_friendly("value", "Value", "Generic", "Array", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    fn struct_accumulator_catalog() -> Vec<NodeMetadata> {
        let mut catalog = accumulator_catalog();
        catalog.extend([
            catalog_meta(
                "struct_make",
                "Make Struct",
                Vec::new(),
                vec![pin_meta("struct", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "struct_set",
                "Set Field",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("struct_in", "Struct", PinType::Input),
                    pin_meta("field", "String", PinType::Input),
                    pin_meta("value", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("struct_out", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "cuid",
                "CUID v2",
                Vec::new(),
                vec![pin_meta("cuid", "String", PinType::Output)],
            ),
            catalog_meta(
                "consume_struct",
                "Consume Struct",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("value", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]);
        catalog
    }

    fn required_struct_set_value_catalog() -> Vec<NodeMetadata> {
        let mut catalog = struct_accumulator_catalog();
        catalog
            .iter_mut()
            .find(|meta| meta.name == "struct_set")
            .expect("struct_set catalog entry")
            .required_inputs = vec!["value".to_string()];
        catalog
    }

    fn command_node_type(commands: &[BoardCommand], ref_id: &str) -> Option<String> {
        commands.iter().find_map(|command| match command {
            BoardCommand::AddNode {
                node_type,
                ref_id: Some(id),
                ..
            } if id == ref_id => Some(node_type.clone()),
            _ => None,
        })
    }

    fn by_ref_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "array_clear_ref",
                "Clear (By Ref)",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("var_ref", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "array_push_ref",
                "Push (By Ref)",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("var_ref", "String", PinType::Input),
                    pin_meta("value", "Generic", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "batch_upsert_local_db",
                "Batch Upsert",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("database", "Struct", PinType::Input),
                    pin_meta_friendly("id_row", "ID Column", "String", "Normal", PinType::Input),
                    pin_meta_friendly("value", "Value", "Generic", "Array", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    #[test]
    fn catalog_aware_reconcile_normalizes_by_ref_variable_names_to_ids() {
        let board = empty_board();
        let catalog = by_ref_catalog();

        let result = reconcile_text_with_catalog(
            &board,
            r#"const rows: Struct[] = []

eventsSimple() {
    arrayClearRef({ varRef: "rows" })
    arrayPushRef({ varRef: "rows", value: {} })
    batchUpsertLocalDb({ database: {}, idRow: "id", value: rows })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref()
                        == Some("array_clear_ref")
                        && pin_id == "var_ref"
                        && value == &flow_like_types::Value::String("var_rows".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref()
                        == Some("array_push_ref")
                        && pin_id == "var_ref"
                        && value == &flow_like_types::Value::String("var_rows".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref()
                        == Some("batch_upsert_local_db")
                        && pin_id == "id_row"
                        && value == &flow_like_types::Value::String("id".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("variable_get")
                        && from_pin == "value_ref"
                        && command_node_type(&result.commands, to_node).as_deref()
                            == Some("batch_upsert_local_db")
                        && to_pin == "value"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_wires_existing_anchored_variable_argument() {
        let mut board = empty_board();
        let mut variable = Variable::new("rows", VariableType::Struct, ValueType::Array);
        variable.id = "var_rows".to_string();
        board.variables.insert(variable.id.clone(), variable);

        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let mut batch = Node::new("batch_upsert_local_db", "Batch Upsert", "", "data");
        batch.id = "batch".to_string();
        batch.add_input_pin("exec_in", "In", "", VariableType::Execution);
        batch.add_input_pin("database", "Database", "", VariableType::Struct);
        batch.add_input_pin("id_row", "ID Column", "", VariableType::String);
        let value_pin = batch.add_input_pin("value", "Value", "", VariableType::Generic);
        value_pin.value_type = ValueType::Array;
        batch.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(batch.id.clone(), batch);

        let result = reconcile_text_with_catalog(
            &board,
            r#"const rows: Struct[] = []   //@v:var_rows

eventsSimple() {   //@n:event
    batchUpsertLocalDb({ database: {}, idRow: "id", value: rows })   //@n:batch
}
"#,
            &by_ref_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "variable_get"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref()
                        == Some("variable_get")
                        && pin_id == "var_ref"
                        && value == &flow_like_types::Value::String("var_rows".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("variable_get")
                        && from_pin == "value_ref"
                        && to_node == "batch"
                        && to_pin == "value"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "batch"
                        && pin_id == "id_row"
                        && value == &flow_like_types::Value::String("id".to_string())
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_recreates_missing_inline_request_chain_for_anchored_call() {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let mut fetch = Node::new("http_fetch", "API Call", "", "http");
        fetch.id = "fetch".to_string();
        fetch.add_input_pin("exec_in", "In", "", VariableType::Execution);
        fetch.add_input_pin("request", "Request", "", VariableType::Struct);
        fetch.add_output_pin("exec_success", "Success", "", VariableType::Execution);
        fetch.add_output_pin("response", "Response", "", VariableType::Struct);
        board.nodes.insert(fetch.id.clone(), fetch);

        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "http_make_request",
                "Make Request",
                vec![
                    pin_meta("method", "String", PinType::Input),
                    pin_meta("url", "String", PinType::Input),
                ],
                vec![pin_meta("request", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "http_set_header",
                "Set Header",
                vec![
                    pin_meta("request", "Struct", PinType::Input),
                    pin_meta("name", "String", PinType::Input),
                    pin_meta("value", "String", PinType::Input),
                ],
                vec![pin_meta("request", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "http_fetch",
                "API Call",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("request", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_success", "Execution", PinType::Output),
                    pin_meta("response", "Struct", PinType::Output),
                ],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"simpleEvent() {   //@n:event
    const aPICall = httpFetch({ request: httpSetHeader({ request: httpMakeRequest({ method: "GET", url: "https://www.reddit.com/r/rust/.rss" }), name: "User-Agent", value: "FlowLikeRedditRSS/1.0" }) })   //@n:fetch
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "http_make_request"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "http_set_header"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref()
                        == Some("http_make_request")
                        && pin_id == "method"
                        && value == &flow_like_types::Value::String("GET".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("http_make_request")
                        && from_pin == "request"
                        && command_node_type(&result.commands, to_node).as_deref()
                            == Some("http_set_header")
                        && to_pin == "request"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("http_set_header")
                        && from_pin == "request"
                        && to_node == "fetch"
                        && to_pin == "request"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_does_not_remove_unanchored_inline_helpers() {
        let mut board = empty_board();

        let mut request = Node::new("http_make_request", "Make Request", "", "http");
        request.id = "request".to_string();
        request
            .add_input_pin("method", "Method", "", VariableType::String)
            .default_value = Some(flow_like_types::json::to_vec("GET").unwrap());
        request
            .add_input_pin("url", "URL", "", VariableType::String)
            .default_value =
            Some(flow_like_types::json::to_vec("https://www.reddit.com/r/rust/.rss").unwrap());
        let request_out = request
            .add_output_pin("request", "Request", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(request.id.clone(), request);

        let mut fetch = Node::new("http_fetch", "API Call", "", "http");
        fetch.id = "fetch".to_string();
        fetch.add_input_pin("exec_in", "In", "", VariableType::Execution);
        let fetch_request = fetch
            .add_input_pin("request", "Request", "", VariableType::Struct)
            .id
            .clone();
        fetch.add_output_pin("exec_success", "Success", "", VariableType::Execution);
        fetch.add_output_pin("response", "Response", "", VariableType::Struct);
        board.nodes.insert(fetch.id.clone(), fetch);

        connect(&mut board, "request", &request_out, "fetch", &fetch_request);

        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "http_make_request",
                "Make Request",
                vec![
                    pin_meta("method", "String", PinType::Input),
                    pin_meta("url", "String", PinType::Input),
                ],
                vec![pin_meta("request", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "http_fetch",
                "API Call",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("request", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_success", "Execution", PinType::Output),
                    pin_meta("response", "Struct", PinType::Output),
                ],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"simpleEvent() {
    const aPICall = httpFetch({ request: httpMakeRequest({ method: "GET", url: "https://www.reddit.com/r/rust/.rss" }) })   //@n:fetch
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::RemoveNode { node_id, .. } if node_id == "request"
            )
        }));
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "http_make_request"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_repairs_anchored_variable_set_assignment() {
        let mut board = empty_board();
        let mut variable = Variable::new("rows", VariableType::Struct, ValueType::Array);
        variable.id = "var_rows".to_string();
        board.variables.insert(variable.id.clone(), variable);

        let mut push = Node::new("array_push", "Push", "", "array");
        push.id = "push".to_string();
        push.add_input_pin("exec_in", "In", "", VariableType::Execution);
        push.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        push.add_output_pin("array_out", "Array", "", VariableType::Struct)
            .value_type = ValueType::Array;
        board.nodes.insert(push.id.clone(), push);

        let mut set_rows = Node::new("variable_set", "Set rows", "", "variables");
        set_rows.id = "set_rows".to_string();
        set_rows.add_input_pin("exec_in", "In", "", VariableType::Execution);
        set_rows.add_input_pin("var_ref", "rows", "", VariableType::String);
        let value_pin = set_rows.add_input_pin("value_in", "Value", "", VariableType::Struct);
        value_pin.value_type = ValueType::Array;
        set_rows.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        set_rows
            .add_output_pin("value_ref", "New Value", "", VariableType::Struct)
            .value_type = ValueType::Array;
        board.nodes.insert(set_rows.id.clone(), set_rows);

        let result = reconcile_text_with_catalog(
            &board,
            r#"const rows: Struct[] = []   //@v:var_rows

push(arrayOut: Struct[]) {   //@n:push
    rows = arrayOut   //@n:set_rows
}
"#,
            &accumulator_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "set_rows"
                        && pin_id == "var_ref"
                        && value == &flow_like_types::Value::String("var_rows".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "push"
                        && from_pin == "array_out"
                        && to_node == "set_rows"
                        && to_pin == "value_in"
            )
        }));
    }

    #[test]
    fn function_return_resolves_a_binding_declared_inside_a_loop_body() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function parseRssXml(): (rows: Generic[]) {
    for (const item of controlForEach({ array: [] })) {
        const batchPush = arrayPush({ arrayIn: [], value: item.value })
    }
    return batchPush.arrayOut
}
"#,
            &accumulator_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, from_pin, to_pin, .. }
                        if command_node_type(&result.commands, from_node).as_deref()
                            == Some("array_push")
                            && from_pin == "array_out"
                            && to_pin == "rows"
                )
            }),
            "the accumulator inside the loop must reach the return pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn function_return_resolves_a_binding_declared_inside_a_branch_arm() {
        let mut catalog = accumulator_catalog();
        catalog.push(catalog_meta(
            "control_branch",
            "Branch",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("condition", "Boolean", PinType::Input),
            ],
            vec![
                pin_meta("true", "Execution", PinType::Output),
                pin_meta("false", "Execution", PinType::Output),
            ],
        ));

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function parseRssXml(): (rows: Generic[]) {
    if (true) {
        const batchPush = arrayPush({ arrayIn: [], value: "one" })
    }
    return batchPush.arrayOut
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, from_pin, to_pin, .. }
                        if command_node_type(&result.commands, from_node).as_deref()
                            == Some("array_push")
                            && from_pin == "array_out"
                            && to_pin == "rows"
                )
            }),
            "the arm-local accumulator must reach the return pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn a_closed_block_binding_never_shadows_an_enclosing_one() {
        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function parseRssXml(): (rows: Generic[]) {
    const batchPush = arrayPush({ arrayIn: [], value: "outer" })
    for (const item of controlForEach({ array: [] })) {
        const batchPush = arrayPush({ arrayIn: [], value: item.value })
    }
    return batchPush.arrayOut
}
"#,
            &accumulator_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let outer = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::UpdateNodePin {
                    node_id,
                    pin_id,
                    value,
                    ..
                } if pin_id == "value"
                    && value == &flow_like_types::Value::String("outer".to_string()) =>
                {
                    Some(node_id.clone())
                }
                _ => None,
            })
            .expect("the enclosing push must be materialized");
        assert!(
            result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, from_pin, to_pin, .. }
                        if from_node == &outer && from_pin == "array_out" && to_pin == "rows"
                )
            }),
            "the enclosing binding must still win over the loop-local one: {:?}",
            result.commands
        );
    }

    #[test]
    fn catalog_aware_reconcile_wires_top_level_variable_accumulator_to_db_insert() {
        let board = empty_board();
        let catalog = accumulator_catalog();

        let result = reconcile_text_with_catalog(
            &board,
            r#"const rows: Struct[] = []

eventsSimple() {
    const push = arrayPush({ arrayIn: rows, value: { id: "one" } })
    rows = push.arrayOut
    batchInsertLocalDb({ database: {}, value: rows })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::CreateVariable {
                    variable_id: Some(variable_id),
                    name,
                    value_type,
                    ..
                } if variable_id == "var_rows" && name == "rows" && value_type == "Array"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "variable_get"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "variable_set"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { pin_id, value, .. }
                    if pin_id == "var_ref"
                        && value == &flow_like_types::Value::String("var_rows".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("array_push")
                        && from_pin == "array_out"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("variable_set")
                        && to_pin == "value_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("variable_get")
                        && from_pin == "value_ref"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("batch_insert_local_db")
                        && to_pin == "value"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_wires_struct_set_chain_into_accumulator_insert() {
        let board = empty_board();
        let catalog = struct_accumulator_catalog();

        let result = reconcile_text_with_catalog(
            &board,
            r#"const rows: Struct[] = []

eventsSimple() {
    let row = structMake()
    row = structSet({ structIn: row, field: "id", value: cuid().cuid })
    row = structSet({ structIn: row, field: "subject", value: "hello" })
    const push = arrayPush({ arrayIn: rows, value: row })
    rows = push.arrayOut
    batchInsertLocalDb({ database: {}, value: rows })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                        && pin_id == "field"
                        && value == &flow_like_types::Value::String("id".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                        && pin_id == "field"
                        && value == &flow_like_types::Value::String("subject".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                        && pin_id == "value"
                        && value == &flow_like_types::Value::String("hello".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("cuid")
                        && from_pin == "cuid"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                        && to_pin == "value"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("struct_set")
                        && from_pin == "struct_out"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("array_push")
                        && to_pin == "value"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("array_push")
                        && from_pin == "array_out"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("variable_set")
                        && to_pin == "value_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("variable_get")
                        && from_pin == "value_ref"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("batch_insert_local_db")
                        && to_pin == "value"
            )
        }));
    }

    #[test]
    fn member_assignment_sugar_reconciles_to_struct_set() {
        let board = empty_board();
        let catalog = struct_accumulator_catalog();

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    let pref = structMake()
    pref.cost_weight = 0.5
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // `pref.cost_weight = 0.5` desugars to a `struct_set` node with field="cost_weight".
        assert!(
            result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                        if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                            && pin_id == "field"
                            && value == &flow_like_types::Value::String("cost_weight".to_string())
                )
            }),
            "expected a struct_set with field=\"cost_weight\"; commands: {:?}",
            result.commands
        );
        // The struct source feeds the struct_set's struct_in (the read-modify-write chain).
        assert!(
            result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, to_node, to_pin, .. }
                        if command_node_type(&result.commands, from_node).as_deref() == Some("struct_make")
                            && command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                            && to_pin == "struct_in"
                )
            }),
            "expected struct_make -> struct_set.struct_in; commands: {:?}",
            result.commands
        );
    }

    fn preferences_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "ai_generative_make_preferences",
                "Make Preferences",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("multimodal", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("preferences", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "struct_set",
                "Set Field",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("struct_in", "Struct", PinType::Input),
                    pin_meta("field", "String", PinType::Input),
                    pin_meta("value", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("struct_out", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "ai_generative_find_model",
                "Find Model",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("preferences", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("model", "Struct", PinType::Output),
                ],
            ),
        ]
    }

    /// Every `ConnectPins` this reconcile emitted, as `(from_node, to_node)` — used to assert no
    /// edge is a self-connection, which `connect_pins` rejects at apply time with "Cannot connect a
    /// node to itself".
    fn self_connections(commands: &[BoardCommand]) -> Vec<(String, String)> {
        commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::ConnectPins {
                    from_node, to_node, ..
                } if from_node == to_node => Some((from_node.clone(), to_node.clone())),
                _ => None,
            })
            .collect()
    }

    fn reroute_catalog() -> Vec<NodeMetadata> {
        let mut catalog = string_format_dynamic_catalog();
        catalog.push(catalog_meta(
            "reroute",
            "Reroute",
            vec![pin_meta("route_in", "Generic", PinType::Input)],
            vec![pin_meta("route_out", "Generic", PinType::Output)],
        ));
        catalog
    }

    /// `seed_function_params` binds every parameter to `ValueSource { node: <the function layer> }`,
    /// so `return <bare param>` used to queue `ConnectPins { from_node: "$0", to_node: "$0" }` with
    /// ZERO diagnostics: `check` reported `valid`, `commit` reported `queued`, and the apply then
    /// hard-errored in `connect_pins` ("Cannot connect a node to itself") and rolled the entire
    /// batch back. The value must instead route through a `reroute` INSIDE the layer, which
    /// `lower::resolve_source` collapses back to the bare parameter reference.
    #[test]
    fn function_return_of_bare_parameter_never_queues_a_layer_self_edge() {
        let board = empty_board();
        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(name: string): (message: string, echoed: string) {
    const formatted = stringFormat({ formatString: "Hello {name}", name: name })
    return formatted.value, name
}
"#,
            &reroute_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            self_connections(&result.commands).is_empty(),
            "returning a bare function parameter must not queue a layer -> layer self-edge; \
             `connect_pins` rejects it and rolls the whole apply batch back: {:?}",
            self_connections(&result.commands)
        );

        // The `echoed` return pin must still be fed, from a real node inside the layer.
        let feed = result
            .commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::ConnectPins {
                    from_node,
                    to_node,
                    to_pin,
                    ..
                } if to_node == "$0" && to_pin == "echoed" => Some(from_node.clone()),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "the `echoed` return pin must be wired: {:#?}",
                    result.commands
                )
            });
        assert_ne!(
            feed, "$0",
            "the pass-through must be a real node, not the layer"
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { node_type, ref_id: Some(ref_id), target_layer, .. }
                    if node_type == "reroute"
                        && ref_id == &feed
                        && target_layer.as_deref() == Some("$0")
            )),
            "the pass-through must be a `reroute` INSIDE the function layer — \
             `control_call_function::find_node_id_by_pin` only searches the layer's own nodes: {:#?}",
            result.commands
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, .. }
                    if from_node == "$0" && from_pin == "name" && to_node == &feed
            )),
            "the pass-through must read the `name` boundary parameter: {:#?}",
            result.commands
        );
    }

    /// The spliced `reroute`'s pins are Generic, so both spliced edges validate against any
    /// contract. The return-type check must therefore happen on the parameter -> return-pin pair
    /// explicitly, or `function f(a: string): (b: int) { return a }` would apply clean and fail at
    /// run time inside `from_value::<T>` at an arbitrary downstream consumer.
    #[test]
    fn function_return_of_mismatched_parameter_is_still_diagnosed() {
        let board = empty_board();
        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(name: string): (message: string, count: int) {
    const formatted = stringFormat({ formatString: "Hello {name}", name: name })
    return formatted.value, name
}
"#,
            &reroute_catalog(),
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("incompatible pin types") && d.contains("`name`")),
            "a parameter returned into an incompatible return pin must be diagnosed: {:?}",
            result.diagnostics
        );
        assert!(self_connections(&result.commands).is_empty());
    }

    #[test]
    fn catalog_aware_reconcile_const_bound_struct_set_avoids_self_connection() {
        let board = empty_board();
        let catalog = preferences_catalog();

        // The struct-field write sugar has already been desugared by the parser into the
        // accumulator form `x = structSet({ structIn: x, … })`. `x` here is a `const`-bound impure
        // single-output node; the reassignment must wire struct_in from the PRIOR source and thread
        // the impure struct_set into the exec chain exactly once — never onto itself.
        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const preferences = aiGenerativeMakePreferences({ multimodal: true })
    preferences = structSet({ structIn: preferences, field: "coding_weight", value: 0.5 })
    const findModel = aiGenerativeFindModel({ preferences: preferences })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            self_connections(&result.commands).is_empty(),
            "reconcile emitted a self-connection: {:?}",
            self_connections(&result.commands)
        );

        // struct_in reads the pre-reassignment source (make_preferences.preferences), not struct_set.
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("ai_generative_make_preferences")
                        && from_pin == "preferences"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                        && to_pin == "struct_in"
            )),
            "expected make_preferences.preferences -> struct_set.struct_in; commands: {:?}",
            result.commands
        );
        // The rebound `preferences` (struct_set.struct_out) feeds the downstream consumer.
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("struct_set")
                        && from_pin == "struct_out"
                        && command_node_type(&result.commands, to_node).as_deref()
                            == Some("ai_generative_find_model")
                        && to_pin == "preferences"
            )),
            "expected struct_set.struct_out -> find_model.preferences; commands: {:?}",
            result.commands
        );
        // The impure struct_set is threaded into the exec chain from make_preferences, once.
        let struct_set_exec_in: Vec<_> = result
            .commands
            .iter()
            .filter(|command| matches!(
                command,
                BoardCommand::ConnectPins { to_node, to_pin, .. }
                    if command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                        && to_pin == "exec_in"
            ))
            .collect();
        assert_eq!(
            struct_set_exec_in.len(),
            1,
            "struct_set exec_in should be wired exactly once; got {struct_set_exec_in:?}"
        );
        assert!(
            matches!(
                struct_set_exec_in[0],
                BoardCommand::ConnectPins { from_node, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("ai_generative_make_preferences")
            ),
            "struct_set exec_in should come from make_preferences; got {struct_set_exec_in:?}"
        );
    }

    #[test]
    fn member_assignment_sugar_on_const_binding_avoids_self_connection() {
        let board = empty_board();
        let catalog = preferences_catalog();

        // Same shape as above but exercising the parser sugar `x.field = value` directly, so the
        // desugaring path is covered end to end.
        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const preferences = aiGenerativeMakePreferences({ multimodal: true })
    preferences.coding_weight = 0.5
    const findModel = aiGenerativeFindModel({ preferences: preferences })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            self_connections(&result.commands).is_empty(),
            "reconcile emitted a self-connection: {:?}",
            self_connections(&result.commands)
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                        && pin_id == "field"
                        && value == &flow_like_types::Value::String("coding_weight".to_string())
            )),
            "expected struct_set field=\"coding_weight\"; commands: {:?}",
            result.commands
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("ai_generative_make_preferences")
                        && command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                        && to_pin == "struct_in"
            )),
            "expected make_preferences -> struct_set.struct_in; commands: {:?}",
            result.commands
        );
    }

    /// Build an `eventsSimple → structSet(seed) → structSet(accumulate) → batchInsertLocalDb`
    /// board. The second `struct_set` reads the seed's `struct_out` (same accumulator) and rebinds
    /// it. With `wired_field == false` its `field` is a literal — the shape lowering sugars to
    /// `row.title = "hello"`; with `wired_field == true` a `cuid` node feeds `field`, so it must
    /// stay the explicit `structSet({…})` form.
    fn board_with_struct_accumulator(wired_field: bool) -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let ev_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut seed = Node::new("struct_set", "Set Field", "", "structs");
        seed.id = "seed".to_string();
        let seed_exec_in = seed
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let seed_exec_out = seed
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        seed.add_input_pin("field", "Field", "", VariableType::String)
            .default_value = Some(br#""created""#.to_vec());
        seed.add_input_pin("value", "Value", "", VariableType::Generic)
            .default_value = Some(b"1".to_vec());
        let seed_struct_out = seed
            .add_output_pin("struct_out", "Struct Out", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(seed.id.clone(), seed);

        let mut acc = Node::new("struct_set", "Set Field", "", "structs");
        acc.id = "acc".to_string();
        let acc_exec_in = acc
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let acc_exec_out = acc
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let acc_struct_in = acc
            .add_input_pin("struct_in", "Struct In", "", VariableType::Struct)
            .id
            .clone();
        let acc_field_id = {
            let field = acc.add_input_pin("field", "Field", "", VariableType::String);
            if !wired_field {
                field.default_value = Some(br#""title""#.to_vec());
            }
            field.id.clone()
        };
        acc.add_input_pin("value", "Value", "", VariableType::Generic)
            .default_value = Some(br#""hello""#.to_vec());
        let acc_struct_out = acc
            .add_output_pin("struct_out", "Struct Out", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(acc.id.clone(), acc);

        let mut sink = Node::new("consume_struct", "Consume Struct", "", "data");
        sink.id = "sink".to_string();
        let sink_exec_in = sink
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let sink_value = sink
            .add_input_pin("value", "Value", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(sink.id.clone(), sink);

        connect(&mut board, "event", &ev_out, "seed", &seed_exec_in);
        connect(&mut board, "seed", &seed_exec_out, "acc", &acc_exec_in);
        connect(&mut board, "seed", &seed_struct_out, "acc", &acc_struct_in);
        connect(&mut board, "acc", &acc_exec_out, "sink", &sink_exec_in);
        connect(&mut board, "acc", &acc_struct_out, "sink", &sink_value);

        if wired_field {
            let mut cuid = Node::new("cuid", "CUID v2", "", "std");
            cuid.id = "cuid".to_string();
            let cuid_out = cuid
                .add_output_pin("cuid", "CUID", "", VariableType::String)
                .id
                .clone();
            board.nodes.insert(cuid.id.clone(), cuid);
            connect(&mut board, "cuid", &cuid_out, "acc", &acc_field_id);
        }

        board
    }

    fn board_with_null_struct_accumulator() -> Board {
        let mut board = board_with_struct_accumulator(false);
        board
            .nodes
            .get_mut("acc")
            .expect("accumulator node")
            .pins
            .values_mut()
            .find(|pin| pin.pin_type == PinType::Input && pin.name == "value")
            .expect("accumulator value pin")
            .default_value = Some(b"null".to_vec());
        board
    }

    #[test]
    fn lower_preserves_explicit_null_struct_set_value() {
        let board = board_with_null_struct_accumulator();
        let text =
            super::super::board_to_flowscript(&board, &flow_like_ast::RenderOptions::default());

        assert!(
            text.contains("row.title = null"),
            "an explicit struct_set.value=null must survive lowering:\n{text}"
        );
        assert!(
            !text.contains("field: \"title\" }).structOut"),
            "lowering must not emit a structSet call with its required value omitted:\n{text}"
        );
    }

    #[test]
    fn explicit_null_struct_set_value_roundtrips_and_satisfies_required_input() {
        let board = board_with_null_struct_accumulator();
        let catalog = required_struct_set_value_catalog();
        let anchored = super::super::board_to_flowscript(
            &board,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );

        let roundtrip = reconcile_text_with_catalog(&board, &anchored, &catalog);
        assert!(
            roundtrip.diagnostics.is_empty(),
            "unchanged null-valued struct_set must remain valid: {:?}\n{anchored}",
            roundtrip.diagnostics
        );
        assert!(
            roundtrip.commands.is_empty(),
            "unchanged null-valued struct_set must be a no-op: {:?}",
            roundtrip.commands
        );

        let unanchored =
            super::super::board_to_flowscript(&board, &flow_like_ast::RenderOptions::default());
        let recreated = reconcile_text_with_catalog(&empty_board(), &unanchored, &catalog);
        assert!(
            recreated.diagnostics.is_empty(),
            "rendered null-valued struct_set must reconcile from scratch: {:?}\n{unanchored}",
            recreated.diagnostics
        );
        assert!(
            recreated.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&recreated.commands, node_id).as_deref()
                        == Some("struct_set")
                        && pin_id == "value"
                        && value == &flow_like_types::Value::Null
            )),
            "reconcile must restore the explicit null value: {:?}",
            recreated.commands
        );
    }

    #[test]
    fn lower_sugars_accumulator_reassignment_and_reconciles_back() {
        let board = board_with_struct_accumulator(false);
        let text =
            super::super::board_to_flowscript(&board, &flow_like_ast::RenderOptions::default());

        // The accumulator reassignment lowers to the dot-form struct-field write; only the seed
        // `struct_set` (which cannot be a `base.field =` write) keeps its explicit call.
        assert!(
            text.contains("row.title = \"hello\""),
            "accumulator reassignment must render as the dot form:\n{text}"
        );
        assert_eq!(
            text.matches("structSet(").count(),
            1,
            "only the seed struct_set stays explicit; the reassignment is sugared:\n{text}"
        );

        // Reconciling the rendered text against an empty board recreates the same struct_set shape:
        // a struct_set with a literal `field`, `struct_in` fed by the seed (base's prior source),
        // `struct_out` rebound into the consumer, and no self-connection.
        let catalog = struct_accumulator_catalog();
        let result = reconcile_text_with_catalog(&empty_board(), &text, &catalog);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            self_connections(&result.commands).is_empty(),
            "reconcile emitted a self-connection: {:?}",
            self_connections(&result.commands)
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                        && pin_id == "field"
                        && value == &flow_like_types::Value::String("title".to_string())
            )),
            "expected struct_set field=\"title\"; commands: {:?}",
            result.commands
        );
        // struct_in of the dot-form struct_set is fed by the seed struct_set (base's prior source).
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("struct_set")
                        && from_pin == "struct_out"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                        && to_pin == "struct_in"
            )),
            "expected struct_set.struct_out -> struct_set.struct_in; commands: {:?}",
            result.commands
        );
        // The rebound accumulator (struct_out) flows into the downstream consumer.
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("struct_set")
                        && from_pin == "struct_out"
                        && command_node_type(&result.commands, to_node).as_deref()
                            == Some("consume_struct")
                        && to_pin == "value"
            )),
            "expected struct_set.struct_out -> consume_struct.value; commands: {:?}",
            result.commands
        );
    }

    #[test]
    fn lower_keeps_wired_field_struct_set_explicit() {
        let board = board_with_struct_accumulator(true);
        let text =
            super::super::board_to_flowscript(&board, &flow_like_ast::RenderOptions::default());

        // A wired (non-literal) `field` cannot be a `row.<field> = value` write, so lowering
        // conservatively keeps BOTH struct_sets in the explicit `structSet({…})` form.
        assert!(
            text.contains("field: cuid()"),
            "a wired-field struct_set must keep its explicit call:\n{text}"
        );
        assert_eq!(
            text.matches("structSet(").count(),
            2,
            "neither the seed nor the wired-field struct_set may sugar to a dot write:\n{text}"
        );
        assert!(
            !text.contains("row.title"),
            "the wired-field update must not render as a dot write:\n{text}"
        );
    }

    #[test]
    fn catalog_aware_reconcile_promotes_loop_local_accumulator_for_db_insert() {
        let board = empty_board();
        let catalog = accumulator_catalog();

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    let rows = []
    for (const item of controlForEach({ array: ["one"] })) {
        const push = arrayPush({ arrayIn: rows, value: item.value })
        rows = push.arrayOut
    }
    batchInsertLocalDb({ database: {}, value: rows })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::CreateVariable {
                    variable_id: Some(variable_id),
                    name,
                    default_value: Some(flow_like_types::Value::Array(values)),
                    value_type,
                    ..
                } if variable_id == "var_rows"
                    && name == "rows"
                    && values.is_empty()
                    && value_type == "Array"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("control_for_each")
                        && from_pin == "done"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("batch_insert_local_db")
                        && to_pin == "exec_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("array_push")
                        && from_pin == "array_out"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("variable_set")
                        && to_pin == "value_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("variable_get")
                        && from_pin == "value_ref"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("batch_insert_local_db")
                        && to_pin == "value"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_resolves_loop_bind_outputs() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array", "Array", "Generic", "Array", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value", "Any", PinType::Output),
                    pin_meta("index", "Integer", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "Any", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    for (const item of controlForEach({ array: ["hello"] })) {
        log({ text: item.value })
    }
}
"#,
            &catalog,
        );

        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.contains("skipped connection")),
            "{:?}",
            result.diagnostics
        );
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "exec_out"
                        && to_node == "$2"
                        && to_pin == "exec_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "value"
                        && to_node == "$2"
                        && to_pin == "text"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_wires_loop_array_body_and_done_outputs() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "mail_imap_inbox",
                "IMAP Inbox",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("connection", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("inbox_struct", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "mail_imap_list",
                "List Mails",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("inbox", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta_friendly(
                        "emails",
                        "Email References",
                        "Struct",
                        "Array",
                        PinType::Output,
                    ),
                ],
            ),
            catalog_meta(
                "control_for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array", "Array", "Generic", "Array", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value", "Struct", PinType::Output),
                    pin_meta("index", "Integer", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "email_imap_inbox_fetch_mail",
                "Fetch Mail",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("email_ref", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("email", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"ingestGmail() {
    const inbox = mailImapInbox({ connection: {} })
    const refs = mailImapList({ inbox: inbox })
    for (const ref of controlForEach({ array: refs.emailReferences })) {
        const mail = emailImapInboxFetchMail({ emailRef: ref.value })
    }
    log({ text: inbox })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, ref_id, .. }
                    if node_type == "events_simple" && ref_id.as_deref() == Some("$0")
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$2"
                        && from_pin == "emails"
                        && to_node == "$3"
                        && to_pin == "array"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$3"
                        && from_pin == "exec_out"
                        && to_node == "$4"
                        && to_pin == "exec_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$3"
                        && from_pin == "done"
                        && to_node == "$5"
                        && to_pin == "exec_in"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_prefers_success_exec_output_over_error() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "http_fetch",
                "API Call",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("request", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta_friendly(
                        "exec_error",
                        "Error",
                        "Execution",
                        "Normal",
                        PinType::Output,
                    ),
                    pin_meta("response", "Struct", PinType::Output),
                    pin_meta_friendly(
                        "exec_success",
                        "Success",
                        "Execution",
                        "Normal",
                        PinType::Output,
                    ),
                ],
            ),
            catalog_meta(
                "http_response_to_text",
                "Response To Text",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("response", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("text", "String", PinType::Output),
                ],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const fetched = httpFetch({ request: {} })
    httpResponseToText({ response: fetched.response })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "exec_success"
                        && to_node == "$2"
                        && to_pin == "exec_in"
            )
        }));
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "exec_error"
                        && to_node == "$2"
                        && to_pin == "exec_in"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_keeps_exec_cursor_across_pure_nodes_and_loop_body() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "http_make_request",
                "Make Request",
                vec![
                    pin_meta("method", "String", PinType::Input),
                    pin_meta("url", "String", PinType::Input),
                ],
                vec![pin_meta("request", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "http_fetch",
                "API Call",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("request", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_success", "Execution", PinType::Output),
                    pin_meta("exec_error", "Execution", PinType::Output),
                    pin_meta("response", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "http_response_to_text",
                "To Text",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("response", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("text", "String", PinType::Output),
                ],
            ),
            catalog_meta(
                "parse_feed",
                "Parse Feed",
                vec![
                    pin_meta("feed_body", "String", PinType::Input),
                    pin_meta("source_url", "String", PinType::Input),
                ],
                vec![
                    pin_meta_friendly("items", "Items", "Struct", "Array", PinType::Output),
                    pin_meta("item_count", "Integer", PinType::Output),
                ],
            ),
            catalog_meta(
                "utils_datetime_now",
                "Now",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("date", "Date", PinType::Output),
                ],
            ),
            catalog_meta(
                "utils_datetime_duration",
                "Add Duration",
                vec![
                    pin_meta("date", "Date", PinType::Input),
                    pin_meta("days", "Integer", PinType::Input),
                ],
                vec![pin_meta("result", "Date", PinType::Output)],
            ),
            catalog_meta(
                "filter_feed_items_by_date",
                "Filter Feed Items by Date",
                vec![
                    pin_meta_friendly("items", "Items", "Struct", "Array", PinType::Input),
                    pin_meta("released_from", "Date", PinType::Input),
                    pin_meta("released_to", "Date", PinType::Input),
                    pin_meta("date_field", "String", PinType::Input),
                    pin_meta("include_undated", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta_friendly(
                        "filtered_items",
                        "Filtered Items",
                        "Struct",
                        "Array",
                        PinType::Output,
                    ),
                    pin_meta("item_count", "Integer", PinType::Output),
                ],
            ),
            catalog_meta(
                "log_info",
                "Print Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "Generic", PinType::Input),
                    pin_meta("toast", "Boolean", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array", "Array", "Generic", "Array", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                    pin_meta("value", "Generic", PinType::Output),
                    pin_meta("index", "Integer", PinType::Output),
                ],
            ),
            catalog_meta(
                "feed_item_to_markdown",
                "Feed Item to Markdown",
                vec![
                    pin_meta("item", "Generic", PinType::Input),
                    pin_meta("include_link", "Boolean", PinType::Input),
                    pin_meta("include_summary", "Boolean", PinType::Input),
                    pin_meta("include_content", "Boolean", PinType::Input),
                ],
                vec![pin_meta("markdown", "String", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const request = httpMakeRequest({ method: "GET", url: "https://blog.rust-lang.org/feed.xml?limit=30" })
    const response = httpFetch({ request: request })
    const feedBody = httpResponseToText({ response: response.response })
    const parsed = parseFeed({ feedBody: feedBody, sourceUrl: "https://blog.rust-lang.org/feed.xml?limit=30" })
    const now = utilsDatetimeNow()
    const from = utilsDatetimeDuration({ date: now, days: -2 })
    const recent = filterFeedItemsByDate({ items: parsed.items, releasedFrom: from, releasedTo: now, dateField: "published", includeUndated: false })
    logInfo({ message: recent.itemCount, toast: false })
    for (const item of controlForEach({ array: recent.filteredItems })) {
        const markdown = feedItemToMarkdown({ item: item.value, includeLink: true, includeSummary: true, includeContent: false })
        logInfo({ message: markdown, toast: false })
    }
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for (from_node, from_pin, to_node, to_pin) in [
            ("$0", "exec_out", "$2", "exec_in"),
            ("$2", "exec_success", "$3", "exec_in"),
            ("$3", "exec_out", "$5", "exec_in"),
            ("$5", "exec_out", "$8", "exec_in"),
            ("$8", "exec_out", "$9", "exec_in"),
            ("$9", "exec_out", "$11", "exec_in"),
        ] {
            assert!(
                result.commands.iter().any(|command| {
                    matches!(
                        command,
                        BoardCommand::ConnectPins {
                            from_node: actual_from_node,
                            from_pin: actual_from_pin,
                            to_node: actual_to_node,
                            to_pin: actual_to_pin,
                            ..
                        } if actual_from_node == from_node
                            && actual_from_pin == from_pin
                            && actual_to_node == to_node
                            && actual_to_pin == to_pin
                    )
                }),
                "missing exec edge {from_node}.{from_pin} -> {to_node}.{to_pin}; commands: {:?}",
                result.commands
            );
        }
    }

    #[test]
    fn catalog_aware_reconcile_wires_explicit_multi_exec_branch_arms() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "custom_split",
                "Custom Split",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_error", "Execution", PinType::Output),
                    pin_meta("exec_success", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log_info",
                "Log Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                Vec::new(),
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    customSplit() {
        exec_success: {
            logInfo({ message: "done" })
        }
        exec_error: {
            logInfo({ message: "failed" })
        }
    }
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for (from_pin, to_node) in [("exec_success", "$2"), ("exec_error", "$3")] {
            assert!(
                result.commands.iter().any(|command| matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, from_pin: actual_pin, to_node: actual_node, to_pin, .. }
                        if from_node == "$1"
                            && actual_pin == from_pin
                            && actual_node == to_node
                            && to_pin == "exec_in"
                )),
                "missing explicit {from_pin} arm wiring: {:?}",
                result.commands
            );
        }
    }

    #[test]
    fn bound_multi_exec_arm_tails_feed_the_following_statement() {
        // A lowered arm block refers back to the preceding bound call with a placeholder:
        //
        //   const split = customSplit()
        //   split { ... }
        //
        // The arm statement must replace the stale `split` cursor with its arm tails without
        // reconnecting `split` to itself. Otherwise the following statement steals the default
        // `exec_success` edge and leaves the success body disconnected.
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "custom_split",
                "Custom Split",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_error", "Execution", PinType::Output),
                    pin_meta("exec_success", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log_info",
                "Log Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const split = customSplit()
    split {
        exec_success: {
            logInfo({ message: "success" })
        }
        exec_error: {
            logInfo({ message: "error" })
        }
    }
    logInfo({ message: "after" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for (from_node, from_pin, to_node) in [
            ("$1", "exec_success", "$2"),
            ("$1", "exec_error", "$3"),
            ("$2", "exec_out", "$4"),
            ("$3", "exec_out", "$4"),
        ] {
            assert!(
                result.commands.iter().any(|command| matches!(
                    command,
                    BoardCommand::ConnectPins {
                        from_node: actual_from_node,
                        from_pin: actual_from_pin,
                        to_node: actual_to_node,
                        to_pin,
                        ..
                    } if actual_from_node == from_node
                        && actual_from_pin == from_pin
                        && actual_to_node == to_node
                        && to_pin == "exec_in"
                )),
                "missing exec edge {from_node}.{from_pin} -> {to_node}.exec_in: {:?}",
                result.commands
            );
        }
        assert!(
            !result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins {
                    from_node,
                    to_node,
                    ..
                } if from_node == "$1" && to_node == "$4"
            )),
            "the following statement must fan in from the arm tails, not bypass them: {:?}",
            result.commands
        );
        assert!(
            !result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins {
                    from_node,
                    to_node,
                    ..
                } if from_node == "$1" && to_node == "$1"
            )),
            "the bound arm block must not reconnect the split node to itself: {:?}",
            result.commands
        );
    }

    #[test]
    fn catalog_aware_reconcile_does_not_guess_unknown_multi_exec_output() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "custom_split",
                "Custom Split",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_error", "Execution", PinType::Output),
                    pin_meta("exec_success", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log_info",
                "Log Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                Vec::new(),
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    customSplit()
    logInfo({ message: "done" })
}
"#,
            &catalog,
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("multiple execution outputs")),
            "{:?}",
            result.diagnostics
        );
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, to_node, .. }
                    if from_node == "$1" && to_node == "$2"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_continues_unknown_streaming_nodes_from_exec_done() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "custom_stream",
                "Custom Stream",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("on_stream", "Execution", PinType::Output),
                    pin_meta("exec_done", "Execution", PinType::Output),
                    pin_meta("result", "String", PinType::Output),
                ],
            ),
            catalog_meta(
                "log_info",
                "Log Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                Vec::new(),
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const streamed = customStream()
    logInfo({ message: streamed.result })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "exec_done"
                        && to_node == "$2"
                        && to_pin == "exec_in"
            )
        }));
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, .. }
                    if from_node == "$1" && from_pin == "on_stream" && to_node == "$2"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_wires_stream_chunk_consumers_from_on_stream_only() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "agent_stream_invoke",
                "Stream Invoke Agent",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("on_stream", "Execution", PinType::Output),
                    pin_meta("chunk", "Struct", PinType::Output),
                    pin_meta("exec_done", "Execution", PinType::Output),
                    pin_meta("response", "Struct", PinType::Output),
                    pin_meta("stats", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "events_chat_push_response_chunk",
                "Push Chunk",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("chunk", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_chat_push_response",
                "Push Response",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("response", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_chat_push_stat",
                "Push Stat",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("stat", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const invokeAgent = agentStreamInvoke()
    eventsChatPushResponseChunk({ chunk: invokeAgent.chunk })
    eventsChatPushResponse({ response: invokeAgent.response })
    eventsChatPushStat({ stat: invokeAgent.stats })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for (from_node, from_pin, to_node, to_pin) in [
            ("$0", "exec_out", "$1", "exec_in"),
            ("$1", "on_stream", "$2", "exec_in"),
            ("$1", "exec_done", "$3", "exec_in"),
            ("$3", "exec_out", "$4", "exec_in"),
        ] {
            assert!(
                result.commands.iter().any(|command| {
                    matches!(
                        command,
                        BoardCommand::ConnectPins {
                            from_node: actual_from_node,
                            from_pin: actual_from_pin,
                            to_node: actual_to_node,
                            to_pin: actual_to_pin,
                            ..
                        } if actual_from_node == from_node
                            && actual_from_pin == from_pin
                            && actual_to_node == to_node
                            && actual_to_pin == to_pin
                    )
                }),
                "missing exec edge {from_node}.{from_pin} -> {to_node}.{to_pin}; commands: {:?}",
                result.commands
            );
        }

        for (from_node, from_pin, to_node) in [
            ("$1", "exec_done", "$2"),
            ("$2", "exec_out", "$3"),
            ("$1", "on_stream", "$3"),
        ] {
            assert!(!result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node: actual_from_node, from_pin: actual_from_pin, to_node: actual_to_node, .. }
                        if actual_from_node == from_node
                            && actual_from_pin == from_pin
                            && actual_to_node == to_node
                )
            }));
        }
    }

    #[test]
    fn catalog_aware_reconcile_reports_missing_variable_refs() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "String", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    variableGet({ varRef: "GMAIL_ADDRESS" })
}
"#,
            &catalog,
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("GMAIL_ADDRESS")
                    && diagnostic.contains("top-level FlowScript variable")),
            "{:?}",
            result.diagnostics
        );
    }

    /// Build `event → makePreferences → structSet(a) → structSet(b)` where the tail `structSet`
    /// (`set_b`) is a single-field accumulator update of the head's `struct_out`, so lowering
    /// re-sugars it to the `record.reasoning_weight = 0.5` dot form (a `Stmt::FieldAssign`).
    fn board_with_struct_accumulator_chain() -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut make = Node::new("make_preferences", "Make Preferences", "", "data");
        make.id = "make".to_string();
        let make_in = make
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let make_exec_out = make
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let make_struct_out = make
            .add_output_pin("preferences", "Preferences", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(make.id.clone(), make);

        let mut set_a = Node::new("struct_set", "Set Field", "", "structs");
        set_a.id = "set_a".to_string();
        let a_in = set_a
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let a_struct_in = set_a
            .add_input_pin("struct_in", "Struct", "", VariableType::Struct)
            .id
            .clone();
        set_a
            .add_input_pin("field", "Field", "", VariableType::String)
            .default_value = Some(b"\"coding_weight\"".to_vec());
        set_a
            .add_input_pin("value", "Value", "", VariableType::Generic)
            .default_value = Some(b"0.3".to_vec());
        let a_exec_out = set_a
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let a_struct_out = set_a
            .add_output_pin("struct_out", "Struct", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(set_a.id.clone(), set_a);

        let mut set_b = Node::new("struct_set", "Set Field", "", "structs");
        set_b.id = "set_b".to_string();
        let b_in = set_b
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let b_struct_in = set_b
            .add_input_pin("struct_in", "Struct", "", VariableType::Struct)
            .id
            .clone();
        set_b
            .add_input_pin("field", "Field", "", VariableType::String)
            .default_value = Some(b"\"reasoning_weight\"".to_vec());
        set_b
            .add_input_pin("value", "Value", "", VariableType::Generic)
            .default_value = Some(b"0.5".to_vec());
        set_b.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        set_b.add_output_pin("struct_out", "Struct", "", VariableType::Struct);
        board.nodes.insert(set_b.id.clone(), set_b);

        connect(&mut board, "event", &event_out, "make", &make_in);
        connect(&mut board, "make", &make_exec_out, "set_a", &a_in);
        connect(&mut board, "make", &make_struct_out, "set_a", &a_struct_in);
        connect(&mut board, "set_a", &a_exec_out, "set_b", &b_in);
        connect(&mut board, "set_a", &a_struct_out, "set_b", &b_struct_in);
        board
    }

    #[test]
    fn field_assign_value_literal_edit_updates_struct_set_value_pin() {
        // Editing the value literal on an existing anchored `record.reasoning_weight = 0.5` dot
        // form (rendered from the tail struct_set accumulator) must update that struct_set's
        // `value` pin in place — the regression this synthesis fixes was a silent no-op here.
        let board = board_with_struct_accumulator_chain();
        let text =
            anchored_text(&board).replace("reasoning_weight = 0.5", "reasoning_weight = 0.7");
        let result = reconcile_text(&board, &text);

        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "set_b" && pin_id == "value" && value.as_f64() == Some(0.7)
            )),
            "expected UpdateNodePin on set_b.value = 0.7; got {:?}",
            result.commands
        );
        // The struct_set already exists; the edit must not duplicate or remove the accumulator node.
        assert!(
            !result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { .. } | BoardCommand::RemoveNode { .. }
            )),
            "field-assign config edit must not add or remove nodes; got {:?}",
            result.commands
        );
    }

    #[test]
    fn deleting_field_assign_line_removes_struct_set_node() {
        // Removing the anchored dot-form line must flag its struct_set for deletion — before the
        // synthesis, `visible` never saw the FieldAssign's node, so it lingered on the board.
        let board = board_with_struct_accumulator_chain();
        let reduced = anchored_text(&board)
            .lines()
            .filter(|line| !line.contains("//@n:set_b"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = reconcile_text(&board, &reduced);

        let removed: Vec<_> = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::RemoveNode { node_id, .. } => Some(node_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            removed,
            vec!["set_b"],
            "deleting the `record.reasoning_weight = …` line must remove only its struct_set; got {:?}",
            result.commands
        );
    }

    #[test]
    fn unchanged_field_assign_roundtrip_emits_nothing() {
        // Reconciling the identical anchored dot-form text against its own board must stay a no-op;
        // this guards against the synthesized struct_set call producing a spurious edit or deletion.
        let board = board_with_struct_accumulator_chain();
        let text = anchored_text(&board);
        let result = reconcile_text(&board, &text);
        assert!(
            result.commands.is_empty(),
            "no-op field-assign round-trip must emit no commands; got {:?} from text:\n{text}",
            result.commands
        );
    }

    fn board_derived_catalog(board: &Board) -> Vec<NodeMetadata> {
        board.nodes.values().map(node_to_metadata).collect()
    }

    /// Board-derived catalogs carry one entry per node INSTANCE, so same-type declarations
    /// legitimately conflict (differently specialized pins). An anchored call validates against
    /// its live node; the conflict must neither fail the call nor invent requirements beyond the
    /// candidates' common ground.
    #[test]
    fn anchored_call_tolerates_conflicting_same_type_declarations() {
        let mut board = empty_board();
        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut sink = Node::new("notify", "Notify", "", "test");
        sink.id = "sink".to_string();
        let sink_exec = sink
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        sink.add_input_pin("message", "Message", "", VariableType::String);
        sink.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(sink.id.clone(), sink);
        connect(&mut board, "event", &event_out, "sink", &sink_exec);

        let mut catalog = board_derived_catalog(&board);
        let mut conflicting = catalog_meta(
            "notify",
            "Notify",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("message", "Struct", PinType::Input),
                pin_meta("channel", "String", PinType::Input),
            ],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        );
        conflicting.required_inputs = vec!["channel".to_string()];
        catalog.push(conflicting);

        let result = reconcile_text_with_catalog(
            &board,
            r#"start() {   //@n:event
    notify({})   //@n:sink
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.is_empty(), "{:?}", result.commands);
    }

    /// An event-level `return <expr>` whose anchored result node is already wired must reuse the
    /// live producer instead of re-resolving the expression through the (possibly conflicting)
    /// catalog and re-emitting the connection.
    #[test]
    fn anchored_event_return_reuses_wired_response_source() {
        let mut board = empty_board();
        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut producer = Node::new("make_value", "Make Value", "", "test");
        producer.id = "producer".to_string();
        let value_out = producer
            .add_output_pin("value", "Value", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(producer.id.clone(), producer);

        let mut ret = Node::new(
            "events_generic_return_result",
            "Return Result",
            "",
            "events",
        );
        ret.id = "ret".to_string();
        ret.set_event_callback(true);
        let ret_exec = ret
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let response = ret
            .add_input_pin("response", "Response", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(ret.id.clone(), ret);
        connect(&mut board, "event", &event_out, "ret", &ret_exec);
        connect(&mut board, "producer", &value_out, "ret", &response);

        let text = anchored_text(&board);
        let result = reconcile_text_with_catalog(&board, &text, &board_derived_catalog(&board));

        assert!(
            result.diagnostics.is_empty(),
            "{:?}\nFlowScript:\n{text}",
            result.diagnostics
        );
        assert!(
            result.commands.is_empty(),
            "event-return roundtrip must be a no-op; got {:?} from:\n{text}",
            result.commands
        );
    }

    /// Repeated same-named exec outputs (repeatable pins like `control_par_execution`'s
    /// `exec_out`) lower to positionally-disambiguated arm labels (`exec_out[#2]`) that reconcile
    /// resolves back to the exact pin — an unchanged roundtrip stays a no-op instead of failing
    /// the duplicate-arm-label structure check.
    #[test]
    fn repeated_exec_output_arm_labels_roundtrip_as_noop() {
        let mut board = empty_board();
        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut par = Node::new("control_par_execution", "Parallel Execution", "", "control");
        par.id = "par".to_string();
        let par_exec_in = par
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let first_out = par
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let second_out = par
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(par.id.clone(), par);
        connect(&mut board, "event", &event_out, "par", &par_exec_in);

        for (index, out_pin) in [first_out, second_out].iter().enumerate() {
            let mut log = Node::new("log", "Log", "", "debug");
            log.id = format!("log-{index}");
            let log_id = log.id.clone();
            let exec_in = log
                .add_input_pin("exec_in", "In", "", VariableType::Execution)
                .id
                .clone();
            log.add_input_pin("message", "Message", "", VariableType::String);
            board.nodes.insert(log.id.clone(), log);
            connect(&mut board, "par", out_pin, &log_id, &exec_in);
        }

        let text = anchored_text(&board);
        assert!(
            text.contains("exec_out[#2]"),
            "second same-named exec arm must carry its positional selector:\n{text}"
        );

        let result = reconcile_text_with_catalog(&board, &text, &board_derived_catalog(&board));
        assert!(
            result.diagnostics.is_empty(),
            "{:?}\nFlowScript:\n{text}",
            result.diagnostics
        );
        assert!(result.commands.is_empty(), "{:?}", result.commands);
    }

    /// A `return` inside a nested handler is the event-return sugar, not a return of the
    /// enclosing function — the function's (empty) return signature must not reject it.
    #[test]
    fn nested_handler_return_is_event_return_not_function_return() {
        let catalog = vec![
            catalog_meta(
                "events_generic",
                "Generic Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_generic_return_result",
                "Return Result",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("response", "Generic", PinType::Input),
                ],
                Vec::new(),
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"function helper() {
    log({ message: "working" })
    fetchPage(url: string) {
        log({ message: url })
        return "ok"
    }
}
"#,
            &catalog,
        );

        assert!(
            !result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("has no matching function return pin")),
            "{:?}",
            result.diagnostics
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { node_type, .. }
                    if node_type == "events_generic_return_result"
            )),
            "the handler return must plan the event-return node: {:?}",
            result.commands
        );
    }

    /// `{}`/`[]` on a pin that is unset on the live board (no default, no edge) is the lowered
    /// representation of "unset"; writing it back is not a configuration edit.
    #[test]
    fn empty_composite_literal_on_unset_anchored_pin_is_a_noop() {
        let mut board = empty_board();
        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut sink = Node::new("struct_sink", "Struct Sink", "", "test");
        sink.id = "sink".to_string();
        let sink_exec = sink
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        sink.add_input_pin("payload", "Payload", "", VariableType::Struct);
        board.nodes.insert(sink.id.clone(), sink);
        connect(&mut board, "event", &event_out, "sink", &sink_exec);

        let result = reconcile_text(
            &board,
            r#"start() {   //@n:event
    structSink({ payload: {} })   //@n:sink
}
"#,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.is_empty(),
            "writing `{{}}` onto an unset pin must not queue an update: {:?}",
            result.commands
        );
    }

    /// Conflicting same-type `variable_get` declarations (one per board instance) still identify
    /// one usable accessor type: minting a new reader picks the deterministic candidate instead
    /// of failing with "unusable".
    #[test]
    fn conflicting_variable_get_declarations_still_mint_a_reader() {
        let mut generic_get = catalog_meta(
            "variable_get",
            "Get Variable",
            vec![pin_meta("var_ref", "String", PinType::Input)],
            vec![pin_meta("value_ref", "Generic", PinType::Output)],
        );
        generic_get.inputs[0].default_value = None;
        let mut specialized_get = catalog_meta(
            "variable_get",
            "Get Variable",
            vec![pin_meta("var_ref", "String", PinType::Input)],
            vec![pin_meta("value_ref", "String", PinType::Output)],
        );
        specialized_get.outputs[0].is_generic = false;

        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            generic_get,
            specialized_get,
        ];

        let result = reconcile_text_with_catalog(
            &empty_board(),
            r#"const greeting: string = "hi"

eventsSimple() {
    log({ message: greeting })
}
"#,
            &catalog,
        );

        assert!(
            !result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("is unusable")),
            "{:?}",
            result.diagnostics
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "variable_get"
            )),
            "{:?}",
            result.commands
        );
    }

    /// A layer that already exceeds the node cap (legacy board) must stay editable — and
    /// re-appliable — as long as the edit does not add net nodes there.
    #[test]
    fn layer_node_limit_ignores_preexisting_overfull_layers_without_net_adds() {
        let mut board = empty_board();
        for index in 0..(MAX_NODES_PER_LAYER + 5) {
            let mut node = Node::new("log", "Log", "", "debug");
            node.id = format!("root-{index}");
            board.nodes.insert(node.id.clone(), node);
        }

        assert!(
            layer_node_limit_violations(&board, &[]).is_none(),
            "a command-free edit must not be rejected for pre-existing population"
        );

        let add = node_limit_add_command("$new", None);
        let diagnostics = layer_node_limit_violations(&board, &[add])
            .expect("growing an overfull layer must still be rejected");
        assert!(diagnostics[0].contains("the root layer"));
    }

    /// A bare variable ref whose wired source is the ASSIGNING `variable_set` sibling (its
    /// `value_ref` passthrough renders as the same bare name) must reuse that edge instead of
    /// minting a duplicate `variable_get`.
    #[test]
    fn variable_ref_reuses_wired_variable_set_passthrough() {
        let mut board = empty_board();
        let mut config = Variable::new("config", VariableType::String, ValueType::Normal);
        config.id = "var-cfg".to_string();
        board.variables.insert(config.id.clone(), config);

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut setter = Node::new("variable_set", "Set Config", "", "variables");
        setter.id = "setter".to_string();
        let set_exec_in = setter
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        setter
            .add_input_pin("var_ref", "Variable", "", VariableType::String)
            .default_value = Some(b"\"var-cfg\"".to_vec());
        setter
            .add_input_pin("value_in", "Value", "", VariableType::String)
            .default_value = Some(b"\"configured\"".to_vec());
        let set_exec_out = setter
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let value_ref_out = setter
            .add_output_pin("value_ref", "Value", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(setter.id.clone(), setter);

        let mut sink = Node::new("log", "Log", "", "debug");
        sink.id = "sink".to_string();
        let sink_exec = sink
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let message = sink
            .add_input_pin("message", "Message", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(sink.id.clone(), sink);

        connect(&mut board, "event", &event_out, "setter", &set_exec_in);
        connect(&mut board, "setter", &set_exec_out, "sink", &sink_exec);
        connect(&mut board, "setter", &value_ref_out, "sink", &message);

        let text = anchored_text(&board);
        let result = reconcile_text_with_catalog(&board, &text, &board_derived_catalog(&board));

        assert!(
            result.diagnostics.is_empty(),
            "{:?}\nFlowScript:\n{text}",
            result.diagnostics
        );
        assert!(
            result.commands.is_empty(),
            "the variable_set passthrough edge must be reused; got {:?} from:\n{text}",
            result.commands
        );
    }

    /// The render→parse text surface normalizes schemas beyond the in-memory interface
    /// projection (e.g. optional `anyOf[enum, null]` folds into an enum containing `null`);
    /// `text_projected_schema` must land in that parse fixed point, so the roundtrip variable
    /// contract comparison sees authored == projected.
    #[test]
    fn text_projected_schema_reaches_the_parse_fixed_point() {
        let schema = r#"{"type":"object","properties":{"kind":{"anyOf":[{"enum":["a","b"]},{"type":"null"}]},"label":{"type":"string"}},"required":["label"]}"#;

        let first = text_projected_schema(schema).expect("schema is interface representable");
        let second = text_projected_schema(&first).expect("projection stays representable");
        assert_eq!(
            flow_like_ast::normalize_schema(&first),
            flow_like_ast::normalize_schema(&second),
            "projection must be idempotent on its own output"
        );
    }
}
