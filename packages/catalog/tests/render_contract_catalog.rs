//! The **render contract over real boards**: what the renderer emits for a board must feed back
//! through parse and reconcile without changing that board.
//!
//! `flow-like-ast`'s `render_contract.rs` pins the language half — that render output parses and
//! is a fixed point. This is the other half, and the one that bites in the product: a document the
//! editor shows is `render(lower(board))`, and Apply feeds it straight back into reconcile. If the
//! renderer drops information the reconciler needs, applying an *untouched* document silently
//! mutates the board.
//!
//! Five invariants, checked against the real catalog:
//!
//! | | invariant | what breaking it looks like in the product |
//! |---|---|---|
//! | **R1** | the rendered text parses | the FlowScript panel cannot open the board |
//! | **R2** | `render(parse(text)) == text` | the document reformats itself on every apply |
//! | **R3** | reconciling it against its own board is a no-op | Apply on an untouched document edits the graph |
//! | **R4** | if R3 fails, applying and repeating converges | **unbounded churn** — every Apply re-plans the same commands forever |
//! | **R5** | no pin reference dangles after an apply | `connected_to` grows without bound and edges never resolve |
//!
//! R4 and R5 exist because R3 failing is survivable if it settles; R3 failing *forever* is not.
//! See [`KNOWN_GAPS`].
//!
//! Corpora, chosen so new work is covered without anyone remembering to add a case:
//!
//! * **catalog surface** — every node the catalog ships, placed on a board. A node added later
//!   with a name, namespace or pin the renderer cannot spell fails here on the day it lands.
//! * **board fixtures** — `tests/ast/**/*.board`, real boards captured from the app.
//! * **handwritten programs** — every `tests/ast/handwritten/*.flow`, applied to an empty board
//!   through the product path and then round-tripped.
//! * **hostile board data** — user-typed names (keywords, punctuation, newlines) on variables,
//!   events and parameters, proving lowering neutralizes them before the renderer sees them.

mod flowscript_support;

use flow_like::flow::ast::MetadataEnricher;
use flow_like::flow::ast::{
    RenderOptions, apply_board_commands_to_board, apply_flowscript_to_board, board_to_flowscript,
    parse, reconcile_text_with_catalog_enriched, render,
};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{BoardCommand, NodeMetadata, node_to_metadata};
use flow_like::flow::node::Node;
use flow_like::flow::variable::{Variable, VariableType};
use flow_like_storage::object_store::path::Path;
use flow_like_types::json::json;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

// ---------------------------------------------------------------------------------------------
// The contract.
// ---------------------------------------------------------------------------------------------

/// The most rounds of apply-and-re-reconcile the contract allows before calling a board
/// non-convergent. One round is the normal "reconcile repaired something" case; a board still
/// producing commands after this many rounds is churning.
const MAX_CONVERGENCE_ROUNDS: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Invariant {
    /// R1: the rendered text does not parse.
    NoParse,
    /// R2: rendering the parsed text again gives different text.
    NotFixedPoint,
    /// R3: reconciling the board's own rendered text plans commands or raises diagnostics.
    NotNoop,
    /// R4: applying those commands and repeating never reaches zero commands.
    NonConvergent,
    /// R5: a pin reference on the board points at a pin that does not exist.
    DanglingPinRef,
    /// R6: the rendered document declares one name twice, so neither can be addressed.
    AmbiguousDeclaration,
}

impl Invariant {
    fn tag(self) -> &'static str {
        match self {
            Invariant::NoParse => "no-parse",
            Invariant::NotFixedPoint => "not-fixed-point",
            Invariant::NotNoop => "not-noop",
            Invariant::NonConvergent => "non-convergent",
            Invariant::DanglingPinRef => "dangling-pin-ref",
            Invariant::AmbiguousDeclaration => "ambiguous-declaration",
        }
    }
}

#[derive(Debug)]
struct Finding {
    case: String,
    invariant: Invariant,
    detail: String,
}

/// Which invariants to check. The catalog-surface corpus places nodes with no wiring, so a
/// reconcile no-op is not meaningful there — only the renderer half is.
#[derive(Clone, Copy)]
struct Scope {
    text_only: bool,
}

const TEXT_ONLY: Scope = Scope { text_only: true };
const FULL: Scope = Scope { text_only: false };

/// Run the contract against `board`, naming findings after `case`.
async fn check_board(
    case: &str,
    board: &Board,
    catalog: &[NodeMetadata],
    enricher: &MetadataEnricher,
    scope: Scope,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let finding = |invariant, detail: String| Finding {
        case: case.to_string(),
        invariant,
        detail,
    };

    // R1 + R2, in both anchor modes. Anchors are what reconcile matches on, so a document that
    // only round-trips without them is not good enough.
    let mut anchored_text = None;
    for anchors in [false, true] {
        let opts = RenderOptions {
            anchors,
            ..RenderOptions::default()
        };
        let text = board_to_flowscript(board, &opts);
        match parse(&text) {
            Ok(reparsed) => {
                let again = render(&reparsed, &opts);
                if again != text {
                    findings.push(finding(
                        Invariant::NotFixedPoint,
                        format!("anchors={anchors}: {}", first_difference(&text, &again)),
                    ));
                }
                // R6, on the anchored form only: the two renders differ solely in anchors, and
                // reporting the same duplicate twice adds nothing.
                if anchors {
                    let duplicates = duplicate_declarations(&reparsed);
                    if !duplicates.is_empty() {
                        findings.push(finding(
                            Invariant::AmbiguousDeclaration,
                            format!("declared more than once: {}", duplicates.join(", ")),
                        ));
                    }
                }
            }
            Err(error) => findings.push(finding(
                Invariant::NoParse,
                format!("anchors={anchors}: {error}\n{}", excerpt(&text, error.line)),
            )),
        }
        if anchors {
            anchored_text = Some(text);
        }
    }

    if scope.text_only {
        return findings;
    }
    let Some(anchored) = anchored_text else {
        return findings;
    };

    // R3: the board's own anchored document must reconcile to nothing.
    let result = reconcile_text_with_catalog_enriched(board, &anchored, catalog, enricher);
    if result.diagnostics.is_empty() && result.commands.is_empty() {
        return findings;
    }
    findings.push(finding(
        Invariant::NotNoop,
        format!(
            "{} command(s), {} diagnostic(s)\n{}",
            result.commands.len(),
            result.diagnostics.len(),
            summarize(&result.commands, &result.diagnostics)
        ),
    ));

    // R4: a repair that settles is survivable; one that repeats forever is not. Apply what
    // reconcile planned and reconcile again until the board stops changing.
    let state = flowscript_support::catalog_state().await;
    let mut converging = board.clone();
    let mut rounds = Vec::new();
    for _ in 0..MAX_CONVERGENCE_ROUNDS {
        let text = board_to_flowscript(
            &converging,
            &RenderOptions {
                anchors: true,
                ..RenderOptions::default()
            },
        );
        let round = reconcile_text_with_catalog_enriched(&converging, &text, catalog, enricher);
        rounds.push(round.commands.len());
        if round.commands.is_empty() {
            break;
        }
        if let Err(error) = apply_board_commands_to_board(
            &mut converging,
            round.commands,
            &flowscript_support::CATALOG.nodes,
            state.clone(),
            None,
        )
        .await
        {
            rounds.push(usize::MAX);
            findings.push(finding(
                Invariant::NonConvergent,
                format!("apply of reconcile's own commands failed: {error:#}"),
            ));
            break;
        }
    }
    if rounds.last().is_some_and(|last| *last > 0) {
        findings.push(finding(
            Invariant::NonConvergent,
            format!(
                "commands per round: {rounds:?} — still planning commands after \
                 {MAX_CONVERGENCE_ROUNDS} rounds, so every Apply re-plans them forever"
            ),
        ));
    }

    // R5: whatever the repair did, it must not leave a pin pointing at nothing.
    let dangling = dangling_pin_refs(&converging);
    if !dangling.is_empty() {
        findings.push(finding(
            Invariant::DanglingPinRef,
            format!(
                "{} pin reference(s) resolve to no pin after applying reconcile's own commands, \
                 e.g.\n  {}",
                dangling.len(),
                dangling
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n  ")
            ),
        ));
    }

    findings
}

/// `depends_on` / `connected_to` entries that name a pin no node on the board owns.
///
/// A connection is stored as a pin id on both endpoints. An id that resolves to nothing is an edge
/// the graph believes in and the engine cannot follow — and, because reconcile keeps re-planning
/// the connection it cannot see, the surviving side accumulates one more dead id per Apply.
fn dangling_pin_refs(board: &Board) -> Vec<String> {
    let nodes = flowscript_support::all_nodes(board);
    // Layer BOUNDARY pins are pins too, and an edge crossing a collapsed layer legitimately ends on
    // one. Scanning only node pins reported every such edge as dangling — 268 false positives on
    // one fixture, which is worse than no check at all: it invented a defect and buried the real
    // one underneath it.
    let known: HashSet<&String> = nodes
        .iter()
        .flat_map(|node| node.pins.keys())
        .chain(board.layers.values().flat_map(|layer| layer.pins.keys()))
        .collect();
    let mut dangling = Vec::new();
    for node in &nodes {
        for pin in node.pins.values() {
            for (kind, referenced) in [
                ("depends_on", &pin.depends_on),
                ("connected_to", &pin.connected_to),
            ] {
                for id in referenced {
                    if !known.contains(id) {
                        dangling.push(format!(
                            "{}({}).{} {kind} -> {id}",
                            node.friendly_name, node.id, pin.name
                        ));
                    }
                }
            }
        }
    }
    dangling.sort();
    dangling
}

/// Names the document declares more than once in the same scope.
///
/// Board names are free text and lowering camelizes them, so two variables called `user id` and
/// `userId`, or two Function layers called `Process Order` and `process_order`, become two
/// declarations spelled identically. Reconcile then cannot tell which declaration means which
/// entity: rather than mis-assigning, it derives nothing, and the document freezes — every Apply
/// silently does nothing. That reads as a clean no-op to R3, which is why it needs its own check.
fn duplicate_declarations(ast: &flow_like_ast::BoardAst) -> Vec<String> {
    let mut duplicates = Vec::new();
    let mut note = |kind: &str, scope: &str, names: Vec<String>, out: &mut Vec<String>| {
        let mut seen = BTreeSet::new();
        for name in names {
            if !seen.insert(name.clone()) {
                out.push(format!("{kind} `{name}` in {scope}"));
            }
        }
    };
    note(
        "variable",
        "the board",
        ast.variables.iter().map(|v| v.name.clone()).collect(),
        &mut duplicates,
    );
    note(
        "interface",
        "the board",
        ast.interfaces.iter().map(|i| i.name.clone()).collect(),
        &mut duplicates,
    );

    fn walk(
        scope: &str,
        functions: &[flow_like_ast::FnDecl],
        events: &[flow_like_ast::EventBlock],
        modules: &[flow_like_ast::ModuleDecl],
        out: &mut Vec<String>,
    ) {
        let mut seen = BTreeSet::new();
        for function in functions {
            if !seen.insert(function.name.clone()) {
                out.push(format!("function `{}` in {scope}", function.name));
            }
            let mut params = BTreeSet::new();
            for param in function.params.iter().chain(&function.returns) {
                if !params.insert(param.name.clone()) {
                    out.push(format!(
                        "parameter `{}` of function `{}`",
                        param.name, function.name
                    ));
                }
            }
        }
        let mut named_events = BTreeSet::new();
        for event in events {
            if let Some(given) = event.event_name.as_ref()
                && !named_events.insert(given.clone())
            {
                out.push(format!("event `{given}` in {scope}"));
            }
            let mut params = BTreeSet::new();
            for param in &event.params {
                if !params.insert(param.name.clone()) {
                    out.push(format!(
                        "parameter `{}` of event `{}`",
                        param.name, event.name
                    ));
                }
            }
        }
        let mut child_modules = BTreeSet::new();
        for module in modules {
            if !child_modules.insert(module.name.clone()) {
                out.push(format!("module `{}` in {scope}", module.name));
            }
            walk(
                &format!("module `{}`", module.name),
                &module.functions,
                &module.events,
                &module.modules,
                out,
            );
        }
    }
    walk(
        "the board",
        &ast.functions,
        &ast.events,
        &ast.modules,
        &mut duplicates,
    );
    duplicates.sort();
    duplicates.dedup();
    duplicates
}

fn summarize(commands: &[BoardCommand], diagnostics: &[String]) -> String {
    let mut out = String::new();
    for diagnostic in diagnostics.iter().take(5) {
        out.push_str(&format!("  diag: {diagnostic}\n"));
    }
    // Commands repeat by kind; a count per kind reads better than 66 near-identical lines.
    let mut kinds: Vec<(&str, usize)> = Vec::new();
    for command in commands {
        let kind = command_kind(command);
        match kinds.iter_mut().find(|(name, _)| *name == kind) {
            Some((_, count)) => *count += 1,
            None => kinds.push((kind, 1)),
        }
    }
    for (kind, count) in kinds {
        out.push_str(&format!("  {count} x {kind}\n"));
    }
    for command in commands.iter().take(3) {
        out.push_str(&format!("  e.g. {command:?}\n"));
    }
    out
}

fn command_kind(command: &BoardCommand) -> &'static str {
    match command {
        BoardCommand::AddNode { .. } => "AddNode",
        BoardCommand::AddPlaceholder { .. } => "AddPlaceholder",
        BoardCommand::RemoveNode { .. } => "RemoveNode",
        BoardCommand::ConnectPins { .. } => "ConnectPins",
        BoardCommand::DisconnectPins { .. } => "DisconnectPins",
        BoardCommand::UpdateNodePin { .. } => "UpdateNodePin",
        BoardCommand::RenameNode { .. } => "RenameNode",
        BoardCommand::MoveToLayer { .. } => "MoveToLayer",
        BoardCommand::CreateVariable { .. } => "CreateVariable",
        BoardCommand::UpdateVariable { .. } => "UpdateVariable",
        BoardCommand::RemoveVariable { .. } => "RemoveVariable",
        _ => "other",
    }
}

fn first_difference(left: &str, right: &str) -> String {
    let left: Vec<&str> = left.lines().collect();
    let right: Vec<&str> = right.lines().collect();
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or("<missing>");
        let b = right.get(index).copied().unwrap_or("<missing>");
        if a != b {
            return format!("line {}:\n    rendered: {a}\n    re-render: {b}", index + 1);
        }
    }
    "no line differs (trailing whitespace?)".to_string()
}

/// The handful of lines around `line`, numbered. A rendered board runs to hundreds of lines and
/// dumping all of it buries the one line that matters.
fn excerpt(text: &str, line: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = line.saturating_sub(4);
    let end = (line + 3).min(lines.len());
    lines[start..end]
        .iter()
        .enumerate()
        .map(|(offset, content)| {
            let number = start + offset + 1;
            let marker = if number == line { ">>" } else { "  " };
            format!("{marker}{number:>5} | {content}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------------------------
// The ratchet.
// ---------------------------------------------------------------------------------------------

/// `case/invariant` pairs that break the contract today.
///
/// **This list may only shrink.** Every test below reports both an unlisted failure and a listed
/// case that started passing, so a fix cannot land while leaving stale suppression behind.
const KNOWN_GAPS: &[(&str, Invariant)] = &[
    // Everything here is a ONE-SHOT repair, not churn: `NonConvergent` is no longer listed for any
    // case, so applying once settles the board. What is left is that the first Apply of an
    // untouched document is not a no-op.
    //
    // `board/ttwctnp…` — two `node `reroute` is missing required inputs: route_in` diagnostics and
    // ZERO commands. The document is fine; reconcile objects to reroute nodes whose `route_in` the
    // board leaves unwired, which is board data, not something the renderer produced.
    ("board/ttwctnp08u18sg2z6nmcqqak", Invariant::NotNoop),
    // `board/bypaw…` — one `UpdateNodePin value_in = Null`, plus two `namespace rss::… is unknown`
    // diagnostics. The `rss` namespace is absent because this test builds the catalog with the
    // default metadata features; that half is an artefact of the harness, not a product defect.
    ("board/bypaw6n2ksuvrw0kcaj14omz", Invariant::NotNoop),
];

fn ratchet(kind: &str, findings: Vec<Finding>, exercised: &BTreeSet<String>) {
    let known: BTreeSet<(&str, Invariant)> = KNOWN_GAPS.iter().copied().collect();
    let hit: BTreeSet<(&str, Invariant)> = findings
        .iter()
        .map(|finding| (finding.case.as_str(), finding.invariant))
        .collect();

    let unexpected: Vec<&Finding> = findings
        .iter()
        .filter(|finding| !known.contains(&(finding.case.as_str(), finding.invariant)))
        .collect();
    // Only cases this run actually exercised can be judged fixed; a gap listed for another corpus
    // is not evidence about this one.
    let fixed: Vec<String> = known
        .iter()
        .filter(|(case, _)| exercised.contains(*case))
        .filter(|key| !hit.contains(key))
        .map(|(case, invariant)| format!("{case} [{}]", invariant.tag()))
        .collect();

    let mut message = String::new();
    if !unexpected.is_empty() {
        message.push_str(&format!(
            "\n{} {kind} render-contract violation(s) not in KNOWN_GAPS:\n\n{}\n",
            unexpected.len(),
            unexpected
                .iter()
                .map(|finding| format!(
                    "{} [{}]\n{}",
                    finding.case,
                    finding.invariant.tag(),
                    finding.detail
                ))
                .collect::<Vec<_>>()
                .join("\n\n")
        ));
    }
    if !fixed.is_empty() {
        message.push_str(&format!(
            "\n{} KNOWN_GAPS entr(ies) now pass — delete them from KNOWN_GAPS so the gap cannot \
             silently reopen:\n  {}\n",
            fixed.len(),
            fixed.join("\n  ")
        ));
    }
    assert!(message.is_empty(), "{message}");
}

// ---------------------------------------------------------------------------------------------
// Corpus: the catalog surface.
// ---------------------------------------------------------------------------------------------

/// A board holding exactly one catalog node, under an event so the document is a whole program.
fn board_with_node(node: &Node) -> Board {
    let mut board = Board::new_detached(Some(format!("probe-{}", node.name)), Path::default());
    let mut entry = Node::new("events_simple", "Contract Probe", "", "events");
    entry.set_start(true);
    entry.add_output_pin("exec_out", "Out", "", VariableType::Execution);

    let mut placed = node.clone();
    placed.coordinates = Some((0.0, 200.0, 0.0));
    // Wire the entry's exec into the node when it takes one, so the node lowers as a statement in
    // the event body rather than as a detached chain. Nodes with no exec input are pure and lower
    // as expressions wherever they are read.
    let entry_exec = entry
        .pins
        .values()
        .find(|pin| pin.data_type == VariableType::Execution)
        .map(|pin| pin.id.clone());
    let node_exec = placed
        .pins
        .values()
        .find(|pin| {
            pin.data_type == VariableType::Execution
                && pin.pin_type == flow_like::flow::pin::PinType::Input
        })
        .map(|pin| pin.id.clone());
    if let (Some(from), Some(to)) = (entry_exec, node_exec) {
        if let Some(pin) = entry.pins.values_mut().find(|pin| pin.id == from) {
            pin.connected_to.insert(to.clone());
        }
        if let Some(pin) = placed.pins.values_mut().find(|pin| pin.id == to) {
            pin.depends_on.insert(from);
        }
    }

    board.nodes.insert(entry.id.clone(), entry);
    board.nodes.insert(placed.id.clone(), placed);
    board
}

/// Every node the catalog ships must render into text that parses and re-renders identically.
///
/// This is the corpus that makes the contract hold for work nobody has written yet: a node added
/// with a namespace, alias or pin name the renderer cannot spell fails here the day it lands,
/// instead of the first time a user places it.
#[tokio::test(flavor = "multi_thread")]
async fn every_catalog_node_renders_into_parseable_text() {
    let (catalog, enricher) = flowscript_support::catalog();
    let mut findings = Vec::new();
    let mut exercised = BTreeSet::new();

    for node in &flowscript_support::CATALOG.nodes {
        let case = format!("catalog/{}", node.name);
        exercised.insert(case.clone());
        let board = board_with_node(node);
        findings.extend(check_board(&case, &board, &catalog, &enricher, TEXT_ONLY).await);
    }

    assert!(
        exercised.len() > 500,
        "expected the full catalog, got {} node(s) — is the metadata feature set right?",
        exercised.len()
    );
    ratchet("catalog-surface", findings, &exercised);
}

// ---------------------------------------------------------------------------------------------
// Corpus: committed board fixtures.
// ---------------------------------------------------------------------------------------------

/// Real boards captured from the app, run through the whole contract including convergence.
///
/// This is the strongest corpus: the boards are large, use functions, layers, loops, streaming
/// handlers and duplicated subgraphs, and they are exactly the shapes that broke before.
#[tokio::test(flavor = "multi_thread")]
async fn board_fixtures_round_trip_without_changing_the_board() {
    let (catalog, enricher) = flowscript_support::catalog();
    let mut fixtures: Vec<PathBuf> = Vec::new();
    flowscript_support::collect_files(
        &flowscript_support::ast_fixture_dir(),
        "board",
        &mut fixtures,
    );
    fixtures.sort();
    assert!(
        !fixtures.is_empty(),
        "no .board fixtures under {:?}",
        flowscript_support::ast_fixture_dir()
    );

    let mut findings = Vec::new();
    let mut exercised = BTreeSet::new();
    for path in &fixtures {
        let case = format!(
            "board/{}",
            path.file_stem().unwrap_or_default().to_string_lossy()
        );
        exercised.insert(case.clone());
        let board = flowscript_support::load_board_fixture(path).await;
        findings.extend(check_board(&case, &board, &catalog, &enricher, FULL).await);
    }

    ratchet("board-fixture", findings, &exercised);
}

// ---------------------------------------------------------------------------------------------
// Corpus: handwritten programs, applied through the product path.
// ---------------------------------------------------------------------------------------------

/// Apply every hand-written fixture to an empty board the way the product does, then hold the
/// resulting board to the contract.
///
/// `handwritten_flowscript.rs` checks the text -> board direction; this is board -> text -> board
/// on the same corpus, so the two together close the loop. `bug-*.flow` fixtures are minimal
/// repros of engine defects and are expected not to apply, so they are skipped rather than listed
/// as gaps.
#[tokio::test(flavor = "multi_thread")]
async fn applied_handwritten_programs_round_trip() {
    let (catalog, enricher) = flowscript_support::catalog();
    let state = flowscript_support::catalog_state().await;
    let mut fixtures: Vec<PathBuf> = Vec::new();
    flowscript_support::collect_files(
        &flowscript_support::handwritten_fixture_dir(),
        "flow",
        &mut fixtures,
    );
    fixtures.sort();

    let mut findings = Vec::new();
    let mut exercised = BTreeSet::new();
    let mut applied_count = 0usize;

    for path in &fixtures {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.starts_with("bug-") || name.ends_with(".wip.flow") {
            continue;
        }
        let case = format!("handwritten/{name}");
        let source = std::fs::read_to_string(path).expect("read fixture");
        let mut board = Board::new_detached(Some(case.clone()), Path::default());
        let applied = apply_flowscript_to_board(
            &mut board,
            &source,
            &flowscript_support::CATALOG.nodes,
            state.clone(),
            None,
            false,
        )
        .await;
        match applied {
            // A fixture that does not apply is `handwritten_flowscript.rs`'s business, not this
            // suite's: there is no board to hold to the render contract.
            Ok(result) if result.diagnostics.is_empty() => {}
            _ => continue,
        }
        applied_count += 1;
        exercised.insert(case.clone());
        findings.extend(check_board(&case, &board, &catalog, &enricher, FULL).await);
    }

    assert!(
        applied_count >= 10,
        "only {applied_count} handwritten fixture(s) applied cleanly — the corpus is not being \
         exercised, check the apply path before trusting a green run"
    );
    ratchet("handwritten", findings, &exercised);
}

// ---------------------------------------------------------------------------------------------
// Corpus: hostile board data.
// ---------------------------------------------------------------------------------------------

/// Names a user can actually type into the app. Board names are free text everywhere — the
/// variable dialog trims and stores, the layer menu lowercases — so every one of these can reach
/// lowering, and lowering is the only thing standing between them and the renderer.
const HOSTILE_NAMES: &[(&str, &str)] = &[
    ("keyword-return", "Return"),
    ("keyword-if", "If"),
    ("keyword-while", "While"),
    ("keyword-const", "Const"),
    ("keyword-function", "Function"),
    ("literal-true", "True"),
    ("literal-false", "False"),
    ("literal-null", "Null"),
    ("space", "my variable"),
    ("dash", "my-variable"),
    ("quote", "my \"quoted\" name"),
    ("line-comment", "rate // per minute"),
    ("anchor-marker", "note //@n:deadbeef"),
    ("newline", "first\nsecond"),
    ("emoji", "revenue \u{1F4B0}"),
    ("leading-digit", "2fa secret"),
    ("punctuation", "a::b.c[0]"),
    ("empty", ""),
];

/// Connect `from_node.from_pin` to `to_node.to_pin`, by pin NAME, writing both endpoints.
fn connect(board: &mut Board, from_node: &str, from_pin: &str, to_node: &str, to_pin: &str) {
    let from_id = board
        .nodes
        .get(from_node)
        .and_then(|node| node.pins.values().find(|pin| pin.name == from_pin))
        .map(|pin| pin.id.clone())
        .unwrap_or_else(|| panic!("{from_node} has no pin `{from_pin}`"));
    let to_id = board
        .nodes
        .get(to_node)
        .and_then(|node| node.pins.values().find(|pin| pin.name == to_pin))
        .map(|pin| pin.id.clone())
        .unwrap_or_else(|| panic!("{to_node} has no pin `{to_pin}`"));
    if let Some(node) = board.nodes.get_mut(from_node)
        && let Some(pin) = node.pins.get_mut(&from_id)
    {
        pin.connected_to.insert(to_id.clone());
    }
    if let Some(node) = board.nodes.get_mut(to_node)
        && let Some(pin) = node.pins.get_mut(&to_id)
    {
        pin.depends_on.insert(from_id);
    }
}

/// Set a pin's stored default by name.
fn set_pin(board: &mut Board, node_id: &str, pin_name: &str, value: flow_like_types::Value) {
    let Some(node) = board.nodes.get_mut(node_id) else {
        return;
    };
    let pin_id = node
        .pins
        .values()
        .find(|pin| pin.name == pin_name)
        .map(|pin| pin.id.clone())
        .unwrap_or_else(|| panic!("node has no pin `{pin_name}`"));
    if let Some(pin) = node.pins.get_mut(&pin_id) {
        pin.default_value = Some(flow_like_types::json::to_vec(&value).unwrap());
    }
}

fn catalog_node(node_type: &str) -> Node {
    flowscript_support::CATALOG
        .nodes
        .iter()
        .find(|node| node.name == node_type)
        .unwrap_or_else(|| panic!("catalog has no `{node_type}`"))
        .clone()
}

/// A board whose variable is named `name` and is both WRITTEN and READ.
///
/// The declaration alone is not enough to exercise the contract: `const <name>` parses for almost
/// any spelling, while the assignment (`<name> = …`, from `variable_set`) and the read (a bare
/// `Expr::Ref`, from `variable_get`) are the forms a bad name actually breaks. An earlier version
/// of this corpus only declared the variable and so reported green while both of those were broken.
fn board_with_hostile_variable(name: &str) -> Board {
    let mut board = Board::new_detached(Some("hostile-variable".to_string()), Path::default());

    let mut variable = Variable::new(
        name,
        VariableType::String,
        flow_like::flow::pin::ValueType::Normal,
    );
    variable.default_value = Some(flow_like_types::json::to_vec(&json!("value")).unwrap());
    let variable_id = variable.id.clone();
    board.variables.insert(variable_id.clone(), variable);

    let mut entry = Node::new("events_simple", "Hostile Probe", "", "events");
    entry.set_start(true);
    entry.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    let entry_id = entry.id.clone();
    board.nodes.insert(entry_id.clone(), entry);

    let set = catalog_node("variable_set");
    let get = catalog_node("variable_get");
    let log = catalog_node("log_info");
    let (set_id, get_id, log_id) = (set.id.clone(), get.id.clone(), log.id.clone());
    for node in [set, get, log] {
        board.nodes.insert(node.id.clone(), node);
    }

    set_pin(&mut board, &set_id, "var_ref", json!(variable_id.clone()));
    set_pin(&mut board, &set_id, "value_in", json!("written"));
    set_pin(&mut board, &get_id, "var_ref", json!(variable_id));

    connect(&mut board, &entry_id, "exec_out", &set_id, "exec_in");
    connect(&mut board, &set_id, "exec_out", &log_id, "exec_in");
    connect(&mut board, &get_id, "value_ref", &log_id, "message");
    board
}

/// A user typing a keyword, a newline or an anchor marker into a variable name must not produce a
/// document the engine cannot read back.
///
/// Lowering camelizes names, which strips punctuation — this test is what keeps that true, and
/// what will catch it if a future change routes a name to the renderer raw.
#[tokio::test(flavor = "multi_thread")]
async fn hostile_board_names_still_round_trip() {
    let (catalog, enricher) = flowscript_support::catalog();
    let mut findings = Vec::new();
    let mut exercised = BTreeSet::new();

    for (id, name) in HOSTILE_NAMES {
        let case = format!("hostile-variable/{id}");
        exercised.insert(case.clone());
        let board = board_with_hostile_variable(name);
        // FULL, not TEXT_ONLY: a name that camelizes onto `true`/`false`/`null` still parses and is
        // still a fixed point — it just stops being a variable read. Only reconciling the document
        // against its own board notices that the wire is gone.
        findings.extend(check_board(&case, &board, &catalog, &enricher, FULL).await);
    }

    ratchet("hostile-name", findings, &exercised);
}

// ---------------------------------------------------------------------------------------------
// Corpus: hostile JSON-schema names.
// ---------------------------------------------------------------------------------------------

/// A struct variable's schema names reach the document as `interface` declarations and as the type
/// of the variable, neither of which has a quoted form in the grammar. The names come from the
/// schema's `title`, its `$defs` keys, and its property names — none of which the product
/// constrains, since a schema can be pasted in or inferred from a sample payload.
fn board_with_hostile_schema(title: &str, def_name: &str, property: &str) -> Board {
    let mut board = Board::new_detached(Some("hostile-schema".to_string()), Path::default());
    let mut entry = Node::new("events_simple", "Schema Probe", "", "events");
    entry.set_start(true);
    entry.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    board.nodes.insert(entry.id.clone(), entry);

    let schema = json!({
        "type": "object",
        "title": title,
        "properties": {
            property: { "type": "string" },
            "nested": { "$ref": format!("#/$defs/{def_name}") },
        },
        "required": [property],
        "$defs": {
            def_name: {
                "type": "object",
                "properties": { "inner": { "type": "string" } },
            },
        },
    });
    let mut variable = Variable::new(
        "payload",
        VariableType::Struct,
        flow_like::flow::pin::ValueType::Normal,
    );
    variable.schema = Some(flow_like_types::json::to_string(&schema).unwrap());
    board.variables.insert(variable.id.clone(), variable);
    board
}

/// A schema whose title, `$defs` key or property name is not an identifier must still render a
/// document the engine can read back.
///
/// The `interface` and type-annotation positions have no quoted form, so nothing downstream can
/// rescue a bad name here — `pascal_case` at the derivation site is the only thing standing between
/// a pasted schema and an unparseable document. This corpus is what keeps that true.
#[tokio::test(flavor = "multi_thread")]
async fn hostile_schema_names_still_round_trip() {
    let (catalog, enricher) = flowscript_support::catalog();
    let mut findings = Vec::new();
    let mut exercised = BTreeSet::new();

    for (id, name) in HOSTILE_NAMES {
        // `empty` is excluded from the `$defs` slot: a `#/$defs/` reference to an empty key is not
        // a schema the product can produce, and JSON cannot express the intent unambiguously.
        let def_name = if name.is_empty() { "Inner" } else { name };
        let case = format!("hostile-schema/{id}");
        exercised.insert(case.clone());
        let board = board_with_hostile_schema(name, def_name, name);
        findings.extend(check_board(&case, &board, &catalog, &enricher, TEXT_ONLY).await);
    }

    ratchet("hostile-schema", findings, &exercised);
}

// ---------------------------------------------------------------------------------------------
// Corpus: names that collide once lowering camelizes them.
// ---------------------------------------------------------------------------------------------

/// Pairs of names a user would read as clearly different but that camelize to one identifier.
const COLLIDING_NAMES: &[(&str, &str, &str)] = &[
    ("space-vs-camel", "user id", "userId"),
    ("snake-vs-camel", "order_total", "orderTotal"),
    ("dash-vs-camel", "api-key", "apiKey"),
    ("case-only", "Retry Count", "retry count"),
    ("punctuation", "rate/limit", "rateLimit"),
];

fn board_with_colliding_variables(first: &str, second: &str) -> Board {
    let mut board = Board::new_detached(Some("collision".to_string()), Path::default());
    let mut entry = Node::new("events_simple", "Collision Probe", "", "events");
    entry.set_start(true);
    entry.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    board.nodes.insert(entry.id.clone(), entry);
    for name in [first, second] {
        let mut variable = Variable::new(
            name,
            VariableType::String,
            flow_like::flow::pin::ValueType::Normal,
        );
        variable.default_value = Some(flow_like_types::json::to_vec(&json!("value")).unwrap());
        board.variables.insert(variable.id.clone(), variable);
    }
    board
}

/// Two board entities whose names camelize alike must not render as one ambiguous document.
///
/// This is the failure mode a no-op check cannot see: reconcile, unable to tell the two
/// declarations apart, derives nothing at all. Apply reports success, the graph does not change,
/// and there is no diagnostic — the board is simply frozen. R6 catches it by looking at the
/// document instead of at the command list.
#[tokio::test(flavor = "multi_thread")]
async fn camelcase_name_collisions_do_not_render_an_ambiguous_document() {
    let (catalog, enricher) = flowscript_support::catalog();
    let mut findings = Vec::new();
    let mut exercised = BTreeSet::new();

    for (id, first, second) in COLLIDING_NAMES {
        let case = format!("collision-variable/{id}");
        exercised.insert(case.clone());
        let board = board_with_colliding_variables(first, second);
        findings.extend(check_board(&case, &board, &catalog, &enricher, TEXT_ONLY).await);
    }

    ratchet("name-collision", findings, &exercised);
}

// ---------------------------------------------------------------------------------------------
// Guard rails on the harness itself.
// ---------------------------------------------------------------------------------------------

/// A suite that silently stops exercising its corpus is worse than no suite. The catalog must be
/// non-trivial and its nodes must carry the FlowScript names the renderer spells them by.
#[test]
fn catalog_harness_is_wired_up() {
    let nodes = &flowscript_support::CATALOG.nodes;
    assert!(
        nodes.len() > 500,
        "catalog has only {} node(s); the metadata features are not enabled",
        nodes.len()
    );
    let metadata: Vec<NodeMetadata> = nodes.iter().map(node_to_metadata).collect();
    let unnamed = metadata
        .iter()
        .filter(|meta| meta.namespace.as_deref().unwrap_or_default().is_empty())
        .count();
    assert_eq!(
        unnamed, 0,
        "{unnamed} catalog node(s) have no FlowScript namespace, so the renderer has nothing to \
         spell them by"
    );
}
