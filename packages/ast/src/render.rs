//! Rendering: `BoardAst -> FlowScript` text. Pure; see `todo/ast.md` §6.

use std::collections::HashMap;

use crate::model::*;
use crate::schema::{normalize_object_schema, normalize_schema};
use crate::text::{is_valid_identifier, quote_string, to_camel_case};

/// Options controlling text rendering.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Emit `//@n:<id>` / `//@v:<id>` / `//@l:<id>` anchor comments for stable round-trip.
    pub anchors: bool,
    /// Indentation unit.
    pub indent: String,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            anchors: false,
            indent: "    ".to_string(),
        }
    }
}

/// Render a board AST into FlowScript source text.
pub fn render(ast: &BoardAst, opts: &RenderOptions) -> String {
    let schema_types = schema_type_map(&ast.interfaces);
    let mut w = Writer {
        out: String::new(),
        opts,
        depth: 0,
        schema_types,
    };
    w.board(ast);
    w.out
}

struct Writer<'a> {
    out: String,
    opts: &'a RenderOptions,
    depth: usize,
    schema_types: HashMap<String, String>,
}

/// Loop keywords shared with the parser (`Stmt::Loop::keyword`).
const PARALLEL_FOR_EACH_KEYWORD: &str = "forEachParallel";
const WHILE_KEYWORD: &str = "while";

/// The head of a [`Stmt::Loop`] as the renderer reads it.
struct LoopHead<'a> {
    keyword: &'a str,
    bind: Option<&'a str>,
    call: &'a Call,
    iterable: Option<&'a Expr>,
    element: Option<&'a str>,
    index: Option<&'a str>,
}

impl Writer<'_> {
    fn board(&mut self, ast: &BoardAst) {
        let mut first_section = true;

        if !ast.uses.is_empty() {
            for decl in &ast.uses {
                self.use_decl(decl);
            }
            first_section = false;
        }

        if !ast.interfaces.is_empty() {
            if !first_section {
                self.out.push('\n');
            }
            for interface in &ast.interfaces {
                self.interface_decl(interface);
            }
            first_section = false;
        }

        if !ast.variables.is_empty() {
            if !first_section {
                self.out.push('\n');
            }
            for var in &ast.variables {
                self.var_decl(var);
            }
            first_section = false;
        }

        for func in &ast.functions {
            if !first_section {
                self.out.push('\n');
            }
            first_section = false;
            self.fn_decl(func);
        }

        for event in &ast.events {
            if !first_section {
                self.out.push('\n');
            }
            first_section = false;
            self.event_block(event);
        }

        // Modules render last, in the order the caller put them in — ordering is a lowering
        // decision, not a rendering one.
        for module in &ast.modules {
            if !first_section {
                self.out.push('\n');
            }
            first_section = false;
            self.module_decl(module);
        }
    }

    fn use_decl(&mut self, decl: &UseDecl) {
        self.out.push_str("use ");
        self.out.push_str(&decl.path.join("::"));
        match &decl.kind {
            UseKind::Namespace => {}
            UseKind::Glob => self.out.push_str("::*"),
            UseKind::Alias(alias) => {
                self.out.push_str(" as ");
                self.out.push_str(alias);
            }
            UseKind::Members(members) => {
                self.out.push_str("::{ ");
                self.out.push_str(&members.join(", "));
                self.out.push_str(" }");
            }
        }
        self.out.push('\n');
    }

    /// A top-level declaration. A scalar default that infers exactly the declared type drops
    /// the annotation (`const x = "a"`), mirroring what the parser accepts; every other shape
    /// (struct, array, interface, no default) keeps `: Type`.
    fn var_decl(&mut self, var: &VarDecl) {
        self.var_decorators(var);
        self.indent();
        let kw = if var.exposed { "let" } else { "const" };
        self.out.push_str(kw);
        self.out.push(' ');
        self.out.push_str(&var.name);
        let annotated = !var
            .default
            .as_ref()
            .is_some_and(|default| var.schema.is_none() && default.infers_scalar_type(&var.ty));
        if annotated {
            self.out.push_str(": ");
            self.out.push_str(&self.render_var_type(var));
        }
        if let Some(default) = &var.default {
            self.out.push_str(" = ");
            self.out.push_str(&render_literal(default));
        }
        self.anchor("v", var.anchor.as_deref());
        self.out.push('\n');
    }

    fn interface_decl(&mut self, interface: &InterfaceDecl) {
        self.indent();
        self.out.push_str("interface ");
        self.out.push_str(&interface.name);
        self.out.push_str(" {\n");
        self.depth += 1;
        for field in &interface.fields {
            self.indent();
            // JSON-schema property names are arbitrary strings; quote the ones that
            // would not lex as identifiers so the interface stays parseable.
            if is_valid_identifier(&field.name) {
                self.out.push_str(&field.name);
            } else {
                self.out.push_str(&quote_string(&field.name));
            }
            if field.optional {
                self.out.push('?');
            }
            self.out.push_str(": ");
            self.out.push_str(&render_interface_type(&field.ty));
            if let Some(default) = &field.default {
                self.out.push_str(" = ");
                self.out.push_str(&render_literal(default));
            }
            self.out.push_str(";\n");
        }
        self.depth -= 1;
        self.indent();
        self.out.push_str("}\n");
    }

    /// Emit `@decorator` lines for a variable's non-keyword settings, one per line,
    /// at the current indentation. New variable settings hook in here (and in the
    /// parser's `decorator` handling) to stay symmetric.
    fn var_decorators(&mut self, var: &VarDecl) {
        for dec in self.var_decorators_of(var) {
            self.indent();
            self.out.push_str(&dec);
            self.out.push('\n');
        }
    }

    fn var_decorators_of(&self, var: &VarDecl) -> Vec<String> {
        let mut decorators = Vec::new();
        if let Some(description) = &var.description {
            decorators.push(format!("@description({})", quote_string(description)));
        }
        if let Some(category) = &var.category {
            decorators.push(format!("@category({})", quote_string(category)));
        }
        if let Some(schema) = &var.schema
            && self.schema_type_name(schema).is_none()
            && normalize_object_schema(schema).is_some()
        {
            decorators.push(format!("@schema({})", quote_string(schema)));
        }
        if var.secret {
            decorators.push("@secret".to_string());
        }
        if !var.editable {
            decorators.push("@readonly".to_string());
        }
        if var.runtime_configured {
            decorators.push("@runtime".to_string());
        }
        decorators
    }

    fn render_var_type(&self, var: &VarDecl) -> String {
        if let Some(schema) = &var.schema
            && let Some(name) = self.schema_type_name(schema)
        {
            let ty = TypeRef::new(name.to_string(), var.ty.container);
            return render_type(&ty);
        }
        render_type(&var.ty)
    }

    fn schema_type_name(&self, schema: &str) -> Option<&str> {
        let normalized = normalize_schema(schema)?;
        self.schema_types.get(&normalized).map(String::as_str)
    }

    fn fn_decl(&mut self, func: &FnDecl) {
        if let Some(cache) = &func.cache {
            self.function_cache_decorator(cache);
        }
        self.indent();
        self.out.push_str("function ");
        self.out.push_str(&func.name);
        self.out.push('(');
        self.params(&func.params);
        self.out.push(')');
        if !func.returns.is_empty() {
            self.out.push_str(": (");
            self.params(&func.returns);
            self.out.push(')');
        }
        self.out.push_str(" {");
        self.anchor("l", func.anchor.as_deref());
        self.out.push('\n');
        self.block(&func.body);
        self.indent();
        self.out.push_str("}\n");
    }

    /// Render function caching in one stable field order. Default-valued fields are omitted; an
    /// entirely default policy is the compact `@cache` flag.
    fn function_cache_decorator(&mut self, cache: &FunctionCache) {
        self.indent();
        let mut fields = Vec::new();
        if cache.namespace != DEFAULT_FUNCTION_CACHE_NAMESPACE {
            fields.push(format!("namespace: {}", quote_string(&cache.namespace)));
        }
        match cache.ttl_seconds {
            Some(DEFAULT_FUNCTION_CACHE_TTL_SECONDS) => {}
            Some(ttl_seconds) => fields.push(format!("ttlSeconds: {ttl_seconds}")),
            // Legacy/programmatic ASTs may still carry `None` for a permanent cache. FlowScript's
            // omission now means five minutes, so preserve permanence with the explicit spelling.
            None => fields.push("ttlSeconds: 0".to_string()),
        }
        if cache.scope != FunctionCacheScope::App {
            fields.push(format!("scope: {}", quote_string(cache.scope.as_str())));
        }

        if fields.is_empty() {
            self.out.push_str("@cache\n");
        } else {
            self.out.push_str("@cache({ ");
            self.out.push_str(&fields.join(", "));
            self.out.push_str(" })\n");
        }
    }

    fn event_block(&mut self, event: &EventBlock) {
        self.indent();
        self.out.push_str(&event.name);
        if let Some(event_name) = event
            .event_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
        {
            self.out.push(' ');
            self.out.push_str(event_name);
        }
        self.out.push('(');
        self.params(&event.params);
        self.out.push_str(") {");
        self.anchor("n", event.anchor.as_deref());
        self.out.push('\n');
        self.block(&event.body);
        self.indent();
        self.out.push_str("}\n");
    }

    fn module_decl(&mut self, module: &ModuleDecl) {
        self.indent();
        self.out.push_str("module ");
        self.out.push_str(&module.name);
        self.out.push_str(" {");
        self.anchor("l", module.anchor.as_deref());
        self.out.push('\n');

        self.depth += 1;
        let mut first_section = true;
        for func in &module.functions {
            if !first_section {
                self.out.push('\n');
            }
            first_section = false;
            self.fn_decl(func);
        }
        for event in &module.events {
            if !first_section {
                self.out.push('\n');
            }
            first_section = false;
            self.event_block(event);
        }
        for nested in &module.modules {
            if !first_section {
                self.out.push('\n');
            }
            first_section = false;
            self.module_decl(nested);
        }
        self.depth -= 1;

        self.indent();
        self.out.push_str("}\n");
    }

    fn block(&mut self, block: &Block) {
        self.depth += 1;
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        self.depth -= 1;
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { name, call, anchor } => {
                self.indent();
                self.out.push_str("const ");
                self.out.push_str(name);
                self.out.push_str(" = ");
                self.out.push_str(&render_call(call));
                self.anchor("n", anchor.as_deref());
                self.out.push('\n');
            }
            Stmt::Destructure {
                fields,
                call,
                anchor,
            } => {
                self.indent();
                self.out.push_str("const { ");
                self.out.push_str(&render_destructure_fields(fields));
                self.out.push_str(" } = ");
                self.out.push_str(&render_call(call));
                self.anchor("n", anchor.as_deref());
                self.out.push('\n');
            }
            Stmt::Call { call, anchor } => {
                self.indent();
                self.out.push_str(&render_call(call));
                self.anchor("n", anchor.as_deref());
                self.out.push('\n');
            }
            Stmt::Branch {
                bind,
                call,
                condition,
                arms,
                anchor,
            } => {
                self.branch(
                    bind.as_deref(),
                    call,
                    condition.as_ref(),
                    arms,
                    anchor.as_deref(),
                );
            }
            Stmt::Loop {
                keyword,
                bind,
                call,
                iterable,
                element,
                index,
                body,
                anchor,
            } => {
                let head = LoopHead {
                    keyword,
                    bind: bind.as_deref(),
                    call,
                    iterable: iterable.as_ref(),
                    element: element.as_deref(),
                    index: index.as_deref(),
                };
                self.loop_stmt(&head, body, anchor.as_deref());
            }
            Stmt::Assign {
                target,
                value,
                anchor,
            } => {
                self.indent();
                self.out.push_str(target);
                self.out.push_str(" = ");
                self.out.push_str(&render_expr(value));
                self.anchor("n", anchor.as_deref());
                self.out.push('\n');
            }
            Stmt::FieldAssign {
                base,
                path,
                value,
                anchor,
            } => {
                self.indent();
                self.out.push_str(base);
                // A bracket-rooted path (`base[0]`) has no separator; a named field (`base.field`)
                // is dot-joined.
                if !path.starts_with('[') {
                    self.out.push('.');
                }
                self.out.push_str(path);
                self.out.push_str(" = ");
                self.out.push_str(&render_expr(value));
                self.anchor("n", anchor.as_deref());
                self.out.push('\n');
            }
            Stmt::LocalAlias {
                name,
                value,
                anchor,
            } => {
                self.indent();
                self.out.push_str("let ");
                self.out.push_str(name);
                self.out.push_str(" = ");
                self.out.push_str(&render_expr(value));
                self.anchor("n", anchor.as_deref());
                self.out.push('\n');
            }
            Stmt::Return { values, anchor } => {
                self.indent();
                self.out.push_str("return");
                if !values.is_empty() {
                    self.out.push(' ');
                    let rendered: Vec<String> = values.iter().map(render_expr).collect();
                    self.out.push_str(&rendered.join(", "));
                }
                self.anchor("n", anchor.as_deref());
                self.out.push('\n');
            }
            Stmt::Local(var) => {
                self.var_decorators(var);
                self.indent();
                self.out.push_str("let ");
                self.out.push_str(&var.name);
                self.out.push_str(": ");
                self.out.push_str(&self.render_var_type(var));
                if let Some(default) = &var.default {
                    self.out.push_str(" = ");
                    self.out.push_str(&render_literal(default));
                }
                self.anchor("v", var.anchor.as_deref());
                self.out.push('\n');
            }
            Stmt::Handler(event) => {
                self.event_block(event);
            }
            Stmt::Comment(text) => {
                self.indent();
                self.out.push_str("// ");
                self.out.push_str(text);
                self.out.push('\n');
            }
        }
    }

    fn branch(
        &mut self,
        bind: Option<&str>,
        call: &Call,
        condition: Option<&Expr>,
        arms: &[BranchArm],
        anchor: Option<&str>,
    ) {
        if let Some(bind) = bind
            && !is_placeholder_call(call)
        {
            self.indent();
            self.out.push_str("const ");
            self.out.push_str(bind);
            self.out.push_str(" = ");
            self.out.push_str(&render_call(call));
            self.anchor("n", anchor);
            self.out.push('\n');
            self.branch_ref(bind, arms, None);
            return;
        }

        if let Some(bind) = bind {
            self.branch_ref(bind, arms, anchor);
            return;
        }

        self.indent();
        // Sugared boolean condition (`control_branch`): render as `if (cond) { } [else { }]`.
        if let Some(cond) = condition {
            match arms {
                [yes, no] => {
                    self.out.push_str("if (");
                    self.out.push_str(&render_expr(cond));
                    self.out.push_str(") {");
                    self.anchor("n", anchor);
                    self.out.push('\n');
                    self.block(&yes.body);
                    self.indent();
                    self.out.push_str("} else {\n");
                    self.block(&no.body);
                    self.indent();
                    self.out.push_str("}\n");
                }
                [single] => {
                    // Only one connected exec output: negate when it is the `false` arm.
                    let negate = single.label.eq_ignore_ascii_case("false");
                    self.out.push_str("if (");
                    if negate {
                        self.out.push_str("!(");
                        self.out.push_str(&render_expr(cond));
                        self.out.push(')');
                    } else {
                        self.out.push_str(&render_expr(cond));
                    }
                    self.out.push_str(") {");
                    self.anchor("n", anchor);
                    self.out.push('\n');
                    self.block(&single.body);
                    self.indent();
                    self.out.push_str("}\n");
                }
                _ => {
                    // Degenerate (no connected arms): emit an empty guarded block. The brace
                    // closes on its own line so a trailing anchor comment cannot swallow it.
                    self.out.push_str("if (");
                    self.out.push_str(&render_expr(cond));
                    self.out.push_str(") {");
                    self.anchor("n", anchor);
                    self.out.push('\n');
                    self.indent();
                    self.out.push_str("}\n");
                }
            }
            return;
        }

        // Two-arm boolean branch renders as `if (...) { } else { }`.
        if let [yes, no] = arms {
            self.out.push_str("if (");
            self.out.push_str(&render_call(call));
            self.out.push_str(") {");
            if !yes.label.is_empty() {
                self.out.push_str(" // ");
                self.out.push_str(&yes.label);
            }
            self.anchor("n", anchor);
            self.out.push('\n');
            self.block(&yes.body);
            self.indent();
            self.out.push_str("} else {");
            if !no.label.is_empty() {
                self.out.push_str(" // ");
                self.out.push_str(&no.label);
            }
            self.out.push('\n');
            self.block(&no.body);
            self.indent();
            self.out.push_str("}\n");
            return;
        }

        // General N-way fan-out: a labelled block per exec output.
        self.out.push_str(&render_call(call));
        self.out.push_str(" {");
        self.anchor("n", anchor);
        self.out.push('\n');
        self.depth += 1;
        for arm in arms {
            self.indent();
            self.out.push_str(&to_camel_case(&arm.label));
            self.out.push_str(": {\n");
            self.block(&arm.body);
            self.indent();
            self.out.push_str("}\n");
        }
        self.depth -= 1;
        self.indent();
        self.out.push_str("}\n");
    }

    fn branch_ref(&mut self, bind: &str, arms: &[BranchArm], anchor: Option<&str>) {
        self.indent();
        self.out.push_str(bind);
        self.out.push_str(" {");
        self.anchor("n", anchor);
        self.out.push('\n');
        self.depth += 1;
        for arm in arms {
            self.indent();
            self.out.push_str(&to_camel_case(&arm.label));
            self.out.push_str(": {\n");
            self.block(&arm.body);
            self.indent();
            self.out.push_str("}\n");
        }
        self.depth -= 1;
        self.indent();
        self.out.push_str("}\n");
    }

    fn loop_stmt(&mut self, head: &LoopHead<'_>, body: &Block, anchor: Option<&str>) {
        if head.iterable.is_some() && head.keyword == PARALLEL_FOR_EACH_KEYWORD {
            self.indent();
            self.out.push_str("@parallel\n");
        }
        self.indent();
        if head.keyword == WHILE_KEYWORD {
            // `while (cond) { … }` — the sugared condition, or the explicit loop-node call.
            self.out.push_str("while (");
            match head.iterable {
                Some(condition) => self.out.push_str(&render_expr(condition)),
                None => self.out.push_str(&render_call(head.call)),
            }
            self.out.push_str(") {");
        } else {
            // `for (const item of array)` / `for (const [i, item] of array)`, or the explicit
            // `for (const handle of forEach(array))`.
            self.out.push_str("for (const ");
            match head.iterable {
                Some(iterable) => {
                    let element = head.element.unwrap_or("_");
                    match head.index {
                        Some(index) => {
                            self.out.push('[');
                            self.out.push_str(index);
                            self.out.push_str(", ");
                            self.out.push_str(element);
                            self.out.push(']');
                        }
                        None => self.out.push_str(element),
                    }
                    self.out.push_str(" of ");
                    self.out.push_str(&render_expr(iterable));
                }
                None => {
                    self.out.push_str(head.bind.unwrap_or("_"));
                    self.out.push_str(" of ");
                    self.out.push_str(&render_call(head.call));
                }
            }
            self.out.push_str(") {");
        }
        self.anchor("n", anchor);
        self.out.push('\n');
        self.block(body);
        self.indent();
        self.out.push_str("}\n");
    }

    fn params(&mut self, params: &[Param]) {
        let rendered: Vec<String> = params
            .iter()
            .map(|p| format!("{}: {}", p.name, render_type(&p.ty)))
            .collect();
        self.out.push_str(&rendered.join(", "));
    }

    fn indent(&mut self) {
        for _ in 0..self.depth {
            self.out.push_str(&self.opts.indent);
        }
    }

    fn anchor(&mut self, kind: &str, id: Option<&str>) {
        if self.opts.anchors
            && let Some(id) = id
        {
            self.out.push_str("   //@");
            self.out.push_str(kind);
            self.out.push(':');
            self.out.push_str(id);
        }
    }
}

/// Collect the `@decorator` lines for a variable's non-keyword settings. Kept as a free
/// function so the surface-syntax mapping lives in one place and mirrors the parser.
pub fn var_decorators_of(var: &VarDecl) -> Vec<String> {
    let mut decorators = Vec::new();
    if let Some(description) = &var.description {
        decorators.push(format!("@description({})", quote_string(description)));
    }
    if let Some(category) = &var.category {
        decorators.push(format!("@category({})", quote_string(category)));
    }
    if let Some(schema) = &var.schema
        && normalize_object_schema(schema).is_some()
    {
        decorators.push(format!("@schema({})", quote_string(schema)));
    }
    if var.secret {
        decorators.push("@secret".to_string());
    }
    if !var.editable {
        decorators.push("@readonly".to_string());
    }
    if var.runtime_configured {
        decorators.push("@runtime".to_string());
    }
    decorators
}

/// Render a `TypeRef` as TS-flavoured type text (`string`, `int[]`, `Map<string, T>`).
pub fn render_type_ref(ty: &TypeRef) -> String {
    match ty.container {
        Container::Normal => ty.base.clone(),
        Container::Array => format!("{}[]", ty.base),
        Container::Map => format!("Map<string, {}>", ty.base),
        Container::Set => format!("Set<{}>", ty.base),
    }
}

pub fn render_interface_type(ty: &InterfaceType) -> String {
    match ty {
        InterfaceType::Named(name) => name.clone(),
        InterfaceType::Array(inner) => {
            let inner_text = render_interface_type(inner);
            // `A | B[]` parses as `A | (B[])`; group union elements explicitly.
            if matches!(**inner, InterfaceType::Union(_)) {
                format!("({inner_text})[]")
            } else {
                format!("{inner_text}[]")
            }
        }
        InterfaceType::Map(inner) => format!("Map<string, {}>", render_interface_type(inner)),
        InterfaceType::Union(members) => members
            .iter()
            .map(render_interface_type)
            .collect::<Vec<_>>()
            .join(" | "),
        InterfaceType::StringLiteral(value) => quote_string(value),
        InterfaceType::Null => "null".to_string(),
        InterfaceType::Any => "any".to_string(),
    }
}

fn render_type(ty: &TypeRef) -> String {
    render_type_ref(ty)
}

fn render_literal(lit: &Literal) -> String {
    match lit {
        Literal::String(s) => quote_string(s),
        Literal::Int(i) => i.to_string(),
        Literal::Float(f) => {
            if f.fract() == 0.0 {
                format!("{f:.1}")
            } else {
                f.to_string()
            }
        }
        Literal::Bool(b) => b.to_string(),
        Literal::Null => "null".to_string(),
        Literal::Json(raw) => raw.clone(),
    }
}

fn render_expr(expr: &Expr) -> String {
    match expr {
        Expr::Call(call) => render_call(call),
        Expr::Ref(name) => name.clone(),
        Expr::Field { base, pin } => format!("{}.{}", render_expr(base), to_camel_case(pin)),
        Expr::Member { base, field } => render_member(&render_expr(base), field),
        Expr::Object(fields) => render_object(fields),
        Expr::Array(items) => {
            let entries: Vec<String> = items.iter().map(render_expr).collect();
            format!("[{}]", entries.join(", "))
        }
        Expr::Index { base, index } => {
            format!("{}[{}]", render_binary_operand(base), render_expr(index))
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => format!(
            "{} ? {} : {}",
            render_binary_operand(cond),
            render_binary_operand(then),
            render_binary_operand(otherwise)
        ),
        Expr::Binary { op, lhs, rhs } => {
            format!(
                "{} {} {}",
                render_binary_operand(lhs),
                op,
                render_binary_operand(rhs)
            )
        }
        Expr::Template { parts } => render_template(parts),
        Expr::Literal(lit) => render_literal(lit),
    }
}

/// `` `text ${expr} text` `` — static text keeps raw newlines; `` ` ``, `\` and a `${` sequence
/// are escaped so the text re-lexes to the same parts.
pub fn render_template(parts: &[TemplatePart]) -> String {
    let mut out = String::from("`");
    for part in parts {
        match part {
            TemplatePart::Text(text) => {
                let mut chars = text.chars().peekable();
                while let Some(ch) = chars.next() {
                    match ch {
                        '`' => out.push_str("\\`"),
                        '\\' => out.push_str("\\\\"),
                        '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        '\u{8}' => out.push_str("\\b"),
                        '\u{c}' => out.push_str("\\f"),
                        _ => out.push(ch),
                    }
                }
            }
            TemplatePart::Expr(expr) => {
                out.push_str("${");
                out.push_str(&render_expr(expr));
                out.push('}');
            }
        }
    }
    out.push('`');
    out
}

fn schema_type_map(interfaces: &[InterfaceDecl]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for interface in interfaces {
        if let Some(schema) = &interface.schema
            && let Some(normalized) = normalize_schema(schema)
        {
            map.insert(normalized, interface.name.clone());
        }
    }
    map
}

/// Render a binary operand, parenthesising nested binary expressions for clarity.
fn render_binary_operand(expr: &Expr) -> String {
    match expr {
        Expr::Binary { .. } | Expr::Ternary { .. } => format!("({})", render_expr(expr)),
        _ => render_expr(expr),
    }
}

/// Render struct data-field access. Identifier-ish paths use dot notation
/// (`base.a.b`, `base.items[0].name`); anything else falls back to bracketed string access.
fn render_member(base: &str, field: &str) -> String {
    if is_plain_field_path(field) {
        format!("{base}.{field}")
    } else {
        format!("{base}[{}]", quote_string(field))
    }
}

fn is_plain_field_path(field: &str) -> bool {
    if field.is_empty() {
        return false;
    }
    let mut chars = field.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    field
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '[' | ']'))
}

fn render_object(fields: &[ObjectField]) -> String {
    if fields.is_empty() {
        return "{}".to_string();
    }
    let entries: Vec<String> = fields
        .iter()
        .map(|f| format!("{}: {}", render_object_key(&f.key), render_expr(&f.value)))
        .collect();
    format!("{{ {} }}", entries.join(", "))
}

fn render_object_key(key: &str) -> String {
    if is_plain_ident(key) {
        key.to_string()
    } else {
        quote_string(key)
    }
}

fn is_plain_ident(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `[receiver.][path::]display(positional, …, { named })`.
fn render_call(call: &Call) -> String {
    let mut out = String::new();
    if let Some(receiver) = &call.receiver {
        out.push_str(&render_receiver(receiver));
        out.push('.');
    }
    for segment in &call.path {
        out.push_str(segment);
        out.push_str("::");
    }
    out.push_str(&call.display);
    out.push('(');
    let mut parts: Vec<String> = call.positional.iter().map(render_expr).collect();
    if !call.args.is_empty() {
        let named: Vec<String> = call
            .args
            .iter()
            .map(|a| format!("{}: {}", to_camel_case(&a.name), render_expr(&a.value)))
            .collect();
        parts.push(format!("{{ {} }}", named.join(", ")));
    }
    out.push_str(&parts.join(", "));
    out.push(')');
    out
}

/// A method receiver: binary/ternary operands are grouped as usual, and a numeric literal is
/// parenthesised too so `(5).abs()` never re-lexes as a float.
fn render_receiver(expr: &Expr) -> String {
    match expr {
        Expr::Literal(Literal::Int(_) | Literal::Float(_)) => format!("({})", render_expr(expr)),
        _ => render_binary_operand(expr),
    }
}

fn render_destructure_fields(fields: &[DestructureField]) -> String {
    fields
        .iter()
        .map(|field| {
            let pin = to_camel_case(&field.pin);
            if pin == field.name {
                pin
            } else {
                format!("{pin}: {}", field.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_placeholder_call(call: &Call) -> bool {
    call.is_placeholder()
}
