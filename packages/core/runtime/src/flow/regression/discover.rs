use crate::flow::{board::Board, node::Node};

/// A board test as discovery reports it: an event start node whose FlowScript
/// alias starts with `test` plus a word boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardTestCase {
    pub node_id: String,
    pub alias: String,
}

/// Private twin of `toModuleIdent` in `packages/ui/lib/flow-modules.ts`.
///
/// NOT `flow_like_ast::text::to_camel_case`, deliberately: the frontend rule
/// differs in three observable ways — an empty result stays empty (never
/// `"node"`), the first character is lowered with the full Unicode mapping
/// (not `to_ascii_lowercase`), and the digit-leading guard checks ASCII digits
/// only (JS `/^\d/`). Discovery aliases must match what the browser computes,
/// so the twin mirrors the TS function exactly.
fn to_module_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upcoming_upper = false;
    let mut first = true;
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            if first {
                out.extend(ch.to_lowercase());
                first = false;
            } else if upcoming_upper {
                out.extend(ch.to_uppercase());
            } else {
                out.push(ch);
            }
            upcoming_upper = false;
        } else if !first {
            // Any separator (`_`, `-`, `:`, `/`, space) upper-cases the next character.
            upcoming_upper = true;
        }
    }
    // A digit-leading identifier lexes as a number and breaks the whole document.
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("_{out}")
    } else {
        out
    }
}

/// FlowScript camelCase alias of an event node's display name, so
/// "Test Empty Cart" and `testEmptyCart` resolve to the same alias FlowScript
/// lowering renders. Twin of `eventAliasOf` in `packages/ui/lib/board-tests.ts`.
pub fn event_alias_of(node: &Node) -> String {
    let display = node.friendly_name.trim();
    to_module_ident(if display.is_empty() {
        &node.name
    } else {
        display
    })
}

/// Whether an alias names a board test: a case-insensitive `test` prefix plus
/// a word boundary (next character absent, or equal to its own uppercase), so
/// `testimonialFeed` is not a test. Twin of `isTestEventAlias` in
/// `packages/ui/lib/board-tests.ts`.
pub fn is_test_event_alias(alias: &str) -> bool {
    if !alias.to_lowercase().starts_with("test") {
        return false;
    }
    match alias.chars().nth(4) {
        None => true,
        Some(next) => next.to_uppercase().eq(std::iter::once(next)),
    }
}

/// A board test is an event start node whose alias starts with `test`.
/// Twin of `isTestEventNode` in `packages/ui/lib/board-tests.ts`.
pub fn is_test_event_node(node: &Node) -> bool {
    node.start == Some(true) && is_test_event_alias(&event_alias_of(node))
}

/// Discover a board's tests, sorted by alias. Twin of `discoverBoardTests` in
/// `packages/ui/lib/board-tests.ts` — a rule change here must also land there,
/// held together by `packages/core/tests/fixtures/board-test-grading.json`.
/// (The TS side sorts with `localeCompare`; for the camelCase identifier
/// aliases this produces, code-point order is equivalent.)
pub fn discover_board_tests(board: &Board) -> Vec<BoardTestCase> {
    let mut cases: Vec<BoardTestCase> = board
        .nodes
        .values()
        .filter(|node| node.start == Some(true))
        .map(|node| (node, event_alias_of(node)))
        .filter(|(_, alias)| is_test_event_alias(alias))
        .map(|(node, alias)| BoardTestCase {
            node_id: node.id.clone(),
            alias,
        })
        .collect();
    cases.sort_by(|a, b| a.alias.cmp(&b.alias));
    cases
}
