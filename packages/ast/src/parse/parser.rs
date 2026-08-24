//! FlowScript parser: `FlowScript text -> BoardAst`.
//!
//! Hand-written recursive-descent + Pratt expression parser — the inverse of [`crate::render`].
//! It targets the existing [`crate::model`] IR (no parallel type hierarchy). Correctness is
//! anchored by the text-idempotency invariant `render(parse(text)) == text` (see tests).
//!
//! This stage is purely syntactic: a [`Call`]'s `node_type` (the catalog id) is not present in
//! the text and is resolved later by the core reconcile phase, so it is left empty here.

use crate::model::*;
use crate::parse::error::ParseError;
use crate::parse::lexer::{TemplatePiece, Tok, Token, lex};
use crate::schema::{apply_interface_schemas, schema_from_interface};
use std::collections::HashSet;

/// Parse FlowScript source into a [`BoardAst`].
pub fn parse(src: &str) -> Result<BoardAst, ParseError> {
    let tokens = lex(src)?;
    let mut parser = Parser {
        src,
        toks: tokens,
        pos: 0,
        depth: 0,
    };
    parser.board()
}

/// Recursion ceiling for nested expressions/blocks. User-authored text feeds this parser
/// directly (editor view, API route), so unbounded recursion is a process-killing DoS.
/// Each level costs ~5 stack frames through the Pratt chain; 128 keeps the worst case
/// well inside a 2 MiB (debug/test) stack while allowing any realistic board.
const MAX_NESTING_DEPTH: usize = 128;

fn binary_operator_precedence(op: &str) -> Option<u8> {
    match op {
        "||" => Some(1),
        "&&" => Some(2),
        "|" => Some(3),
        "^" => Some(4),
        "==" | "!=" | "===" | "!==" => Some(5),
        ">" | ">=" | "<" | "<=" => Some(6),
        "+" | "-" => Some(7),
        "*" | "/" | "%" => Some(8),
        "**" => Some(9),
        _ => None,
    }
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
}

/// A parsed `@decorator`, optionally carrying a string or the structured `@cache` settings.
struct Decorator {
    name: String,
    arg: Option<DecoratorArg>,
}

enum DecoratorArg {
    String(String),
    Cache(FunctionCache),
}

impl Parser<'_> {
    // ---- token cursor ---------------------------------------------------------------------

    fn cur(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn cur_token(&self) -> &Token {
        &self.toks[self.pos]
    }

    fn line(&self) -> usize {
        self.toks[self.pos].line
    }

    fn at_eof(&self) -> bool {
        matches!(self.cur(), Tok::Eof)
    }

    fn bump(&mut self) -> Tok {
        let tok = self.toks[self.pos].tok.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        tok
    }

    fn err(&self, message: impl Into<String>) -> ParseError {
        let t = self.cur_token();
        ParseError::new(message, t.line, t.col)
    }

    fn expect(&mut self, want: &Tok) -> Result<(), ParseError> {
        if self.cur() == want {
            self.bump();
            Ok(())
        } else {
            Err(self.err(format!("expected `{want:?}`, found `{:?}`", self.cur())))
        }
    }

    fn eat(&mut self, want: &Tok) -> bool {
        if self.cur() == want {
            self.bump();
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> Result<String, ParseError> {
        match self.cur().clone() {
            Tok::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.err(format!("expected identifier, found `{other:?}`"))),
        }
    }

    fn is_ident(&self, name: &str) -> bool {
        matches!(self.cur(), Tok::Ident(n) if n == name)
    }

    // ---- trailing comments (labels / anchors) ---------------------------------------------

    /// Consume a *trailing* anchor comment (`//@n:id`) if present; returns the id.
    /// Only the known anchor kinds (`n`/`v`/`l`) qualify — any other `@…` comment is an
    /// ordinary user comment and must not be swallowed as an anchor.
    ///
    /// The anchor must not be the first thing on its source line. `render::Writer::anchor`
    /// appends to the current line and never opens one, so this rejects nothing the renderer
    /// emits — but it stops a finished statement from swallowing an anchor authored on the next
    /// line, which today both mis-attributes identity and can mutate the anchor kind (a `//@n:`
    /// stolen by `fn_decl` is re-emitted as `//@l:`). Anchors are the only stable identity across
    /// a round-trip, so a stolen one makes reconcile rewrite one node with another's content and
    /// report the real one as deleted.
    ///
    /// This mirrors the positional rule `take_label_on_line` already applies to arm labels.
    fn take_anchor(&mut self) -> Option<String> {
        // An optional `;` terminator sits between the statement and its trailing anchor.
        if matches!(self.cur(), Tok::Semi)
            && matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                Some(Tok::Comment(_))
            )
        {
            self.bump();
        }
        if self.comment_starts_its_line() {
            return None;
        }
        self.take_anchor_before_arms()
    }

    /// Anchor consumption for a block whose body is nothing but labelled branch arms
    /// (`call(…) { … }` / `bind { … }`). `BranchArm` has no anchor field, so a comment before the
    /// first arm can only be the branch's — accept it there even on its own line, and re-render it
    /// trailing. Without this exemption the strict rule above turns currently-parsing documents
    /// into `expected identifier, found Comment(...)` at the first arm.
    fn take_anchor_before_arms(&mut self) -> Option<String> {
        if let Tok::Comment(text) = self.cur()
            && let Some(rest) = text.strip_prefix('@')
            && let Some((kind, id)) = rest.split_once(':')
            && matches!(kind, "n" | "v" | "l")
        {
            let id = id.to_string();
            self.bump();
            return Some(id);
        }
        None
    }

    /// True when the cursor is a comment that is the first non-whitespace on its source line.
    ///
    /// Byte-based, not `Token::line`-based: a multi-line string literal records its *start* line,
    /// so a genuinely trailing anchor after `const a = "x\ny"` would compare unequal and be
    /// wrongly rejected. The lexer also splits `{ // label   //@n:id` into two comment tokens and
    /// points the second at the embedded `//@`, whose line prefix is non-empty — so that
    /// renderer-emitted form is correctly treated as trailing. Do not "simplify" this to a column
    /// or token-index check.
    fn comment_starts_its_line(&self) -> bool {
        if !matches!(self.cur(), Tok::Comment(_)) {
            return false;
        }
        let byte = self.cur_token().byte;
        let line_start = self.src[..byte].rfind('\n').map_or(0, |i| i + 1);
        self.src[line_start..byte].trim().is_empty()
    }

    /// Consume a trailing non-anchor comment on `line` (a branch arm label) if present.
    ///
    /// Newlines are otherwise insignificant to the parser, but labels are deliberately trailing
    /// syntax (`{ // exec_success`). A normal first-line comment inside the block must stay in the
    /// block and must not turn a boolean `if` into the labelled call-branch form.
    fn take_label_on_line(&mut self, line: usize) -> Option<String> {
        if let Tok::Comment(text) = self.cur()
            && self.cur_token().line == line
            && !text.starts_with('@')
        {
            let label = text.clone();
            self.bump();
            return Some(label);
        }
        None
    }

    // ---- decorators -----------------------------------------------------------------------

    /// Parse zero or more leading `@decorator` lines. Most carry either no argument (`@secret`)
    /// or one string (`@category("…")`); `@cache` additionally accepts a settings object.
    fn decorators(&mut self) -> Result<Vec<Decorator>, ParseError> {
        let mut decorators = Vec::new();
        while matches!(self.cur(), Tok::At) {
            self.bump();
            let name = self.ident()?;
            let arg = if self.eat(&Tok::LParen) {
                let value = match (name.as_str(), self.cur().clone()) {
                    ("cache", Tok::LBrace) => DecoratorArg::Cache(self.cache_decorator_settings()?),
                    (_, Tok::Str(s)) => {
                        self.bump();
                        DecoratorArg::String(s)
                    }
                    other => {
                        return Err(self.err(format!(
                            "expected string decorator argument (or an object for `@cache`), found `{other:?}`"
                        )));
                    }
                };
                self.expect(&Tok::RParen)?;
                Some(value)
            } else {
                None
            };
            decorators.push(Decorator { name, arg });
        }
        Ok(decorators)
    }

    /// Parse the canonical cache settings object. Fields are deliberately parsed here instead of
    /// as a general expression so duplicate/unknown keys and invalid policy values fail with a
    /// precise decorator diagnostic.
    fn cache_decorator_settings(&mut self) -> Result<FunctionCache, ParseError> {
        self.expect(&Tok::LBrace)?;
        let mut settings = FunctionCache::default();
        let mut seen = HashSet::new();

        while !matches!(self.cur(), Tok::RBrace) {
            let field = self.arg_key()?;
            if !seen.insert(field.clone()) {
                return Err(self.err(format!("duplicate field `{field}` in `@cache` decorator")));
            }
            self.expect(&Tok::Colon)?;
            match field.as_str() {
                "namespace" => match self.cur().clone() {
                    Tok::Str(value) => {
                        self.bump();
                        settings.namespace = value;
                    }
                    other => {
                        return Err(self.err(format!(
                            "`@cache` field `namespace` must be a string, found `{other:?}`"
                        )));
                    }
                },
                "ttlSeconds" => match self.cur().clone() {
                    Tok::Int(value) if value >= 0 => {
                        self.bump();
                        settings.ttl_seconds = Some(value as u64);
                    }
                    Tok::UInt(value) => {
                        self.bump();
                        settings.ttl_seconds = Some(value);
                    }
                    other => {
                        return Err(self.err(format!(
                            "`@cache` field `ttlSeconds` must be a non-negative integer, found `{other:?}`"
                        )));
                    }
                },
                "scope" => match self.cur().clone() {
                    Tok::Str(value) if value == "app" => {
                        self.bump();
                        settings.scope = FunctionCacheScope::App;
                    }
                    Tok::Str(value) if value == "user" => {
                        self.bump();
                        settings.scope = FunctionCacheScope::User;
                    }
                    Tok::Str(value) => {
                        return Err(self.err(format!(
                            "`@cache` field `scope` must be \"app\" or \"user\", found {value:?}"
                        )));
                    }
                    other => {
                        return Err(self.err(format!(
                            "`@cache` field `scope` must be a string, found `{other:?}`"
                        )));
                    }
                },
                other => {
                    return Err(self.err(format!(
                        "unknown field `{other}` in `@cache` decorator; expected `namespace`, `ttlSeconds`, or `scope`"
                    )));
                }
            }

            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RBrace)?;
        Ok(settings)
    }

    /// Apply parsed decorators to a variable declaration, erroring on unknown ones or argument
    /// mismatches. Mirrors `render::var_decorators_of`.
    fn apply_var_decorators(
        &self,
        var: &mut VarDecl,
        decorators: &[Decorator],
    ) -> Result<(), ParseError> {
        for dec in decorators {
            match dec.name.as_str() {
                "secret" => {
                    self.expect_no_arg(dec)?;
                    var.secret = true;
                }
                "readonly" => {
                    self.expect_no_arg(dec)?;
                    var.editable = false;
                }
                "runtime" => {
                    self.expect_no_arg(dec)?;
                    var.runtime_configured = true;
                }
                "category" => var.category = Some(self.expect_arg(dec)?),
                "description" => var.description = Some(self.expect_arg(dec)?),
                "schema" => var.schema = Some(self.expect_arg(dec)?),
                other => return Err(self.err(format!("unknown decorator `@{other}`"))),
            }
        }
        Ok(())
    }

    /// Apply decorators that belong to a function declaration. Presence of `@cache` enables
    /// caching; a bare decorator uses FlowScript's global, five-minute, app-scoped defaults.
    fn apply_fn_decorators(
        &self,
        func: &mut FnDecl,
        decorators: &[Decorator],
    ) -> Result<(), ParseError> {
        for dec in decorators {
            match dec.name.as_str() {
                "cache" => {
                    if func.cache.is_some() {
                        return Err(self.err("duplicate `@cache` decorator on function"));
                    }
                    func.cache = Some(match &dec.arg {
                        None => FunctionCache::default(),
                        Some(DecoratorArg::Cache(settings)) => settings.clone(),
                        Some(DecoratorArg::String(_)) => {
                            return Err(self
                                .err("decorator `@cache` takes a settings object, not a string"));
                        }
                    });
                }
                other => {
                    return Err(self.err(format!("unknown decorator `@{other}` on function")));
                }
            }
        }
        Ok(())
    }

    fn expect_no_arg(&self, dec: &Decorator) -> Result<(), ParseError> {
        if dec.arg.is_some() {
            return Err(self.err(format!(
                "decorator `@{}` does not take an argument",
                dec.name
            )));
        }
        Ok(())
    }

    fn expect_arg(&self, dec: &Decorator) -> Result<String, ParseError> {
        match &dec.arg {
            Some(DecoratorArg::String(value)) => Ok(value.clone()),
            Some(DecoratorArg::Cache(_)) => Err(self.err(format!(
                "decorator `@{}` requires a string argument",
                dec.name
            ))),
            None => Err(self.err(format!(
                "decorator `@{}` requires a string argument",
                dec.name
            ))),
        }
    }

    // ---- top level ------------------------------------------------------------------------

    fn board(&mut self) -> Result<BoardAst, ParseError> {
        let mut ast = BoardAst::default();
        while !self.at_eof() {
            if self.eat(&Tok::Semi) {
                continue;
            }
            let decorators = self.decorators()?;
            match self.cur().clone() {
                Tok::Ident(kw) if kw == "use" => {
                    if !decorators.is_empty() {
                        return Err(self.err("decorators on `use` declarations are not supported"));
                    }
                    self.use_decls(&mut ast.uses)?;
                }
                Tok::Ident(kw) if kw == "interface" => {
                    if !decorators.is_empty() {
                        return Err(self.err("decorators on interfaces are not yet supported"));
                    }
                    ast.interfaces.push(self.interface_decl()?);
                }
                Tok::Ident(kw) if kw == "const" || kw == "let" => {
                    let mut var = self.var_decl(kw == "let")?;
                    self.apply_var_decorators(&mut var, &decorators)?;
                    ast.variables.push(var);
                }
                Tok::Ident(kw) if kw == "function" => {
                    let mut func = self.fn_decl()?;
                    self.apply_fn_decorators(&mut func, &decorators)?;
                    ast.functions.push(func);
                }
                Tok::Ident(_) if self.at_module_header() => {
                    if !decorators.is_empty() {
                        return Err(self.err("decorators on modules are not supported"));
                    }
                    ast.modules.push(self.module_decl()?);
                }
                Tok::Ident(_) => {
                    if !decorators.is_empty() {
                        return Err(self.err("decorators on events are not yet supported"));
                    }
                    ast.events.push(self.event_block()?);
                }
                Tok::Comment(_) => {
                    if !decorators.is_empty() {
                        return Err(self.err(
                            "decorators must be immediately followed by a variable declaration",
                        ));
                    }
                    // Stray top-level comment (no AST slot): skip.
                    self.bump();
                }
                other => {
                    return Err(self.err(format!("unexpected token at top level: `{other:?}`")));
                }
            }
        }
        apply_interface_schemas(&mut ast);
        Ok(ast)
    }

    // ---- declarations ---------------------------------------------------------------------

    /// `use a::b, c::*` — one or more comma-separated use-trees after a consumed-by-us `use`.
    fn use_decls(&mut self, out: &mut Vec<UseDecl>) -> Result<(), ParseError> {
        self.bump(); // use
        loop {
            out.push(self.use_tree()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        Ok(())
    }

    /// Rust `use`-tree subset: `a::b`, `a::b::*`, `a::b::{ x, y }`, `a::b as x`.
    fn use_tree(&mut self) -> Result<UseDecl, ParseError> {
        let mut path = vec![self.ident()?];
        while self.eat(&Tok::PathSep) {
            match self.cur().clone() {
                Tok::Op(op) if op == "*" => {
                    self.bump();
                    return Ok(UseDecl {
                        path,
                        kind: UseKind::Glob,
                    });
                }
                Tok::LBrace => {
                    self.bump();
                    let mut members = Vec::new();
                    while !matches!(self.cur(), Tok::RBrace) {
                        members.push(self.ident()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(&Tok::RBrace)?;
                    if members.is_empty() {
                        return Err(self.err("`use` member list must name at least one member"));
                    }
                    return Ok(UseDecl {
                        path,
                        kind: UseKind::Members(members),
                    });
                }
                _ => path.push(self.ident()?),
            }
        }
        if self.is_ident("as") {
            self.bump();
            return Ok(UseDecl {
                path,
                kind: UseKind::Alias(self.ident()?),
            });
        }
        Ok(UseDecl {
            path,
            kind: UseKind::Namespace,
        })
    }

    fn interface_decl(&mut self) -> Result<InterfaceDecl, ParseError> {
        self.bump(); // interface
        let name = self.ident()?;
        self.expect(&Tok::LBrace)?;
        let mut fields = Vec::new();
        while !matches!(self.cur(), Tok::RBrace) {
            // Non-identifier JSON-schema property names render as quoted strings.
            let field_name = match self.cur().clone() {
                Tok::Str(name) => {
                    self.bump();
                    name
                }
                _ => self.ident()?,
            };
            let optional = self.eat(&Tok::Question);
            self.expect(&Tok::Colon)?;
            let ty = self.interface_type()?;
            let default = if self.eat(&Tok::Assign) {
                Some(self.literal()?)
            } else {
                None
            };
            let _ = self.eat(&Tok::Semi) || self.eat(&Tok::Comma);
            fields.push(InterfaceField {
                name: field_name,
                ty,
                optional,
                default,
            });
        }
        self.bump(); // }
        let mut interface = InterfaceDecl {
            name,
            fields,
            schema: None,
        };
        interface.schema = schema_from_interface(&interface);
        Ok(interface)
    }

    /// Parse a variable declaration. `exposed` is set when the keyword was `let`.
    /// Assumes the `const`/`let` keyword is the current token.
    ///
    /// `const x = <literal>` without a type annotation infers the type from the literal and
    /// canonicalizes to the explicit `const x: string = "…"` form when rendered.
    fn var_decl(&mut self, exposed: bool) -> Result<VarDecl, ParseError> {
        self.bump(); // const | let
        let name = self.ident()?;
        let (ty, default) = if self.eat(&Tok::Assign) {
            let literal_token = self.cur_token().clone();
            let literal = self.literal()?;
            let Some(ty) = literal.inferred_type() else {
                return Err(ParseError::new(
                    format!(
                        "cannot infer the type of `{name}` from `null`; add a type annotation (e.g. `const {name}: string = null`)"
                    ),
                    literal_token.line,
                    literal_token.col,
                ));
            };
            (ty, Some(literal))
        } else {
            self.expect(&Tok::Colon)?;
            let ty = self.type_ref()?;
            let default = if self.eat(&Tok::Assign) {
                Some(self.literal()?)
            } else {
                None
            };
            (ty, default)
        };
        let anchor = self.take_anchor();
        Ok(VarDecl {
            name,
            ty,
            default,
            exposed,
            secret: false,
            editable: true,
            runtime_configured: false,
            category: None,
            description: None,
            schema: None,
            anchor,
        })
    }

    fn fn_decl(&mut self) -> Result<FnDecl, ParseError> {
        self.bump(); // function
        let name = self.ident()?;
        self.expect(&Tok::LParen)?;
        let params = self.params(&Tok::RParen)?;
        self.expect(&Tok::RParen)?;
        let returns = if self.eat(&Tok::Colon) {
            self.expect(&Tok::LParen)?;
            let returns = self.params(&Tok::RParen)?;
            self.expect(&Tok::RParen)?;
            returns
        } else {
            Vec::new()
        };
        self.expect(&Tok::LBrace)?;
        let anchor = self.take_anchor();
        let body = self.block_body()?;
        Ok(FnDecl {
            name,
            params,
            returns,
            body,
            cache: None,
            anchor,
        })
    }

    fn event_block(&mut self) -> Result<EventBlock, ParseError> {
        let name = self.ident()?;
        // Optional given name (`eventsSimple dashboardLoad() { }`): the first identifier selects
        // the event type, the second names this specific entry.
        let event_name = if matches!(self.cur(), Tok::Ident(_)) {
            Some(self.ident()?)
        } else {
            None
        };
        self.expect(&Tok::LParen)?;
        let params = self.params(&Tok::RParen)?;
        self.expect(&Tok::RParen)?;
        self.expect(&Tok::LBrace)?;
        let anchor = self.take_anchor();
        let body = self.block_body()?;
        Ok(EventBlock {
            name,
            node_type: String::new(),
            event_name,
            params,
            body,
            anchor,
        })
    }

    /// Whether the cursor opens a module block.
    ///
    /// `module` is a *contextual* keyword: it claims the statement only in the exact shape
    /// `module <ident> {` at a declaration position. Everywhere else (`module` as a binding, a pin
    /// key, an event name, a call argument) it stays an ordinary identifier, so adding modules
    /// cannot break a board that already uses the word.
    fn at_module_header(&self) -> bool {
        self.is_ident("module")
            && matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                Some(Tok::Ident(_))
            )
            && matches!(self.toks.get(self.pos + 2).map(|t| &t.tok), Some(Tok::LBrace))
    }

    /// `module name { … }` — assumes [`Self::at_module_header`] just returned true.
    fn module_decl(&mut self) -> Result<ModuleDecl, ParseError> {
        self.bump(); // module
        let name = self.ident()?;
        self.expect(&Tok::LBrace)?;
        let anchor = self.take_anchor();
        let mut decl = ModuleDecl {
            name,
            anchor,
            functions: Vec::new(),
            events: Vec::new(),
            modules: Vec::new(),
        };
        self.module_body(&mut decl)?;
        Ok(decl)
    }

    /// Parse a module body up to and including its closing `}`. Nesting is bounded by the same
    /// budget as blocks and expressions so user-authored input cannot overflow the stack.
    fn module_body(&mut self, decl: &mut ModuleDecl) -> Result<(), ParseError> {
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(self.err("module nesting too deep"));
        }
        self.depth += 1;
        let result = self.module_body_inner(decl);
        self.depth -= 1;
        result
    }

    fn module_body_inner(&mut self, decl: &mut ModuleDecl) -> Result<(), ParseError> {
        while !matches!(self.cur(), Tok::RBrace) {
            if self.at_eof() {
                return Err(self.err(format!(
                    "unexpected end of input inside module `{}`",
                    decl.name
                )));
            }
            if self.eat(&Tok::Semi) {
                continue;
            }
            let decorators = self.decorators()?;
            match self.cur().clone() {
                Tok::Ident(kw) if kw == "function" => {
                    let mut func = self.fn_decl()?;
                    self.apply_fn_decorators(&mut func, &decorators)?;
                    decl.functions.push(func);
                }
                Tok::Ident(kw) if kw == "const" || kw == "let" => {
                    return Err(self.err(format!(
                        "`{kw}` is not allowed inside a `module` block: variables are declared in main.flow"
                    )));
                }
                Tok::Ident(kw) if kw == "use" || kw == "interface" => {
                    return Err(self.err(format!(
                        "`{kw}` is not allowed inside a `module` block: use/interface declarations belong at the top of the file"
                    )));
                }
                Tok::Ident(_) if self.at_module_header() => {
                    if !decorators.is_empty() {
                        return Err(self.err("decorators on modules are not supported"));
                    }
                    decl.modules.push(self.module_decl()?);
                }
                Tok::Ident(_) => {
                    if !decorators.is_empty() {
                        return Err(self.err("decorators on events are not yet supported"));
                    }
                    decl.events.push(self.event_block()?);
                }
                Tok::Comment(_) => {
                    if !decorators.is_empty() {
                        return Err(self.err(
                            "decorators must be immediately followed by a declaration",
                        ));
                    }
                    // Stray comment inside a module body (no AST slot): skip.
                    self.bump();
                }
                other => {
                    return Err(self.err(format!(
                        "unexpected token inside module `{}`: `{other:?}`",
                        decl.name
                    )));
                }
            }
        }
        self.bump(); // }
        Ok(())
    }

    /// Parse a comma-separated `name: Type` parameter list up to (not including) `end`.
    fn params(&mut self, end: &Tok) -> Result<Vec<Param>, ParseError> {
        let mut params = Vec::new();
        while self.cur() != end {
            let name = self.ident()?;
            self.expect(&Tok::Colon)?;
            let ty = self.type_ref()?;
            params.push(Param { name, ty });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        Ok(params)
    }

    fn type_ref(&mut self) -> Result<TypeRef, ParseError> {
        let base = self.ident()?;
        // `Map<string, T>` / `Set<T>` containers.
        if base == "Map" && self.cur() == &Tok::Op("<".to_string()) {
            self.bump(); // <
            let _ = self.ident()?; // key type (always `string` in render)
            self.expect(&Tok::Comma)?;
            let inner = self.ident()?;
            self.expect(&Tok::Op(">".to_string()))?;
            return Ok(TypeRef::new(inner, Container::Map));
        }
        if base == "Set" && self.cur() == &Tok::Op("<".to_string()) {
            self.bump(); // <
            let inner = self.ident()?;
            self.expect(&Tok::Op(">".to_string()))?;
            return Ok(TypeRef::new(inner, Container::Set));
        }
        if self.cur() == &Tok::LBracket {
            self.bump();
            self.expect(&Tok::RBracket)?;
            return Ok(TypeRef::new(base, Container::Array));
        }
        Ok(TypeRef::new(base, Container::Normal))
    }

    fn interface_type(&mut self) -> Result<InterfaceType, ParseError> {
        let mut members = vec![self.interface_type_primary()?];
        while self.cur() == &Tok::Op("|".to_string()) {
            self.bump();
            members.push(self.interface_type_primary()?);
        }
        if members.len() == 1 {
            Ok(members.pop().unwrap())
        } else {
            Ok(InterfaceType::Union(members))
        }
    }

    fn interface_type_primary(&mut self) -> Result<InterfaceType, ParseError> {
        let mut ty = match self.cur().clone() {
            // Grouping, e.g. `(string | null)[]` — the renderer parenthesises unions
            // under an array suffix so they don't reparse as `string | (null[])`.
            // Count the group against the recursion budget: parenthesised types recurse into
            // `interface_type`, so deeply nested `((((…))))` would otherwise bypass the limit
            // and overflow the stack on user-authored input.
            Tok::LParen => {
                if self.depth >= MAX_NESTING_DEPTH {
                    return Err(self.err("interface type nesting too deep"));
                }
                self.depth += 1;
                self.bump();
                let inner = self.interface_type();
                self.depth -= 1;
                let inner = inner?;
                self.expect(&Tok::RParen)?;
                inner
            }
            Tok::Str(value) => {
                self.bump();
                InterfaceType::StringLiteral(value)
            }
            Tok::Ident(name) if name == "null" => {
                self.bump();
                InterfaceType::Null
            }
            Tok::Ident(name) if name == "any" => {
                self.bump();
                InterfaceType::Any
            }
            Tok::Ident(name) if name == "Map" => {
                self.bump();
                self.expect(&Tok::Op("<".to_string()))?;
                let key = self.ident()?;
                if key != "string" {
                    return Err(self.err("interface Map key type must be `string`"));
                }
                self.expect(&Tok::Comma)?;
                let inner = self.interface_type()?;
                self.expect(&Tok::Op(">".to_string()))?;
                InterfaceType::Map(Box::new(inner))
            }
            Tok::Ident(name) => {
                self.bump();
                InterfaceType::Named(name)
            }
            other => {
                return Err(self.err(format!("unexpected token in interface type: `{other:?}`")));
            }
        };

        while self.cur() == &Tok::LBracket {
            self.bump();
            self.expect(&Tok::RBracket)?;
            ty = InterfaceType::Array(Box::new(ty));
        }

        Ok(ty)
    }

    // ---- blocks & statements --------------------------------------------------------------

    /// Parse statements until a closing `}` (which is consumed). Assumes the opening `{`
    /// (and any trailing label/anchor) was already consumed.
    fn block_body(&mut self) -> Result<Block, ParseError> {
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(self.err("block nesting too deep"));
        }
        self.depth += 1;
        let result = self.block_body_inner();
        self.depth -= 1;
        result
    }

    fn block_body_inner(&mut self) -> Result<Block, ParseError> {
        let mut stmts = Vec::new();
        while !matches!(self.cur(), Tok::RBrace) {
            if self.at_eof() {
                return Err(self.err("unexpected end of input inside block"));
            }
            if self.eat(&Tok::Semi) {
                continue;
            }
            if matches!(self.cur(), Tok::LBrace) {
                self.bump();
                let nested = self.block_body()?;
                stmts.extend(nested.stmts);
                continue;
            }
            stmts.push(self.stmt()?);
        }
        self.bump(); // }
        Ok(Block { stmts })
    }

    fn stmt(&mut self) -> Result<Stmt, ParseError> {
        // Leading decorators bind to a following local `let` declaration, or `@parallel` to a
        // sugared `for` loop.
        if matches!(self.cur(), Tok::At) {
            let decorators = self.decorators()?;
            if self.is_ident("for") {
                return self.parallel_for_stmt(&decorators);
            }
            if !self.is_ident("let") {
                return Err(self.err(
                    "decorators are only supported on `let` declarations and `@parallel for` loops",
                ));
            }
            let mut var = self.local_decl()?;
            self.apply_var_decorators(&mut var, &decorators)?;
            return Ok(Stmt::Local(var));
        }
        match self.cur().clone() {
            Tok::Comment(text) => {
                self.bump();
                Ok(Stmt::Comment(text))
            }
            Tok::Ident(kw) if kw == "use" => {
                Err(self.err("`use` declarations are only allowed at the top level"))
            }
            Tok::Ident(kw) if kw == "const" => self.let_stmt(),
            Tok::Ident(kw) if kw == "let" => self.local_or_assignment_stmt(),
            Tok::Ident(kw) if kw == "return" => self.return_stmt(),
            Tok::Ident(kw) if kw == "if" => self.branch_stmt(),
            Tok::Ident(kw) if kw == "for" => self.for_stmt(),
            Tok::Ident(kw) if kw == "while" => self.while_stmt(),
            Tok::Ident(_) => self.ident_stmt(),
            other => Err(self.err(format!("unexpected token in block: `{other:?}`"))),
        }
    }

    /// `const name = call(...)` — an SSA binding of an impure node output.
    /// `const name = expr` is accepted as model-authored alias sugar and canonicalizes to
    /// `let name = expr` when rendered.
    fn let_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.bump(); // const
        if let Some(stmt) = self.destructure_stmt()? {
            return Ok(stmt);
        }
        let name = self.ident()?;
        self.expect(&Tok::Assign)?;
        let value = self.expr()?;
        let anchor = self.take_anchor();
        match value {
            Expr::Call(call) => Ok(Stmt::Let { name, call, anchor }),
            value => Ok(Stmt::LocalAlias {
                name,
                value,
                anchor,
            }),
        }
    }

    /// `{ a, b: c } = call(...)` after a consumed `const`/`let`. Fields bind output pins by name
    /// (`{ pin }` or `{ pin: name }`). Returns `None` when the cursor is not a pattern. Array
    /// patterns are rejected on purpose: they would depend on output pin order, which is not a
    /// user-visible contract.
    fn destructure_stmt(&mut self) -> Result<Option<Stmt>, ParseError> {
        match self.cur() {
            Tok::LBrace => {}
            Tok::LBracket => {
                return Err(self.err(
                    "array destructuring is not supported; use object destructuring by output name (`const { a, b } = call(...)`)",
                ));
            }
            _ => return Ok(None),
        }
        self.bump(); // {
        let mut fields = Vec::new();
        while !matches!(self.cur(), Tok::RBrace) {
            let pin = self.ident()?;
            let name = if self.eat(&Tok::Colon) {
                self.ident()?
            } else {
                pin.clone()
            };
            fields.push(DestructureField { pin, name });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RBrace)?;
        if fields.is_empty() {
            return Err(self.err("a destructuring pattern must bind at least one output"));
        }
        self.expect(&Tok::Assign)?;
        let value_token = self.cur_token().clone();
        let Expr::Call(call) = self.expr()? else {
            return Err(ParseError::new(
                "object destructuring requires a call on the right-hand side",
                value_token.line,
                value_token.col,
            ));
        };
        let anchor = self.take_anchor();
        Ok(Some(Stmt::Destructure {
            fields,
            call,
            anchor,
        }))
    }

    /// `let name: Type = default` — a function-local variable declaration.
    fn local_decl(&mut self) -> Result<VarDecl, ParseError> {
        self.bump(); // let
        let name = self.ident()?;
        self.expect(&Tok::Colon)?;
        let ty = self.type_ref()?;
        let default = if self.eat(&Tok::Assign) {
            Some(self.literal()?)
        } else {
            None
        };
        let anchor = self.take_anchor();
        Ok(VarDecl {
            name,
            ty,
            default,
            exposed: false,
            secret: false,
            editable: true,
            runtime_configured: false,
            category: None,
            description: None,
            schema: None,
            anchor,
        })
    }

    /// `let name: Type = default` or `let name = expr`.
    fn local_or_assignment_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.bump(); // let
        if let Some(stmt) = self.destructure_stmt()? {
            return Ok(stmt);
        }
        let name = self.ident()?;
        if matches!(self.cur(), Tok::Assign) {
            self.bump(); // =
            let value = self.expr()?;
            let anchor = self.take_anchor();
            return Ok(Stmt::LocalAlias {
                name,
                value,
                anchor,
            });
        }

        self.expect(&Tok::Colon)?;
        let ty = self.type_ref()?;
        let default = if self.eat(&Tok::Assign) {
            Some(self.literal()?)
        } else {
            None
        };
        let anchor = self.take_anchor();
        Ok(Stmt::Local(VarDecl {
            name,
            ty,
            default,
            exposed: false,
            secret: false,
            editable: true,
            runtime_configured: false,
            category: None,
            description: None,
            schema: None,
            anchor,
        }))
    }

    fn return_stmt(&mut self) -> Result<Stmt, ParseError> {
        let return_line = self.line();
        self.bump(); // return
        let mut values = Vec::new();
        // A value list, if present, sits on the same source line as `return`.
        if !matches!(self.cur(), Tok::RBrace | Tok::Semi) && self.line() == return_line {
            values.push(self.expr()?);
            while self.eat(&Tok::Comma) {
                values.push(self.expr()?);
            }
        }
        let anchor = self.take_anchor();
        Ok(Stmt::Return { values, anchor })
    }

    /// Detect a nested event-handler header (`name(params) { … }`). A call argument is never an
    /// identifier directly followed by `:` (named arguments live inside `{ … }`), so a bare typed
    /// parameter list (`name(ident:`) or an empty list whose body is not a branch-arm map
    /// unambiguously marks a handler.
    fn looks_like_handler(&self) -> bool {
        if !matches!(self.cur(), Tok::Ident(_)) {
            return false;
        }
        // A named handler (`eventsSimple dashboardLoad(...)`) carries a second identifier before
        // the parameter list; nothing else places two bare identifiers back to back.
        let lparen = if matches!(
            self.toks.get(self.pos + 1).map(|t| &t.tok),
            Some(Tok::Ident(_))
        ) {
            self.pos + 2
        } else {
            self.pos + 1
        };
        if !matches!(self.toks.get(lparen).map(|t| &t.tok), Some(Tok::LParen)) {
            return false;
        }
        // Empty params `name()` — a handler unless the following block is a branch-arm map.
        if matches!(self.toks.get(lparen + 1).map(|t| &t.tok), Some(Tok::RParen)) {
            if !matches!(self.toks.get(lparen + 2).map(|t| &t.tok), Some(Tok::LBrace)) {
                return false;
            }
            return !self.brace_opens_branch_arms(lparen + 2);
        }
        // Typed params `name(ident : …` — never produced by an object-arg call.
        matches!(
            self.toks.get(lparen + 1).map(|t| &t.tok),
            Some(Tok::Ident(_))
        ) && matches!(self.toks.get(lparen + 2).map(|t| &t.tok), Some(Tok::Colon))
    }

    /// True if the block opened at `brace_pos` (a `{`) begins a branch-arm map (`label: { … }`),
    /// skipping an immediately-trailing anchor/label comment.
    fn brace_opens_branch_arms(&self, brace_pos: usize) -> bool {
        let mut i = brace_pos + 1;
        if matches!(self.toks.get(i).map(|t| &t.tok), Some(Tok::Comment(_))) {
            i += 1;
        }
        matches!(self.toks.get(i).map(|t| &t.tok), Some(Tok::Ident(_)))
            && matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Colon))
            && matches!(self.toks.get(i + 2).map(|t| &t.tok), Some(Tok::LBrace))
    }

    /// A statement beginning with an identifier: an assignment, a call, or an N-way branch.
    fn ident_stmt(&mut self) -> Result<Stmt, ParseError> {
        // A nested event handler (`name(params) { … }`) — an independent trigger entry that
        // closes over the enclosing scope, distinct from an object-arg call/branch.
        if self.looks_like_handler() {
            return Ok(Stmt::Handler(self.event_block()?));
        }
        // Assignment: `target = expr` (the target is always a bare variable name). The compound
        // form `target += expr` desugars to `target = target + expr`.
        match self.toks.get(self.pos + 1).map(|t| t.tok.clone()) {
            Some(Tok::Assign) => {
                let target = self.ident()?;
                self.bump(); // =
                let value = self.expr()?;
                let anchor = self.take_anchor();
                return Ok(Stmt::Assign {
                    target,
                    value,
                    anchor,
                });
            }
            Some(Tok::CompoundAssign(op)) => {
                let target = self.ident()?;
                self.bump(); // op=
                let rhs = self.expr()?;
                let anchor = self.take_anchor();
                return Ok(Stmt::Assign {
                    value: compound_assign_value(op, Expr::Ref(target.clone()), rhs),
                    target,
                    anchor,
                });
            }
            _ => {}
        }
        let value = self.expr()?;
        // `base.field = expr` (or `base.a.b`, `base.items[0]`) — a struct-field write. Kept as a
        // first-class `Stmt::FieldAssign` (round-trips back to the dot form); reconcile expands it
        // to `structSet({ structIn: base, field: "path", value })` and rebinds `base`.
        if matches!(self.cur(), Tok::Assign | Tok::CompoundAssign(_)) {
            let (base, path) = lvalue_to_field_path(&value).filter(|(_, p)| !p.is_empty()).ok_or_else(
                || self.err("assignment target must be a variable or a struct field path (e.g. `x.field`)"),
            )?;
            let compound = match self.bump() {
                Tok::CompoundAssign(op) => Some(op),
                _ => None,
            };
            let rhs = self.expr()?;
            let anchor = self.take_anchor();
            let rhs = match compound {
                Some(op) => compound_assign_value(op, value, rhs),
                None => rhs,
            };
            return Ok(Stmt::FieldAssign {
                base,
                path,
                value: rhs,
                anchor,
            });
        }
        // `call(...) { … }` — a general N-way branch fan-out.
        if matches!(self.cur(), Tok::LBrace) {
            self.bump(); // {
            // Arm-only body: an own-line anchor here is unambiguous, so stay permissive.
            // See `take_anchor_before_arms`. This is the single exempt call site.
            let anchor = self.take_anchor_before_arms();
            let mut arms = Vec::new();
            while !matches!(self.cur(), Tok::RBrace) {
                let label = self.ident()?;
                self.expect(&Tok::Colon)?;
                self.expect(&Tok::LBrace)?;
                let body = self.block_body()?;
                arms.push(BranchArm { label, body });
            }
            self.bump(); // }
            return match value {
                Expr::Call(call) => Ok(Stmt::Branch {
                    bind: None,
                    call,
                    condition: None,
                    arms,
                    anchor,
                }),
                Expr::Ref(bind) => Ok(Stmt::Branch {
                    bind: Some(bind),
                    call: placeholder_call(),
                    condition: None,
                    arms,
                    anchor,
                }),
                _ => Err(self.err("branch fan-out requires a call or local branch binding")),
            };
        }
        let call = match value {
            Expr::Call(call) => call,
            _ => return Err(self.err("expected a call statement")),
        };
        let anchor = self.take_anchor();
        Ok(Stmt::Call { call, anchor })
    }

    fn branch_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.bump(); // if
        self.expect(&Tok::LParen)?;
        let leading_bang = matches!(self.cur(), Tok::Bang);
        let cond = self.expr()?;
        self.expect(&Tok::RParen)?;
        // A leading `!` spanning the WHOLE condition is the renderer's single-`False`-arm form
        // (`if (!(cond)) { … }`). `!a && b` parses to a binary root and an author-written
        // `boolNot(…)` carries no leading bang, so neither is re-sugared into it: doing so would
        // delete a real `bool_not` node and flip the arm True -> False.
        let negated = leading_bang
            .then(|| not_call_operand(&cond).cloned())
            .flatten();
        let true_brace_line = self.line();
        self.expect(&Tok::LBrace)?;
        // A trailing non-anchor comment is an exec-pin LABEL only in the labelled call-branch
        // form, which the renderer emits solely when `condition` is `None`. On a boolean or
        // negated condition the renderer never emits one, so it is ordinary user text — but it
        // must still be LIFTED off the brace line, because the anchor can sit behind it
        // (`{ // note   //@n:id`) and `take_anchor` only inspects the cursor. Re-insert it as the
        // block's first statement.
        let condition_is_call = negated.is_none() && matches!(cond, Expr::Call(_));
        let brace_comment = self.take_label_on_line(true_brace_line);
        let (true_label, leading_note) = if condition_is_call {
            (brace_comment, None)
        } else {
            (None, brace_comment)
        };
        let anchor = self.take_anchor();
        let mut true_body = self.block_body()?;
        if let Some(note) = leading_note {
            true_body.stmts.insert(0, Stmt::Comment(note));
        }

        let mut else_label = None;
        let mut else_body = None;
        if self.is_ident("else") {
            self.bump(); // else
            if self.is_ident("if") {
                // `else if (c) { … }` desugars to the nested `else { if (c) { … } }` ladder the
                // renderer emits. `expr()` restores `self.depth` on the way out, so a ladder
                // accumulates no budget of its own — this guard IS what bounds the recursion.
                if self.depth >= MAX_NESTING_DEPTH {
                    return Err(self.err("block nesting too deep"));
                }
                self.depth += 1;
                let nested = self.branch_stmt();
                self.depth -= 1;
                else_body = Some(Block {
                    stmts: vec![nested?],
                });
            } else {
                let else_brace_line = self.line();
                self.expect(&Tok::LBrace)?;
                let else_comment = self.take_label_on_line(else_brace_line);
                let mut body = self.block_body()?;
                // Gate on `true_label.is_some()`, NOT on `condition_is_call`: with a call
                // condition and no true label the old code took this comment and then never read
                // it, silently deleting user text.
                if true_label.is_some() {
                    else_label = else_comment;
                } else if let Some(note) = else_comment {
                    body.stmts.insert(0, Stmt::Comment(note));
                }
                else_body = Some(body);
            }
        }

        // Negated single-arm form: the branch's `False` exec output is the only connected arm.
        // With an `else` both arms exist, so the negation stays inside the condition instead.
        if let Some(inner) = negated
            && else_body.is_none()
        {
            return Ok(Stmt::Branch {
                bind: None,
                call: placeholder_call(),
                condition: Some(inner),
                arms: vec![BranchArm {
                    label: "False".to_string(),
                    body: true_body,
                }],
                anchor,
            });
        }

        if let Some(label) = true_label {
            // Labelled form: the condition expression is the branch node call itself.
            let call = match cond {
                Expr::Call(call) => call,
                _ => return Err(self.err("labelled branch requires a call condition")),
            };
            let mut arms = vec![BranchArm {
                label,
                body: true_body,
            }];
            if let Some(body) = else_body {
                arms.push(BranchArm {
                    label: else_label.unwrap_or_default(),
                    body,
                });
            }
            return Ok(Stmt::Branch {
                bind: None,
                call,
                condition: None,
                arms,
                anchor,
            });
        }

        // Sugared boolean condition form (`if (cond) { } [else { }]`).
        let arms = if let Some(body) = else_body {
            vec![
                BranchArm {
                    label: "True".to_string(),
                    body: true_body,
                },
                BranchArm {
                    label: "False".to_string(),
                    body,
                },
            ]
        } else {
            vec![BranchArm {
                label: "True".to_string(),
                body: true_body,
            }]
        };
        Ok(Stmt::Branch {
            bind: None,
            call: placeholder_call(),
            condition: Some(cond),
            arms,
            anchor,
        })
    }

    /// `@parallel for (…)` — the sugared `for…of` over `control_par_for_each`. Only the sugared
    /// head is accepted: an explicit loop-node call already names its node type.
    fn parallel_for_stmt(&mut self, decorators: &[Decorator]) -> Result<Stmt, ParseError> {
        for dec in decorators {
            if dec.name != "parallel" {
                return Err(self.err(format!("unknown decorator `@{}` on a loop", dec.name)));
            }
            self.expect_no_arg(dec)?;
        }
        let for_token = self.cur_token().clone();
        let mut stmt = self.for_stmt()?;
        let Stmt::Loop {
            keyword, iterable, ..
        } = &mut stmt
        else {
            unreachable!("for_stmt returns a loop");
        };
        if iterable.is_none() {
            return Err(ParseError::new(
                "`@parallel` applies to the sugared `for (const item of array)` form; an explicit loop-node call already names its node type",
                for_token.line,
                for_token.col,
            ));
        }
        *keyword = PARALLEL_FOR_EACH_KEYWORD.to_string();
        Ok(stmt)
    }

    /// `for (const item of array)`, `for (const [i, item] of array)` or the explicit
    /// `for (const handle of loopCall(…))`. The parser cannot tell a loop-node call from a pure
    /// call returning an array, so a plain-identifier head whose iterable is a call stays the
    /// handle form and reconcile decides by the resolved node type.
    fn for_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.bump(); // for
        self.expect(&Tok::LParen)?;
        if !self.is_ident("const") {
            return Err(self.err("expected `const` in for-of loop"));
        }
        self.bump(); // const
        let (index, name) = if self.eat(&Tok::LBracket) {
            let index = self.ident()?;
            self.expect(&Tok::Comma)?;
            let element = self.ident()?;
            self.expect(&Tok::RBracket)?;
            (Some(index), element)
        } else {
            (None, self.ident()?)
        };
        if !self.is_ident("of") {
            return Err(self.err("expected `of` in for-of loop"));
        }
        self.bump(); // of
        let head = self.expr()?;
        self.expect(&Tok::RParen)?;
        self.expect(&Tok::LBrace)?;
        let anchor = self.take_anchor();
        let body = self.block_body()?;
        let keyword = FOR_EACH_KEYWORD.to_string();
        match head {
            Expr::Call(call) if index.is_none() => Ok(Stmt::Loop {
                keyword,
                bind: Some(name),
                call,
                iterable: None,
                element: None,
                index: None,
                body,
                anchor,
            }),
            head => Ok(Stmt::Loop {
                keyword,
                bind: None,
                call: placeholder_call(),
                iterable: Some(head),
                element: Some(name),
                index,
                body,
                anchor,
            }),
        }
    }

    /// `while (cond)` or the explicit `while (loopCall(…))`; see `for_stmt` for why a call head
    /// is kept as the call form.
    fn while_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.bump(); // while
        self.expect(&Tok::LParen)?;
        let head = self.expr()?;
        self.expect(&Tok::RParen)?;
        self.expect(&Tok::LBrace)?;
        let anchor = self.take_anchor();
        let body = self.block_body()?;
        let keyword = WHILE_KEYWORD.to_string();
        match head {
            Expr::Call(call) => Ok(Stmt::Loop {
                keyword,
                bind: None,
                call,
                iterable: None,
                element: None,
                index: None,
                body,
                anchor,
            }),
            head => Ok(Stmt::Loop {
                keyword,
                bind: None,
                call: placeholder_call(),
                iterable: Some(head),
                element: None,
                index: None,
                body,
                anchor,
            }),
        }
    }

    // ---- expressions (Pratt) --------------------------------------------------------------

    fn expr(&mut self) -> Result<Expr, ParseError> {
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(self.err("expression nesting too deep"));
        }
        self.depth += 1;
        let result = self.ternary();
        self.depth -= 1;
        result
    }

    fn ternary(&mut self) -> Result<Expr, ParseError> {
        let cond = self.binary()?;
        if matches!(self.cur(), Tok::Question) {
            self.bump();
            let then = self.binary()?;
            self.expect(&Tok::Colon)?;
            let otherwise = self.binary()?;
            return Ok(Expr::Ternary {
                cond: Box::new(cond),
                then: Box::new(then),
                otherwise: Box::new(otherwise),
            });
        }
        Ok(cond)
    }

    /// Parse binary expressions with JavaScript-like precedence. FlowScript's renderer
    /// parenthesises nested binary operands, but model-authored source frequently omits those
    /// redundant parentheses (`a == b && c == d`), so the reader must still preserve the usual
    /// operator semantics.
    fn binary(&mut self) -> Result<Expr, ParseError> {
        self.binary_precedence(0)
    }

    fn binary_precedence(&mut self, minimum: u8) -> Result<Expr, ParseError> {
        let mut lhs = self.unary()?;
        while let Tok::Op(op) = self.cur().clone() {
            let Some(precedence) = binary_operator_precedence(&op) else {
                break;
            };
            if precedence < minimum {
                break;
            }
            self.bump();
            if self.depth >= MAX_NESTING_DEPTH {
                return Err(self.err("binary expression nesting too deep"));
            }
            self.depth += 1;
            // Exponentiation is right-associative; all other supported operators are
            // left-associative.
            let next_minimum = if op == "**" {
                precedence
            } else {
                precedence + 1
            };
            let rhs = self.binary_precedence(next_minimum);
            self.depth -= 1;
            let rhs = rhs?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    /// Prefix `!` and `-`.
    ///
    /// `!` desugars to a `boolNot({ boolean: … })` call, which is exactly what the renderer
    /// emits for a pure single-data-output node — so the result is a fixpoint. `Tok::Bang` was
    /// previously reachable only in `branch_stmt`'s negated form, which is what makes routing
    /// `binary_precedence` through here non-breaking by construction.
    ///
    /// `-x` desugars to `0 - x` (canonical rendering): reconcile picks `int_subtract` or
    /// `float_subtract` from the operand type, which the parser cannot do blind. A negative
    /// literal such as `-1` lexes as one token and never reaches this arm.
    fn unary(&mut self) -> Result<Expr, ParseError> {
        let negation = match self.cur() {
            Tok::Bang => Some(false),
            Tok::Op(op) if op == "-" => Some(true),
            _ => None,
        };
        let Some(numeric) = negation else {
            return self.postfix();
        };
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(self.err("expression nesting too deep"));
        }
        self.bump(); // ! | -
        self.depth += 1;
        let operand = self.unary();
        self.depth -= 1;
        let operand = operand?;
        Ok(if numeric {
            Expr::Binary {
                op: "-".to_string(),
                lhs: Box::new(Expr::Literal(Literal::Int(0))),
                rhs: Box::new(operand),
            }
        } else {
            not_call(operand)
        })
    }

    fn postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.primary()?;
        loop {
            match self.cur() {
                Tok::Dot => {
                    self.bump();
                    let name = self.ident()?;
                    // `receiver.method(...)` — the member is a call, not an access.
                    if matches!(self.cur(), Tok::LParen) {
                        expr = self.call_tail(name, Vec::new(), Some(Box::new(expr)))?;
                        continue;
                    }
                    // A camelCase-stable identifier could be either an output-pin selection or a
                    // struct data field; they render identically, so pick `Member` only when the
                    // key carries a separator (which a pin name never would).
                    expr = if is_camel_fixed_point(&name) {
                        Expr::Field {
                            base: Box::new(expr),
                            pin: name,
                        }
                    } else {
                        Expr::Member {
                            base: Box::new(expr),
                            field: name,
                        }
                    };
                }
                Tok::LBracket => {
                    self.bump();
                    let index = self.expr()?;
                    self.expect(&Tok::RBracket)?;
                    // The renderer uses bracketed string syntax for struct keys that cannot be
                    // written as a plain identifier (`value["row-rejection-reason"]`). Preserve
                    // that as `Member`, the same AST shape the renderer started from. Numeric or
                    // dynamic brackets remain collection indexes.
                    expr = match index {
                        Expr::Literal(Literal::String(field)) => Expr::Member {
                            base: Box::new(expr),
                            field,
                        },
                        index => Expr::Index {
                            base: Box::new(expr),
                            index: Box::new(index),
                        },
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.cur().clone() {
            Tok::Str(_) | Tok::Int(_) | Tok::Float(_) => Ok(Expr::Literal(self.literal()?)),
            Tok::Template(pieces) => {
                self.bump();
                self.template_expr(&pieces)
            }
            Tok::LParen => {
                self.bump();
                let inner = self.expr()?;
                self.expect(&Tok::RParen)?;
                Ok(inner)
            }
            Tok::LBrace => self.object_or_json(),
            Tok::LBracket => self.array_or_json(),
            Tok::Ident(name) => match name.as_str() {
                "true" => {
                    self.bump();
                    Ok(Expr::Literal(Literal::Bool(true)))
                }
                "false" => {
                    self.bump();
                    Ok(Expr::Literal(Literal::Bool(false)))
                }
                "null" => {
                    self.bump();
                    Ok(Expr::Literal(Literal::Null))
                }
                _ => {
                    let start = self.cur_token().clone();
                    self.bump();
                    if matches!(self.cur(), Tok::PathSep) {
                        self.path_call(name, &start)
                    } else if matches!(self.cur(), Tok::LParen) {
                        self.call_tail(name, Vec::new(), None)
                    } else {
                        Ok(Expr::Ref(name))
                    }
                }
            },
            other => Err(self.err(format!("unexpected token in expression: `{other:?}`"))),
        }
    }

    /// Assemble a template literal from its lexed pieces, parsing every `${ … }` as a full
    /// expression with its own cursor over the interpolation source. Diagnostics inside an
    /// interpolation are re-based onto the enclosing document.
    fn template_expr(&mut self, pieces: &[TemplatePiece]) -> Result<Expr, ParseError> {
        let mut parts = Vec::with_capacity(pieces.len());
        for piece in pieces {
            match piece {
                TemplatePiece::Text(text) => parts.push(TemplatePart::Text(text.clone())),
                TemplatePiece::Expr { src, line, col } => {
                    let expr = self.parse_interpolation(src).map_err(|err| {
                        let col = if err.line == 1 {
                            err.col + col - 1
                        } else {
                            err.col
                        };
                        ParseError::new(err.message, err.line + line - 1, col)
                    })?;
                    parts.push(TemplatePart::Expr(expr));
                }
            }
        }
        Ok(Expr::Template { parts })
    }

    fn parse_interpolation(&self, src: &str) -> Result<Expr, ParseError> {
        let mut inner = Parser {
            src,
            toks: lex(src)?,
            pos: 0,
            depth: self.depth,
        };
        let expr = inner.expr()?;
        if !inner.at_eof() {
            return Err(inner.err(format!(
                "unexpected token after `${{ … }}` expression: `{:?}`",
                inner.cur()
            )));
        }
        Ok(expr)
    }

    /// `a::b::c(...)` — the first segment was already consumed and the cursor is on `::`.
    /// A path is only ever a callee: there is no value a bare `a::b` could denote.
    fn path_call(&mut self, first: String, start: &Token) -> Result<Expr, ParseError> {
        let mut path = vec![first];
        while self.eat(&Tok::PathSep) {
            path.push(self.ident()?);
        }
        if !matches!(self.cur(), Tok::LParen) {
            return Err(ParseError::new(
                format!(
                    "namespace paths can only be called: expected `(` after `{}`",
                    path.join("::")
                ),
                start.line,
                start.col,
            ));
        }
        let display = path.pop().expect("a path has at least two segments");
        self.call_tail(display, path, None)
    }

    /// Parse the `(...)` tail of a call whose callee was already consumed.
    ///
    /// `call_args := expr ("," expr)* ("," named_obj)? | named_obj | ε` — a `{ … }` in the LAST
    /// argument slot is the named-argument object (the JS options-object convention and the
    /// renderer's sole form); a `{ … }` anywhere earlier is a positional struct-literal value.
    fn call_tail(
        &mut self,
        display: String,
        path: Vec<String>,
        receiver: Option<Box<Expr>>,
    ) -> Result<Expr, ParseError> {
        self.expect(&Tok::LParen)?;
        let mut positional = Vec::new();
        let mut args = Vec::new();
        while !matches!(self.cur(), Tok::RParen) {
            if matches!(self.cur(), Tok::LBrace) && self.brace_is_last_call_argument() {
                args = self.named_args()?;
                self.eat(&Tok::Comma);
                break;
            }
            positional.push(self.expr()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen)?;
        Ok(Expr::Call(Call {
            node_type: String::new(),
            display,
            path,
            receiver,
            positional,
            args,
            anchor: None,
        }))
    }

    /// True when the `{` at the cursor closes the argument list: its matching `}` is followed by
    /// `)`, optionally after a trailing comma. Brace depth alone suffices — every other bracket
    /// kind balances inside it, and string contents are already single tokens.
    fn brace_is_last_call_argument(&self) -> bool {
        let mut depth = 0usize;
        let mut index = self.pos;
        while let Some(token) = self.toks.get(index) {
            match token.tok {
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let mut next = index + 1;
                        if matches!(self.toks.get(next).map(|t| &t.tok), Some(Tok::Comma)) {
                            next += 1;
                        }
                        return matches!(self.toks.get(next).map(|t| &t.tok), Some(Tok::RParen));
                    }
                }
                Tok::Eof => return false,
                _ => {}
            }
            index += 1;
        }
        false
    }

    /// The trailing `{ name: value, … }` argument object.
    fn named_args(&mut self) -> Result<Vec<Arg>, ParseError> {
        self.expect(&Tok::LBrace)?;
        let mut args = Vec::new();
        while !matches!(self.cur(), Tok::RBrace) {
            let name = self.arg_key()?;
            self.expect(&Tok::Colon)?;
            let value = self.expr()?;
            args.push(Arg { name, value });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RBrace)?;
        Ok(args)
    }

    fn arg_key(&mut self) -> Result<String, ParseError> {
        match self.cur().clone() {
            Tok::Ident(name) => {
                self.bump();
                Ok(name)
            }
            Tok::Str(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.err(format!("expected argument name, found `{other:?}`"))),
        }
    }

    /// Disambiguate a `{ … }` between a canonical-JSON literal default and a struct literal.
    fn object_or_json(&mut self) -> Result<Expr, ParseError> {
        if let Some(raw) = self.try_canonical_json()? {
            return Ok(Expr::Literal(Literal::Json(raw)));
        }
        self.expect(&Tok::LBrace)?;
        let mut fields = Vec::new();
        while !matches!(self.cur(), Tok::RBrace) {
            let key = self.arg_key()?;
            self.expect(&Tok::Colon)?;
            let value = self.expr()?;
            fields.push(ObjectField { key, value });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RBrace)?;
        Ok(Expr::Object(fields))
    }

    /// Disambiguate a `[ … ]` between a canonical-JSON literal and a reference/expression array.
    fn array_or_json(&mut self) -> Result<Expr, ParseError> {
        if let Some(raw) = self.try_canonical_json()? {
            return Ok(Expr::Literal(Literal::Json(raw)));
        }
        self.expect(&Tok::LBracket)?;
        let mut items = Vec::new();
        while !matches!(self.cur(), Tok::RBracket) {
            items.push(self.expr()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RBracket)?;
        Ok(Expr::Array(items))
    }

    /// If the `{`/`[` at the cursor opens a span of *canonical* compact JSON (exactly what the
    /// renderer emits for [`Literal::Json`]), capture the raw text and skip its tokens.
    fn try_canonical_json(&mut self) -> Result<Option<String>, ParseError> {
        let start = self.cur_token().byte;
        let Some(end) = json_span_end(self.src, start) else {
            return Ok(None);
        };
        let raw = &self.src[start..end];
        if !is_compact_json(raw) {
            return Ok(None);
        }
        let raw = raw.to_string();
        // Advance past every token that belongs to the JSON span.
        while self.cur_token().byte < end && !self.at_eof() {
            self.bump();
        }
        Ok(Some(raw))
    }

    fn literal(&mut self) -> Result<Literal, ParseError> {
        match self.cur().clone() {
            Tok::Str(s) => {
                self.bump();
                Ok(Literal::String(s))
            }
            Tok::Int(i) => {
                self.bump();
                Ok(Literal::Int(i))
            }
            Tok::Float(f) => {
                self.bump();
                Ok(Literal::Float(f))
            }
            Tok::Ident(name) if name == "true" => {
                self.bump();
                Ok(Literal::Bool(true))
            }
            Tok::Ident(name) if name == "false" => {
                self.bump();
                Ok(Literal::Bool(false))
            }
            Tok::Ident(name) if name == "null" => {
                self.bump();
                Ok(Literal::Null)
            }
            Tok::LBrace | Tok::LBracket => {
                if let Some(raw) = self.try_canonical_json()? {
                    Ok(Literal::Json(raw))
                } else {
                    Err(self.err("a `{…}`/`[…]` initializer must be compact canonical JSON: double-quoted keys, no spaces, JSON escapes only"))
                }
            }
            other => Err(self.err(format!("expected literal, found `{other:?}`"))),
        }
    }
}

/// Loop keywords carried by [`Stmt::Loop`]; core maps them to the loop node types.
const FOR_EACH_KEYWORD: &str = "forEach";
const PARALLEL_FOR_EACH_KEYWORD: &str = "forEachParallel";
const WHILE_KEYWORD: &str = "while";

/// A placeholder call for sugared boolean branches whose original node is not surfaced in text.
/// Catalog coupling: `!x` has no AST-level representation, so it lowers to the boolean-NOT node.
/// See `packages/catalog/std/src/utils/bool/not.rs`. This is this crate's only catalog coupling —
/// the module doc's "purely syntactic" claim is qualified by exactly this pair of constants.
const NOT_CALL_DISPLAY: &str = "boolNot";
const NOT_CALL_INPUT: &str = "boolean";

fn not_call(operand: Expr) -> Expr {
    Expr::Call(Call {
        node_type: String::new(),
        display: NOT_CALL_DISPLAY.to_string(),
        path: Vec::new(),
        receiver: None,
        positional: Vec::new(),
        args: vec![Arg {
            name: NOT_CALL_INPUT.to_string(),
            value: operand,
        }],
        anchor: None,
    })
}

/// The operand of a `boolNot({ boolean: x })` call, matching the root shape only.
fn not_call_operand(expr: &Expr) -> Option<&Expr> {
    let Expr::Call(call) = expr else { return None };
    if call.display != NOT_CALL_DISPLAY
        || !call.path.is_empty()
        || call.receiver.is_some()
        || !call.positional.is_empty()
        || call.args.len() != 1
    {
        return None;
    }
    let arg = &call.args[0];
    (arg.name == NOT_CALL_INPUT).then_some(&arg.value)
}

fn placeholder_call() -> Call {
    Call::placeholder()
}

/// The value of a compound assignment `target op= rhs`, desugared to `target op rhs`.
fn compound_assign_value(op: String, target: Expr, rhs: Expr) -> Expr {
    Expr::Binary {
        op,
        lhs: Box::new(target),
        rhs: Box::new(rhs),
    }
}

/// Flattens an lvalue member/index chain rooted at a variable into `(base_variable, dot_path)`:
/// `pref.cost_weight` → `("pref", "cost_weight")`, `p.a.b` → `("p", "a.b")`,
/// `p.items[0].name` → `("p", "items[0].name")`. Returns `None` for non-static lvalues.
fn lvalue_to_field_path(expr: &Expr) -> Option<(String, String)> {
    // `.field` renders as `Expr::Field` for camelCase-stable keys and `Expr::Member` otherwise;
    // as an assignment target both are struct field-path segments.
    let dot = |base: &Expr, key: &str| -> Option<(String, String)> {
        let (var, path) = lvalue_to_field_path(base)?;
        let joined = if path.is_empty() {
            key.to_string()
        } else {
            format!("{path}.{key}")
        };
        Some((var, joined))
    };
    match expr {
        Expr::Ref(name) => Some((name.clone(), String::new())),
        Expr::Member { base, field } => dot(base, field),
        Expr::Field { base, pin } => dot(base, pin),
        Expr::Index { base, index } => {
            let (var, path) = lvalue_to_field_path(base)?;
            let Expr::Literal(Literal::Int(i)) = &**index else {
                return None;
            };
            Some((var, format!("{path}[{i}]")))
        }
        _ => None,
    }
}

/// True when `s` is a camelCase fixed point (no separators), so a `.s` access renders the same
/// whether treated as an output-pin selection or a struct field.
fn is_camel_fixed_point(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Find the byte offset just past the balanced `{…}`/`[…]` span starting at `start`, skipping
/// string contents. Returns `None` if unbalanced.
fn json_span_end(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'{' | b'[' => depth += 1,
                b'}' | b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// True when `raw` is valid JSON in *compact* form — i.e. it has no insignificant whitespace
/// outside of string literals. This is exactly the shape the renderer emits for
/// [`Literal::Json`], and it cleanly separates JSON defaults (`{"a":1}`, `[1,2,3]`) from
/// struct/array literals (`{ a: 1 }`, `[1, 2, 3]`, `[ref]`), without depending on serde's map
/// key ordering. Any literal that is *both* compact JSON and a structural value (e.g. `[1]`,
/// `{}`) re-renders identically either way, so the round-trip stays lossless.
fn is_compact_json(raw: &str) -> bool {
    if serde_json::from_str::<serde_json::Value>(raw).is_err() {
        return false;
    }
    let bytes = raw.as_bytes();
    let mut in_str = false;
    let mut escaped = false;
    for &c in bytes {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else if c == b'"' {
            in_str = true;
        } else if c.is_ascii_whitespace() {
            return false;
        }
    }
    true
}
