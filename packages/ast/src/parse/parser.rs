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
use crate::parse::lexer::{Tok, Token, lex};
use crate::schema::{apply_interface_schemas, schema_from_interface};

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

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
}

/// A parsed `@decorator`, optionally carrying a single string argument.
struct Decorator {
    name: String,
    arg: Option<String>,
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

    /// Consume a trailing anchor comment (`//@n:id`) if present; returns the id.
    /// Only the known anchor kinds (`n`/`v`/`l`) qualify — any other `@…` comment is an
    /// ordinary user comment and must not be swallowed as an anchor.
    fn take_anchor(&mut self) -> Option<String> {
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

    /// Consume a trailing non-anchor comment (a branch arm label) if present.
    fn take_label(&mut self) -> Option<String> {
        if let Tok::Comment(text) = self.cur()
            && !text.starts_with('@')
        {
            let label = text.clone();
            self.bump();
            return Some(label);
        }
        None
    }

    // ---- decorators -----------------------------------------------------------------------

    /// Parse zero or more leading `@decorator` lines. Each is a bare flag (`@secret`) or carries
    /// a single string argument (`@category("…")`).
    fn decorators(&mut self) -> Result<Vec<Decorator>, ParseError> {
        let mut decorators = Vec::new();
        while matches!(self.cur(), Tok::At) {
            self.bump();
            let name = self.ident()?;
            let arg = if self.eat(&Tok::LParen) {
                let value = match self.cur().clone() {
                    Tok::Str(s) => {
                        self.bump();
                        s
                    }
                    other => {
                        return Err(self.err(format!(
                            "expected string decorator argument, found `{other:?}`"
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
        dec.arg.clone().ok_or_else(|| {
            self.err(format!(
                "decorator `@{}` requires a string argument",
                dec.name
            ))
        })
    }

    // ---- top level ------------------------------------------------------------------------

    fn board(&mut self) -> Result<BoardAst, ParseError> {
        let mut ast = BoardAst::default();
        while !self.at_eof() {
            let decorators = self.decorators()?;
            match self.cur().clone() {
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
                    if !decorators.is_empty() {
                        return Err(self.err("decorators on functions are not yet supported"));
                    }
                    ast.functions.push(self.fn_decl()?);
                }
                Tok::Ident(_) => {
                    if !decorators.is_empty() {
                        return Err(self.err("decorators on events are not yet supported"));
                    }
                    ast.events.push(self.event_block()?);
                }
                Tok::Comment(_) => {
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
    fn var_decl(&mut self, exposed: bool) -> Result<VarDecl, ParseError> {
        self.bump(); // const | let
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
            anchor,
        })
    }

    fn event_block(&mut self) -> Result<EventBlock, ParseError> {
        let name = self.ident()?;
        self.expect(&Tok::LParen)?;
        let params = self.params(&Tok::RParen)?;
        self.expect(&Tok::RParen)?;
        self.expect(&Tok::LBrace)?;
        let anchor = self.take_anchor();
        let body = self.block_body()?;
        Ok(EventBlock {
            name,
            node_type: String::new(),
            params,
            body,
            anchor,
        })
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
        // Leading decorators bind to a following local `let` declaration.
        if matches!(self.cur(), Tok::At) {
            let decorators = self.decorators()?;
            if !self.is_ident("let") {
                return Err(self.err("decorators are only supported on `let` declarations"));
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
        if !matches!(self.cur(), Tok::RBrace) && self.line() == return_line {
            values.push(self.expr()?);
            while self.eat(&Tok::Comma) {
                values.push(self.expr()?);
            }
        }
        let anchor = self.take_anchor();
        Ok(Stmt::Return { values, anchor })
    }

    /// Detect a nested event-handler header (`name(params) { … }`). Call/branch arguments are
    /// always an object literal (`name({ … })`), so a bare typed parameter list (`name(ident:`)
    /// or an empty list whose body is not a branch-arm map unambiguously marks a handler.
    fn looks_like_handler(&self) -> bool {
        if !matches!(self.cur(), Tok::Ident(_)) {
            return false;
        }
        if !matches!(
            self.toks.get(self.pos + 1).map(|t| &t.tok),
            Some(Tok::LParen)
        ) {
            return false;
        }
        // Empty params `name()` — a handler unless the following block is a branch-arm map.
        if matches!(
            self.toks.get(self.pos + 2).map(|t| &t.tok),
            Some(Tok::RParen)
        ) {
            if !matches!(
                self.toks.get(self.pos + 3).map(|t| &t.tok),
                Some(Tok::LBrace)
            ) {
                return false;
            }
            return !self.brace_opens_branch_arms(self.pos + 3);
        }
        // Typed params `name(ident : …` — never produced by an object-arg call.
        matches!(
            self.toks.get(self.pos + 2).map(|t| &t.tok),
            Some(Tok::Ident(_))
        ) && matches!(
            self.toks.get(self.pos + 3).map(|t| &t.tok),
            Some(Tok::Colon)
        )
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
        // Assignment: `target = expr` (the target is always a bare variable name).
        if matches!(
            self.toks.get(self.pos + 1).map(|t| &t.tok),
            Some(Tok::Assign)
        ) {
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
        let value = self.expr()?;
        // `base.field = expr` (or `base.a.b`, `base.items[0]`) — a struct-field write. Kept as a
        // first-class `Stmt::FieldAssign` (round-trips back to the dot form); reconcile expands it
        // to `structSet({ structIn: base, field: "path", value })` and rebinds `base`.
        if matches!(self.cur(), Tok::Assign) {
            let (base, path) = lvalue_to_field_path(&value).filter(|(_, p)| !p.is_empty()).ok_or_else(
                || self.err("assignment target must be a variable or a struct field path (e.g. `x.field`)"),
            )?;
            self.bump(); // =
            let rhs = self.expr()?;
            let anchor = self.take_anchor();
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
            let anchor = self.take_anchor();
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
        // Negated single-arm form: `if (!(cond)) { … }`.
        let negated = matches!(self.cur(), Tok::Bang);
        if negated {
            self.bump(); // !
            self.expect(&Tok::LParen)?;
            let cond = self.expr()?;
            self.expect(&Tok::RParen)?;
            self.expect(&Tok::RParen)?;
            self.expect(&Tok::LBrace)?;
            let anchor = self.take_anchor();
            let body = self.block_body()?;
            return Ok(Stmt::Branch {
                bind: None,
                call: placeholder_call(),
                condition: Some(cond),
                arms: vec![BranchArm {
                    label: "False".to_string(),
                    body,
                }],
                anchor,
            });
        }
        let cond = self.expr()?;
        self.expect(&Tok::RParen)?;
        self.expect(&Tok::LBrace)?;
        // A trailing non-anchor comment marks the labelled (call-based) branch form. The anchor
        // comment can FOLLOW the label on the same line (`{ // exec_out   //@n:id`) — the lexer
        // splits them into separate Comment tokens, so consume the anchor after the label or the
        // branch node counts as deleted on reconcile.
        let true_label = self.take_label();
        let anchor = self.take_anchor();
        let true_body = self.block_body()?;

        let mut else_label = None;
        let mut else_body = None;
        if self.is_ident("else") {
            self.bump(); // else
            self.expect(&Tok::LBrace)?;
            else_label = self.take_label();
            else_body = Some(self.block_body()?);
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

    fn for_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.bump(); // for
        self.expect(&Tok::LParen)?;
        if !self.is_ident("const") {
            return Err(self.err("expected `const` in for-of loop"));
        }
        self.bump(); // const
        let bind = self.ident()?;
        if !self.is_ident("of") {
            return Err(self.err("expected `of` in for-of loop"));
        }
        self.bump(); // of
        let call = self.expr_call()?;
        self.expect(&Tok::RParen)?;
        self.expect(&Tok::LBrace)?;
        let anchor = self.take_anchor();
        let body = self.block_body()?;
        Ok(Stmt::Loop {
            keyword: "forEach".to_string(),
            bind: Some(bind),
            call,
            body,
            anchor,
        })
    }

    fn while_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.bump(); // while
        self.expect(&Tok::LParen)?;
        let call = self.expr_call()?;
        self.expect(&Tok::RParen)?;
        self.expect(&Tok::LBrace)?;
        let anchor = self.take_anchor();
        let body = self.block_body()?;
        Ok(Stmt::Loop {
            keyword: "while".to_string(),
            bind: None,
            call,
            body,
            anchor,
        })
    }

    fn expr_call(&mut self) -> Result<Call, ParseError> {
        match self.expr()? {
            Expr::Call(call) => Ok(call),
            _ => Err(self.err("expected a call expression")),
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

    /// Single-precedence left-associative binary parse. The renderer fully parenthesises
    /// nested binary/ternary operands, so explicit parens carry all the grouping.
    fn binary(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.postfix()?;
        while let Tok::Op(op) = self.cur().clone() {
            self.bump();
            let rhs = self.postfix()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.primary()?;
        loop {
            match self.cur() {
                Tok::Dot => {
                    self.bump();
                    let name = self.ident()?;
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
                    expr = Expr::Index {
                        base: Box::new(expr),
                        index: Box::new(index),
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
                    self.bump();
                    if matches!(self.cur(), Tok::LParen) {
                        self.call_tail(name)
                    } else {
                        Ok(Expr::Ref(name))
                    }
                }
            },
            other => Err(self.err(format!("unexpected token in expression: `{other:?}`"))),
        }
    }

    /// Parse the `(...)` tail of a call whose display name was already consumed.
    fn call_tail(&mut self, display: String) -> Result<Expr, ParseError> {
        self.expect(&Tok::LParen)?;
        let mut args = Vec::new();
        if !matches!(self.cur(), Tok::RParen) {
            // Non-empty calls always render their arguments inside a single `{ … }` object.
            self.expect(&Tok::LBrace)?;
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
        }
        self.expect(&Tok::RParen)?;
        Ok(Expr::Call(Call {
            node_type: String::new(),
            display,
            args,
            anchor: None,
        }))
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
                    Err(self.err("expected a literal value"))
                }
            }
            other => Err(self.err(format!("expected literal, found `{other:?}`"))),
        }
    }
}

/// A placeholder call for sugared boolean branches whose original node is not surfaced in text.
fn placeholder_call() -> Call {
    Call {
        node_type: String::new(),
        display: String::new(),
        args: Vec::new(),
        anchor: None,
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
