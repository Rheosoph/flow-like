use serde::{Deserialize, Serialize};

use super::examples::{FLOWSCRIPT_DOMAIN_EXAMPLES, FLOWSCRIPT_FEW_SHOT_EXAMPLES};

/// Prompt-only experiment. Tool schemas, draft validation and execution limits are unchanged.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardPromptProfile {
    #[default]
    Legacy,
    Focused,
}

/// The host selects authoring only after it has restricted the model-facing tool surface.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BoardPromptMode {
    #[default]
    General,
    Authoring,
}

/// Host evidence for the narrow ordinary-session rollout. This does not authorize execution.
pub struct BoardPromptEligibility<'a> {
    pub request: &'a str,
    pub current_source: &'a str,
    /// Must come from the isolated runner's static validation of the actual detached board.
    pub current_board_is_isolated_json: bool,
    /// Includes additional host guidance, UI context and partially parsed context envelopes.
    pub has_host_augmentations: bool,
}

/// Selects Focused only for explicit Generic Event transformations with no domain integration.
/// Hosts must separately require a board-only authoring session. Ambiguity keeps Legacy.
pub fn select_ordinary_board_prompt_profile(
    eligibility: &BoardPromptEligibility<'_>,
) -> BoardPromptProfile {
    if !eligibility.current_board_is_isolated_json
        || eligibility.has_host_augmentations
        || !eligibility.request.is_ascii()
    {
        return BoardPromptProfile::Legacy;
    }
    let request = expanded_words(eligibility.request);
    let request_words: Vec<_> = request
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let explicit_generic = request_words
        .windows(2)
        .any(|pair| pair == ["generic", "event"] || pair == ["events", "generic"]);
    let explicit_values = request_words.iter().any(|word| {
        matches!(
            *word,
            "json"
                | "payload"
                | "string"
                | "strings"
                | "bool"
                | "boolean"
                | "int"
                | "integer"
                | "float"
                | "number"
                | "numbers"
        )
    });
    let explicit_behavior = request_words.iter().any(|word| {
        matches!(
            *word,
            "trim"
                | "trims"
                | "trimmed"
                | "lowercase"
                | "uppercase"
                | "normalize"
                | "normalized"
                | "normalise"
                | "normalised"
                | "compare"
                | "compute"
                | "subtract"
                | "add"
                | "classify"
                | "transform"
                | "transformation"
                | "calculate"
                | "sum"
                | "multiply"
                | "divide"
        )
    });
    let explicit_input = request_words.iter().any(|word| {
        matches!(
            *word,
            "payload"
                | "input"
                | "inputs"
                | "parameter"
                | "parameters"
                | "argument"
                | "arguments"
                | "field"
                | "fields"
        )
    });
    let domains = PromptDomains::infer(eligibility.request, eligibility.current_source);
    if !explicit_generic
        || !explicit_values
        || !explicit_behavior
        || !explicit_input
        || domains.any()
    {
        return BoardPromptProfile::Legacy;
    }
    // Named input/output services are open-ended. Keep unfamiliar "from/to" targets on
    // Legacy rather than relying on an ever-growing list of service brands.
    let unfamiliar_endpoint = request_words.iter().enumerate().any(|(index, word)| {
        if !matches!(*word, "from" | "to") {
            return false;
        }
        let target = request_words[index + 1..].iter().copied().find(|word| {
            !matches!(
                *word,
                "the" | "a" | "an" | "this" | "its" | "provided" | "typed"
            )
        });
        !matches!(
            target,
            Some(
                "payload"
                    | "input"
                    | "inputs"
                    | "parameter"
                    | "parameters"
                    | "argument"
                    | "arguments"
                    | "field"
                    | "fields"
                    | "string"
                    | "strings"
                    | "json"
                    | "integer"
                    | "int"
                    | "number"
                    | "numbers"
                    | "float"
                    | "boolean"
                    | "bool"
                    | "uppercase"
                    | "lowercase"
                    | "upper"
                    | "lower"
                    | "return"
                    | "trim"
                    | "normalize"
                    | "normalise"
                    | "compare"
                    | "compute"
                    | "subtract"
                    | "add"
                    | "classify"
                    | "transform"
                    | "calculate"
                    | "sum"
                    | "multiply"
                    | "divide"
            )
        )
    });
    if unfamiliar_endpoint {
        return BoardPromptProfile::Legacy;
    }
    let context = expanded_words(&format!(
        "{}\n{}",
        eligibility.request, eligibility.current_source
    ));
    let has_integration_or_state = context
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| {
            matches!(
                word,
                "app"
                    | "apps"
                    | "application"
                    | "applications"
                    | "web"
                    | "website"
                    | "deploy"
                    | "publish"
                    | "http"
                    | "https"
                    | "url"
                    | "urls"
                    | "api"
                    | "rest"
                    | "webhook"
                    | "network"
                    | "integrate"
                    | "integration"
                    | "integrations"
                    | "sync"
                    | "synchronize"
                    | "synchronise"
                    | "import"
                    | "export"
                    | "external"
                    | "remote"
                    | "socket"
                    | "connect"
                    | "fetch"
                    | "send"
                    | "post"
                    | "schedule"
                    | "scheduled"
                    | "cron"
                    | "timer"
                    | "chat"
                    | "wasm"
                    | "cache"
                    | "cached"
                    | "caching"
                    | "variable"
                    | "variables"
                    | "state"
                    | "stateful"
                    | "loop"
                    | "loops"
                    | "iterate"
                    | "iteration"
                    | "while"
                    | "foreach"
                    | "macro"
                    | "macros"
                    | "credential"
                    | "credentials"
                    | "secret"
                    | "secrets"
                    | "oauth"
                    | "token"
                    | "tokens"
            )
        });
    if has_integration_or_state {
        BoardPromptProfile::Legacy
    } else {
        BoardPromptProfile::Focused
    }
}

fn expanded_words(text: &str) -> String {
    let mut expanded = String::new();
    let mut previous_lowercase = false;
    for character in text.chars() {
        if character.is_uppercase() && previous_lowercase {
            expanded.push(' ');
        }
        expanded.extend(character.to_lowercase());
        previous_lowercase = character.is_lowercase();
    }
    expanded
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PromptDomains {
    pub database: bool,
    pub ui: bool,
    pub files: bool,
    pub mail: bool,
    pub ai: bool,
    pub retrieval: bool,
}

impl PromptDomains {
    pub(super) fn infer(request: &str, source: &str) -> Self {
        // Include both prose and existing calls: a short edit request must not erase the
        // guidance for a domain already present on the board. Camel-case aliases count too.
        let expanded = expanded_words(&format!("{request}\n{source}"));
        let words: std::collections::HashSet<_> = expanded
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        let any = |terms: &[&str]| terms.iter().any(|term| words.contains(term));
        let mail = any(&["email", "mail", "inbox", "smtp", "imap", "outlook", "gmail"]);
        let retrieval = any(&[
            "knowledge",
            "retrieval",
            "rag",
            "embedding",
            "embeddings",
            "vector",
            "semantic",
            "ingest",
            "ingestion",
            "chunk",
            "chunks",
        ]);
        let ai = retrieval
            || any(&[
                "ai",
                "llm",
                "agent",
                "model",
                "prompt",
                "summarize",
                "summarise",
            ]);
        let ui = any(&[
            "ui",
            "a2ui",
            "widget",
            "widgets",
            "page",
            "pages",
            "dashboard",
            "chart",
            "charts",
            "button",
            "buttons",
            "form",
            "forms",
            "frontend",
            "screen",
            "screens",
            "component",
            "components",
            "progress",
            "indicator",
            "indicators",
            "interface",
            "interfaces",
            "view",
            "views",
            "element",
            "elements",
            "onload",
            "onclick",
            "render",
            "display",
        ]);
        let database = mail
            || retrieval
            || any(&[
                "db",
                "database",
                "databases",
                "table",
                "tables",
                "sql",
                "datafusion",
                "dataset",
                "datasets",
                "query",
                "queries",
                "analytics",
                "dashboard",
                "persist",
                "persistent",
                "save",
                "saved",
                "store",
                "stored",
                "storage",
                "record",
                "records",
                "crm",
                "inventory",
                "insert",
                "upsert",
                "lance",
                "lancedb",
                "delta",
            ]);
        let files = mail
            || retrieval
            || any(&[
                "file",
                "files",
                "folder",
                "folders",
                "directory",
                "directories",
                "path",
                "paths",
                "flowpath",
                "storage",
                "upload",
                "download",
                "attachment",
                "attachments",
                "csv",
                "pdf",
                "document",
                "documents",
            ]);
        let explicit_transform = any(&[
            "json", "payload", "generic", "string", "strings", "boolean", "bool", "integer", "int",
            "float", "number", "numbers",
        ]) && any(&[
            "return",
            "returns",
            "trim",
            "lowercase",
            "uppercase",
            "lower",
            "upper",
            "normalize",
            "normalise",
            "compare",
            "compute",
            "computed",
            "subtract",
            "add",
            "classify",
            "transform",
            "transformation",
            "calculate",
        ]);
        // Unclear and non-English requests retain every domain. This experiment only prunes
        // when public request/source evidence identifies a domain or a data transformation.
        if request.trim().is_empty()
            || !request.is_ascii()
            || !(database || ui || files || mail || ai || explicit_transform)
        {
            return Self::all();
        }
        Self {
            database,
            ui,
            files,
            mail,
            ai,
            retrieval,
        }
    }

    fn all() -> Self {
        Self {
            database: true,
            ui: true,
            files: true,
            mail: true,
            ai: true,
            retrieval: true,
        }
    }

    fn any(self) -> bool {
        self.database || self.ui || self.files || self.mail || self.ai || self.retrieval
    }
}

pub(super) fn focused_examples(domains: PromptDomains) -> String {
    let mut result = String::from(FOCUSED_CORE_EXAMPLES);
    let mut append = |document: &str, heading: &str| {
        if let Some((_, section)) = document.split_once(heading) {
            let body = section
                .split_once("\n### ")
                .map_or(section, |(body, _)| body);
            result.push_str("\n\n");
            result.push_str(heading);
            result.push_str(body);
        }
    };
    if domains.database {
        append(
            FLOWSCRIPT_FEW_SHOT_EXAMPLES,
            "### 5. DataFusion over Open Database follows open -> session -> register -> SQL",
        );
    }
    if domains.ui && domains.database {
        append(
            FLOWSCRIPT_FEW_SHOT_EXAMPLES,
            "### 7. Dashboard onLoad: query data, then populate page elements and widgets",
        );
        append(
            FLOWSCRIPT_FEW_SHOT_EXAMPLES,
            "### 8. Drive a dashboard chart/table directly from a DataFusion query",
        );
    }
    if domains.mail {
        append(
            FLOWSCRIPT_DOMAIN_EXAMPLES,
            "### Email round-trip: fetch unseen mail, send a tagged draft for approval, persist, mark seen",
        );
    }
    if domains.ai {
        append(
            FLOWSCRIPT_DOMAIN_EXAMPLES,
            "### LLM invoke plus struct-field arithmetic (read the field, coerce, then write it back)",
        );
    }
    if domains.retrieval {
        append(
            FLOWSCRIPT_DOMAIN_EXAMPLES,
            "### Knowledge ingest: extract, chunk, embed, persist searchable rows",
        );
        append(
            FLOWSCRIPT_DOMAIN_EXAMPLES,
            "### Semantic search with an explicit empty-result path",
        );
    }
    result
}

pub(crate) const FOCUSED_CORE_EXAMPLES: &str = r##"
## FOCUSED FLOWSCRIPT REFERENCE
Examples follow the request and source; other capabilities remain available. Read exact catalog
declarations and usage notes. Ground database/storage/UI IDs with read-only tools unless supplied
by the manifest.

- Catalog calls use `namespace::alias({ exactPin: value })`. Rendered calls pass their first one or
  two inputs positionally in declaration order (`log::info(msg)`, `db::open(name, { userScoped:
  true })`); keep those and name every argument you add. `::` selects a namespace; `.` reads a
  value field or invokes a declared method. A declaration with `this:` supports a receiver method;
  do not supply the receiver a second time.
- `use ns::*` belongs at the top of the document. Qualified calls need no import. Legacy camel-case
  aliases still resolve; prefer the spelling in the returned declaration. Repeat identically named
  input keys in declaration order when a declaration repeats them. Never invent pin selectors.
- A single-output call is its value: `const label = text.trim()`. Destructure multi-output calls by
  exact pin name. A struct field is Generic unless its live schema says otherwise; use declared
  conversion/accessor nodes before typed arithmetic or comparison. Never guess business fields.
- Author calls inside Event or `function` bodies. Top-level variables have literal defaults and
  cannot invoke nodes. Local `const`/`let` aliases accept expressions, calls, objects and arrays.
  Use `{ field: value }`, never `{ field = value }`. Preserve existing anchors and module wrappers.
- Helpers start with `function` and declare named output pins. Keep declarations and calls in one
  document; bind exact parameter names. One output is the value itself. A helper's only `return`
  must be its trailing top-level statement. Assign a literal-initialized `let` in branches or loops,
  then return it after the control block. Nested, early and multiple function returns are rejected.
  This restriction does not change Event-handler returns.
- Arithmetic, comparisons, boolean operators and template literals are compiler-supported
  expressions. Computed objects and arrays lower to native struct/array calls. Never invoke an LLM
  for arithmetic. Literal aliases do not run nodes.
- Plain `if (condition) { ... } else { ... }` and `for (const item of items) { ... }` are supported.
  Use `let` for reassignment across branches or loops; a `const` cannot be reassigned. Preserve both
  decision outcomes and explicit success/error ordering. Multi-output execution arms must use the
  exact labels in the declaration, as described in EXECUTION FLOW AND MULTI-OUTPUT NODES.
- `return` in an Event emits exactly one value. Generic Event parameters define payload fields.
  Use `eventsSimple` for schedule-compatible entry nodes; app Event records/sinks are configured
  separately. Helpers are reusable logic called from Events, not independently registered Events.
- Impure helpers must close control arms and end on a single-continuation statement before
  returning, so the outgoing execution pin is connected. Helpers without outputs need no return.

### Repeated helper calls keep their own arguments
Retrieve integer multiplication declarations before adapting this shape.
```flowscript-verified
function square(value: int): (area: int) {
    const area = value * value
    return area
}

eventsGeneric tileAreas(sideA: int, sideB: int) {
    const squareA = square({ value: sideA })
    const squareB = square({ value: sideB })
    return [squareA, squareB]
}
```

### Computed fields and arrays are ordinary expressions
```flowscript-verified
function makePayload(title: string, revision: int): (payload: Struct) {
    const nextRevision = revision + 1
    const row = { title: title, revision: nextRevision }
    const rows = [row]
    return { rows: rows }
}

eventsGeneric buildPayload(title: string, revision: int) {
    const payload = makePayload({ title: title, revision: revision })
    return payload
}
```

### A boolean branch preserves both outputs
```flowscript-verified
eventsGeneric chooseLabel(enabled: bool, left: string, right: string) {
    if (enabled) {
        return { label: left.trim() }
    } else {
        return { label: right.trim() }
    }
}
```
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copilot::prompts::board::{
        board_sdk_flowscript_system_prompt_with_options,
        board_sdk_flowscript_system_prompt_with_profile,
    };

    #[test]
    fn ordinary_rollout_requires_explicit_transform_and_trusted_host_evidence() {
        let request = "Create a Generic Event that returns normalized string payload fields.";
        let source = "eventsGeneric clean(value: string) { return value.trim() }";
        let eligible = |request, current_source, board_ok, augmentations| {
            select_ordinary_board_prompt_profile(&BoardPromptEligibility {
                request,
                current_source,
                current_board_is_isolated_json: board_ok,
                has_host_augmentations: augmentations,
            })
        };
        for source in ["", source] {
            assert_eq!(
                eligible(request, source, true, false),
                BoardPromptProfile::Focused
            );
        }
        assert_eq!(
            eligible(
                "Use eventsGeneric to calculate an integer from the payload",
                "",
                true,
                false
            ),
            BoardPromptProfile::Focused,
        );
        for (request, source, board_ok, augmentations) in [
            (request, source, false, false),
            (request, source, true, true),
            ("", source, true, false),
            ("Make it work", source, true, false),
            ("Normalize a JSON payload", source, true, false),
            ("Explain this Generic Event JSON", source, true, false),
            ("Generic Event JSON", source, true, false),
            ("Generic Event: ändere den String", source, true, false),
            (
                "Create a Generic Event to return JSON from an API",
                "",
                true,
                false,
            ),
            (
                "Create a Generic Event to return JSON from Salesforce",
                "",
                true,
                false,
            ),
            (
                "Generic Event: normalize string payload fields and integrate Salesforce",
                "",
                true,
                false,
            ),
            (
                "Generic Event: normalize string payload fields from Salesforce",
                "",
                true,
                false,
            ),
            (
                "Generic Event: normalize string payload fields and return JSON to Salesforce",
                "",
                true,
                false,
            ),
            (
                "Create an app with a Generic Event returning a string",
                "",
                true,
                false,
            ),
            (
                "Generic Event: return the JSON payload after sending email",
                "",
                true,
                false,
            ),
            (
                "Generic Event: compute a float for a component progress indicator",
                "",
                true,
                false,
            ),
            (
                "Generic Event: return a string for an interface view",
                "",
                true,
                false,
            ),
            (
                "Generic Event: return JSON and save a database row",
                "",
                true,
                false,
            ),
            (
                "Generic Event: return a JSON result using a cached helper",
                "",
                true,
                false,
            ),
            (
                "Generic Event: return a string from a file",
                "",
                true,
                false,
            ),
            (
                "Generic Event: return a string using an LLM",
                "",
                true,
                false,
            ),
            (
                request,
                "eventsGeneric f() { const result = http::fetch({}) }",
                true,
                false,
            ),
            (
                request,
                "eventsGeneric f() { const result = a2uiSetText({}) }",
                true,
                false,
            ),
        ] {
            assert_eq!(
                eligible(request, source, board_ok, augmentations),
                BoardPromptProfile::Legacy,
                "request={request:?} source={source:?}",
            );
        }
    }

    #[test]
    fn authoring_guidance_tracks_hidden_tools_without_changing_source() {
        let source = "eventsGeneric literal() {\n    return { text: \"catalog_search emit_commands get_node_details\" } //@n:keep\n}";
        for profile in [BoardPromptProfile::Legacy, BoardPromptProfile::Focused] {
            let static_prompt = board_sdk_flowscript_system_prompt_with_options(
                "",
                30,
                "",
                profile,
                BoardPromptMode::Authoring,
            );
            for hidden in crate::flow::copilot::WORKFLOW_AUTHORING_HIDDEN_TOOLS {
                assert!(
                    !static_prompt.contains(hidden),
                    "{profile:?} advertises {hidden}"
                );
            }
            for contract in [
                "plan_board_scope",
                "get_declarations",
                "write_flowscript",
                "patch_flowscript",
                "test_flowscript",
                "commit_flowscript",
                "expected_revision",
                "## DATA AND DATABASE WORKFLOWS",
                "## DASHBOARDS, PAGES, AND WIDGETS",
                "function name(params)",
                "already-persisted board",
            ] {
                assert!(
                    static_prompt.contains(contract),
                    "missing authoring contract {contract}"
                );
            }
            assert!(!static_prompt.contains("## EXPLAINING, REVIEWING, AND DEBUGGING WORKFLOWS"));
            let prompt = board_sdk_flowscript_system_prompt_with_options(
                source,
                30,
                "Return a JSON string",
                profile,
                BoardPromptMode::Authoring,
            );
            assert!(prompt.contains(&format!("```ts\n{source}\n```")));
            assert_eq!(prompt.matches(source).count(), 1);
        }
    }

    #[test]
    fn authoring_projection_preserves_every_verified_compiler_example() {
        fn verified_examples(prompt: &str) -> Vec<&str> {
            prompt
                .split("```flowscript-verified\n")
                .skip(1)
                .map(|section| section.split_once("\n```").expect("closed example").0)
                .collect()
        }
        for profile in [BoardPromptProfile::Legacy, BoardPromptProfile::Focused] {
            let legacy = board_sdk_flowscript_system_prompt_with_profile("", 30, "", profile);
            let general = board_sdk_flowscript_system_prompt_with_options(
                "",
                30,
                "",
                profile,
                BoardPromptMode::General,
            );
            let authoring = board_sdk_flowscript_system_prompt_with_options(
                "",
                30,
                "",
                profile,
                BoardPromptMode::Authoring,
            );
            assert_eq!(legacy, general);
            let examples = verified_examples(&general);
            assert!(!examples.is_empty());
            assert_eq!(examples, verified_examples(&authoring));
            assert!(general.contains("## WHEN TO USE emit_commands INSTEAD"));
            assert!(general.contains("## EXPLAINING, REVIEWING, AND DEBUGGING WORKFLOWS"));
        }
    }

    #[test]
    fn direct_authoring_keeps_full_legacy_guidance_and_literal_board_context() {
        use crate::copilot::prompts::{board_system_prompt, board_system_prompt_with_mode};
        let source =
            "eventsGeneric literal() { return \"emit_commands catalog_search\" } //@n:keep";
        let context = r#"{"notes":"get_node_details list_board_nodes emit_commands","selected_nodes":["keep"]}"#;
        let prompt = board_system_prompt_with_mode(
            context,
            source,
            30,
            true,
            true,
            BoardPromptMode::Authoring,
        );
        assert!(prompt.contains(&format!("```ts\n{source}\n```")));
        assert!(prompt.contains(context));
        assert_eq!(prompt.matches(source).count(), 1);
        assert_eq!(prompt.matches(context).count(), 1);
        let static_prompt =
            board_system_prompt_with_mode("{}", "", 30, true, true, BoardPromptMode::Authoring);
        for hidden in crate::flow::copilot::WORKFLOW_AUTHORING_HIDDEN_TOOLS {
            assert!(
                !static_prompt.contains(hidden),
                "direct prompt advertises {hidden}"
            );
        }
        for required in [
            "plan_board_scope",
            "test_flowscript",
            "commit_flowscript",
            "search_templates",
            "query_logs",
            "## DATA AND DATABASE WORKFLOWS",
            "## DASHBOARDS, PAGES, AND WIDGETS",
            "### Email round-trip",
            "### Knowledge ingest",
            "### Semantic search",
            "## Graph Context",
            "## Layer Context",
        ] {
            assert!(static_prompt.contains(required), "missing {required}");
        }
        let default = board_system_prompt(context, source, 30, true, true);
        assert_eq!(
            default,
            board_system_prompt_with_mode(
                context,
                source,
                30,
                true,
                true,
                BoardPromptMode::General,
            )
        );
        let examples = |text: &str| {
            text.split("```flowscript-verified\n")
                .skip(1)
                .map(|part| {
                    part.split_once("\n```")
                        .expect("closed example")
                        .0
                        .to_string()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(examples(&default), examples(&prompt));
        assert!(!examples(&prompt).is_empty());
        assert!(default.contains("get_node_details"));
        let without_optional_tools =
            board_system_prompt_with_mode("{}", "", 30, false, false, BoardPromptMode::Authoring);
        assert!(!without_optional_tools.contains("search_templates"));
        assert!(!without_optional_tools.contains("query_logs"));
    }

    #[test]
    fn deterministic_transform_prunes_unrelated_domains_and_keeps_core_contracts() {
        let prompt = board_sdk_flowscript_system_prompt_with_profile(
            "",
            30,
            "Create a Generic Event that returns normalized string payload fields.",
            BoardPromptProfile::Focused,
        );
        let legacy =
            board_sdk_flowscript_system_prompt_with_profile("", 30, "", BoardPromptProfile::Legacy);
        assert!(
            prompt.len() * 100 < legacy.len() * 65,
            "focused={} legacy={}",
            prompt.len(),
            legacy.len()
        );
        assert!(!prompt.contains("## DATA AND DATABASE WORKFLOWS"));
        assert!(!prompt.contains("## DASHBOARDS, PAGES, AND WIDGETS"));
        assert!(!prompt.contains("### Email round-trip"));
        for contract in [
            "commit_flowscript",
            "test_flowscript",
            "expected_revision",
            "//@n:",
            "EXECUTION FLOW AND MULTI-OUTPUT NODES",
            "function square",
            "const rows = [row]",
        ] {
            assert!(prompt.contains(contract), "missing {contract}");
        }
    }

    #[test]
    fn requested_domains_and_existing_source_keep_their_guidance() {
        for (request, source, headings) in [
            (
                "Create a dashboard with database queries",
                "",
                vec![
                    "## DATA AND DATABASE WORKFLOWS",
                    "## DASHBOARDS, PAGES, AND WIDGETS",
                    "### 5. DataFusion",
                    "### 7. Dashboard onLoad",
                    "### 8. Drive a dashboard chart/table",
                ],
            ),
            (
                "Normalize a string payload",
                "eventsSimple() { const page = a2uiSetText({}) }",
                vec!["## DASHBOARDS, PAGES, AND WIDGETS"],
            ),
            (
                "Compute a float for this component's progress indicator",
                "",
                vec!["## DASHBOARDS, PAGES, AND WIDGETS"],
            ),
            (
                "Return a normalized string for the interface view",
                "",
                vec!["## DASHBOARDS, PAGES, AND WIDGETS"],
            ),
            (
                "Return a normalized JSON value",
                "eventsSimple() { const db = openLocalDb({}) }",
                vec!["## DATA AND DATABASE WORKFLOWS"],
            ),
            (
                "Process incoming email",
                "",
                vec![
                    "### Email round-trip",
                    "## DATA AND DATABASE WORKFLOWS",
                    "## FILES ARE FlowPath HANDLES",
                ],
            ),
            (
                "Use an LLM to revise a draft",
                "",
                vec!["### LLM invoke plus struct-field arithmetic"],
            ),
            (
                "Ingest knowledge documents for semantic search",
                "",
                vec![
                    "### Knowledge ingest",
                    "### Semantic search",
                    "## DATA AND DATABASE WORKFLOWS",
                ],
            ),
            (
                "Make it work",
                "",
                vec![
                    "## DATA AND DATABASE WORKFLOWS",
                    "## DASHBOARDS, PAGES, AND WIDGETS",
                ],
            ),
            (
                "Normalisiere die Eingabe für diese Anwendung",
                "",
                vec![
                    "## DATA AND DATABASE WORKFLOWS",
                    "## DASHBOARDS, PAGES, AND WIDGETS",
                ],
            ),
        ] {
            let prompt = board_sdk_flowscript_system_prompt_with_profile(
                source,
                30,
                request,
                BoardPromptProfile::Focused,
            );
            for heading in headings {
                assert!(
                    prompt.contains(heading),
                    "request={request}, source={source}, missing={heading}"
                );
            }
        }
    }

    #[test]
    fn focused_core_examples_parse_as_complete_flowscript() {
        let mut count = 0;
        for section in FOCUSED_CORE_EXAMPLES
            .split("```flowscript-verified\n")
            .skip(1)
        {
            let source = section.split_once("\n```").unwrap().0;
            flow_like_ast::parse(source).expect("focused example must parse");
            count += 1;
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn legacy_profile_is_default_and_ignores_domain_selection() {
        assert_eq!(BoardPromptProfile::default(), BoardPromptProfile::Legacy);
        let explicit = board_sdk_flowscript_system_prompt_with_profile(
            "eventsSimple() {}",
            30,
            "Normalize JSON",
            BoardPromptProfile::Legacy,
        );
        assert_eq!(
            explicit,
            super::super::board_sdk_flowscript_system_prompt("eventsSimple() {}", 30)
        );
        assert!(explicit.contains(FLOWSCRIPT_FEW_SHOT_EXAMPLES));
        assert!(explicit.contains(FLOWSCRIPT_DOMAIN_EXAMPLES));
    }
}
