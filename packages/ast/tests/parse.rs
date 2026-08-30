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
    assert_idempotent("const inputText = \"hi\"\n", &RenderOptions::default());
}

#[test]
fn roundtrip_exposed_variable() {
    assert_idempotent("let exposedFlag = true\n", &RenderOptions::default());
}

#[test]
fn roundtrip_secret_decorator() {
    let text = "@secret\nconst apiKey = \"\"\n";
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
fn roundtrip_bare_function_cache_decorator() {
    let text = "@cache\nfunction lookup(key: string): (value: string) {\n    return key\n}\n";
    let ast = parse(text).expect("bare cache decorator should parse");
    let cache = ast.functions[0]
        .cache
        .as_ref()
        .expect("bare decorator should enable caching");
    assert_eq!(cache.namespace, "global");
    assert_eq!(cache.ttl_seconds, Some(300));
    assert_eq!(cache.scope, flow_like_ast::FunctionCacheScope::App);
    assert_eq!(render(&ast, &RenderOptions::default()), text);
}

#[test]
fn empty_function_cache_settings_use_and_render_as_semantic_defaults() {
    let ast =
        parse("@cache({})\nfunction lookup() {\n}\n").expect("empty cache settings should parse");
    assert_eq!(
        ast.functions[0].cache,
        Some(flow_like_ast::FunctionCache::default())
    );
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "@cache\nfunction lookup() {\n}\n"
    );
}

#[test]
fn function_cache_fields_parse_in_any_order_and_render_canonically() {
    use flow_like_ast::FunctionCacheScope;

    let source = "@cache({ scope: \"user\", ttlSeconds: 3600, namespace: \"pricing\" })\nfunction quote() {\n}\n";
    let expected = "@cache({ namespace: \"pricing\", ttlSeconds: 3600, scope: \"user\" })\nfunction quote() {\n}\n";
    let ast = parse(source).expect("structured cache decorator should parse");
    let cache = ast.functions[0]
        .cache
        .as_ref()
        .expect("function should carry cache settings");
    assert_eq!(cache.namespace, "pricing");
    assert_eq!(cache.ttl_seconds, Some(3600));
    assert_eq!(cache.scope, FunctionCacheScope::User);
    assert_eq!(render(&ast, &RenderOptions::default()), expected);
}

#[test]
fn explicit_function_cache_defaults_render_as_bare_decorator() {
    let source = "@cache({ namespace: \"global\", ttlSeconds: 300, scope: \"app\" })\nfunction lookup() {\n}\n";
    let ast = parse(source).expect("explicit defaults should parse");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "@cache\nfunction lookup() {\n}\n"
    );
}

#[test]
fn explicit_zero_function_cache_ttl_remains_permanent() {
    let source = "@cache({ ttlSeconds: 0 })\nfunction lookup() {\n}\n";
    let ast = parse(source).expect("zero cache TTL should parse");
    assert_eq!(
        ast.functions[0].cache.as_ref().unwrap().ttl_seconds,
        Some(0)
    );
    assert_eq!(render(&ast, &RenderOptions::default()), source);
}

#[test]
fn deserialized_function_cache_uses_flowscript_semantic_defaults() {
    let cache: flow_like_ast::FunctionCache =
        serde_json::from_value(serde_json::json!({})).expect("cache JSON should deserialize");
    assert_eq!(cache, flow_like_ast::FunctionCache::default());
    assert_eq!(cache.namespace, "global");
    assert_eq!(cache.ttl_seconds, Some(300));
}

#[test]
fn function_cache_ttl_supports_the_full_persisted_u64_range() {
    let source = format!(
        "@cache({{ ttlSeconds: {} }})\nfunction lookup() {{\n}}\n",
        u64::MAX
    );
    let ast = parse(&source).expect("the largest persisted cache TTL should parse");
    assert_eq!(
        ast.functions[0].cache.as_ref().unwrap().ttl_seconds,
        Some(u64::MAX)
    );
    assert_eq!(render(&ast, &RenderOptions::default()), source);
}

#[test]
fn rejects_duplicate_or_unknown_function_cache_fields() {
    let duplicate =
        parse("@cache({ namespace: \"a\", namespace: \"b\" })\nfunction lookup() {\n}\n")
            .unwrap_err();
    assert!(duplicate.message.contains("duplicate field `namespace`"));

    let unknown = parse("@cache({ prefix: \"a\" })\nfunction lookup() {\n}\n").unwrap_err();
    assert!(unknown.message.contains("unknown field `prefix`"));
}

#[test]
fn rejects_invalid_function_cache_values_and_arguments() {
    for (source, expected) in [
        (
            "@cache({ ttlSeconds: -1 })\nfunction lookup() {\n}\n",
            "non-negative integer",
        ),
        (
            "@cache({ ttlSeconds: 1.5 })\nfunction lookup() {\n}\n",
            "non-negative integer",
        ),
        (
            "@cache({ namespace: 7 })\nfunction lookup() {\n}\n",
            "namespace` must be a string",
        ),
        (
            "@cache({ scope: \"team\" })\nfunction lookup() {\n}\n",
            "must be \"app\" or \"user\"",
        ),
        (
            "@cache(\"pricing\")\nfunction lookup() {\n}\n",
            "takes a settings object",
        ),
    ] {
        let err = parse(source).unwrap_err();
        assert!(
            err.message.contains(expected),
            "expected {expected:?} in {:?}",
            err.message
        );
    }
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
        "const floor = -9223372036854775808\n",
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
fn interface_date_fields_generate_date_time_schema_without_a_date_definition() {
    let text = "interface AuditRow {\n    checkpoints?: Date[];\n    created_at: Date;\n    updated_at?: Date | null = null;\n}\n\nconst auditRow: AuditRow = {}\n";
    let ast = parse(text).expect("Date fields should parse in FlowScript interfaces");
    let schema: serde_json::Value = serde_json::from_str(
        ast.variables[0]
            .schema
            .as_deref()
            .expect("interface variable should carry generated schema"),
    )
    .expect("generated interface schema should be JSON");

    assert_eq!(
        schema.pointer("/properties/created_at/format"),
        Some(&serde_json::Value::String("date-time".to_string()))
    );
    assert_eq!(
        schema.pointer("/properties/checkpoints/items/format"),
        Some(&serde_json::Value::String("date-time".to_string()))
    );
    assert!(
        schema["properties"]["updated_at"]["anyOf"]
            .as_array()
            .is_some_and(|variants| variants.iter().any(|variant| {
                variant.get("format").and_then(serde_json::Value::as_str) == Some("date-time")
            })),
        "nullable Date should retain a date-time variant: {schema}"
    );
    assert!(
        schema
            .get("$defs")
            .and_then(serde_json::Value::as_object)
            .is_none_or(|defs| !defs.contains_key("Date")),
        "the built-in Date type must not become an unresolved schema ref: {schema}"
    );
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn date_formatted_json_schema_fields_render_as_date_and_survive_reparse() {
    let source_schema = serde_json::json!({
        "title": "AuditRow",
        "type": "object",
        "properties": {
            "checkpoints": {
                "type": "array",
                "items": { "type": "string", "format": "date-time" }
            },
            "created_at": { "type": "string", "format": "date" },
            "observed_at": { "$ref": "#/$defs/UtcInstant" },
            "updated_at": { "type": ["string", "null"], "format": "date-time" }
        },
        "required": ["created_at", "observed_at"],
        "$defs": {
            "UtcInstant": { "type": "string", "format": "date-time" }
        }
    });
    let source = format!(
        "@schema({})\nconst auditRow: Struct = {{}}\n",
        quote_string(&source_schema.to_string())
    );
    let mut ast = parse(&source).expect("legacy schema-decorated variable should parse");
    ast.interfaces = flow_like_ast::interfaces_for_variables(&ast.variables);
    let rendered = render(&ast, &RenderOptions::default());

    for expected_field in [
        "checkpoints?: Date[];",
        "created_at: Date;",
        "observed_at: Date;",
        "updated_at?: Date | null;",
    ] {
        assert!(
            rendered.contains(expected_field),
            "rendered interface omitted `{expected_field}`:\n{rendered}"
        );
    }

    let reparsed = parse(&rendered).expect("rendered Date interface should reparse");
    let reparsed_schema: serde_json::Value = serde_json::from_str(
        reparsed.variables[0]
            .schema
            .as_deref()
            .expect("reparsed interface should regenerate its schema"),
    )
    .expect("reparsed interface schema should be JSON");

    for pointer in [
        "/properties/checkpoints/items/format",
        "/properties/created_at/format",
        "/properties/observed_at/format",
    ] {
        assert_eq!(
            reparsed_schema
                .pointer(pointer)
                .and_then(serde_json::Value::as_str),
            Some("date-time"),
            "Date schema was not retained at {pointer}: {reparsed_schema}"
        );
    }
    assert!(
        reparsed_schema["properties"]["updated_at"]["anyOf"]
            .as_array()
            .is_some_and(|variants| variants.iter().any(|variant| {
                variant.get("format").and_then(serde_json::Value::as_str) == Some("date-time")
            })),
        "nullable Date should survive schema -> FlowScript -> schema: {reparsed_schema}"
    );
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
            cache: None,
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

// ---- loops (phase 3) ---------------------------------------------------------------------

/// A boolean loop head is the sugared `while (cond)`: `!done` parses to a `boolNot(…)` call
/// (its canonical spelling), which the parser stores as the head and reconcile turns into the
/// loop node's condition because `bool_not` is not a loop node.
#[test]
fn boolean_loop_heads_are_sugared_loop_conditions() {
    let text =
        "eventsSimple() {\n    while (!done) {\n        logInfo({ message: \"x\" })\n    }\n}\n";
    let ast = parse(text).expect("boolean loop heads parse");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "eventsSimple() {\n    while (boolNot({ boolean: done })) {\n        logInfo({ message: \"x\" })\n    }\n}\n"
    );
    // A call head with a plain binding is the handle form; reconcile reclassifies a non-loop
    // call as the iterable.
    let text = "eventsSimple() {\n    for (const v of !items) {\n    }\n}\n";
    let flow_like_ast::Stmt::Loop {
        bind,
        call,
        iterable,
        ..
    } = first_stmt(text)
    else {
        panic!("expected a loop");
    };
    assert!(iterable.is_none());
    assert_eq!(bind.as_deref(), Some("v"));
    assert_eq!(call.display, "boolNot");
}

#[test]
fn roundtrip_sugared_loop_forms() {
    for text in [
        "eventsSimple() {\n    for (const item of items) {\n        logInfo({ message: item })\n    }\n}\n",
        "eventsSimple() {\n    for (const [i, item] of items) {\n        logInfo({ message: i })\n    }\n}\n",
        "eventsSimple() {\n    @parallel\n    for (const item of items) {\n    }\n}\n",
        "eventsSimple() {\n    @parallel\n    for (const [i, item] of user.sources) {\n    }\n}\n",
        "eventsSimple() {\n    while (i < 3) {\n        i = i + 1\n    }\n}\n",
        "eventsSimple() {\n    while (boolNot({ boolean: done })) {\n    }\n}\n",
        "eventsSimple() {\n    for (const [i, x] of items.chunk(2)) {\n    }\n}\n",
        "eventsSimple() {\n    for (const x of items.chunk({ size: 2 })) {\n        logInfo({ message: x.value })\n    }\n}\n",
        "eventsSimple() {\n    for (const x of items[0].rows) {\n    }\n}\n",
    ] {
        assert_idempotent(text, &RenderOptions::default());
    }
    assert_idempotent(
        "eventsSimple() {\n    for (const item of items) {   //@n:loop1\n    }\n    @parallel\n    for (const [i, item] of items) {   //@n:loop2\n    }\n    while (done == false) {   //@n:loop3\n    }\n}\n",
        &anchored_opts(),
    );
}

#[test]
fn sugared_loop_heads_carry_iterable_and_bindings() {
    let flow_like_ast::Stmt::Loop {
        keyword,
        bind,
        call,
        iterable,
        element,
        index,
        ..
    } = first_stmt("eventsSimple() {\n    for (const item of items) {\n    }\n}\n")
    else {
        panic!("expected a loop");
    };
    assert_eq!(keyword, "forEach");
    assert!(bind.is_none() && call.display.is_empty());
    assert!(matches!(iterable, Some(flow_like_ast::Expr::Ref(name)) if name == "items"));
    assert_eq!(element.as_deref(), Some("item"));
    assert!(index.is_none());

    let flow_like_ast::Stmt::Loop {
        keyword,
        iterable,
        element,
        index,
        ..
    } = first_stmt(
        "eventsSimple() {\n    @parallel\n    for (const [i, x] of items.chunk(2)) {\n    }\n}\n",
    )
    else {
        panic!("expected a loop");
    };
    assert_eq!(keyword, "forEachParallel");
    assert!(matches!(iterable, Some(flow_like_ast::Expr::Call(_))));
    assert_eq!(
        (element.as_deref(), index.as_deref()),
        (Some("x"), Some("i"))
    );

    // A plain-identifier head over a call keeps the explicit handle form; reconcile decides
    // whether the call is a loop node.
    let flow_like_ast::Stmt::Loop {
        bind,
        call,
        iterable,
        ..
    } = first_stmt("eventsSimple() {\n    for (const x of items.chunk({ size: 2 })) {\n    }\n}\n")
    else {
        panic!("expected a loop");
    };
    assert_eq!(bind.as_deref(), Some("x"));
    assert_eq!(call.display, "chunk");
    assert!(iterable.is_none());

    let flow_like_ast::Stmt::Loop {
        keyword,
        iterable,
        element,
        ..
    } = first_stmt("eventsSimple() {\n    while (i < 3) {\n    }\n}\n")
    else {
        panic!("expected a loop");
    };
    assert_eq!(keyword, "while");
    assert!(matches!(iterable, Some(flow_like_ast::Expr::Binary { .. })));
    assert!(element.is_none());
}

/// `@parallel` settles the head ambiguity rather than being rejected by it.
///
/// A `for` head that is a call is ambiguous by construction — `for_stmt` says so itself — because
/// `controlForEach({ … })` and `links.toArray()` are the same shape and only the resolved node type
/// tells them apart. Rejecting a call head under `@parallel` therefore rejected the renderer's own
/// output for any parallel loop over a computed array. The decorator only exists on the sugared
/// form, so it is taken as the answer, and a head that really is a loop-node call is reconcile's to
/// diagnose.
#[test]
fn parallel_decorator_takes_a_call_head_as_the_sugared_array() {
    let ast = parse(
        "eventsSimple() {\n    @parallel\n    for (const item of links.toArray()) {\n    }\n}\n",
    )
    .expect("a computed array head is the sugared form");
    let flow_like_ast::model::Stmt::Loop {
        keyword,
        iterable,
        element,
        ..
    } = &ast.events[0].body.stmts[0]
    else {
        panic!("expected a loop");
    };
    assert_eq!(keyword, "forEachParallel");
    assert_eq!(element.as_deref(), Some("item"));
    assert!(
        iterable.is_some(),
        "the call head is the array, not the node"
    );

    let err =
        parse("eventsSimple() {\n    @parallel(\"x\")\n    for (const h of items) {\n    }\n}\n")
            .expect_err("argument");
    assert!(
        err.message.contains("does not take an argument"),
        "{}",
        err.message
    );

    let err = parse("eventsSimple() {\n    @secret\n    for (const h of items) {\n    }\n}\n")
        .expect_err("unknown loop decorator");
    assert!(
        err.message.contains("unknown decorator `@secret`"),
        "{}",
        err.message
    );
}

// ---- template literals (phase 3) ---------------------------------------------------------

#[test]
fn roundtrip_template_literals() {
    for text in [
        "eventsSimple() {\n    let m = `hello ${name}`\n}\n",
        "eventsSimple() {\n    let m = `Topic ${label}\nGoal: ${source.goal}`\n}\n",
        "eventsSimple() {\n    let m = `${a.b} and ${f({ x: 1 })} or ${cond ? \"a\" : \"b\"}`\n}\n",
        "eventsSimple() {\n    let m = `outer ${`inner ${x}`} done`\n}\n",
        "eventsSimple() {\n    let m = `literal {name} braces`\n}\n",
        "eventsSimple() {\n    let m = `escaped \\` tick, \\${not} and back\\\\slash`\n}\n",
        "eventsSimple() {\n    let m = `tab\\there\\r`\n}\n",
        "eventsSimple() {\n    let m = ``\n}\n",
        "eventsSimple() {\n    logInfo({ message: `${count} item(s)` })\n}\n",
        "eventsSimple() {\n    let m = `quote \" and ' inside`\n}\n",
        "eventsSimple() {\n    let m = `${\"str with } brace\"} ${g({ a: { b: 1 } })}`\n}\n",
    ] {
        assert_idempotent(text, &RenderOptions::default());
    }
}

#[test]
fn template_literal_parts_are_text_and_expressions() {
    let flow_like_ast::Stmt::LocalAlias { value, .. } =
        first_stmt("eventsSimple() {\n    let m = `Topic ${label}\nGoal: ${source.goal}`\n}\n")
    else {
        panic!("expected a local alias");
    };
    let flow_like_ast::Expr::Template { parts } = value else {
        panic!("expected a template literal, got {value:?}");
    };
    assert_eq!(parts.len(), 4);
    assert!(matches!(&parts[0], flow_like_ast::TemplatePart::Text(t) if t == "Topic "));
    assert!(matches!(
        &parts[1],
        flow_like_ast::TemplatePart::Expr(flow_like_ast::Expr::Ref(name)) if name == "label"
    ));
    assert!(matches!(&parts[2], flow_like_ast::TemplatePart::Text(t) if t == "\nGoal: "));
    assert!(matches!(
        &parts[3],
        flow_like_ast::TemplatePart::Expr(flow_like_ast::Expr::Field { pin, .. }) if pin == "goal"
    ));

    // Single-quoted and double-quoted strings normalize the same way a template's escapes do.
    let flow_like_ast::Stmt::LocalAlias { value, .. } =
        first_stmt("eventsSimple() {\n    let m = `a\\`b\\${c}\\u0041`\n}\n")
    else {
        panic!("expected a local alias");
    };
    let flow_like_ast::Expr::Template { parts } = value else {
        panic!("expected a template literal");
    };
    assert!(matches!(&parts[0], flow_like_ast::TemplatePart::Text(t) if t == "a`b${c}A"));
}

#[test]
fn template_literal_errors_are_positioned_in_the_document() {
    let err = parse("eventsSimple() {\n    let m = `open ${x}\n}\n").expect_err("unterminated");
    assert!(
        err.message.contains("unterminated template literal"),
        "{}",
        err.message
    );
    assert_eq!((err.line, err.col), (2, 13));

    let err = parse("eventsSimple() {\n    let m = `x ${a +} y`\n}\n").expect_err("bad expr");
    assert_eq!(err.line, 2);
    assert!(err.col > 13, "{err:?}");

    let err = parse("eventsSimple() {\n    let m = `x ${} y`\n}\n").expect_err("empty");
    assert!(err.message.contains("empty `${}`"), "{}", err.message);

    let err = parse("eventsSimple() {\n    let m = `x ${a b} y`\n}\n").expect_err("trailing");
    assert!(
        err.message.contains("unexpected token after"),
        "{}",
        err.message
    );

    let err = parse("eventsSimple() {\n    let m = `bad \\q escape`\n}\n").expect_err("escape");
    assert!(err.message.contains("invalid escape"), "{}", err.message);
}

#[test]
fn template_literal_after_binary_operator_and_as_receiver() {
    assert_idempotent(
        "eventsSimple() {\n    let m = `a${x}` + `b`\n    let n = `x ${y}`.trim()\n    let o = `${1 - 2}`\n}\n",
        &RenderOptions::default(),
    );
}

// ---- hand-writability leniency (phase 0) -------------------------------------------------

/// `-x` desugars to `0 - x`, which reconcile lowers to `int_subtract`/`float_subtract` by
/// operand type. A negative literal keeps lexing as one token, and the unary binds tighter
/// than any binary operator.
#[test]
fn unary_minus_desugars_to_zero_minus_operand() {
    let ast = parse("eventsSimple() {\n    const a = intAdd({ integer1: -x, integer2: 1 })\n}\n")
        .expect("unary minus must parse");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "eventsSimple() {\n    const a = intAdd({ integer1: 0 - x, integer2: 1 })\n}\n"
    );

    let ast = parse("eventsSimple() {\n    let y = -x * 2 + -(a.b)\n}\n").expect("parses");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "eventsSimple() {\n    let y = ((0 - x) * 2) + (0 - a.b)\n}\n"
    );

    assert_idempotent(
        "eventsSimple() {\n    const a = intAdd({ integer1: -1, integer2: 1 })\n}\n",
        &RenderOptions::default(),
    );
}

/// Statement terminators are optional noise: any number of `;` between statements, before a
/// closing brace, at top level, and between a statement and its trailing anchor.
#[test]
fn semicolons_are_skipped_everywhere_and_never_rendered() {
    let source = "const n: int = 1;;\n\neventsSimple() {\n    ;\n    const a = foo({ x: n });   //@n:A\n    bar();\n    return;\n    ;\n};\n";
    let ast = parse(source).expect("semicolons must be accepted");
    assert_eq!(
        render(&ast, &anchored_opts()),
        "const n = 1\n\neventsSimple() {\n    const a = foo({ x: n })   //@n:A\n    bar()\n    return\n}\n"
    );
    assert_idempotent(
        "function f(): (out: int) {\n    return 1\n}\n",
        &RenderOptions::default(),
    );
    let ast = parse("function f(): (out: int) {\n    return 1;\n}\n").expect("parses");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "function f(): (out: int) {\n    return 1\n}\n"
    );
}

/// Single-quoted strings lex with the same escape rules as double-quoted ones and render
/// double-quoted.
#[test]
fn single_quoted_strings_render_double_quoted() {
    let ast = parse(
        "const greeting = 'say \"hi\"'\n\neventsSimple() {\n    logInfo({ message: 'it\\'s\\n' })\n}\n",
    )
    .expect("single-quoted strings must lex");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "const greeting = \"say \\\"hi\\\"\"\n\neventsSimple() {\n    logInfo({ message: \"it's\\n\" })\n}\n"
    );
    assert!(parse("eventsSimple() {\n    logInfo({ message: 'open })\n}\n").is_err());
}

/// Top-level declarations infer their type from a literal initializer; scalar declarations
/// render without the annotation while struct/array defaults canonicalize to the annotated
/// form. `null` carries no type and keeps requiring an annotation.
#[test]
fn top_level_declarations_infer_types_from_literals() {
    let ast = parse(
        "const s = \"a\"\nlet n = 5\nconst f = 1.5\nlet b = true\nconst o = {\"k\":1}\nconst xs = [1,2]\n",
    )
    .expect("inferred declarations must parse");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "const s = \"a\"\nlet n = 5\nconst f = 1.5\nlet b = true\nconst o: Struct = {\"k\":1}\nconst xs: any[] = [1,2]\n"
    );
    assert!(ast.variables[1].exposed, "`let` stays exposed");
    assert_eq!(
        render(
            &parse("const s: string = \"a\"\nconst n: float = 1\nconst t: string\n").unwrap(),
            &RenderOptions::default()
        ),
        "const s = \"a\"\nconst n: float = 1\nconst t: string\n",
        "a default that infers another type keeps its annotation"
    );

    let err = parse("const nothing = null\n").expect_err("null needs an annotation");
    assert!(
        err.message.contains("add a type annotation"),
        "{}",
        err.message
    );
    assert_eq!((err.line, err.col), (1, 17));
    assert!(
        parse("const call = foo()\n").is_err(),
        "only literals are inferable"
    );
}

/// `x += v` (and `-=`, `*=`, `/=`) desugar to `x = x + v`; the same works on a struct field
/// path. `+=` must not lex as `+` followed by `=`.
#[test]
fn compound_assignment_desugars_to_binary_assign() {
    let ast = parse(
        "eventsSimple() {\n    count += 1\n    total -= n * 2\n    scale *= 2.0\n    ratio /= 4\n    cfg.hits += 1\n}\n",
    )
    .expect("compound assignment must parse");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "eventsSimple() {\n    count = count + 1\n    total = total - (n * 2)\n    scale = scale * 2.0\n    ratio = ratio / 4\n    cfg.hits = cfg.hits + 1\n}\n"
    );
    let flow_like_ast::Stmt::Assign { target, value, .. } = &ast.events[0].body.stmts[0] else {
        panic!("expected an assignment");
    };
    assert_eq!(target, "count");
    assert!(matches!(
        value,
        flow_like_ast::Expr::Binary { op, lhs, .. }
            if op == "+" && matches!(lhs.as_ref(), flow_like_ast::Expr::Ref(name) if name == "count")
    ));
    assert!(matches!(
        &ast.events[0].body.stmts[4],
        flow_like_ast::Stmt::FieldAssign { base, path, .. } if base == "cfg" && path == "hits"
    ));
}

// ---- phase 2a: namespaces, method calls, positional args, `use`, destructuring -------------

fn first_stmt(text: &str) -> flow_like_ast::Stmt {
    let ast = parse(text).expect("parse should succeed");
    ast.events[0].body.stmts[0].clone()
}

fn first_call(text: &str) -> flow_like_ast::Call {
    match first_stmt(text) {
        flow_like_ast::Stmt::Call { call, .. } | flow_like_ast::Stmt::Let { call, .. } => call,
        other => panic!("expected a call statement, got {other:?}"),
    }
}

#[test]
fn roundtrip_namespace_path_calls() {
    assert_idempotent(
        "eventsSimple() {\n    const t = string::trim({ string: s })\n    ai::ml::model::read({ path: p })\n}\n",
        &RenderOptions::default(),
    );
    let call = first_call("eventsSimple() {\n    ai::ml::model::read({ path: p })\n}\n");
    assert_eq!(call.path, vec!["ai", "ml", "model"]);
    assert_eq!(call.display, "read");
    assert!(call.receiver.is_none());
    assert!(call.positional.is_empty());
    assert_eq!(call.args[0].name, "path");
}

#[test]
fn roundtrip_method_calls() {
    for text in [
        "eventsSimple() {\n    const t = s.trim()\n}\n",
        "eventsSimple() {\n    const t = s.contains(\"?\")\n}\n",
        "eventsSimple() {\n    const t = s.contains(\"?\", { ignoreCase: true })\n}\n",
        "eventsSimple() {\n    const t = x.a.b().c[0].d()\n}\n",
        "eventsSimple() {\n    const t = (a ? b : c).trim()\n}\n",
        "eventsSimple() {\n    const t = (a + b).toString()\n}\n",
        "eventsSimple() {\n    const t = (5).abs()\n}\n",
        "eventsSimple() {\n    const t = \"lit\".trim()\n}\n",
        "eventsSimple() {\n    let t = f().g().h\n}\n",
    ] {
        assert_idempotent(text, &RenderOptions::default());
    }

    let call =
        first_call("eventsSimple() {\n    const t = s.contains(\"?\", { ignoreCase: true })\n}\n");
    assert_eq!(call.display, "contains");
    assert!(call.path.is_empty());
    assert!(matches!(
        call.receiver.as_deref(),
        Some(flow_like_ast::Expr::Ref(name)) if name == "s"
    ));
    assert_eq!(call.positional.len(), 1);
    assert!(matches!(
        &call.positional[0],
        flow_like_ast::Expr::Literal(flow_like_ast::Literal::String(s)) if s == "?"
    ));
    assert_eq!(call.args.len(), 1);
    assert_eq!(call.args[0].name, "ignoreCase");

    // `x.a.b().c[0].d()`: the outer call's receiver is the index expression, whose base chains
    // back through a field on an inner method call.
    let call = first_call("eventsSimple() {\n    const t = x.a.b().c[0].d()\n}\n");
    assert_eq!(call.display, "d");
    let flow_like_ast::Expr::Index { base, .. } = call.receiver.as_deref().expect("receiver")
    else {
        panic!("expected an index receiver");
    };
    let flow_like_ast::Expr::Field { base, pin } = base.as_ref() else {
        panic!("expected `.c` field");
    };
    assert_eq!(pin, "c");
    let flow_like_ast::Expr::Call(inner) = base.as_ref() else {
        panic!("expected inner method call `b()`");
    };
    assert_eq!(inner.display, "b");
    assert!(matches!(
        inner.receiver.as_deref(),
        Some(flow_like_ast::Expr::Field { pin, .. }) if pin == "a"
    ));

    // Numeric literal receivers are canonicalised into parentheses.
    let ast = parse("eventsSimple() {\n    const t = 5.abs()\n}\n").expect("parses");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "eventsSimple() {\n    const t = (5).abs()\n}\n"
    );
}

#[test]
fn bang_on_method_call_desugars_to_bool_not() {
    let ast = parse("eventsSimple() {\n    const t = !s.isEmpty()\n}\n").expect("parses");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "eventsSimple() {\n    const t = boolNot({ boolean: s.isEmpty() })\n}\n"
    );
}

#[test]
fn positional_arguments_precede_the_trailing_named_object() {
    assert_idempotent(
        "eventsSimple() {\n    f({ a: 1 })\n    f({ a: 1 }, { b: 2 })\n    f(x, y)\n    f(1, \"a\", { b: 2 })\n    f()\n}\n",
        &RenderOptions::default(),
    );
    // A sole `{}` is the empty named object and keeps canonicalising to `f()`.
    let ast = parse("eventsSimple() {\n    f({})\n}\n").expect("parses");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "eventsSimple() {\n    f()\n}\n"
    );

    let call = first_call("eventsSimple() {\n    f({ a: 1 })\n}\n");
    assert!(call.positional.is_empty());
    assert_eq!(call.args.len(), 1);

    let call = first_call("eventsSimple() {\n    f({ a: 1 }, { b: 2 })\n}\n");
    assert_eq!(call.positional.len(), 1);
    assert!(
        matches!(&call.positional[0], flow_like_ast::Expr::Object(fields) if fields[0].key == "a")
    );
    assert_eq!(call.args.len(), 1);
    assert_eq!(call.args[0].name, "b");

    let call = first_call("eventsSimple() {\n    f({})\n}\n");
    assert!(call.positional.is_empty());
    assert!(call.args.is_empty());

    // A trailing comma after the named object is tolerated and never rendered.
    let ast = parse("eventsSimple() {\n    f(x, { b: 2 },)\n}\n").expect("parses");
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "eventsSimple() {\n    f(x, { b: 2 })\n}\n"
    );
}

#[test]
fn method_calls_nest_inside_object_values_and_named_arguments() {
    assert_idempotent(
        "eventsSimple() {\n    f({ a: s.trim(), b: { c: x.y() } })\n    let o = { k: a.b(), n: ns::g(1) }\n}\n",
        &RenderOptions::default(),
    );
    let call = first_call("eventsSimple() {\n    f({ a: s.trim() })\n}\n");
    assert!(matches!(
        &call.args[0].value,
        flow_like_ast::Expr::Call(inner) if inner.display == "trim" && inner.receiver.is_some()
    ));
}

#[test]
fn roundtrip_use_declarations() {
    let text = "use ai::ml\nuse ai::ml::*\nuse a::b as x\nuse ui::{ setElementText, navigateTo }\n\nconst s = \"\"\n\neventsSimple() {\n    x::run()\n}\n";
    assert_idempotent(text, &RenderOptions::default());
    let ast = parse(text).expect("parses");
    use flow_like_ast::{UseDecl, UseKind};
    assert_eq!(
        ast.uses,
        vec![
            UseDecl {
                path: vec!["ai".into(), "ml".into()],
                kind: UseKind::Namespace,
            },
            UseDecl {
                path: vec!["ai".into(), "ml".into()],
                kind: UseKind::Glob,
            },
            UseDecl {
                path: vec!["a".into(), "b".into()],
                kind: UseKind::Alias("x".into()),
            },
            UseDecl {
                path: vec!["ui".into()],
                kind: UseKind::Members(vec!["setElementText".into(), "navigateTo".into()]),
            },
        ]
    );

    // A comma-separated list is one declaration per tree and renders one per line.
    let ast = parse("use string::*, array::*;\n\neventsSimple() {\n}\n").expect("parses");
    assert_eq!(ast.uses.len(), 2);
    assert_eq!(
        render(&ast, &RenderOptions::default()),
        "use string::*\nuse array::*\n\neventsSimple() {\n}\n"
    );

    // `use` lines precede interfaces, separated by a blank line.
    assert_idempotent(
        "use ai::ml\n\ninterface Row {\n    id: string;\n}\n\nconst row: Row = {}\n",
        &RenderOptions::default(),
    );
}

#[test]
fn roundtrip_object_destructuring() {
    let text = "eventsSimple() {\n    const { text, usage: u } = ai::invoke({ model: m })\n    const { hash } = content.md5()\n}\n";
    assert_idempotent(text, &RenderOptions::default());
    let flow_like_ast::Stmt::Destructure {
        fields,
        call,
        anchor,
    } = first_stmt(text)
    else {
        panic!("expected a destructuring statement");
    };
    assert_eq!(
        fields,
        vec![
            flow_like_ast::DestructureField {
                pin: "text".into(),
                name: "text".into(),
            },
            flow_like_ast::DestructureField {
                pin: "usage".into(),
                name: "u".into(),
            },
        ]
    );
    assert_eq!(call.path, vec!["ai"]);
    assert_eq!(call.display, "invoke");
    assert!(anchor.is_none());

    // `let { … }` is accepted and canonicalises to `const`; anchors stay trailing.
    let ast = parse("eventsSimple() {\n    let { a } = f()   //@n:node1\n}\n").expect("parses");
    assert_eq!(
        render(&ast, &anchored_opts()),
        "eventsSimple() {\n    const { a } = f()   //@n:node1\n}\n"
    );
    assert!(matches!(
        &ast.events[0].body.stmts[0],
        flow_like_ast::Stmt::Destructure { anchor: Some(anchor), .. } if anchor == "node1"
    ));
}

#[test]
fn method_and_path_calls_in_every_statement_position() {
    assert_idempotent(
        "function f(s: string): (out: string) {\n    return s.trim()\n}\n\neventsSimple() {\n    s.trim()\n    log::info({ message: s })\n    if (s.isEmpty()) {\n        log::warn({ message: \"empty\" })\n    }\n    for (const x of items.chunk({ size: 2 })) {\n        log::info({ message: x.value })\n    }\n    while (it.hasNext()) {\n        it.next()\n    }\n    s.trim()   //@n:a1\n}\n",
        &anchored_opts(),
    );
    let flow_like_ast::Stmt::Loop { call, .. } =
        first_stmt("eventsSimple() {\n    for (const x of items.chunk({ size: 2 })) {\n    }\n}\n")
    else {
        panic!("expected a loop");
    };
    assert_eq!(call.display, "chunk");
    assert!(call.receiver.is_some());

    let text = "eventsSimple() {\n    for (const x of control::forEach({ array: items })) {\n    }\n    while (control::whileLoop({ condition: c })) {\n    }\n}\n";
    assert_idempotent(text, &RenderOptions::default());
    let flow_like_ast::Stmt::Loop { call, .. } = first_stmt(text) else {
        panic!("expected a loop");
    };
    assert_eq!(call.path, vec!["control"]);
    assert_eq!(call.display, "forEach");
}

#[test]
fn bare_namespace_path_is_rejected() {
    let err = parse("eventsSimple() {\n    let x = a::b\n}\n").expect_err("a path is not a value");
    assert!(
        err.message.contains("namespace paths can only be called"),
        "{}",
        err.message
    );
    assert_eq!((err.line, err.col), (2, 13));
}

#[test]
fn array_destructuring_is_rejected() {
    let err = parse("eventsSimple() {\n    const [a, b] = f()\n}\n").expect_err("array patterns");
    assert!(
        err.message
            .contains("use object destructuring by output name"),
        "{}",
        err.message
    );
    let err = parse("eventsSimple() {\n    const { a } = x.y\n}\n").expect_err("non-call rhs");
    assert!(err.message.contains("requires a call"), "{}", err.message);
}

#[test]
fn use_inside_a_block_is_rejected() {
    let err = parse("eventsSimple() {\n    use string::*\n}\n").expect_err("block-level use");
    assert!(err.message.contains("top level"), "{}", err.message);
}

#[test]
fn call_serde_omits_empty_phase2_fields_and_accepts_their_absence() {
    let call = first_call("eventsSimple() {\n    f({ a: 1 })\n}\n");
    let json = serde_json::to_value(&call).expect("serializes");
    assert!(json.get("path").is_none());
    assert!(json.get("receiver").is_none());
    assert!(json.get("positional").is_none());
    let legacy = serde_json::json!({
        "node_type": "log_info",
        "display": "logInfo",
        "args": [],
        "anchor": null
    });
    let call: flow_like_ast::Call = serde_json::from_value(legacy).expect("legacy shape loads");
    assert!(call.path.is_empty() && call.receiver.is_none() && call.positional.is_empty());
}

// ---- module blocks (phase 3a) --------------------------------------------------------------

#[test]
fn roundtrip_module_blocks() {
    let text = concat!(
        "module checkout {\n",
        "    function helper(x: string): (out: string) {\n",
        "        return x\n",
        "    }\n",
        "\n",
        "    eventsSimple onLoad() {\n",
        "        logInfo({ message: \"hi\" })\n",
        "    }\n",
        "\n",
        "    module payments {\n",
        "        function charge(amount: float): (ok: bool) {\n",
        "            return true\n",
        "        }\n",
        "    }\n",
        "}\n",
        "\n",
        "module shipping {\n",
        "    eventsSimple shipped() {\n",
        "        logInfo({ message: \"shipped\" })\n",
        "    }\n",
        "}\n",
    );
    assert_idempotent(text, &RenderOptions::default());

    let ast = parse(text).expect("module blocks parse");
    assert_eq!(ast.modules.len(), 2);
    assert_eq!(ast.modules[0].name, "checkout");
    assert_eq!(ast.modules[0].functions.len(), 1);
    assert_eq!(ast.modules[0].events.len(), 1);
    assert_eq!(ast.modules[0].modules[0].name, "payments");
    assert_eq!(ast.modules[0].modules[0].functions[0].name, "charge");
    assert_eq!(ast.modules[1].name, "shipping");
    assert!(ast.functions.is_empty() && ast.events.is_empty());

    // Rendering is stable across passes, not just equal to the source once.
    let once = render(&ast, &RenderOptions::default());
    let twice = render(&parse(&once).expect("re-parses"), &RenderOptions::default());
    assert_eq!(once, twice);
}

#[test]
fn modules_render_after_the_other_sections() {
    let text = concat!(
        "use string::*\n",
        "\n",
        "const goal = \"ship\"\n",
        "\n",
        "function root(): (out: string) {\n",
        "    return goal\n",
        "}\n",
        "\n",
        "eventsSimple() {\n",
        "    logInfo({ message: goal })\n",
        "}\n",
        "\n",
        "module checkout {\n",
        "    function helper(): (out: string) {\n",
        "        return \"a\"\n",
        "    }\n",
        "}\n",
    );
    assert_idempotent(text, &RenderOptions::default());
}

#[test]
fn empty_module_block_round_trips() {
    let text = "module checkout {\n}\n";
    assert_idempotent(text, &RenderOptions::default());
    let ast = parse(text).expect("empty module parses");
    assert_eq!(ast.modules.len(), 1);
    assert!(ast.modules[0].functions.is_empty());
    assert!(ast.modules[0].events.is_empty());
    assert!(ast.modules[0].modules.is_empty());
}

#[test]
fn module_anchor_survives_a_round_trip() {
    let text = concat!(
        "module checkout {   //@l:mod1\n",
        "    function helper(): (out: string) {   //@l:fn1\n",
        "        return \"a\"\n",
        "    }\n",
        "\n",
        "    module payments {   //@l:mod2\n",
        "    }\n",
        "}\n",
    );
    assert_idempotent(text, &anchored_opts());

    let ast = parse(text).expect("anchored module parses");
    assert_eq!(ast.modules[0].anchor.as_deref(), Some("mod1"));
    assert_eq!(ast.modules[0].functions[0].anchor.as_deref(), Some("fn1"));
    assert_eq!(ast.modules[0].modules[0].anchor.as_deref(), Some("mod2"));
}

#[test]
fn module_is_a_contextual_keyword() {
    // `module` only opens a block in the exact `module <ident> {` shape; everywhere else it is an
    // ordinary identifier and must keep parsing as one.
    let text = concat!(
        "eventsSimple() {\n",
        "    const module = loadModule({ name: \"a\" })\n",
        "    logInfo({ message: module, module: module.id })\n",
        "}\n",
    );
    assert_idempotent(text, &RenderOptions::default());
    let ast = parse(text).expect("`module` as a binding parses");
    assert!(ast.modules.is_empty());

    // An event *type* or event *name* spelled `module` is still an event, not a module block.
    let ast = parse("module() {\n    logInfo({ message: \"x\" })\n}\n").expect("event `module`");
    assert!(ast.modules.is_empty());
    assert_eq!(ast.events[0].name, "module");

    let ast = parse("eventsSimple module() {\n    logInfo({ message: \"x\" })\n}\n")
        .expect("event named `module`");
    assert!(ast.modules.is_empty());
    assert_eq!(ast.events[0].event_name.as_deref(), Some("module"));
}

#[test]
fn variable_declarations_inside_a_module_are_rejected() {
    for source in [
        "module checkout {\n    const x: string = \"a\"\n}\n",
        "module checkout {\n    let x: string = \"a\"\n}\n",
    ] {
        let err = parse(source).expect_err("module bodies hold no variables");
        assert!(
            err.message.contains("variables are declared in main.flow"),
            "{}",
            err.message
        );
    }
}

#[test]
fn use_and_interface_declarations_inside_a_module_are_rejected() {
    for source in [
        "module checkout {\n    use foo::*\n}\n",
        "module checkout {\n    interface X {}\n}\n",
    ] {
        let err = parse(source).expect_err("module bodies hold no use/interface declarations");
        assert!(
            err.message
                .contains("declarations belong at the top of the file"),
            "{}",
            err.message
        );
    }
}

#[test]
fn unterminated_module_block_names_the_module() {
    let err = parse("module checkout {\n    function helper() {\n    }\n").expect_err("unclosed");
    assert!(
        err.message.contains("inside module `checkout`"),
        "{}",
        err.message
    );
}

#[test]
fn roundtrip_detached_blocks() {
    let text = concat!(
        "eventsSimple() {\n",
        "    logInfo({ message: \"reachable\" })\n",
        "}\n",
        "\n",
        "detached {\n",
        "    logInfo({ message: \"first\" })\n",
        "}\n",
        "\n",
        "detached {\n",
        "    logInfo({ message: \"second\" })\n",
        "}\n",
    );
    assert_idempotent(text, &RenderOptions::default());

    let ast = parse(text).expect("parse should succeed");
    assert_eq!(
        ast.detached.len(),
        2,
        "each chain keeps its own block rather than merging into one"
    );
    assert_eq!(ast.detached[0].stmts.len(), 1);
}

#[test]
fn detached_renders_after_events_and_before_modules() {
    let text = concat!(
        "eventsSimple() {\n",
        "    logInfo({ message: \"e\" })\n",
        "}\n",
        "\n",
        "detached {\n",
        "    logInfo({ message: \"d\" })\n",
        "}\n",
        "\n",
        "module checkout {\n",
        "    detached {\n",
        "        logInfo({ message: \"m\" })\n",
        "    }\n",
        "}\n",
    );
    assert_idempotent(text, &RenderOptions::default());

    let ast = parse(text).expect("parse should succeed");
    assert_eq!(ast.detached.len(), 1);
    assert_eq!(ast.modules[0].detached.len(), 1);
}

#[test]
fn detached_statement_anchors_survive_a_round_trip() {
    let text = "detached {\n    logInfo({ message: \"x\" })   //@n:orphan\n}\n";
    assert_idempotent(text, &anchored_opts());

    let ast = parse(text).expect("parse should succeed");
    assert_eq!(ast.detached[0].root_anchor(), Some("orphan"));
}

#[test]
fn detached_is_a_contextual_keyword() {
    // `detached` opens a block only in the exact `detached {` shape; an event block always has a
    // parameter list, so both spellings stay reachable.
    let ast =
        parse("detached() {\n    logInfo({ message: \"x\" })\n}\n").expect("event `detached`");
    assert!(ast.detached.is_empty());
    assert_eq!(ast.events[0].name, "detached");

    let ast = parse("eventsSimple detached() {\n    logInfo({ message: \"x\" })\n}\n")
        .expect("event named `detached`");
    assert!(ast.detached.is_empty());
    assert_eq!(ast.events[0].event_name.as_deref(), Some("detached"));

    let text = concat!(
        "eventsSimple() {\n",
        "    const detached = loadThing({ name: \"a\" })\n",
        "    logInfo({ message: detached, detached: detached.id })\n",
        "}\n",
    );
    assert_idempotent(text, &RenderOptions::default());
    assert!(
        parse(text)
            .expect("`detached` as a binding parses")
            .detached
            .is_empty()
    );
}
