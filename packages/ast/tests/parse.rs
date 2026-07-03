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
fn roundtrip_member_vs_field() {
    // `.rows` (camel, pin-like) and `.report_id` (snake, data field) must both survive verbatim.
    let text = "onStart() {\n    const row = read({ id: query.rows[0].report_id })\n}\n";
    assert_idempotent(text, &RenderOptions::default());
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
fn rejects_unknown_decorator() {
    let err: ParseError = parse("@bogus\nconst x: string = \"\"\n").unwrap_err();
    assert!(err.message.contains("bogus"));
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
