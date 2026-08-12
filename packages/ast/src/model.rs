//! Typed intermediate representation for FlowScript.
//!
//! `BoardAst` is the lossless-as-possible pivot between the `Board` graph model and the
//! text-domain DSL ("FlowScript"). This crate owns the *language half*: the IR plus pure
//! operations on it (render, parse, lint, signature formatting). Lowering (`Board ->
//! BoardAst`) and reconcile (`BoardAst -> commands`) live in `flow-like` core, which depends
//! on this crate. See `todo/ast.md`.

use serde::{Deserialize, Serialize};

/// Root of the text-domain representation of a board.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BoardAst {
    /// Source board id (metadata, not rendered in-band).
    pub board_id: String,
    /// Top-level struct interfaces. These are the readable FlowScript form of JSON schemas;
    /// variables typed with an interface still carry the generated schema internally.
    #[serde(default)]
    pub interfaces: Vec<InterfaceDecl>,
    /// Top-level variable declarations.
    pub variables: Vec<VarDecl>,
    /// Function definitions (from `LayerType::Function`).
    pub functions: Vec<FnDecl>,
    /// Exec entrypoints (start / event-callback nodes), each owning a block.
    pub events: Vec<EventBlock>,
}

/// A TypeScript-like interface declaration used as the readable surface for struct schemas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceDecl {
    pub name: String,
    pub fields: Vec<InterfaceField>,
    /// Generated JSON Schema string for this interface when known. This is not rendered
    /// directly; it lets variables keep the exact board schema metadata in the background.
    #[serde(default)]
    pub schema: Option<String>,
}

/// One property of an [`InterfaceDecl`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceField {
    pub name: String,
    pub ty: InterfaceType,
    pub optional: bool,
    #[serde(default)]
    pub default: Option<Literal>,
}

/// Type expression used inside interface declarations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InterfaceType {
    Named(String),
    Array(Box<InterfaceType>),
    Map(Box<InterfaceType>),
    Union(Vec<InterfaceType>),
    StringLiteral(String),
    Null,
    Any,
}

/// A board/layer variable declaration (`let name: Type = default`).
///
/// Keyword-level state (`exposed`) maps to the `let`/`const` keyword; every other non-default
/// setting is surfaced as a `@decorator` line above the declaration so the round-trip is lossless.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarDecl {
    pub name: String,
    pub ty: TypeRef,
    pub default: Option<Literal>,
    pub exposed: bool,
    pub secret: bool,
    /// Whether the variable is user-editable. Defaults to `true`; rendered as `@readonly` when false.
    #[serde(default = "default_true")]
    pub editable: bool,
    /// Whether the variable is configured per-user at runtime. Rendered as `@runtime`.
    #[serde(default)]
    pub runtime_configured: bool,
    /// Optional UI grouping category. Rendered as `@category("…")`.
    #[serde(default)]
    pub category: Option<String>,
    /// Optional human description. Rendered as `@description("…")`.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional JSON schema (for struct types), preserved verbatim.
    /// Rendered through FlowScript interfaces when possible; `@schema` is legacy syntax.
    #[serde(default)]
    pub schema: Option<String>,
    /// Stable identity anchor (the variable id), preserved for round-trip.
    pub anchor: Option<String>,
}

/// serde default helper: `true`.
fn default_true() -> bool {
    true
}

/// A function definition lowered from a `Function` layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub returns: Vec<Param>,
    pub body: Block,
    /// Result-cache policy for this function. Presence maps to an enabled layer cache and is
    /// rendered as `@cache` (defaults) or `@cache({ ... })` immediately above the declaration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<FunctionCache>,
    /// Stable identity anchor (the layer id).
    pub anchor: Option<String>,
}

/// Default namespace used by a bare `@cache` decorator.
pub const DEFAULT_FUNCTION_CACHE_NAMESPACE: &str = "global";
/// Default entry lifetime used by a bare `@cache` decorator, in seconds.
pub const DEFAULT_FUNCTION_CACHE_TTL_SECONDS: u64 = 300;

/// Result-cache settings attached to a FlowScript function.
///
/// The language calls `prefix` a namespace because that is the cache-backend concept exposed to
/// authors. FlowScript defaults to the global namespace with a five-minute lifetime; an explicit
/// lifetime of zero keeps entries until they are invalidated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionCache {
    #[serde(default = "default_function_cache_namespace")]
    pub namespace: String,
    #[serde(default = "default_function_cache_ttl_seconds")]
    pub ttl_seconds: Option<u64>,
    #[serde(default)]
    pub scope: FunctionCacheScope,
}

fn default_function_cache_namespace() -> String {
    DEFAULT_FUNCTION_CACHE_NAMESPACE.to_string()
}

fn default_function_cache_ttl_seconds() -> Option<u64> {
    Some(DEFAULT_FUNCTION_CACHE_TTL_SECONDS)
}

impl Default for FunctionCache {
    fn default() -> Self {
        Self {
            namespace: default_function_cache_namespace(),
            ttl_seconds: default_function_cache_ttl_seconds(),
            scope: FunctionCacheScope::App,
        }
    }
}

/// Who may share a cached function result.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FunctionCacheScope {
    /// Shared by callers of the app.
    #[default]
    App,
    /// Private to the user who triggered the run.
    User,
}

impl FunctionCacheScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::User => "user",
        }
    }
}

/// A named, typed function parameter or return value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub ty: TypeRef,
}

/// An exec entrypoint block (`onStart { … }`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBlock {
    /// Block header keyword (camelCase, derived from the entry node).
    pub name: String,
    /// The entry node's catalog type.
    pub node_type: String,
    /// The event's given name (`eventsSimple dashboardLoad() { }`), applied to the entry node as
    /// its friendly name. `None` keeps the catalog default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_name: Option<String>,
    /// The event's payload outputs, surfaced as a typed parameter list (`name: Type`). These are
    /// the entry node's data output pins (often user-configured) that the body consumes.
    #[serde(default)]
    pub params: Vec<Param>,
    pub body: Block,
    /// Stable identity anchor (the entry node id).
    pub anchor: Option<String>,
}

/// An ordered sequence of statements.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

/// A single statement inside a block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Stmt {
    /// `const name = call(...)` — impure node whose output is consumed downstream.
    Let {
        name: String,
        call: Call,
        anchor: Option<String>,
    },
    /// `call(...)` — impure node with no captured output.
    Call { call: Call, anchor: Option<String> },
    /// A branch/loop node opening one nested block per exec output pin.
    Branch {
        /// Optional binding name for a branch call whose data outputs are consumed inside arms.
        /// Rendered as `const name = call(...)` followed by `name { arm: { ... } }`.
        bind: Option<String>,
        call: Call,
        /// Sugared boolean condition (set for `control_branch`); when present the branch
        /// renders as `if (condition)` instead of `if (call(...))`.
        condition: Option<Expr>,
        arms: Vec<BranchArm>,
        anchor: Option<String>,
    },
    /// A loop node (`forEach`/`forEachParallel`/`while`). The loop handle binding (its
    /// `value`/`index`/`iter` outputs) is introduced for use inside `body`; statements after
    /// the loop's `done` exec follow as siblings in the enclosing block.
    Loop {
        /// Loop keyword (`forEach`, `forEachParallel`, `while`).
        keyword: String,
        /// Binding name for the loop handle, if its data outputs are consumed.
        bind: Option<String>,
        call: Call,
        body: Block,
        anchor: Option<String>,
    },
    /// `name = expr` — a variable assignment (lowered from `variable_set`).
    Assign {
        target: String,
        value: Expr,
        anchor: Option<String>,
    },
    /// `base.path = value` — a struct-field write (the readable surface of a single-field
    /// `struct_set` accumulator, e.g. `preferences.coding_weight = 0.5`). `base` is the root
    /// variable name; `path` is the field path WITHOUT a leading dot (`coding_weight`, `a.b`,
    /// `items[0].name`, or a bracket-rooted `[0]`). Reconcile expands it to the
    /// `structSet({ structIn: base, field: "path", value })` form and rebinds `base` to
    /// `struct_out`; lowering re-sugars such an accumulator `struct_set` back to this.
    FieldAssign {
        base: String,
        path: String,
        value: Expr,
        anchor: Option<String>,
    },
    /// `let name = expr` — a function-local alias/mutable accumulator introduced by FlowScript.
    LocalAlias {
        name: String,
        value: Expr,
        anchor: Option<String>,
    },
    /// `return a, b` — function layer outputs, or an event/tool-entry result
    /// (`events_generic_return_result`). `anchor` is that result node's id when the return maps to
    /// a concrete node (event returns), letting reconcile match it instead of duplicating.
    Return {
        values: Vec<Expr>,
        #[serde(default)]
        anchor: Option<String>,
    },
    /// `let name: Type = default` — a function-local (layer) variable declaration.
    Local(VarDecl),
    /// A nested event handler (`name(params) { … }`) — a `start`/`event_callback` trigger node
    /// that lives inside a function scope and closes over its locals (e.g. an agent tool entry).
    /// It is an independent entry point, not part of the enclosing chain.
    Handler(EventBlock),
    /// A free-standing comment line.
    Comment(String),
}

/// One exec-output arm of a branch/loop node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchArm {
    /// The exec output pin name (e.g. `True`, `False`, `loop`).
    pub label: String,
    pub body: Block,
}

/// An expression: a node call, a binding reference, an output selection, or a literal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expr {
    Call(Call),
    /// Reference to a `let` binding, function parameter, or board variable.
    Ref(String),
    /// Select a named output pin of an expression (`expr.pinName`, camelCased).
    Field {
        base: Box<Expr>,
        pin: String,
    },
    /// Access a data field of a struct value (`expr.field` / `expr["field"]`).
    /// Unlike [`Expr::Field`] the key is a runtime data key, rendered verbatim.
    Member {
        base: Box<Expr>,
        field: String,
    },
    /// A struct literal (`{ field: value, … }`).
    Object(Vec<ObjectField>),
    /// An array literal (`[a, b, …]`), used for reference lists (e.g. agent tool refs).
    Array(Vec<Expr>),
    /// Index access into a collection (`base[index]`), sugared from `arrayGet`.
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    /// A ternary selection (`cond ? then : otherwise`), sugared from `utilsTypesSelect`.
    Ternary {
        cond: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
    },
    /// A binary operator expression (`lhs op rhs`), sugared from comparison/arithmetic nodes.
    Binary {
        op: String,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Literal(Literal),
}

/// One `key: value` entry of an [`Expr::Object`] struct literal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectField {
    pub key: String,
    pub value: Expr,
}

/// A node invocation with positional + named (trailing object) arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Call {
    /// Catalog node type (e.g. `ai_generative_find_model`).
    pub node_type: String,
    /// JS-flavoured display name (e.g. `aiGenerativeFindModel`).
    pub display: String,
    pub args: Vec<Arg>,
    /// Stable identity anchor (the node id).
    pub anchor: Option<String>,
}

/// A call argument bound to an input pin by name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arg {
    /// Input pin name this argument binds to.
    pub name: String,
    pub value: Expr,
}

/// A literal value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Literal {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
    /// Raw JSON for structured defaults (objects/arrays) that have no scalar form.
    Json(String),
}

/// Container shape of a typed value.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Container {
    Normal,
    Array,
    Map,
    Set,
}

/// A TS-flavoured type reference (`string`, `int[]`, `Map<string, Struct>`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeRef {
    pub base: String,
    pub container: Container,
}

impl TypeRef {
    pub fn new(base: impl Into<String>, container: Container) -> Self {
        Self {
            base: base.into(),
            container,
        }
    }
}
