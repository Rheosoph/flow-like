//! The **render contract**: text the renderer emits must be text the parser accepts, and it must
//! mean the same thing on the way back.
//!
//! Every FlowScript document the product shows a user is `render(lower(board))`, and every apply
//! feeds that same text back through `parse` into reconcile. So the renderer is not free to emit
//! whatever it likes: any output that fails to parse, that re-renders differently, or that
//! re-parses into a *different* AST shape is a product defect — the editor shows a document the
//! engine cannot read back.
//!
//! Three invariants are checked here, over a matrix of every position where a caller-controlled
//! string reaches the output:
//!
//! * **PARSES** — `parse(render(ast))` succeeds.
//! * **FIXED POINT** — `render(parse(render(ast))) == render(ast)`; the canonical form is stable,
//!   so an apply of untouched text is a textual no-op.
//! * **MEANING** — the re-parsed AST has the same *shape* as the rendered one. A name that renders
//!   as a bare `true` and re-parses as `Literal(Bool)` instead of `Ref` is worse than a parse
//!   error: it silently drops a wire.
//!
//! This file is the language half only (no board, no catalog) so it runs in milliseconds.
//! `render_contract_catalog.rs` in `flow-like-catalog` runs the same contract over real boards.
//!
//! ## Known gaps
//!
//! [`KNOWN_GAPS`] lists the `position/input` pairs that violate the contract today. It is a
//! *ratchet*, not a suppression list: an entry that starts passing fails the run just as loudly as
//! a new failure, so closing a gap forces its removal and the list can only shrink.

use flow_like_ast::model::*;
use flow_like_ast::naming::KEYWORDS;
use flow_like_ast::{RenderOptions, is_valid_identifier, parse, render};
use std::collections::BTreeSet;

// ---------------------------------------------------------------------------------------------
// Corpus: strings that reach an identifier position in the output.
// ---------------------------------------------------------------------------------------------

/// How a hostile string can reach the renderer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reach {
    /// Lowering camelizes this position through `to_camel_case`, which strips every character the
    /// lexer would choke on. Only a keyword collision survives — so these ARE reachable from a
    /// board a user can build today by naming a variable `Return` or a function `If`.
    Board,
    /// `to_camel_case` neutralizes this input before it can reach a camelized position. It can
    /// still arrive verbatim in the positions lowering does NOT camelize (struct field paths,
    /// object keys, interface fields), and it is defence in depth everywhere else.
    Synthetic,
}

/// One caller-controlled string, and whether a real board can produce it.
struct Input {
    id: &'static str,
    value: &'static str,
    reach: Reach,
    /// Positions this string is meaningful in. Empty means every position; a value that only
    /// exists in one grammar slot (a pin-occurrence selector, say) names that slot instead of
    /// being multiplied across the whole matrix into cases the product cannot produce.
    only: &'static [&'static str],
}

const fn board(id: &'static str, value: &'static str) -> Input {
    Input {
        id,
        value,
        reach: Reach::Board,
        only: &[],
    }
}

const fn synthetic(id: &'static str, value: &'static str) -> Input {
    Input {
        id,
        value,
        reach: Reach::Synthetic,
        only: &[],
    }
}

const fn scoped(id: &'static str, value: &'static str, only: &'static [&'static str]) -> Input {
    Input {
        id,
        value,
        reach: Reach::Synthetic,
        only,
    }
}

/// Keyword collisions are listed one by one rather than generated from [`KEYWORDS`] so that adding
/// a keyword to the language shows up here as a deliberate decision instead of silently widening
/// the matrix. `keywords_are_all_covered` keeps the two in sync.
const INPUTS: &[Input] = &[
    board("kw-function", "function"),
    board("kw-const", "const"),
    board("kw-let", "let"),
    board("kw-for", "for"),
    board("kw-of", "of"),
    board("kw-if", "if"),
    board("kw-else", "else"),
    board("kw-while", "while"),
    board("kw-return", "return"),
    board("kw-true", "true"),
    board("kw-false", "false"),
    board("kw-null", "null"),
    board("kw-interface", "interface"),
    board("kw-use", "use"),
    board("kw-as", "as"),
    // `module` and `detached` open a block but are not in KEYWORDS; they are still worth pinning.
    board("kw-module", "module"),
    board("kw-detached", "detached"),
    synthetic("space", "my name"),
    synthetic("dash", "my-name"),
    synthetic("dquote", "my\"name"),
    synthetic("backslash", "my\\name"),
    synthetic("backtick", "my`name"),
    synthetic("line-comment", "a//b"),
    synthetic("block-comment", "a/*b*/c"),
    synthetic("anchor-marker", "a//@n:zzz"),
    synthetic("newline", "a\nb"),
    synthetic("crlf", "a\r\nb"),
    synthetic("tab", "a\tb"),
    synthetic("emoji", "n\u{1F600}me"),
    board("non-ascii-letter", "n\u{e4}me"),
    synthetic("leading-digit", "2fa"),
    synthetic("all-digits", "12345"),
    synthetic("empty", ""),
    synthetic("whitespace-only", "   "),
    synthetic("dollar", "$name"),
    synthetic("path-sep", "a::b"),
    synthetic("rbrace", "a}b"),
    synthetic("lbrace", "a{b"),
    synthetic("rparen", "a)b"),
    synthetic("bracket", "a[0]b"),
    synthetic("dot", "a.b"),
    synthetic("double-dot", "a..b"),
    synthetic("semicolon", "a;b"),
    synthetic("colon", "a:b"),
    synthetic("comma", "a,b"),
    synthetic("dollar-brace", "a${b}c"),
    // A struct key keeps its case (`Expr::Member` writes it raw) while an output-pin selection is
    // camelized (`Expr::Field`). The two share one surface form, so a capitalized key is the case
    // where rendering twice can disagree with itself.
    synthetic("uppercase", "DisplayName"),
    synthetic("all-caps", "ID"),
    synthetic("underscore-lead", "_internal"),
    // Repeated same-named exec outputs are addressed by occurrence (`exec_out[#2]`). The selector
    // has to survive rendering or the second arm of an N-way fan-out cannot be resolved — and
    // `control_par_execution` ships three exec outputs, so no unusual board is needed.
    scoped("pin-occurrence", "exec_out[#2]", &["branch-arm-label"]),
];

// ---------------------------------------------------------------------------------------------
// Positions: every place a caller-controlled string is written into the output.
// ---------------------------------------------------------------------------------------------

/// One place a string reaches the rendered document, as a constructor from that string to a
/// complete document containing it.
struct Position {
    id: &'static str,
    /// Whether lowering camelizes this position. A non-camelized position can receive ANY of
    /// [`INPUTS`] from a real board, so `Reach::Synthetic` inputs gate it too.
    camelized: bool,
    build: fn(&str) -> BoardAst,
}

fn doc(events: Vec<EventBlock>) -> BoardAst {
    BoardAst {
        events,
        ..Default::default()
    }
}

fn event(name: &str, params: Vec<Param>, stmts: Vec<Stmt>) -> EventBlock {
    EventBlock {
        name: name.to_string(),
        node_type: "events_simple".to_string(),
        event_name: None,
        params,
        body: Block { stmts },
        anchor: Some("entry-node".to_string()),
    }
}

/// The stock event every single-statement position hangs off, so the document is always a complete
/// program rather than a fragment.
fn host(stmts: Vec<Stmt>) -> BoardAst {
    doc(vec![event("onTick", Vec::new(), stmts)])
}

fn string_ty() -> TypeRef {
    TypeRef::new("string", Container::Normal)
}

fn var_decl(name: &str) -> VarDecl {
    VarDecl {
        name: name.to_string(),
        ty: string_ty(),
        default: None,
        exposed: false,
        secret: false,
        editable: true,
        runtime_configured: false,
        category: None,
        description: None,
        schema: None,
        anchor: Some("var-id".to_string()),
    }
}

fn call(display: &str) -> Call {
    Call {
        node_type: "log".to_string(),
        display: display.to_string(),
        path: Vec::new(),
        receiver: None,
        positional: Vec::new(),
        args: vec![Arg {
            name: "message".to_string(),
            value: Expr::Literal(Literal::String("hi".to_string())),
        }],
        anchor: Some("call-node".to_string()),
    }
}

/// A statement that binds `value` so the expression positions below are all in consuming position.
fn bind(value: Expr) -> Stmt {
    Stmt::LocalAlias {
        name: "bound".to_string(),
        value,
        anchor: Some("alias-node".to_string()),
    }
}

const POSITIONS: &[Position] = &[
    Position {
        id: "event-header-name",
        camelized: true,
        build: |s| doc(vec![event(s, Vec::new(), Vec::new())]),
    },
    Position {
        id: "event-given-name",
        camelized: true,
        build: |s| {
            let mut block = event("eventsSimple", Vec::new(), Vec::new());
            block.event_name = Some(s.to_string());
            doc(vec![block])
        },
    },
    Position {
        id: "event-param-name",
        camelized: true,
        build: |s| {
            let params = vec![Param {
                name: s.to_string(),
                ty: string_ty(),
            }];
            doc(vec![event("onTick", params, Vec::new())])
        },
    },
    Position {
        id: "variable-name",
        camelized: true,
        build: |s| BoardAst {
            variables: vec![var_decl(s)],
            ..host(Vec::new())
        },
    },
    Position {
        id: "variable-read",
        camelized: true,
        build: |s| BoardAst {
            variables: vec![var_decl(s)],
            ..host(vec![bind(Expr::Ref(s.to_string()))])
        },
    },
    Position {
        id: "variable-assign-target",
        camelized: true,
        build: |s| BoardAst {
            variables: vec![var_decl(s)],
            ..host(vec![Stmt::Assign {
                target: s.to_string(),
                value: Expr::Literal(Literal::String("v".to_string())),
                anchor: Some("set-node".to_string()),
            }])
        },
    },
    Position {
        id: "let-binding-name",
        camelized: true,
        build: |s| {
            host(vec![Stmt::Let {
                name: s.to_string(),
                call: call("log"),
                anchor: Some("call-node".to_string()),
            }])
        },
    },
    Position {
        id: "local-alias-name",
        camelized: true,
        build: |s| {
            host(vec![Stmt::LocalAlias {
                name: s.to_string(),
                value: Expr::Literal(Literal::Int(1)),
                anchor: None,
            }])
        },
    },
    Position {
        id: "local-var-name",
        camelized: true,
        build: |s| host(vec![Stmt::Local(var_decl(s))]),
    },
    Position {
        id: "destructure-binding-name",
        camelized: true,
        build: |s| {
            host(vec![Stmt::Destructure {
                fields: vec![DestructureField {
                    pin: "result".to_string(),
                    name: s.to_string(),
                }],
                call: call("log"),
                anchor: Some("call-node".to_string()),
            }])
        },
    },
    Position {
        id: "destructure-pin-name",
        camelized: true,
        build: |s| {
            host(vec![Stmt::Destructure {
                fields: vec![DestructureField {
                    pin: s.to_string(),
                    name: "bound".to_string(),
                }],
                call: call("log"),
                anchor: Some("call-node".to_string()),
            }])
        },
    },
    Position {
        id: "function-name",
        camelized: true,
        build: |s| BoardAst {
            functions: vec![FnDecl {
                name: s.to_string(),
                params: Vec::new(),
                returns: Vec::new(),
                body: Block::default(),
                cache: None,
                anchor: Some("fn-layer".to_string()),
            }],
            ..host(Vec::new())
        },
    },
    Position {
        id: "function-param-name",
        camelized: true,
        build: |s| BoardAst {
            functions: vec![FnDecl {
                name: "helper".to_string(),
                params: vec![Param {
                    name: s.to_string(),
                    ty: string_ty(),
                }],
                returns: Vec::new(),
                body: Block::default(),
                cache: None,
                anchor: Some("fn-layer".to_string()),
            }],
            ..host(Vec::new())
        },
    },
    Position {
        id: "function-return-name",
        camelized: true,
        build: |s| BoardAst {
            functions: vec![FnDecl {
                name: "helper".to_string(),
                params: Vec::new(),
                returns: vec![Param {
                    name: s.to_string(),
                    ty: string_ty(),
                }],
                body: Block::default(),
                cache: None,
                anchor: Some("fn-layer".to_string()),
            }],
            ..host(Vec::new())
        },
    },
    Position {
        id: "module-name",
        camelized: true,
        build: |s| BoardAst {
            modules: vec![ModuleDecl {
                name: s.to_string(),
                anchor: Some("module-layer".to_string()),
                functions: Vec::new(),
                events: Vec::new(),
                detached: Vec::new(),
                modules: Vec::new(),
            }],
            ..host(Vec::new())
        },
    },
    Position {
        id: "call-display",
        camelized: true,
        build: |s| {
            host(vec![Stmt::Call {
                call: call(s),
                anchor: Some("call-node".to_string()),
            }])
        },
    },
    Position {
        id: "call-namespace-path",
        camelized: true,
        build: |s| {
            let mut c = call("trim");
            c.path = vec![s.to_string()];
            host(vec![Stmt::Call {
                call: c,
                anchor: Some("call-node".to_string()),
            }])
        },
    },
    Position {
        id: "call-arg-name",
        camelized: true,
        build: |s| {
            let mut c = call("log");
            c.args = vec![Arg {
                name: s.to_string(),
                value: Expr::Literal(Literal::Int(1)),
            }];
            host(vec![Stmt::Call {
                call: c,
                anchor: Some("call-node".to_string()),
            }])
        },
    },
    Position {
        id: "output-pin-select",
        camelized: true,
        build: |s| {
            host(vec![bind(Expr::Field {
                base: Box::new(Expr::Ref("bound".to_string())),
                pin: s.to_string(),
            })])
        },
    },
    Position {
        id: "branch-arm-label",
        camelized: true,
        build: |s| {
            host(vec![Stmt::Branch {
                bind: None,
                call: Call {
                    node_type: "control_par_execution".to_string(),
                    display: "parallel".to_string(),
                    path: Vec::new(),
                    receiver: None,
                    positional: Vec::new(),
                    args: Vec::new(),
                    anchor: Some("branch-node".to_string()),
                },
                condition: None,
                arms: vec![BranchArm {
                    label: s.to_string(),
                    body: Block::default(),
                }],
                anchor: Some("branch-node".to_string()),
            }])
        },
    },
    Position {
        id: "loop-element-binding",
        camelized: true,
        build: |s| {
            host(vec![Stmt::Loop {
                keyword: "forEach".to_string(),
                bind: None,
                call: Call::placeholder(),
                iterable: Some(Expr::Ref("items".to_string())),
                element: Some(s.to_string()),
                index: None,
                body: Block::default(),
                anchor: Some("loop-node".to_string()),
            }])
        },
    },
    Position {
        id: "interface-name",
        camelized: false,
        build: |s| BoardAst {
            interfaces: vec![InterfaceDecl {
                name: s.to_string(),
                fields: vec![InterfaceField {
                    name: "id".to_string(),
                    ty: InterfaceType::Named("string".to_string()),
                    optional: false,
                    default: None,
                }],
                schema: None,
            }],
            ..host(Vec::new())
        },
    },
    Position {
        id: "interface-field-name",
        camelized: false,
        build: |s| BoardAst {
            interfaces: vec![InterfaceDecl {
                name: "Row".to_string(),
                fields: vec![InterfaceField {
                    name: s.to_string(),
                    ty: InterfaceType::Named("string".to_string()),
                    optional: false,
                    default: None,
                }],
                schema: None,
            }],
            ..host(Vec::new())
        },
    },
    Position {
        id: "object-literal-key",
        camelized: false,
        build: |s| {
            host(vec![bind(Expr::Object(vec![ObjectField {
                key: s.to_string(),
                value: Expr::Literal(Literal::Int(1)),
            }]))])
        },
    },
    Position {
        id: "struct-member-read",
        camelized: false,
        build: |s| {
            host(vec![bind(Expr::Member {
                base: Box::new(Expr::Ref("row".to_string())),
                field: s.to_string(),
            })])
        },
    },
    Position {
        id: "struct-field-assign-path",
        camelized: false,
        build: |s| {
            host(vec![Stmt::FieldAssign {
                base: "row".to_string(),
                path: s.to_string(),
                value: Expr::Literal(Literal::Int(1)),
                anchor: Some("set-node".to_string()),
            }])
        },
    },
    Position {
        id: "string-literal-value",
        camelized: false,
        build: |s| host(vec![bind(Expr::Literal(Literal::String(s.to_string())))]),
    },
    Position {
        id: "template-text",
        camelized: false,
        build: |s| {
            host(vec![bind(Expr::Template {
                parts: vec![
                    TemplatePart::Text(s.to_string()),
                    TemplatePart::Expr(Expr::Ref("bound".to_string())),
                ],
            })])
        },
    },
    Position {
        id: "comment-text",
        camelized: false,
        build: |s| host(vec![Stmt::Comment(s.to_string())]),
    },
    Position {
        id: "variable-description",
        camelized: false,
        build: |s| {
            let mut v = var_decl("token");
            v.description = Some(s.to_string());
            BoardAst {
                variables: vec![v],
                ..host(Vec::new())
            }
        },
    },
    Position {
        id: "variable-category",
        camelized: false,
        build: |s| {
            let mut v = var_decl("token");
            v.category = Some(s.to_string());
            BoardAst {
                variables: vec![v],
                ..host(Vec::new())
            }
        },
    },
    Position {
        id: "function-cache-namespace",
        camelized: false,
        build: |s| BoardAst {
            functions: vec![FnDecl {
                name: "helper".to_string(),
                params: Vec::new(),
                returns: Vec::new(),
                body: Block::default(),
                cache: Some(FunctionCache {
                    namespace: s.to_string(),
                    ttl_seconds: Some(60),
                    scope: FunctionCacheScope::default(),
                }),
                anchor: Some("fn-layer".to_string()),
            }],
            ..host(Vec::new())
        },
    },
    Position {
        id: "variable-default-json",
        camelized: false,
        build: |s| BoardAst {
            variables: vec![VarDecl {
                ty: TypeRef::new("Struct", Container::Normal),
                default: Some(Literal::Json(format!("{{\"key\": {:?}}}", s))),
                ..var_decl("payload")
            }],
            ..host(Vec::new())
        },
    },
    Position {
        id: "type-base-name",
        camelized: false,
        build: |s| BoardAst {
            variables: vec![VarDecl {
                ty: TypeRef::new(s, Container::Normal),
                ..var_decl("token")
            }],
            ..host(Vec::new())
        },
    },
];

// ---------------------------------------------------------------------------------------------
// The contract.
// ---------------------------------------------------------------------------------------------

/// Which invariant a case violates. Ordered from most to least severe: a document that does not
/// parse is dead, one that re-parses to a different meaning is silently wrong.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Violation {
    /// `parse(render(ast))` failed.
    NoParse,
    /// `render(parse(render(ast)))` differs from `render(ast)`.
    NotFixedPoint,
    /// The re-parsed AST has a different shape than the rendered one.
    MeaningChanged,
    /// An anchor present in the rendered text did not survive the round trip.
    AnchorLost,
}

impl Violation {
    fn tag(self) -> &'static str {
        match self {
            Violation::NoParse => "no-parse",
            Violation::NotFixedPoint => "not-fixed-point",
            Violation::MeaningChanged => "meaning-changed",
            Violation::AnchorLost => "anchor-lost",
        }
    }
}

/// Run the whole contract against one document. Returns every invariant it breaks, with detail.
fn check(ast: &BoardAst) -> Vec<(Violation, String)> {
    let mut failures = Vec::new();
    for anchors in [false, true] {
        let opts = RenderOptions {
            anchors,
            ..RenderOptions::default()
        };
        let text = render(ast, &opts);
        let reparsed = match parse(&text) {
            Ok(reparsed) => reparsed,
            Err(error) => {
                failures.push((
                    Violation::NoParse,
                    format!("anchors={anchors}: {error:?}\n--- rendered ---\n{text}"),
                ));
                continue;
            }
        };
        let again = render(&reparsed, &opts);
        if again != text {
            failures.push((
                Violation::NotFixedPoint,
                format!(
                    "anchors={anchors}: first render != second\n--- first ---\n{text}--- second ---\n{again}"
                ),
            ));
        }
        let before = shape(ast);
        let after = shape(&reparsed);
        if before != after {
            failures.push((
                Violation::MeaningChanged,
                format!(
                    "anchors={anchors}: AST shape changed\n  before: {before}\n  after : {after}\n--- rendered ---\n{text}"
                ),
            ));
        }
        if anchors {
            let lost: Vec<&String> = {
                let have = anchors_of(&reparsed);
                anchors_of(ast)
                    .into_iter()
                    .filter(|a| !have.contains(*a))
                    .collect()
            };
            if !lost.is_empty() {
                failures.push((
                    Violation::AnchorLost,
                    format!("lost anchors {lost:?}\n--- rendered ---\n{text}"),
                ));
            }
        }
    }
    failures
}

/// A structural fingerprint of a document: enough to catch a `Ref` that came back as a literal, a
/// call that came back as a return, or a statement that vanished — but blind to the names
/// themselves, which the fixed-point check already covers.
fn shape(ast: &BoardAst) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "uses={} interfaces={} vars={} fns={} events={} modules={} detached={}",
        ast.uses.len(),
        ast.interfaces.len(),
        ast.variables.len(),
        ast.functions.len(),
        ast.events.len(),
        ast.modules.len(),
        ast.detached.len(),
    ));
    for event in &ast.events {
        out.push_str(&format!(" [event params={} ", event.params.len()));
        block_shape(&event.body, &mut out);
        out.push(']');
    }
    for function in &ast.functions {
        out.push_str(&format!(
            " [fn params={} returns={} ",
            function.params.len(),
            function.returns.len()
        ));
        block_shape(&function.body, &mut out);
        out.push(']');
    }
    for module in &ast.modules {
        out.push_str(&format!(
            " [module fns={} events={}]",
            module.functions.len(),
            module.events.len()
        ));
    }
    for interface in &ast.interfaces {
        out.push_str(&format!(" [iface fields={}]", interface.fields.len()));
    }
    out
}

fn block_shape(block: &Block, out: &mut String) {
    let mut previous_was_comment = false;
    for stmt in &block.stmts {
        // A multi-line board comment renders as one `//` line per line and comes back as that many
        // `Stmt::Comment`s. That is a deliberate normalization, not lost meaning, so a run of
        // comments counts as one unit.
        let is_comment = matches!(stmt, Stmt::Comment(_));
        if is_comment && previous_was_comment {
            continue;
        }
        previous_was_comment = is_comment;
        out.push_str(stmt_tag(stmt));
        out.push(';');
        match stmt {
            Stmt::Branch { arms, .. } => {
                for arm in arms {
                    out.push('{');
                    block_shape(&arm.body, out);
                    out.push('}');
                }
            }
            Stmt::Loop { body, .. } => {
                out.push('{');
                block_shape(body, out);
                out.push('}');
            }
            Stmt::Handler(event) => {
                out.push('{');
                block_shape(&event.body, out);
                out.push('}');
            }
            Stmt::Let { call, .. } | Stmt::Call { call, .. } => {
                out.push_str(&format!("args={}", call.args.len()));
            }
            Stmt::LocalAlias { value, .. } | Stmt::Assign { value, .. } => {
                out.push_str(expr_tag(value));
            }
            _ => {}
        }
    }
}

fn stmt_tag(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::Let { .. } => "Let",
        Stmt::Destructure { .. } => "Destructure",
        Stmt::Call { .. } => "Call",
        Stmt::Branch { .. } => "Branch",
        Stmt::Loop { .. } => "Loop",
        Stmt::Assign { .. } => "Assign",
        Stmt::FieldAssign { .. } => "FieldAssign",
        Stmt::LocalAlias { .. } => "LocalAlias",
        Stmt::Return { .. } => "Return",
        Stmt::Local(_) => "Local",
        Stmt::Handler(_) => "Handler",
        Stmt::Comment(_) => "Comment",
    }
}

fn expr_tag(expr: &Expr) -> &'static str {
    match expr {
        Expr::Call(_) => "Call",
        Expr::Ref(_) => "Ref",
        // `a.b` is one surface form for two IR shapes: selecting a node's output pin and reading a
        // struct key. The parser cannot tell them apart without the board, so it always yields
        // `Field` and reconcile re-resolves it. Collapsing them here keeps that deliberate
        // ambiguity out of the contract while still catching a `Ref` that came back a literal.
        Expr::Field { .. } | Expr::Member { .. } => "Field|Member",
        Expr::Object(_) => "Object",
        Expr::Array(_) => "Array",
        Expr::Index { .. } => "Index",
        Expr::Ternary { .. } => "Ternary",
        Expr::Binary { .. } => "Binary",
        Expr::Template { .. } => "Template",
        Expr::Literal(_) => "Literal",
    }
}

fn anchors_of(ast: &BoardAst) -> BTreeSet<&String> {
    let mut out = BTreeSet::new();
    fn push<'a>(anchor: &'a Option<String>, out: &mut BTreeSet<&'a String>) {
        if let Some(anchor) = anchor.as_ref() {
            out.insert(anchor);
        }
    }
    for variable in &ast.variables {
        push(&variable.anchor, &mut out);
    }
    fn walk<'a>(block: &'a Block, out: &mut BTreeSet<&'a String>) {
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let { anchor, .. }
                | Stmt::Destructure { anchor, .. }
                | Stmt::Call { anchor, .. }
                | Stmt::Assign { anchor, .. }
                | Stmt::FieldAssign { anchor, .. }
                | Stmt::LocalAlias { anchor, .. }
                | Stmt::Return { anchor, .. } => {
                    if let Some(anchor) = anchor.as_ref() {
                        out.insert(anchor);
                    }
                }
                Stmt::Branch { anchor, arms, .. } => {
                    if let Some(anchor) = anchor.as_ref() {
                        out.insert(anchor);
                    }
                    for arm in arms {
                        walk(&arm.body, out);
                    }
                }
                Stmt::Loop { anchor, body, .. } => {
                    if let Some(anchor) = anchor.as_ref() {
                        out.insert(anchor);
                    }
                    walk(body, out);
                }
                Stmt::Local(var) => {
                    if let Some(anchor) = var.anchor.as_ref() {
                        out.insert(anchor);
                    }
                }
                Stmt::Handler(event) => {
                    if let Some(anchor) = event.anchor.as_ref() {
                        out.insert(anchor);
                    }
                    walk(&event.body, out);
                }
                Stmt::Comment(_) => {}
            }
        }
    }
    for event in &ast.events {
        push(&event.anchor, &mut out);
        walk(&event.body, &mut out);
    }
    for function in &ast.functions {
        push(&function.anchor, &mut out);
        walk(&function.body, &mut out);
    }
    for module in &ast.modules {
        push(&module.anchor, &mut out);
        for function in &module.functions {
            push(&function.anchor, &mut out);
            walk(&function.body, &mut out);
        }
        for event in &module.events {
            push(&event.anchor, &mut out);
            walk(&event.body, &mut out);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The ratchet.
// ---------------------------------------------------------------------------------------------

/// `position/input/violation` triples where the renderer is not total.
///
/// **This list may only shrink.** A case that starts passing fails `render_contract_matrix` with a
/// "now passes, remove it" message, so a fix cannot land while leaving stale suppression behind.
///
/// **None of these is reachable from a board.** Every name here is sanitized before the renderer
/// ever sees it — `flow_like_ast::declared_identifier` for anything lowering declares (variables,
/// functions, parameters, modules, event headers) and `pascal_case` for schema-derived interface
/// and type names. What is left is a deliberate design position rather than an unfixed bug:
/// renaming is a *naming* concern that has to be consistent across the declaration, every
/// reference, and reconcile's view of both, so exactly one component does it. Adding a second
/// renamer in the renderer would create two of them, and two renamers that disagree is the bug
/// class this whole suite exists to prevent.
///
/// So these entries are a tripwire, not a backlog: they fail the moment some future path hands the
/// renderer a name that skipped the sanitizer. The reachability claim itself is not taken on faith
/// — `hostile_board_names_still_round_trip` and `hostile_schema_names_still_round_trip` in
/// `flow-like-catalog/tests/render_contract_catalog.rs` build real boards out of these same
/// strings, and both were checked to go red when their sanitizer is disabled.
const KNOWN_GAPS: &[(&str, &str, Violation)] = &[
    // --- variable-read ---
    // A bare `Expr::Ref` spelled `true`/`false`/`null` re-parses as a literal, so the read stops
    // being a wire — the document still parses and is still a fixed point, it just means something
    // else. Guarded: `declared_identifier` renders such a variable `true2`.
    ("variable-read", "kw-false", Violation::MeaningChanged),
    ("variable-read", "kw-null", Violation::MeaningChanged),
    ("variable-read", "kw-true", Violation::MeaningChanged),
    // --- variable-assign-target ---
    // `name = value` from a `variable_set`, where the name is a keyword (`return = …`). Guarded by
    // `declared_identifier`; `hostile_board_names_still_round_trip` covers the board path.
    ("variable-assign-target", "kw-const", Violation::NoParse),
    ("variable-assign-target", "kw-for", Violation::NoParse),
    ("variable-assign-target", "kw-if", Violation::NoParse),
    ("variable-assign-target", "kw-let", Violation::NoParse),
    ("variable-assign-target", "kw-return", Violation::NoParse),
    ("variable-assign-target", "kw-use", Violation::NoParse),
    ("variable-assign-target", "kw-while", Violation::NoParse),
    // --- event-header-name ---
    // `name() {` — the event block header. Guarded by `declared_identifier` in `event_alias`.
    ("event-header-name", "kw-const", Violation::NoParse),
    ("event-header-name", "kw-function", Violation::NoParse),
    ("event-header-name", "kw-interface", Violation::NoParse),
    ("event-header-name", "kw-let", Violation::NoParse),
    ("event-header-name", "kw-use", Violation::NoParse),
    // --- call-display ---
    // The identifier immediately before `(`. A Function layer named `Return` would render
    // `return({ … })`, which the parser reads as a return statement and the call node disappears.
    // Guarded twice over: `declared_identifier` for Function-layer names, and `check_names` (run
    // over the committed signature set by `clean_catalog_has_no_collisions`) rejects a catalog
    // alias that is a keyword or not an identifier.
    ("call-display", "kw-const", Violation::NoParse),
    ("call-display", "kw-false", Violation::NoParse),
    ("call-display", "kw-for", Violation::NoParse),
    ("call-display", "kw-if", Violation::NoParse),
    ("call-display", "kw-let", Violation::NoParse),
    ("call-display", "kw-null", Violation::NoParse),
    ("call-display", "kw-return", Violation::MeaningChanged),
    ("call-display", "kw-return", Violation::NotFixedPoint),
    ("call-display", "kw-true", Violation::NoParse),
    ("call-display", "kw-use", Violation::NoParse),
    ("call-display", "kw-while", Violation::NoParse),
    // --- call-namespace-path ---
    // `ns::alias` path segments, from node namespace metadata. Guarded by `check_names`, which no
    // catalog node can be added without satisfying.
    ("call-namespace-path", "kw-const", Violation::NoParse),
    ("call-namespace-path", "kw-false", Violation::NoParse),
    ("call-namespace-path", "kw-for", Violation::NoParse),
    ("call-namespace-path", "kw-if", Violation::NoParse),
    ("call-namespace-path", "kw-let", Violation::NoParse),
    ("call-namespace-path", "kw-null", Violation::NoParse),
    ("call-namespace-path", "kw-return", Violation::NoParse),
    ("call-namespace-path", "kw-true", Violation::NoParse),
    ("call-namespace-path", "kw-use", Violation::NoParse),
    ("call-namespace-path", "kw-while", Violation::NoParse),
    // --- interface-name ---
    // `interface Name {`. There is no quoted form for a type name in the grammar, so the guard has
    // to be at the derivation site: `interfaces_from_schema` runs every title, `$defs` key and
    // `$ref` target through `pascal_case`, and `interfaces_for_variables` then uniquifies them.
    // `hostile_schema_names_still_round_trip` covers the board path.
    ("interface-name", "all-digits", Violation::NoParse),
    ("interface-name", "anchor-marker", Violation::NoParse),
    ("interface-name", "backslash", Violation::NoParse),
    ("interface-name", "backtick", Violation::NoParse),
    ("interface-name", "block-comment", Violation::NoParse),
    ("interface-name", "bracket", Violation::NoParse),
    ("interface-name", "colon", Violation::NoParse),
    ("interface-name", "comma", Violation::NoParse),
    ("interface-name", "crlf", Violation::NoParse),
    ("interface-name", "dash", Violation::NoParse),
    ("interface-name", "dollar-brace", Violation::NoParse),
    ("interface-name", "dot", Violation::NoParse),
    ("interface-name", "double-dot", Violation::NoParse),
    ("interface-name", "dquote", Violation::NoParse),
    ("interface-name", "emoji", Violation::NoParse),
    ("interface-name", "empty", Violation::NoParse),
    ("interface-name", "lbrace", Violation::NoParse),
    ("interface-name", "leading-digit", Violation::NoParse),
    ("interface-name", "line-comment", Violation::NoParse),
    ("interface-name", "newline", Violation::NoParse),
    ("interface-name", "path-sep", Violation::NoParse),
    ("interface-name", "rbrace", Violation::NoParse),
    ("interface-name", "rparen", Violation::NoParse),
    ("interface-name", "semicolon", Violation::NoParse),
    ("interface-name", "space", Violation::NoParse),
    ("interface-name", "tab", Violation::NoParse),
    ("interface-name", "whitespace-only", Violation::NoParse),
    // --- type-base-name ---
    // `const x: Base` / `param: Base`. `TypeRef::base` is written raw; it is either a primitive
    // spelling or an interface name resolved through `interface_name_for_schema`, so it inherits
    // the `pascal_case` guard above and always agrees with the declaration it names.
    ("type-base-name", "all-digits", Violation::NoParse),
    ("type-base-name", "anchor-marker", Violation::AnchorLost),
    ("type-base-name", "anchor-marker", Violation::NotFixedPoint),
    ("type-base-name", "backslash", Violation::NoParse),
    ("type-base-name", "backtick", Violation::NoParse),
    ("type-base-name", "block-comment", Violation::NoParse),
    ("type-base-name", "bracket", Violation::NoParse),
    ("type-base-name", "colon", Violation::NoParse),
    ("type-base-name", "comma", Violation::NoParse),
    ("type-base-name", "crlf", Violation::NoParse),
    ("type-base-name", "crlf", Violation::NotFixedPoint),
    ("type-base-name", "dash", Violation::NoParse),
    ("type-base-name", "dollar-brace", Violation::NoParse),
    ("type-base-name", "dot", Violation::NoParse),
    ("type-base-name", "double-dot", Violation::NoParse),
    ("type-base-name", "dquote", Violation::NoParse),
    ("type-base-name", "emoji", Violation::NoParse),
    ("type-base-name", "empty", Violation::NoParse),
    ("type-base-name", "lbrace", Violation::NoParse),
    ("type-base-name", "leading-digit", Violation::NoParse),
    ("type-base-name", "line-comment", Violation::AnchorLost),
    ("type-base-name", "line-comment", Violation::NotFixedPoint),
    ("type-base-name", "newline", Violation::NoParse),
    ("type-base-name", "newline", Violation::NotFixedPoint),
    ("type-base-name", "path-sep", Violation::NoParse),
    ("type-base-name", "rbrace", Violation::NoParse),
    ("type-base-name", "rparen", Violation::NoParse),
    ("type-base-name", "semicolon", Violation::NoParse),
    ("type-base-name", "semicolon", Violation::NotFixedPoint),
    ("type-base-name", "space", Violation::NoParse),
    ("type-base-name", "space", Violation::NotFixedPoint),
    ("type-base-name", "tab", Violation::NoParse),
    ("type-base-name", "tab", Violation::NotFixedPoint),
    ("type-base-name", "whitespace-only", Violation::NoParse),
];

// ---------------------------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------------------------

/// Compare what a run found against [`KNOWN_GAPS`], failing on both an unlisted violation and a
/// listed one that started passing. `exercised` limits the "now passes" half to the cases the
/// caller ran, so one test cannot declare another's gaps fixed.
fn ratchet(
    kind: &str,
    found: &[(&str, &str, Violation, String)],
    exercised: &BTreeSet<(&str, &str)>,
) {
    let known: BTreeSet<(&str, &str, Violation)> = KNOWN_GAPS.iter().copied().collect();
    let hit: BTreeSet<(&str, &str, Violation)> = found
        .iter()
        .map(|(position, input, violation, _)| (*position, *input, *violation))
        .collect();

    let unexpected: Vec<String> = found
        .iter()
        .filter(|(position, input, violation, _)| !known.contains(&(*position, *input, *violation)))
        .map(|(position, input, violation, detail)| {
            format!("{position}/{input} [{}]\n{detail}", violation.tag())
        })
        .collect();
    let fixed: Vec<String> = known
        .iter()
        .filter(|(position, input, _)| exercised.contains(&(*position, *input)))
        .filter(|key| !hit.contains(key))
        .map(|(position, input, violation)| format!("{position}/{input} [{}]", violation.tag()))
        .collect();

    let mut message = String::new();
    if !unexpected.is_empty() {
        message.push_str(&format!(
            "\n{} {kind} violation(s) not in KNOWN_GAPS:\n\n{}\n",
            unexpected.len(),
            unexpected.join("\n\n")
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

/// Every string in [`INPUTS`], in every position in [`POSITIONS`], must satisfy the contract —
/// except the cases enumerated in [`KNOWN_GAPS`], which must still fail.
#[test]
fn render_contract_matrix() {
    let mut found: Vec<(&str, &str, Violation, String)> = Vec::new();
    let mut exercised: BTreeSet<(&str, &str)> = BTreeSet::new();

    for position in POSITIONS {
        for input in INPUTS {
            // A camelized position can only ever receive a camelized string, so feeding it raw
            // punctuation would test a shape the product cannot produce. Non-camelized positions
            // take the whole corpus.
            if position.camelized && input.reach == Reach::Synthetic {
                continue;
            }
            if !input.only.is_empty() && !input.only.contains(&position.id) {
                continue;
            }
            exercised.insert((position.id, input.id));
            for (violation, detail) in check(&(position.build)(input.value)) {
                found.push((position.id, input.id, violation, detail));
            }
        }
    }

    ratchet("render-contract", &found, &exercised);
}

/// The matrix is only as good as its corpus. If a keyword is added to the language without a
/// matching `kw-*` entry, every identifier position silently stops being tested against it.
#[test]
fn keywords_are_all_covered() {
    let covered: BTreeSet<&str> = INPUTS
        .iter()
        .filter(|input| input.id.starts_with("kw-"))
        .map(|input| input.value)
        .collect();
    let missing: Vec<&&str> = KEYWORDS
        .iter()
        .filter(|kw| !covered.contains(**kw))
        .collect();
    assert!(
        missing.is_empty(),
        "KEYWORDS gained {missing:?} — add a `board(\"kw-…\", …)` entry to INPUTS so every \
         identifier position is tested against it"
    );
}

/// The corpus classification must be honest: a `Reach::Board` input is one that survives
/// `to_camel_case` unchanged (that is what makes it reachable from a real board), and a
/// `Reach::Synthetic` one is a string camelization neutralizes.
#[test]
fn corpus_reachability_matches_camelization() {
    let mut wrong = Vec::new();
    for input in INPUTS {
        let camelized = flow_like_ast::to_camel_case(input.value);
        let survives = camelized == input.value;
        match (input.reach, survives) {
            (Reach::Board, false) => wrong.push(format!(
                "{}: marked Board but to_camel_case({:?}) = {:?} — lowering neutralizes it, mark \
                 it synthetic",
                input.id, input.value, camelized
            )),
            (Reach::Synthetic, true) => wrong.push(format!(
                "{}: marked Synthetic but to_camel_case({:?}) is a no-op — a board can produce it, \
                 mark it board",
                input.id, input.value
            )),
            _ => {}
        }
    }
    assert!(
        wrong.is_empty(),
        "corpus misclassified:\n  {}",
        wrong.join("\n  ")
    );
}

/// Names that are already perfectly ordinary identifiers.
const WELL_FORMED_NAMES: &[&str] = &["alpha", "someName", "_private", "a1", "x"];

/// Documents built entirely from valid identifiers must satisfy the contract.
///
/// Gaps found here are listed under the `well-formed` input id in [`KNOWN_GAPS`] and are the most
/// serious entries in it: nobody had to type anything unusual to reach them, so the renderer is
/// simply wrong for ordinary boards.
#[test]
fn well_formed_documents_round_trip() {
    let mut found: Vec<(&str, &str, Violation, String)> = Vec::new();
    let mut exercised: BTreeSet<(&str, &str)> = BTreeSet::new();
    for position in POSITIONS {
        for name in WELL_FORMED_NAMES {
            assert!(
                is_valid_identifier(name),
                "{name:?} is in WELL_FORMED_NAMES but does not lex as an identifier"
            );
            exercised.insert((position.id, "well-formed"));
            for (violation, detail) in check(&(position.build)(name)) {
                found.push((
                    position.id,
                    "well-formed",
                    violation,
                    format!("name {name:?}: {detail}"),
                ));
            }
        }
    }
    ratchet("well-formed render-contract", &found, &exercised);
}

/// Parsing must not depend on line endings.
///
/// A document that reaches the engine over a Windows editor, a clipboard, or an HTTP body arrives
/// CRLF-terminated. If the lexer keeps the `\r`, every anchor id gains a trailing carriage return,
/// none of them match the board, and reconcile reads every anchored node, variable and layer as
/// deleted — the most destructive outcome the pipeline has.
#[test]
fn crlf_documents_parse_identically_to_lf() {
    let mut failures = Vec::new();
    for position in POSITIONS {
        let ast = (position.build)("alpha");
        let lf = render(
            &ast,
            &RenderOptions {
                anchors: true,
                ..RenderOptions::default()
            },
        );
        let crlf = lf.replace('\n', "\r\n");
        let Ok(from_lf) = parse(&lf) else { continue };
        let from_crlf = match parse(&crlf) {
            Ok(parsed) => parsed,
            Err(error) => {
                failures.push(format!(
                    "{}: CRLF form does not parse: {error:?}",
                    position.id
                ));
                continue;
            }
        };
        let (want, got) = (anchors_of(&from_lf), anchors_of(&from_crlf));
        if want != got {
            failures.push(format!(
                "{}: anchors differ between LF and CRLF\n  lf  : {want:?}\n  crlf: {got:?}",
                position.id
            ));
        }
        let (want, got) = (shape(&from_lf), shape(&from_crlf));
        if want != got {
            failures.push(format!(
                "{}: shape differs between LF and CRLF\n  lf  : {want}\n  crlf: {got}",
                position.id
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} document(s) parse differently with CRLF line endings:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// Trailing whitespace and a missing final newline must not change the parse either — both are
/// routine for text that has been through an editor, a diff tool, or a JSON round trip.
#[test]
fn incidental_whitespace_does_not_change_the_parsed_document() {
    let mut failures = Vec::new();
    for position in POSITIONS {
        let ast = (position.build)("alpha");
        let base = render(
            &ast,
            &RenderOptions {
                anchors: true,
                ..RenderOptions::default()
            },
        );
        let Ok(want) = parse(&base) else { continue };
        let variants = [
            ("no trailing newline", base.trim_end().to_string()),
            (
                "trailing spaces",
                base.lines()
                    .map(|l| format!("{l}  "))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            ("extra blank lines", base.replace('\n', "\n\n")),
        ];
        for (label, variant) in variants {
            match parse(&variant) {
                Ok(got) if shape(&got) == shape(&want) && anchors_of(&got) == anchors_of(&want) => {
                }
                Ok(got) => failures.push(format!(
                    "{} [{label}]: parsed differently\n  want: {}\n  got : {}",
                    position.id,
                    shape(&want),
                    shape(&got)
                )),
                Err(error) => failures.push(format!(
                    "{} [{label}]: does not parse: {error:?}",
                    position.id
                )),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} whitespace variant(s) change the parsed document:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// A statement that is nothing but a call must parse, whatever its receiver is.
///
/// Method form puts the receiver first, so an impure node whose receiver is a literal and whose
/// output nobody reads renders a statement that *begins* with that literal — `"payload".sha256()`.
/// The parser has no statement that starts with a literal, so the line is rejected outright, and
/// the board it came from can no longer be opened in the FlowScript panel. Any board with a
/// method-form node whose receiver pin holds a typed-in value and whose output is unwired reaches
/// this, which is an ordinary thing to build.
#[test]
fn statement_position_calls_parse_whatever_the_receiver_is() {
    let receivers: &[(&str, Expr)] = &[
        ("ident", Expr::Ref("value".to_string())),
        (
            "string-literal",
            Expr::Literal(Literal::String("payload".to_string())),
        ),
        ("int-literal", Expr::Literal(Literal::Int(42))),
        ("bool-literal", Expr::Literal(Literal::Bool(true))),
        (
            "array-literal",
            Expr::Array(vec![Expr::Literal(Literal::Int(1))]),
        ),
        (
            "template",
            Expr::Template {
                parts: vec![TemplatePart::Text("a".to_string())],
            },
        ),
        (
            "call",
            Expr::Call(Call {
                node_type: "string_trim".to_string(),
                display: "trim".to_string(),
                path: Vec::new(),
                receiver: None,
                positional: Vec::new(),
                args: Vec::new(),
                anchor: None,
            }),
        ),
    ];

    let mut found: Vec<(&str, &str, Violation, String)> = Vec::new();
    let mut exercised: BTreeSet<(&str, &str)> = BTreeSet::new();
    for (id, receiver) in receivers {
        let mut call = call("sha256");
        call.receiver = Some(Box::new(receiver.clone()));
        let ast = host(vec![Stmt::Call {
            call,
            anchor: Some("call-node".to_string()),
        }]);
        exercised.insert(("statement-call-receiver", id));
        for (violation, detail) in check(&ast) {
            found.push(("statement-call-receiver", id, violation, detail));
        }
    }
    ratchet("statement-position receiver", &found, &exercised);
}
