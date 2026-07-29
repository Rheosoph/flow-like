//! Parser tests: per-construct round-trips plus full-fixture text idempotency.
//!
//! The acceptance invariant is `render(parse(text)) == text` for the committed `.flow` and
//! `.anchored.flow` fixtures, which guarantees the parser is the faithful inverse of the
//! renderer over everything the renderer actually emits.

use flow_like_ast::parse::ParseError;
use flow_like_ast::{RenderOptions, normalize_schema, parse, quote_string, render};

fn anchored_opts() -> RenderOptions {
    RenderOptions {
        anchors: true,
        ..Default::default()
    }
}

/// Assert `render(parse(text)) == text`.
fn assert_idempotent(text: &str, opts: &RenderOptions) {
    let ast = parse(text).expect("parse should succeed");
    let rendered = render(&ast, opts);
    assert_eq!(rendered, text, "round-trip mismatch");
}

// ---- per-construct round-trips -----------------------------------------------------------

#[test]
fn roundtrip_variable_with_default() {
    assert_idempotent(
        "const inputText: string = \"hi\"\n",
        &RenderOptions::default(),
    );
}

#[test]
fn roundtrip_exposed_variable() {
    assert_idempotent("let exposedFlag: bool = true\n", &RenderOptions::default());
}

#[test]
fn roundtrip_secret_decorator() {
    let text = "@secret\nconst apiKey: string = \"\"\n";
    let ast = parse(text).expect("parse should succeed");
    assert!(ast.variables[0].secret, "secret decorator should set flag");
    assert_eq!(render(&ast, &RenderOptions::default()), text);
}

#[test]
fn rejects_comment_between_secret_decorator_and_variable() {
    let err = parse("@secret\n// note\nconst password: string = \"real\"\n").unwrap_err();
    assert!(err.message.contains("immediately followed"));
}

#[test]
fn roundtrip_all_decorators() {
    // Every non-keyword variable setting surfaces as a decorator and round-trips.
    let text = "@description(\"the api key\")\n@category(\"Secrets\")\n@schema(\"{\\\"type\\\":\\\"string\\\"}\")\n@secret\n@readonly\n@runtime\nconst apiKey: string = \"\"\n";
    let ast = parse(text).expect("parse should succeed");
    let var = &ast.variables[0];
    assert_eq!(var.description.as_deref(), Some("the api key"));
    assert_eq!(var.category.as_deref(), Some("Secrets"));
    assert_eq!(var.schema.as_deref(), Some("{\"type\":\"string\"}"));
    assert!(var.secret);
    assert!(!var.editable);
    assert!(var.runtime_configured);
    assert_eq!(render(&ast, &RenderOptions::default()), text);
}

#[test]
fn roundtrip_interface_schema_variable() {
    let text = "interface ReportEntry {\n    title: string;\n    uri: string;\n    summary?: string | null = null;\n    tags?: string[] = [];\n}\n\nconst reportEntry: ReportEntry = {}\n";
    let ast = parse(text).expect("parse should accept interface declarations");
    let var = &ast.variables[0];

    assert_eq!(var.ty.base, "Struct");
    assert!(
        var.schema.is_some(),
        "interface type should generate schema"
    );
    assert_eq!(render(&ast, &RenderOptions::default()), text);
}

#[test]
fn parses_interface_fields_without_semicolons() {
    let text = "interface ReportEntry {\n    title: string\n    uri: string\n}\n\nconst reportEntry: ReportEntry = {}\n";
    let ast = parse(text).expect("parse should accept newline-separated interface fields");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "interface ReportEntry {\n    title: string;\n    uri: string;\n}\n\nconst reportEntry: ReportEntry = {}\n"
    );
}

#[test]
fn interface_schema_matches_legacy_schema_decorator() {
    let interface_text = "interface ReportEntry {\n    title: string;\n    uri: string;\n    summary?: string | null = null;\n    tags?: string[] = [];\n}\n\nconst reportEntry: ReportEntry = {}\n";
    let interface_ast = parse(interface_text).expect("interface form should parse");
    let generated_schema = interface_ast.variables[0]
        .schema
        .as_ref()
        .expect("interface variable should carry generated schema");

    let decorator_text = format!(
        "@schema({})\nconst reportEntry: Struct = {{}}\n",
        quote_string(generated_schema)
    );
    let decorator_ast = parse(&decorator_text).expect("decorator form should parse");

    assert_eq!(
        normalize_schema(
            decorator_ast.variables[0]
                .schema
                .as_ref()
                .expect("decorator schema should be preserved")
        ),
        normalize_schema(generated_schema),
        "interface-generated schema must match the legacy @schema path"
    );
}

#[test]
fn roundtrip_readonly_and_runtime() {
    let text = "@readonly\n@runtime\nlet userToken: string\n";
    let ast = parse(text).expect("parse should succeed");
    assert!(!ast.variables[0].editable);
    assert!(ast.variables[0].runtime_configured);
    assert_eq!(render(&ast, &RenderOptions::default()), text);
}

#[test]
fn rejects_arg_on_flag_decorator() {
    let err = parse("@secret(\"x\")\nconst k: string = \"\"\n").unwrap_err();
    assert!(err.message.contains("does not take an argument"));
}

#[test]
fn rejects_missing_arg_on_valued_decorator() {
    let err = parse("@category\nconst k: string = \"\"\n").unwrap_err();
    assert!(err.message.contains("requires a string argument"));
}

#[test]
fn roundtrip_json_default_vs_struct_literal() {
    // Canonical compact JSON stays a JSON literal; spaced struct literal stays an Object.
    let text = "onStart() {\n    structSet({ structIn: {}, payload: {\"a\":1} })\n}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn roundtrip_array_refs_vs_json_array() {
    let text = "onStart() {\n    agentTools({ tools: [search, fetchPage], ids: [1,2,3] })\n}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn roundtrip_condition_branch() {
    let text = "onStart() {\n    if (counter > 0) {\n        log({ text: \"pos\" })\n    } else {\n        log({ text: \"neg\" })\n    }\n}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn roundtrip_bound_exec_branch() {
    let text = "onStart() {\n    const apiCall = httpFetch({ request: request })\n    apiCall {\n        execSuccess: {\n            const text = httpResponseToText({ response: apiCall.response })\n        }\n        execError: {\n            logWarn({ message: \"fetch failed\" })\n        }\n    }\n}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn roundtrip_ternary_and_binary() {
    let text = "onStart() {\n    const value = pick({ result: (length({ s: name }) > 10) ? a() : b.value })\n}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn binary_expressions_use_standard_precedence() {
    let text = r#"onStart() {
    if (sender == expected && marker == "OK" || override) {
    }
}
"#;
    let ast = parse(text).expect("mixed boolean comparisons should parse");
    let flow_like_ast::Stmt::Branch {
        condition: Some(condition),
        ..
    } = &ast.events[0].body.stmts[0]
    else {
        panic!("expected conditional branch")
    };

    let flow_like_ast::Expr::Binary { op, lhs, rhs } = condition else {
        panic!("expected binary condition")
    };
    assert_eq!(op, "||");
    assert!(matches!(
        lhs.as_ref(),
        flow_like_ast::Expr::Binary { op, .. } if op == "&&"
    ));
    assert!(matches!(rhs.as_ref(), flow_like_ast::Expr::Ref(name) if name == "override"));

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "onStart() {\n    if (((sender == expected) && (marker == \"OK\")) || override) {\n    }\n}\n"
    );
}

#[test]
fn subtraction_is_not_lexed_as_a_negative_rhs() {
    let text = "onStart() {\n    let result = 10-3*2\n}\n";
    let ast = parse(text).expect("subtraction without whitespace should parse");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "onStart() {\n    let result = 10 - (3 * 2)\n}\n"
    );
}

#[test]
fn roundtrip_minimum_integer_literal() {
    assert_idempotent(
        "const floor: int = -9223372036854775808\n",
        &RenderOptions::default(),
    );
}

#[test]
fn roundtrip_member_vs_field() {
    // `.rows` (camel, pin-like) and `.report_id` (snake, data field) must both survive verbatim.
    let text = "onStart() {\n    const row = read({ id: query.rows[0].report_id })\n}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn bracketed_string_is_a_member_while_numeric_bracket_is_an_index() {
    let text = "onStart() {\n    consume({ reason: inputValues[\"row-rejection-reason\"], first: rows[0] })\n}\n";
    let ast = parse(text).expect("bracket access should parse");
    let flow_like_ast::Stmt::Call { call, .. } = &ast.events[0].body.stmts[0] else {
        panic!("expected call statement")
    };

    assert!(matches!(
        &call.args[0].value,
        flow_like_ast::Expr::Member { base, field }
            if field == "row-rejection-reason"
                && matches!(base.as_ref(), flow_like_ast::Expr::Ref(name) if name == "inputValues")
    ));
    assert!(matches!(
        &call.args[1].value,
        flow_like_ast::Expr::Index { base, index }
            if matches!(base.as_ref(), flow_like_ast::Expr::Ref(name) if name == "rows")
                && matches!(
                    index.as_ref(),
                    flow_like_ast::Expr::Literal(flow_like_ast::Literal::Int(0))
                )
    ));
    assert_eq!(render(&ast, &RenderOptions::default()), text);
}

#[test]
fn parses_const_alias_sugar_for_non_call_exprs() {
    let text = "onStart() {\n    const date = mail.date\n    const label = \"ready\"\n}\n";
    let ast = parse(text).expect("const aliases should parse even when the RHS is not a call");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "onStart() {\n    let date = mail.date\n    let label = \"ready\"\n}\n"
    );
}

#[test]
fn roundtrip_for_loop() {
    let text = "onStart() {\n    for (const item of forEach({ array: data.rows })) {\n        log({ text: item.value })\n    }\n}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn parses_untyped_let_assignment_sugar() {
    let text = "onStart() {\n    let rows = []\n    let row = structMake()\n    rows = arrayPush({ arrayIn: rows, value: row })\n}\n";
    let ast = parse(text).expect("parse should accept model-authored let assignment sugar");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "onStart() {\n    let rows = []\n    let row = structMake()\n    rows = arrayPush({ arrayIn: rows, value: row })\n}\n"
    );
}

#[test]
fn parses_bare_group_blocks_inside_event_body() {
    let text = "onStart() {\n    {\n        const request = httpMakeRequest({ method: \"GET\", url: \"https://example.com\" })\n        const response = httpFetch({ request: request.request })\n    }\n}\n";
    let ast = parse(text).expect("parse should accept grouped statement blocks");

    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "onStart() {\n    const request = httpMakeRequest({ method: \"GET\", url: \"https://example.com\" })\n    const response = httpFetch({ request: request.request })\n}\n"
    );
}

#[test]
fn parses_model_authored_gmail_ingest_shape() {
    let text = r#"@category("Gmail")
const GMAIL_ADDRESS: string = "GMAIL_ADDRESS"
@category("Gmail")
const GMAIL_APP_PASSWORD: string = "GMAIL_APP_PASSWORD"
@category("AI")
const OPENAI_API_KEY: string = "OPENAI_API_KEY"
@category("Embedding")
const EMBEDDING_BIT_NAME: string = "EMBEDDING_BIT"

ingestGmail() {
    const imapConn = emailImapConnect({ host: "imap.gmail.com", port: 993, username: GMAIL_ADDRESS, password: GMAIL_APP_PASSWORD, encryption: "Tls" })
    const inbox = mailImapInbox({ connection: imapConn, inbox: "INBOX" })
    const db = openLocalDb({ name: "gmail_vectors", userScoped: true, batchSize: 1000 })
    const embeddingModel = loadModel({ bit: { name: EMBEDDING_BIT_NAME } })
    const llmModel = aiGenerativeBuildOpenai({ apiKey: OPENAI_API_KEY })
    let rows = []

    for (const e of controlForEach({ array: emailRefs })) {
        const mail = emailImapInboxFetchMail({ emailRef: e.value })
        let subject = ""
        const subjectGet = structGet({ struct: mail, field: "subject" })
        if (subjectGet.found) { subject = subjectGet.value }

        let body = ""
        const bodyTextGet = structGet({ struct: mail, field: "bodyText" })
        if (bodyTextGet.found) { body = bodyTextGet.value } else {
            const bodyHtmlGet = structGet({ struct: mail, field: "bodyHtml" })
            if (bodyHtmlGet.found) { body = bodyHtmlGet.value }
        }

        const content = stringJoin({ strings: [subject, "\n\n", body], separator: "" })
        const chunks = chunkText({ text: content, model: embeddingModel, capacity: 1000, overlap: 200, markdown: false })
        for (const c of controlForEach({ array: chunks })) {
            const vector = embedDocument({ queryString: c.value, model: embeddingModel })
            const classification = aiGenerativeInvokeSimple({ model: llmModel, prompt: c.value })
            let row = structMake()
            row = structSet({ structIn: row, field: "id", value: cuid() })
            row = structSet({ structIn: row, field: "subject", value: subject })
            row = structSet({ structIn: row, field: "content", value: c.value })
            row = structSet({ structIn: row, field: "vector", value: vector })
            row = structSet({ structIn: row, field: "sentiment", value: classification.result })
            rows = arrayPush({ arrayIn: rows, value: row })
        }
    }

    const insertError = batchInsertLocalDb({ database: db, value: rows })
    return insertError
}
"#;

    let ast = parse(text).expect("parse should accept model-authored Gmail ingest FlowScript");
    let rendered = render(&ast, &RenderOptions::default());

    assert!(rendered.contains("ingestGmail()"));
    assert!(rendered.contains("let rows = []"));
    assert!(rendered.contains("for (const e of controlForEach"));
    assert!(rendered.contains("if (subjectGet.found)"));
}

#[test]
fn roundtrip_function_with_return() {
    let text =
        "function add(a: int, b: int): (sum: int) {\n    return sum({ x: a, y: b }).result\n}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn roundtrip_named_event() {
    let text = "eventsSimple dashboardLoad() {\n    logInfo({ message: \"hi\" })\n}\n";
    assert_idempotent(text, &RenderOptions::default());
    let ast = parse(text).expect("named event parses");
    assert_eq!(ast.events[0].name, "eventsSimple");
    assert_eq!(ast.events[0].event_name.as_deref(), Some("dashboardLoad"));
}

#[test]
fn roundtrip_named_event_with_params() {
    let text = "eventsGeneric addTargetAction(actionId: string) {\n    logInfo({ message: actionId })\n}\n";
    assert_idempotent(text, &RenderOptions::default());
    let ast = parse(text).expect("named generic event parses");
    assert_eq!(ast.events[0].name, "eventsGeneric");
    assert_eq!(ast.events[0].event_name.as_deref(), Some("addTargetAction"));
    assert_eq!(ast.events[0].params.len(), 1);
}

#[test]
fn unnamed_event_keeps_no_event_name() {
    let ast = parse("eventsSimple() {\n    logInfo({ message: \"hi\" })\n}\n")
        .expect("unnamed event parses");
    assert_eq!(ast.events[0].name, "eventsSimple");
    assert_eq!(ast.events[0].event_name, None);
}

#[test]
fn roundtrip_named_nested_handler() {
    let text = "eventsSimple() {\n    eventsSimple cronPass() {\n        logInfo({ message: \"tick\" })\n    }\n}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn rejects_unknown_decorator() {
    let err: ParseError = parse("@bogus\nconst x: string = \"\"\n").unwrap_err();
    assert!(err.message.contains("bogus"));
}

#[test]
fn labelled_branch_keeps_anchor_after_arm_label() {
    // The renderer emits arm label and anchor on one line (`{ // label   //@n:id`); the
    // lexer must split them so the branch keeps its identity anchor across round-trips.
    let text = "function guard(path: string) {\n    if (pathExists({ path: path })) { // exec_out_exists   //@n:branch1\n    } else { // exec_out_missing\n    }\n}\n";
    let ast = parse(text).expect("parse should succeed");
    match &ast.functions[0].body.stmts[0] {
        flow_like_ast::Stmt::Branch { anchor, arms, .. } => {
            assert_eq!(anchor.as_deref(), Some("branch1"));
            assert_eq!(arms[0].label, "exec_out_exists");
            assert_eq!(arms[1].label, "exec_out_missing");
        }
        other => panic!("expected labelled branch, got {other:?}"),
    }
    assert_idempotent(text, &anchored_opts());
}

#[test]
fn first_line_branch_comments_are_not_mistaken_for_exec_labels() {
    let text = r#"function decide(approved: bool) {
    if (approved) {
        // approved path
    } else {
        // revision path
    }
}
"#;

    let ast = parse(text).expect("ordinary comments inside boolean branches must parse");
    match &ast.functions[0].body.stmts[0] {
        flow_like_ast::Stmt::Branch {
            condition: Some(_),
            arms,
            ..
        } => {
            assert_eq!(arms[0].label, "True");
            assert_eq!(arms[1].label, "False");
            assert!(matches!(
                arms[0].body.stmts.first(),
                Some(flow_like_ast::Stmt::Comment(comment)) if comment == "approved path"
            ));
            assert!(matches!(
                arms[1].body.stmts.first(),
                Some(flow_like_ast::Stmt::Comment(comment)) if comment == "revision path"
            ));
        }
        other => panic!("expected a boolean branch, got {other:?}"),
    }
}

#[test]
fn roundtrip_array_of_union_interface_type() {
    // `string | null[]` would reparse as `string | (null[])`; unions under an array
    // suffix must render grouped.
    let text =
        "interface Entry {\n    tags?: (string | null)[] = [];\n}\n\nconst entry: Entry = {}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn interface_any_field_with_default_keeps_schema() {
    let text = "interface Cfg {\n    payload?: any = null;\n}\n\nconst cfg: Cfg = {}\n";
    let ast = parse(text).expect("parse should succeed");
    assert!(
        ast.variables[0].schema.is_some(),
        "an `any` field with a default must not wipe the generated schema"
    );
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn roundtrip_quoted_interface_field_names() {
    // JSON-schema property names are arbitrary strings; non-identifier names render quoted.
    let text = "interface Row {\n    \"content-type\": string;\n    \"created at\"?: float;\n}\n\nconst row: Row = {}\n";
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn interface_dedup_renames_references_too() {
    use flow_like_ast::{Container, TypeRef, VarDecl, interfaces_for_variables};

    fn struct_var(name: &str, schema: &str) -> VarDecl {
        VarDecl {
            name: name.to_string(),
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
        }
    }

    // Two schema families both define a `$defs` entry named `Meta` with different shapes.
    let alpha = struct_var(
        "alpha",
        r##"{"type":"object","properties":{"m":{"$ref":"#/$defs/Meta"}},"$defs":{"Meta":{"type":"object","properties":{"a":{"type":"string"}}}}}"##,
    );
    let beta = struct_var(
        "beta",
        r##"{"type":"object","properties":{"m":{"$ref":"#/$defs/Meta"}},"$defs":{"Meta":{"type":"object","properties":{"b":{"type":"integer"}}}}}"##,
    );

    let interfaces = interfaces_for_variables(&[alpha, beta]);
    let names: Vec<&str> = interfaces.iter().map(|d| d.name.as_str()).collect();
    assert!(
        names.contains(&"Meta") && names.contains(&"Meta2"),
        "colliding $defs interfaces must be deduplicated: {names:?}"
    );

    let beta_root = interfaces
        .iter()
        .find(|d| d.name == "Beta")
        .expect("beta root interface");
    let field_ty = flow_like_ast::render::render_interface_type(&beta_root.fields[0].ty);
    assert_eq!(
        field_ty, "Meta2",
        "the renamed interface must be re-referenced by its own family"
    );
}

#[test]
fn lexes_power_and_xor_operators() {
    // `int_power`/`float_power` render as `**`, `bool_xor` as `^`; both must tokenize.
    let text = "function calc(a: int, b: int) {\n    return ((a ** b) == (a ^ b))\n}\n";
    parse(text).expect("`**` and `^` should tokenize");
}

#[test]
fn lexes_non_ascii_identifiers() {
    // to_camel_case keeps unicode alphanumerics, so rendered names can carry them.
    let text = "const größe: float = 1.5\n";
    let ast = parse(text).expect("non-ASCII identifiers should lex");
    assert_eq!(ast.variables[0].name, "größe");
}

#[test]
fn deep_nesting_errors_instead_of_overflowing() {
    let mut expr = String::from("1");
    for _ in 0..2000 {
        expr = format!("({expr})");
    }
    let text = format!("function calc(a: int) {{\n    return {expr}\n}}\n");
    let err = parse(&text).expect_err("deep nesting must be a parse error, not a crash");
    assert!(err.message.contains("nesting too deep"));
}

#[test]
fn deep_parenthesized_interface_type_errors_instead_of_overflowing() {
    // Parenthesised interface types recurse into `interface_type`; the recursion budget must
    // cover them so `((((…))))` can't overflow the stack on user-authored input.
    let mut ty = String::from("string");
    for _ in 0..2000 {
        ty = format!("({ty})");
    }
    let text = format!("interface Deep {{\n    field: {ty};\n}}\n\nconst d: Deep = {{}}\n");
    let err = parse(&text).expect_err("deep parenthesised type must error, not crash");
    assert!(err.message.contains("nesting too deep"));
}

#[test]
fn trailing_at_comment_is_not_an_anchor() {
    let text = "function noop(a: int) {\n    logInfo({ message: a }) //@todo revisit\n}\n";
    let ast = parse(text).expect("parse should succeed");
    match &ast.functions[0].body.stmts[0] {
        flow_like_ast::Stmt::Call { anchor, .. } => {
            assert_eq!(
                anchor.as_deref(),
                None,
                "user comment must not become an anchor"
            );
        }
        other => panic!("expected call, got {other:?}"),
    }
}

#[test]
fn comment_with_embedded_anchor_pattern_splits_only_on_anchor_kinds() {
    // `//@x:` sequences that are not anchor kinds stay part of the label text.
    let text = "function guard(path: string) {\n    if (pathExists({ path: path })) { // see //@q:not-an-anchor\n    } else {\n    }\n}\n";
    let ast = parse(text).expect("parse should succeed");
    match &ast.functions[0].body.stmts[0] {
        flow_like_ast::Stmt::Branch { anchor, arms, .. } => {
            assert_eq!(anchor.as_deref(), None);
            assert_eq!(arms[0].label, "see //@q:not-an-anchor");
        }
        other => panic!("expected labelled branch, got {other:?}"),
    }
}

#[test]
fn member_assignment_parses_to_field_assign() {
    let text = "function f() {\n    const pref = makePrefs({ multimodal: true }).preferences\n    pref.cost_weight = 0.5\n}\n";
    let ast = parse(text).expect("member field assignment should parse");
    match ast.functions[0].body.stmts.last().expect("has statements") {
        flow_like_ast::Stmt::FieldAssign {
            base, path, value, ..
        } => {
            assert_eq!(base, "pref", "keeps the base variable");
            assert_eq!(path, "cost_weight", "field path carries no leading dot");
            assert!(matches!(
                value,
                flow_like_ast::Expr::Literal(flow_like_ast::Literal::Float(_))
            ));
        }
        other => panic!("expected a field assignment, got {other:?}"),
    }
    // The dot form is first-class: it round-trips verbatim instead of re-rendering `structSet(`.
    let rendered = render(&ast, &RenderOptions::default());
    assert!(
        rendered.contains("pref.cost_weight = 0.5"),
        "field write must render as the dot form:\n{rendered}"
    );
    assert!(
        !rendered.contains("structSet("),
        "field write must not re-render as an explicit structSet:\n{rendered}"
    );
    // The dot form itself round-trips verbatim (the surrounding `const` alias canonicalizes to
    // `let`, so idempotency is asserted on a self-contained snippet).
    assert_idempotent(
        "function f() {\n    pref.cost_weight = 0.5\n}\n",
        &RenderOptions::default(),
    );
}

#[test]
fn nested_member_assignment_builds_dot_path() {
    let text = "function f() {\n    const p = makePrefs({}).preferences\n    p.a.b = 1\n}\n";
    let ast = parse(text).expect("nested member assignment should parse");
    let flow_like_ast::Stmt::FieldAssign { base, path, .. } =
        ast.functions[0].body.stmts.last().unwrap()
    else {
        panic!("expected a field assignment");
    };
    assert_eq!(base, "p");
    assert_eq!(path, "a.b", "nested fields join into a dot path");
    assert_idempotent(
        "function f() {\n    p.a.b = 1\n}\n",
        &RenderOptions::default(),
    );
}

#[test]
fn field_assign_dot_form_is_idempotent() {
    // Snake-case field (dot separator) and a value that is itself a member/pin access.
    assert_idempotent(
        "function f() {\n    x.a_b = 1\n}\n",
        &RenderOptions::default(),
    );
    assert_idempotent(
        "function f() {\n    x.field = call().out\n}\n",
        &RenderOptions::default(),
    );
}

#[test]
fn field_assign_bracket_and_nested_paths_roundtrip() {
    // A bracket-rooted path (`items[0]`) carries no leading dot; mixed field/index paths render
    // back verbatim.
    let ast = parse("function f() {\n    items[0] = 1\n}\n").expect("bracket lvalue should parse");
    let flow_like_ast::Stmt::FieldAssign { base, path, .. } =
        ast.functions[0].body.stmts.last().unwrap()
    else {
        panic!("expected a field assignment");
    };
    assert_eq!(base, "items");
    assert_eq!(path, "[0]", "bracket-rooted path has no leading dot");
    assert_idempotent(
        "function f() {\n    items[0] = 1\n}\n",
        &RenderOptions::default(),
    );
    assert_idempotent(
        "function f() {\n    row.items[0].name = \"x\"\n}\n",
        &RenderOptions::default(),
    );
}

#[test]
fn field_assign_renders_from_hand_built_ast() {
    use flow_like_ast::{Block, BoardAst, Expr, FnDecl, Literal, Stmt};

    let field_assign = |anchor: Option<&str>| Stmt::FieldAssign {
        base: "row".to_string(),
        path: "id".to_string(),
        value: Expr::Literal(Literal::Int(7)),
        anchor: anchor.map(str::to_string),
    };
    let build = |stmt: Stmt| BoardAst {
        functions: vec![FnDecl {
            name: "f".to_string(),
            params: Vec::new(),
            returns: Vec::new(),
            body: Block { stmts: vec![stmt] },
            anchor: None,
        }],
        ..BoardAst::default()
    };

    let plain = render(&build(field_assign(None)), &RenderOptions::default());
    assert!(
        plain.contains("row.id = 7"),
        "hand-built field assign renders as the dot form:\n{plain}"
    );

    let anchored = render(&build(field_assign(Some("set1"))), &anchored_opts());
    assert!(
        anchored.contains("row.id = 7   //@n:set1"),
        "anchored field assign renders its node anchor:\n{anchored}"
    );
}

// ---- full-fixture idempotency ------------------------------------------------------------

const FIXTURE_A: &str = include_str!("../../../tests/ast/bypaw6n2ksuvrw0kcaj14omz.flow");
const FIXTURE_A_ANCHORED: &str =
    include_str!("../../../tests/ast/bypaw6n2ksuvrw0kcaj14omz.anchored.flow");
const FIXTURE_B: &str = include_str!("../../../tests/ast/ttwctnp08u18sg2z6nmcqqak.flow");
const FIXTURE_B_ANCHORED: &str =
    include_str!("../../../tests/ast/ttwctnp08u18sg2z6nmcqqak.anchored.flow");
/// Dashboard board that drives pages/widgets: DataFusion SQL feeding `a2ui*` element/widget calls.
const FIXTURE_DASHBOARD: &str =
    include_str!("../../../tests/ast/widgets-pages/bypaw6n2ksuvrw0kcaj14omz.flow");
const FIXTURE_DASHBOARD_ANCHORED: &str =
    include_str!("../../../tests/ast/widgets-pages/bypaw6n2ksuvrw0kcaj14omz.anchored.flow");

#[test]
fn fixture_a_idempotent() {
    assert_idempotent(FIXTURE_A, &RenderOptions::default());
}

#[test]
fn fixture_a_anchored_idempotent() {
    assert_idempotent(FIXTURE_A_ANCHORED, &anchored_opts());
}

#[test]
fn fixture_b_idempotent() {
    assert_idempotent(FIXTURE_B, &RenderOptions::default());
}

#[test]
fn fixture_b_anchored_idempotent() {
    assert_idempotent(FIXTURE_B_ANCHORED, &anchored_opts());
}

#[test]
fn fixture_dashboard_idempotent() {
    assert_idempotent(FIXTURE_DASHBOARD, &RenderOptions::default());
}

#[test]
fn fixture_dashboard_anchored_idempotent() {
    assert_idempotent(FIXTURE_DASHBOARD_ANCHORED, &anchored_opts());
}

// ---- anchors must be trailing (C1) -------------------------------------------------------

/// Anchors (`//@n:` / `//@v:` / `//@l:`) are the ONLY stable identity across a round-trip.
/// `take_anchor` used to grab the next anchor comment regardless of position, so a statement
/// swallowed the anchor authored on the FOLLOWING line: reconcile then rewrote that node with the
/// wrong statement's content, created a duplicate for the statement whose anchor was taken, and
/// reported the original as deleted.
#[test]
fn own_line_anchor_is_not_stolen_by_the_previous_statement() {
    let ast = parse(
        "eventsSimple() {\n    const a = foo({ x: 1 })\n    //@n:NODE_B\n    const b = bar({ y: 2 })\n}\n",
    )
    .expect("parses");
    let rendered = render(&ast, &anchored_opts());
    assert!(
        !rendered.contains("foo({ x: 1 })   //@n:NODE_B"),
        "the anchor on the next line must not attach to the previous statement:\n{rendered}"
    );
}

/// The renderer emits a board comment whose text happens to look like an anchor as
/// `// @n:X` (space after `//`). Parsing that must not consume it as an anchor, or the line is
/// silently converted into identity metadata and the comment disappears.
#[test]
fn renderer_emitted_comment_containing_anchor_text_round_trips() {
    assert_idempotent(
        "eventsSimple() {\n    // @n:X\n    logInfo({ message: \"hi\" })\n}\n",
        &anchored_opts(),
    );
}

/// A fan-out body contains nothing but labelled arms, and `BranchArm` has no anchor field, so an
/// anchor before the first arm is unambiguously the branch's. It stays accepted on its own line
/// and is re-rendered trailing — without this exemption, currently-parsing documents would fail
/// with `expected identifier, found Comment(...)`.
#[test]
fn branch_fanout_keeps_an_anchor_on_the_line_after_the_brace() {
    for source in [
        "eventsSimple() {\n    httpFetch({ request: r }) {\n//@n:F\n        execSuccess: {\n            logInfo({ message: \"ok\" })\n        }\n    }\n}\n",
        "eventsSimple() {\n    const h = httpFetch({ request: r })\n    h {\n//@n:B\n        execSuccess: {\n            logInfo({ message: \"ok\" })\n        }\n    }\n}\n",
    ] {
        let ast = parse(source).expect("fan-out parses");
        let rendered = render(&ast, &anchored_opts());
        assert!(
            rendered.contains("//@n:F") || rendered.contains("//@n:B"),
            "the branch anchor must survive:\n{rendered}"
        );
    }
}

/// The positional check is byte-based, not `Token::line`-based: a multi-line string literal
/// records its START line, so a line comparison would wrongly reject a genuinely trailing anchor
/// after one. Guards against a "simplification" back to `Token::line`.
#[test]
fn trailing_anchor_after_a_multiline_string_is_still_taken() {
    let ast = parse("eventsSimple() {\n    const a = foo({ x: \"one\ntwo\" })   //@n:KEEP\n}\n")
        .expect("parses");
    let rendered = render(&ast, &anchored_opts());
    assert!(
        rendered.contains("//@n:KEEP"),
        "a trailing anchor after a multi-line string must still be taken:\n{rendered}"
    );
}

// ---- escapes, unary `!`, `else if` (D-series) --------------------------------------------

/// `\b`/`\f` must lex: a `Literal::Json` span is re-emitted verbatim after serde_json validates
/// it, and the lexer runs over the whole file first — so without them a JSON default that would
/// round-trip byte-exactly failed at lex time.
#[test]
fn accepts_json_escape_set_and_single_quote() {
    for text in ["\"a\\bb\"", "\"a\\fb\"", "\"it\\'s\"", "\"a\\/b\""] {
        let source = format!("eventsSimple() {{\n    logInfo({{ message: {text} }})\n}}\n");
        parse(&source).unwrap_or_else(|e| panic!("{text} must lex: {e:?}"));
    }
}

/// `\'` denotes a character that needs no escape, so it normalizes away — same as `\/` and
/// `\uXXXX` already do. Pinned so the normalization is contractual, not accidental.
#[test]
fn escaped_single_quote_normalizes_to_a_bare_apostrophe() {
    let ast = parse("eventsSimple() {\n    logInfo({ message: \"it\\'s\" })\n}\n").expect("parses");
    let rendered = render(&ast, &RenderOptions::default());
    assert!(rendered.contains("\"it's\""), "{rendered}");
}

/// Unknown escapes stay a HARD error. Passing them through would silently turn a regex `"\d+"`
/// into `"d+"`, which applies cleanly and fails at run time.
#[test]
fn rejects_unknown_escape_so_regex_backslashes_stay_loud() {
    for text in ["\"\\d+\"", "\"a\\zb\""] {
        let source = format!("eventsSimple() {{\n    logInfo({{ message: {text} }})\n}}\n");
        assert!(parse(&source).is_err(), "{text} must stay a hard error");
    }
}

/// `if (!(cond)) { … }` is the renderer's own single-arm form and must be byte-stable.
#[test]
fn renderer_negated_single_arm_form_is_a_fixpoint() {
    assert_idempotent(
        "eventsSimple() {\n    if (!(flag)) {\n        logInfo({ message: \"no\" })\n    }\n}\n",
        &RenderOptions::default(),
    );
}

/// `if (!c) { } else { }` was a HARD ERROR (`expected Colon, found LParen`). It now parses, with
/// the negation carried by a real `boolNot` node because both arms exist.
#[test]
fn negated_condition_with_else_parses_and_is_a_fixpoint() {
    let source = "eventsSimple() {\n    if (boolNot({ boolean: flag })) {\n        logInfo({ message: \"a\" })\n    } else {\n        logInfo({ message: \"b\" })\n    }\n}\n";
    assert_idempotent(source, &RenderOptions::default());
    let ast = parse("eventsSimple() {\n    if (!flag) {\n        logInfo({ message: \"a\" })\n    } else {\n        logInfo({ message: \"b\" })\n    }\n}\n")
        .expect("`if (!c) {} else {}` must parse");
    assert_eq!(render(&ast, &RenderOptions::default()), source);
}

/// `else if` desugars to the nested ladder the renderer emits.
#[test]
fn else_if_desugars_to_the_nested_ladder() {
    let ast = parse("eventsSimple() {\n    if (a) {\n        logInfo({ message: \"a\" })\n    } else if (b) {\n        logInfo({ message: \"b\" })\n    }\n}\n")
        .expect("`else if` must parse");
    let rendered = render(&ast, &RenderOptions::default());
    assert!(rendered.contains("} else {"), "{rendered}");
    assert!(!rendered.contains("else if"), "{rendered}");
    assert_eq!(
        render(
            &parse(&rendered).expect("reparse"),
            &RenderOptions::default()
        ),
        rendered
    );
}

/// A loop head must be a loop-node call. `boolNot` IS an `Expr::Call` but has zero exec outputs,
/// so accepting it built a node whose body was never wired, with no diagnostics at all.
#[test]
fn boolean_loop_head_is_rejected_with_an_actionable_message() {
    for source in [
        "eventsSimple() {\n    while (!done) {\n        logInfo({ message: \"x\" })\n    }\n}\n",
        "eventsSimple() {\n    for (const v of !items) {\n        logInfo({ message: \"x\" })\n    }\n}\n",
    ] {
        let err = parse(source).expect_err("a boolean loop head must be rejected");
        assert!(err.message.contains("loop-node call"), "{:?}", err.message);
    }
}

/// `-x` has no catalog lowering, so it stays an error — but an actionable one instead of a
/// `Debug`-formatted token dump. A negative literal is unaffected.
#[test]
fn unary_minus_is_rejected_with_a_workaround_and_negative_literals_still_parse() {
    let err = parse("eventsSimple() {\n    const a = intAdd({ integer1: -x, integer2: 1 })\n}\n")
        .expect_err("unary minus must be rejected");
    assert!(err.message.contains("0 - x"), "{:?}", err.message);
    parse("eventsSimple() {\n    const a = intAdd({ integer1: -1, integer2: 1 })\n}\n")
        .expect("a negative literal must still parse");
}
