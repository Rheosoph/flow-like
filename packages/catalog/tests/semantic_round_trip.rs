//! The **semantic round trip**: the FlowScript the editor shows for a board must mean that board.
//!
//! `render_contract_catalog.rs` proves the text is stable: it parses, is a fixed point and
//! reconciles to nothing against its own board. Text can pass all of that and still say something
//! else. A statement after an `if/else` that renders inside one arm keeps its anchor and so
//! reconciles to nothing, yet it tells the reader, and every board the text is applied to, that it
//! runs on one path only.
//!
//! So every board here is rebuilt from its own unanchored text, on an empty board, through the
//! product apply path, and the two boards are compared by what they do: per entry point, the
//! execution order with its branches, loops and joins, and every value each step reads. Ids,
//! positions, reroutes, presentational layers and how pure helper nodes are shared do not count.
//!
//! Corpora: the board fixtures under `tests/ast`, every handwritten program that applies, and the
//! control-flow shapes below that real projects got wrong.

mod flowscript_support;

use flow_like::flow::ast::{RenderOptions, apply_flowscript_to_board, board_to_flowscript};
use flow_like::flow::board::{Board, Layer, LayerType};
use flow_like::flow::node::Node;
use flow_like::flow::pin::{Pin, PinType};
use flow_like::flow::variable::VariableType;
use flow_like_storage::object_store::path::Path;
use flow_like_types::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;

const REROUTE: &str = "reroute";

/// Cases whose rebuilt board does something else today. **This list may only shrink**: a listed
/// case that starts passing fails the run until it is removed.
///
/// Every entry below is a document whose UNANCHORED text does not apply (anchored text does, so
/// the render contract passes); none changes meaning silently.
const KNOWN_GAPS: &[&str] = &[
    // Two opened globs export the same member (`datetime::format` / `string::format`,
    // `map::make` / `struct::make`). `derive_uses` sees only the members a document calls, so it
    // cannot know the second glob shadows a bare call; lowering has no catalog to ask.
    "board/bypaw6n2ksuvrw0kcaj14omz",
    "handwritten/t1-collection-triad.flow",
    // `rss::*` is a package namespace the test catalog lacks (harness artefact), plus the above.
    "board/widgets-pages/bypaw6n2ksuvrw0kcaj14omz",
    // Calls of module functions (`loadVariables`) do not resolve unanchored, and `files::exists`
    // renders as a statement although it has two connected execution outputs.
    "board/ttwctnp08u18sg2z6nmcqqak",
    // Pins an `on_update` mints from the board (`sort`/`maxBy` `key`, `array::unique` key,
    // image `write` options) are rendered as arguments a freshly placed node does not have yet.
    "handwritten/t2-rest-retry.flow",
    "handwritten/t2-widget-action.flow",
    "handwritten/t3-onnx-inference.flow",
    "handwritten/t4-freight-tower.flow",
    "handwritten/t4-scraper-runtime.flow",
    "handwritten/t4-warehouse-reliability.flow",
];

// ---------------------------------------------------------------------------------------------
// What a board does.
// ---------------------------------------------------------------------------------------------

/// Catalog default of every input pin, by (node type, pin name). An unset pin and a pin set to
/// its catalog default mean the same thing.
static CATALOG_DEFAULTS: LazyLock<HashMap<(String, String), Value>> = LazyLock::new(|| {
    flowscript_support::CATALOG
        .nodes
        .iter()
        .flat_map(|node| {
            node.pins.values().filter_map(move |pin| {
                let value =
                    flow_like_types::json::from_slice(pin.default_value.as_deref()?).ok()?;
                Some(((node.name.clone(), pin.name.clone()), value))
            })
        })
        .collect()
});

#[derive(Clone, Copy)]
enum Owner<'a> {
    Node(&'a Node),
    Layer(&'a Layer),
}

enum Target<'a> {
    Node(&'a Node),
    /// A Function layer's own execution output: the function returns.
    Exit,
}

fn is_exec(pin: &Pin) -> bool {
    pin.data_type == VariableType::Execution
}

fn declared(name: &str) -> String {
    flow_like_ast::declared_identifier(name)
}

/// A board indexed for walking. Edges are read from both ends because some boards record an edge
/// on one side only.
struct Graph<'a> {
    pins: HashMap<&'a str, (&'a Pin, Owner<'a>)>,
    nodes: HashMap<&'a str, &'a Node>,
    layers: HashMap<&'a str, &'a Layer>,
    variables: HashMap<&'a str, String>,
    incoming: HashMap<&'a str, BTreeSet<&'a str>>,
    outgoing: HashMap<&'a str, BTreeSet<&'a str>>,
}

impl<'a> Graph<'a> {
    fn new(board: &'a Board) -> Self {
        let mut pins = HashMap::new();
        let mut nodes = HashMap::new();
        for node in board
            .nodes
            .values()
            .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        {
            nodes.insert(node.id.as_str(), node);
            for pin in node.pins.values() {
                pins.insert(pin.id.as_str(), (pin, Owner::Node(node)));
            }
        }
        let mut layers = HashMap::new();
        for layer in board.layers.values() {
            layers.insert(layer.id.as_str(), layer);
            for pin in layer.pins.values() {
                pins.insert(pin.id.as_str(), (pin, Owner::Layer(layer)));
            }
        }
        let mut incoming: HashMap<&str, BTreeSet<&str>> = HashMap::new();
        let mut outgoing: HashMap<&str, BTreeSet<&str>> = HashMap::new();
        for (pin, _) in pins.values() {
            for target in &pin.connected_to {
                outgoing.entry(pin.id.as_str()).or_default().insert(target);
                incoming.entry(target.as_str()).or_default().insert(&pin.id);
            }
            for source in &pin.depends_on {
                incoming.entry(pin.id.as_str()).or_default().insert(source);
                outgoing.entry(source.as_str()).or_default().insert(&pin.id);
            }
        }
        let variables = board
            .variables
            .values()
            .chain(
                board
                    .layers
                    .values()
                    .flat_map(|layer| layer.variables.values()),
            )
            .map(|variable| (variable.id.as_str(), declared(&variable.name)))
            .collect();
        Self {
            pins,
            nodes,
            layers,
            variables,
            incoming,
            outgoing,
        }
    }

    /// Where an execution output leads, through reroutes and presentational bridge pins.
    fn exec_targets(&self, pin: &'a Pin) -> Vec<Target<'a>> {
        let mut targets = Vec::new();
        let mut seen = HashSet::new();
        self.collect_exec_targets(pin.id.as_str(), &mut targets, &mut seen);
        targets
    }

    fn collect_exec_targets(
        &self,
        pin_id: &'a str,
        targets: &mut Vec<Target<'a>>,
        seen: &mut HashSet<&'a str>,
    ) {
        for &target_id in self.outgoing.get(pin_id).into_iter().flatten() {
            if !seen.insert(target_id) {
                continue;
            }
            let Some((target, owner)) = self.pins.get(target_id).copied() else {
                continue;
            };
            match owner {
                Owner::Node(node) if node.name == REROUTE => {
                    for output in node
                        .pins
                        .values()
                        .filter(|pin| pin.pin_type == PinType::Output && is_exec(pin))
                    {
                        self.collect_exec_targets(&output.id, targets, seen);
                    }
                }
                Owner::Node(node) => targets.push(Target::Node(node)),
                Owner::Layer(layer) if matches!(layer.r#type, LayerType::Function) => {
                    if target.pin_type == PinType::Output {
                        targets.push(Target::Exit);
                    }
                }
                Owner::Layer(_) => self.collect_exec_targets(target_id, targets, seen),
            }
        }
    }

    /// The pins that produce an input's value, through reroutes and presentational bridges.
    fn sources(&self, pin_id: &'a str, seen: &mut HashSet<&'a str>) -> Vec<(&'a Pin, Owner<'a>)> {
        let mut sources = Vec::new();
        for &source_id in self.incoming.get(pin_id).into_iter().flatten() {
            if !seen.insert(source_id) {
                continue;
            }
            let Some((source, owner)) = self.pins.get(source_id).copied() else {
                continue;
            };
            match owner {
                Owner::Node(node) if node.name == REROUTE => {
                    for input in node
                        .pins
                        .values()
                        .filter(|pin| pin.pin_type == PinType::Input)
                    {
                        sources.extend(self.sources(&input.id, seen));
                    }
                }
                Owner::Layer(layer) if !matches!(layer.r#type, LayerType::Function) => {
                    sources.extend(self.sources(source_id, seen));
                }
                _ => sources.push((source, owner)),
            }
        }
        sources
    }

    /// A literal written into the text form of the board: ids of variables, functions and nodes
    /// become their names, since ids never survive a rebuild.
    fn normalize(&self, value: &Value) -> String {
        match value {
            Value::String(text) => {
                if let Some(name) = self.variables.get(text.as_str()) {
                    return format!("var:{name}");
                }
                if let Some(layer) = self.layers.get(text.as_str()) {
                    return format!("fn:{}", declared(&layer.name));
                }
                if let Some(node) = self.nodes.get(text.as_str()) {
                    return format!("node:{}", node.name);
                }
                format!("{text:?}")
            }
            Value::Array(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| self.normalize(item))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            other => other.to_string(),
        }
    }
}

fn is_impure(node: &Node) -> bool {
    node.pins.values().any(is_exec)
}

fn sorted_pins(node: &Node, pin_type: PinType, exec: bool) -> Vec<&Pin> {
    let mut pins: Vec<&Pin> = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == pin_type && is_exec(pin) == exec)
        .collect();
    pins.sort_by(|a, b| a.name.cmp(&b.name).then(a.index.cmp(&b.index)));
    pins
}

/// One walk over the execution graph from a single entry point.
struct Walk<'g, 'a> {
    graph: &'g Graph<'a>,
    order: HashMap<&'a str, usize>,
    lines: Vec<String>,
}

impl<'g, 'a> Walk<'g, 'a> {
    fn new(graph: &'g Graph<'a>) -> Self {
        Self {
            graph,
            order: HashMap::new(),
            lines: Vec::new(),
        }
    }

    fn visit(&mut self, node: &'a Node, depth: usize) {
        let indent = "  ".repeat(depth);
        if let Some(index) = self.order.get(node.id.as_str()) {
            self.lines.push(format!("{indent}goto #{index}"));
            return;
        }
        let index = self.order.len();
        self.order.insert(node.id.as_str(), index);
        let label = self.label(node);
        self.lines.push(format!("{indent}#{index} {label}"));
        let outputs = sorted_pins(node, PinType::Output, true);
        let single = outputs.len() == 1;
        for output in outputs {
            let mut targets = self.graph.exec_targets(output);
            if targets.is_empty() {
                continue;
            }
            targets.sort_by_key(|target| match target {
                Target::Node(node) => (node.name.clone(), self.label(node)),
                Target::Exit => (String::new(), String::new()),
            });
            let depth = if single {
                depth
            } else {
                self.lines.push(format!("{indent}  {}:", output.name));
                depth + 2
            };
            for target in targets {
                match target {
                    Target::Node(next) => self.visit(next, depth),
                    Target::Exit => self.lines.push(format!("{}exit", "  ".repeat(depth))),
                }
            }
        }
    }

    fn label(&self, node: &'a Node) -> String {
        let mut expanding = HashSet::new();
        self.call(node, &mut expanding)
    }

    fn call(&self, node: &'a Node, expanding: &mut HashSet<&'a str>) -> String {
        let args = sorted_pins(node, PinType::Input, false)
            .into_iter()
            .filter_map(|pin| {
                let value = self.input(node, pin, expanding)?;
                Some(format!("{}={value}", pin.name))
            })
            .collect::<Vec<_>>();
        format!("{}({})", node.name, args.join(", "))
    }

    fn input(
        &self,
        node: &'a Node,
        pin: &'a Pin,
        expanding: &mut HashSet<&'a str>,
    ) -> Option<String> {
        let sources = self.graph.sources(&pin.id, &mut HashSet::new());
        if !sources.is_empty() {
            let mut values = sources
                .into_iter()
                .map(|(source, owner)| self.value(source, owner, expanding))
                .collect::<Vec<_>>();
            values.sort();
            return Some(values.join(" | "));
        }
        let value: Value = flow_like_types::json::from_slice(pin.default_value.as_deref()?).ok()?;
        let default = CATALOG_DEFAULTS.get(&(node.name.clone(), pin.name.clone()));
        if default == Some(&value) || (default.is_none() && value.is_null()) {
            return None;
        }
        Some(self.graph.normalize(&value))
    }

    fn value(&self, pin: &'a Pin, owner: Owner<'a>, expanding: &mut HashSet<&'a str>) -> String {
        match owner {
            Owner::Layer(_) => format!("param:{}", declared(&pin.name)),
            Owner::Node(node) if is_impure(node) => match self.order.get(node.id.as_str()) {
                Some(index) => format!("#{index}.{}", pin.name),
                None => format!("@{}.{}", node.name, pin.name),
            },
            Owner::Node(node) => {
                if !expanding.insert(node.id.as_str()) {
                    return format!("cycle:{}", node.name);
                }
                let call = self.call(node, expanding);
                expanding.remove(node.id.as_str());
                // An empty constructor and the empty literal are the same value.
                match call.as_str() {
                    "make_array()" => "[]".to_string(),
                    "struct_make()" => "{}".to_string(),
                    _ => format!("{call}.{}", pin.name),
                }
            }
        }
    }
}

/// What `board` does, as text: its variables, then one walk per entry point, function and
/// unreachable chain, in a canonical order.
fn semantics(board: &Board) -> String {
    let graph = Graph::new(board);
    let mut sections = Vec::new();
    let mut reached: HashSet<&str> = HashSet::new();

    let mut variables = board
        .variables
        .values()
        .chain(
            board
                .layers
                .values()
                .flat_map(|layer| layer.variables.values()),
        )
        .map(|variable| {
            let default = variable
                .default_value
                .as_deref()
                .and_then(|bytes| flow_like_types::json::from_slice::<Value>(bytes).ok())
                .filter(|value| !value.is_null())
                .map(|value| format!(" = {}", graph.normalize(&value)))
                .unwrap_or_default();
            format!(
                "var {}: {:?}/{:?}{default}",
                declared(&variable.name),
                variable.data_type,
                variable.value_type
            )
        })
        .collect::<Vec<_>>();
    variables.sort();
    sections.push(variables.join("\n"));

    let mut nodes: Vec<&Node> = graph.nodes.values().copied().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    for node in &nodes {
        let has_exec_input = !sorted_pins(node, PinType::Input, true).is_empty();
        if has_exec_input || sorted_pins(node, PinType::Output, true).is_empty() {
            continue;
        }
        let mut walk = Walk::new(&graph);
        walk.lines.push(format!(
            "entry {} \"{}\"",
            node.name,
            declared(&node.friendly_name)
        ));
        walk.visit(node, 1);
        reached.extend(walk.order.keys().copied());
        sections.push(walk.lines.join("\n"));
    }

    let mut layers: Vec<&Layer> = graph
        .layers
        .values()
        .copied()
        .filter(|layer| matches!(layer.r#type, LayerType::Function))
        .collect();
    layers.sort_by(|a, b| a.id.cmp(&b.id));
    for layer in layers {
        let mut walk = Walk::new(&graph);
        let mut boundary: Vec<&Pin> = layer.pins.values().filter(|pin| !is_exec(pin)).collect();
        boundary.sort_by_key(|pin| (pin.pin_type == PinType::Output, pin.index));
        let signature = boundary
            .iter()
            .map(|pin| {
                format!(
                    "{}{}: {:?}/{:?}",
                    if pin.pin_type == PinType::Output {
                        "-> "
                    } else {
                        ""
                    },
                    declared(&pin.name),
                    pin.data_type,
                    pin.value_type
                )
            })
            .collect::<Vec<_>>();
        walk.lines.push(format!(
            "function {}({})",
            declared(&layer.name),
            signature.join(", ")
        ));
        for entry in layer
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Input && is_exec(pin))
        {
            for target in graph.exec_targets(entry) {
                match target {
                    Target::Node(node) => walk.visit(node, 1),
                    Target::Exit => walk.lines.push("  exit".to_string()),
                }
            }
        }
        for pin in boundary
            .iter()
            .filter(|pin| pin.pin_type == PinType::Output)
        {
            let mut values = graph
                .sources(&pin.id, &mut HashSet::new())
                .into_iter()
                .map(|(source, owner)| walk.value(source, owner, &mut HashSet::new()))
                .collect::<Vec<_>>();
            values.sort();
            walk.lines.push(format!(
                "  return {} = {}",
                declared(&pin.name),
                values.join(" | ")
            ));
        }
        reached.extend(walk.order.keys().copied());
        sections.push(walk.lines.join("\n"));
    }

    for node in &nodes {
        if reached.contains(node.id.as_str()) || node.name == REROUTE || !is_impure(node) {
            continue;
        }
        let fed = sorted_pins(node, PinType::Input, true)
            .iter()
            .any(|pin| !graph.sources(&pin.id, &mut HashSet::new()).is_empty());
        if fed {
            continue;
        }
        let mut walk = Walk::new(&graph);
        walk.lines.push("detached".to_string());
        walk.visit(node, 1);
        reached.extend(walk.order.keys().copied());
        sections.push(walk.lines.join("\n"));
    }

    sections[1..].sort();
    sections.join("\n\n")
}

// ---------------------------------------------------------------------------------------------
// The check.
// ---------------------------------------------------------------------------------------------

/// Rebuild `board` from its own unanchored text on an empty board.
async fn rebuild(board: &Board) -> Result<Board, String> {
    let text = board_to_flowscript(
        board,
        &RenderOptions {
            anchors: false,
            ..RenderOptions::default()
        },
    );
    let mut rebuilt = Board::new_detached(Some("semantic-rebuild".into()), Path::default());
    let result = apply_flowscript_to_board(
        &mut rebuilt,
        &text,
        &flowscript_support::CATALOG.nodes,
        flowscript_support::catalog_state().await,
        None,
        false,
    )
    .await
    .map_err(|error| format!("apply failed: {error:#}\n{text}"))?;
    if !result.diagnostics.is_empty() {
        return Err(format!(
            "its text does not apply: {:?}\n{text}",
            result.diagnostics
        ));
    }
    Ok(rebuilt)
}

/// The first line where two semantics differ, with a little context.
fn first_difference(expected: &str, actual: &str) -> String {
    let expected: Vec<&str> = expected.lines().collect();
    let actual: Vec<&str> = actual.lines().collect();
    let line = expected
        .iter()
        .zip(&actual)
        .position(|(left, right)| left != right)
        .unwrap_or(expected.len().min(actual.len()));
    let window =
        |lines: &[&str]| lines[line.saturating_sub(4)..(line + 6).min(lines.len())].join("\n");
    format!(
        "first difference at line {}\n--- board\n{}\n--- rebuilt from its text\n{}",
        line + 1,
        window(&expected),
        window(&actual)
    )
}

async fn check(case: &str, board: &Board) -> Option<(String, String)> {
    let rebuilt = match rebuild(board).await {
        Ok(rebuilt) => rebuilt,
        Err(error) => return Some((case.to_string(), error)),
    };
    let expected = semantics(board);
    let actual = semantics(&rebuilt);
    (expected != actual).then(|| (case.to_string(), first_difference(&expected, &actual)))
}

fn ratchet(findings: Vec<(String, String)>, exercised: &BTreeSet<String>) {
    let failing: BTreeSet<&str> = findings.iter().map(|(case, _)| case.as_str()).collect();
    let unexpected = findings
        .iter()
        .filter(|(case, _)| !KNOWN_GAPS.contains(&case.as_str()))
        .map(|(case, detail)| format!("{case}\n{detail}"))
        .collect::<Vec<_>>();
    let fixed = KNOWN_GAPS
        .iter()
        .filter(|case| exercised.contains(**case) && !failing.contains(**case))
        .collect::<Vec<_>>();
    let mut message = String::new();
    if !unexpected.is_empty() {
        message.push_str(&format!(
            "\n{} board(s) mean something else once rebuilt from their own text:\n\n{}\n",
            unexpected.len(),
            unexpected.join("\n\n")
        ));
    }
    if !fixed.is_empty() {
        message.push_str(&format!(
            "\nKNOWN_GAPS entries now pass, delete them: {fixed:?}\n"
        ));
    }
    assert!(message.is_empty(), "{message}");
}

async fn board_from(case: &str, source: &str) -> Board {
    let mut board = Board::new_detached(Some(case.into()), Path::default());
    let result = apply_flowscript_to_board(
        &mut board,
        source,
        &flowscript_support::CATALOG.nodes,
        flowscript_support::catalog_state().await,
        None,
        false,
    )
    .await
    .unwrap_or_else(|error| panic!("{case}: seed does not apply: {error:#}"));
    assert!(
        result.diagnostics.is_empty(),
        "{case}: {:?}",
        result.diagnostics
    );
    board
}

// ---------------------------------------------------------------------------------------------
// Corpora.
// ---------------------------------------------------------------------------------------------

/// Control-flow shapes whose meaning lives in where execution goes next.
const SHAPES: &[(&str, &str)] = &[
    (
        "shape/after-if-else",
        r#"eventsSimple run() {
    if (1 > 0) {
        log::info({ message: "yes", toast: false })
    } else {
        log::info({ message: "no", toast: false })
    }
    log::info({ message: "after", toast: false })
}
"#,
    ),
    (
        "shape/after-if-without-else",
        r#"eventsSimple run() {
    if (1 > 0) {
        log::info({ message: "yes", toast: false })
    }
    log::info({ message: "after", toast: false })
}
"#,
    ),
    (
        "shape/after-nested-if-else",
        r#"eventsSimple run() {
    if (1 > 0) {
        if (2 > 1) {
            log::info({ message: "inner yes", toast: false })
        } else {
            log::info({ message: "inner no", toast: false })
        }
        log::info({ message: "after inner", toast: false })
    } else {
        log::info({ message: "no", toast: false })
    }
    log::info({ message: "after", toast: false })
}
"#,
    ),
    (
        "shape/after-if-else-with-an-unhandled-error-arm",
        r#"eventsSimple run() {
    if (1 > 0) {
        const response = http::fetch({ request: http::request({ method: "GET", url: "https://example.com" }) })
        response {
            execSuccess: {
                log::info({ message: "fetched", toast: false })
            }
        }
    } else {
        log::info({ message: "skipped", toast: false })
    }
    log::info({ message: "after", toast: false })
}
"#,
    ),
    (
        "shape/after-if-else-with-an-unwired-error-output",
        r#"eventsSimple run() {
    if (1 > 0) {
        http::fetch({ request: http::request({ method: "GET", url: "https://example.com" }) })
        log::info({ message: "fetched", toast: false })
    } else {
        log::info({ message: "skipped", toast: false })
    }
    log::info({ message: "after", toast: false })
}
"#,
    ),
    (
        "shape/after-if-else-in-loop",
        r#"eventsSimple run() {
    for (const row of ["a", "b"]) {
        if (row == "a") {
            log::info({ message: row, toast: false })
        } else {
            log::warn({ message: row, toast: false })
        }
        log::info({ message: "next row", toast: false })
    }
    log::info({ message: "done", toast: false })
}
"#,
    ),
    (
        "shape/let-reset-in-loop",
        r#"eventsSimple run() {
    for (const row of ["a", "b"]) {
        let total = 0
        for (const x of [1, 2]) {
            total = total + x
        }
        log::info({ message: "row", toast: false })
    }
}
"#,
    ),
    (
        "shape/function-after-if-else",
        r#"function check(value: int): (ok: bool) {
    let ok = false
    if (value > 0) {
        ok = true
        log::info({ message: "positive", toast: false })
    } else {
        log::warn({ message: "not positive", toast: false })
    }
    log::info({ message: "checked", toast: false })
    return ok
}

eventsSimple run() {
    const verdict = check({ value: 3 })
    log::info({ message: "verdict", toast: false })
}
"#,
    ),
];

#[tokio::test(flavor = "multi_thread")]
async fn control_flow_shapes_mean_the_same_after_a_rebuild() {
    let mut findings = Vec::new();
    let mut exercised = BTreeSet::new();
    for (case, source) in SHAPES {
        exercised.insert(case.to_string());
        let board = board_from(case, source).await;
        findings.extend(check(case, &board).await);
    }
    ratchet(findings, &exercised);
}

#[tokio::test(flavor = "multi_thread")]
async fn board_fixtures_mean_the_same_after_a_rebuild() {
    let mut fixtures: Vec<PathBuf> = Vec::new();
    flowscript_support::collect_files(
        &flowscript_support::ast_fixture_dir(),
        "board",
        &mut fixtures,
    );
    fixtures.sort();
    assert!(!fixtures.is_empty(), "no .board fixtures under tests/ast");
    let mut findings = Vec::new();
    let mut exercised = BTreeSet::new();
    let root = flowscript_support::ast_fixture_dir();
    for path in &fixtures {
        let relative = path.strip_prefix(&root).unwrap_or(path).with_extension("");
        let case = format!("board/{}", relative.to_string_lossy());
        exercised.insert(case.clone());
        let board = flowscript_support::load_board_fixture(path).await;
        findings.extend(check(&case, &board).await);
    }
    ratchet(findings, &exercised);
}

#[tokio::test(flavor = "multi_thread")]
async fn handwritten_programs_mean_the_same_after_a_rebuild() {
    let mut fixtures: Vec<PathBuf> = Vec::new();
    flowscript_support::collect_files(
        &flowscript_support::handwritten_fixture_dir(),
        "flow",
        &mut fixtures,
    );
    fixtures.sort();
    let state = flowscript_support::catalog_state().await;
    let mut findings = Vec::new();
    let mut exercised = BTreeSet::new();
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
        if !matches!(applied, Ok(result) if result.diagnostics.is_empty()) {
            continue;
        }
        exercised.insert(case.clone());
        findings.extend(check(&case, &board).await);
    }
    assert!(
        exercised.len() >= 10,
        "only {} handwritten program(s) applied; the corpus is not being exercised",
        exercised.len()
    );
    ratchet(findings, &exercised);
}

/// The comparison has to see what it exists for: dropping the reset of a loop's `let` or moving
/// a statement after an `if/else` into one arm must change the semantics.
#[tokio::test(flavor = "multi_thread")]
async fn semantics_tell_a_moved_join_and_a_missing_reset_apart() {
    let joined = board_from(
        "probe/joined",
        r#"eventsSimple run() {
    if (1 > 0) {
        log::info({ message: "yes", toast: false })
    } else {
        log::info({ message: "no", toast: false })
    }
    log::info({ message: "after", toast: false })
}
"#,
    )
    .await;
    let one_arm = board_from(
        "probe/one-arm",
        r#"eventsSimple run() {
    if (1 > 0) {
        log::info({ message: "yes", toast: false })
        log::info({ message: "after", toast: false })
    } else {
        log::info({ message: "no", toast: false })
    }
}
"#,
    )
    .await;
    assert_ne!(semantics(&joined), semantics(&one_arm));

    let (_, reset_source) = SHAPES
        .iter()
        .find(|(case, _)| *case == "shape/let-reset-in-loop")
        .expect("the loop-reset shape");
    let reset = board_from("probe/reset", reset_source).await;
    let mut unreset = reset.clone();
    let reset_ids: Vec<String> = flowscript_support::all_nodes(&unreset)
        .into_iter()
        .filter(|node| {
            node.name == "variable_set"
                && node.pins.values().any(|pin| {
                    pin.name == "value_in"
                        && pin.depends_on.is_empty()
                        && pin.default_value.as_deref() == Some(b"0".as_slice())
                })
        })
        .map(|node| node.id.clone())
        .collect();
    assert_eq!(reset_ids.len(), 1, "the loop's `let` resets its variable");
    unreset.nodes.retain(|id, _| !reset_ids.contains(id));
    for layer in unreset.layers.values_mut() {
        layer.nodes.retain(|id, _| !reset_ids.contains(id));
    }
    assert_ne!(semantics(&reset), semantics(&unreset));
}
