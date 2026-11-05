use flow_like::flow::copilot::HostTools;
use serde_json::Value;

/// Tools the API runs itself: they touch Lance tables and read objects, nothing heavy.
const IN_PROCESS: [&str; 20] = [
    "secret_list",
    "secret_request",
    "message_owner",
    "list_people",
    "message_person",
    "schedule_task",
    "list_tasks",
    "cancel_task",
    "email_send",
    "email_reply",
    "file_list",
    "file_read",
    "knowledge_create",
    "knowledge_add",
    "knowledge_search",
    "knowledge_list",
    "knowledge_drop",
    "data_sources",
    "data_query",
    "connector_tools",
];

/// Tools that write files, parse documents, run code or call a provider whose answer is untrusted: the executor only.
const EXECUTOR_ONLY: [&str; 17] = [
    "skill_load",
    "skill_propose",
    "skill_call",
    "skill_run",
    "file_save",
    "file_write",
    "file_summarize",
    "file_extract",
    "pdf_render",
    "excel_inspect",
    "document_create",
    "document_fill",
    "code_run",
    "image_generate",
    "connector_call",
    "connector_request",
    "web_request",
];

/// File types the API cannot turn into text; reading or indexing one needs the converters.
const DOCUMENT_EXTENSIONS: [&str; 12] = [
    "pdf", "docx", "doc", "pptx", "ppt", "xlsx", "xls", "odt", "ods", "odp", "epub", "rtf",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    InProcess,
    Executor,
    Refuse(&'static str),
}

const NEEDS_EXECUTOR: &str =
    "This call runs on the executor, which this server does not have configured. Nothing was run.";

pub(crate) fn is_bot_tool(tool: &str) -> bool {
    IN_PROCESS.contains(&tool) || EXECUTOR_ONLY.contains(&tool)
}

pub(crate) fn route(tool: &str, args: &Value, has_executor: bool) -> Option<Route> {
    if !is_bot_tool(tool) {
        return None;
    }
    let heavy = EXECUTOR_ONLY.contains(&tool)
        || (wants_executor(tool, args) && !(tool == "connector_tools" && !has_executor));
    Some(match (heavy, has_executor) {
        (false, _) => Route::InProcess,
        (true, true) => Route::Executor,
        (true, false) => Route::Refuse(NEEDS_EXECUTOR),
    })
}

fn wants_executor(tool: &str, args: &Value) -> bool {
    match tool {
        "file_read" => args.get("file").is_some_and(names_a_document),
        "knowledge_add" => args
            .get("files")
            .and_then(Value::as_array)
            .is_some_and(|files| files.iter().any(names_a_document)),
        "data_query" => args
            .get("save_as")
            .is_some_and(|save_as| save_as.get("file").is_some()),
        "connector_tools" => args
            .get("names")
            .and_then(Value::as_array)
            .is_some_and(|names| !names.is_empty()),
        "data_sources" => args.get("operation").and_then(Value::as_str) == Some("attach"),
        _ => false,
    }
}

fn names_a_document(file: &Value) -> bool {
    let path = match file {
        Value::String(path) => path.as_str(),
        Value::Object(object) => object
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        _ => return false,
    };
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        DOCUMENT_EXTENSIONS
            .iter()
            .any(|document| document.eq_ignore_ascii_case(extension))
    })
}

/// What a bot turn may be offered: the in-process set always, the executor's tools when an
/// executor takes tool jobs, and `code_run` only for the languages that executor runs.
pub(crate) fn host_tools(has_executor: bool, code_languages: &[String]) -> HostTools {
    let mut names: Vec<&str> = IN_PROCESS.to_vec();
    if has_executor {
        names.extend(EXECUTOR_ONLY);
    }
    HostTools::only(&names).with_code_languages(if has_executor { code_languages } else { &[] })
}

/// Every executor bundle (`server`, `executor`, `server-execute`) builds the sandbox with Python
/// and JavaScript, so an unset variable offers both; an empty one turns `code_run` off.
const DEFAULT_CODE_LANGUAGES: &str = "python,javascript";

pub(crate) fn code_languages_from_env() -> Vec<String> {
    parse_languages(Some(
        std::env::var("EXECUTOR_TOOL_CODE_LANGUAGES")
            .as_deref()
            .unwrap_or(DEFAULT_CODE_LANGUAGES),
    ))
}

fn parse_languages(value: Option<&str>) -> Vec<String> {
    let mut languages: Vec<String> = Vec::new();
    for raw in value.unwrap_or_default().split(',') {
        let language = match raw.trim().to_lowercase().as_str() {
            "" => continue,
            "py" | "python3" => "python".to_string(),
            "js" | "node" | "nodejs" => "javascript".to_string(),
            other => other.to_string(),
        };
        if !languages.contains(&language) {
            languages.push(language);
        }
    }
    languages
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn light_tools_stay_in_the_api() {
        for tool in IN_PROCESS {
            assert_eq!(
                route(tool, &json!({}), true),
                Some(Route::InProcess),
                "{tool}"
            );
            assert_eq!(
                route(tool, &json!({}), false),
                Some(Route::InProcess),
                "{tool}"
            );
        }
        assert_eq!(route("database_tool", &json!({}), true), None);
    }

    #[test]
    fn heavy_tools_go_to_the_executor_or_are_refused() {
        for tool in EXECUTOR_ONLY {
            assert_eq!(
                route(tool, &json!({}), true),
                Some(Route::Executor),
                "{tool}"
            );
            assert!(matches!(
                route(tool, &json!({}), false),
                Some(Route::Refuse(_))
            ));
        }
    }

    #[test]
    fn every_untrusted_tool_is_executor_only_and_never_in_process() {
        let untrusted = [
            "document_create",
            "document_fill",
            "skill_load",
            "skill_propose",
            "skill_call",
            "skill_run",
            "connector_call",
            "connector_request",
            "image_generate",
        ];
        for tool in untrusted {
            assert!(
                EXECUTOR_ONLY.contains(&tool),
                "{tool} must be executor-only"
            );
            assert!(
                !IN_PROCESS.contains(&tool),
                "{tool} must not run in the API"
            );
            assert_eq!(
                route(tool, &json!({}), true),
                Some(Route::Executor),
                "{tool}"
            );
            assert!(matches!(
                route(tool, &json!({}), false),
                Some(Route::Refuse(_))
            ));
        }
        assert!(!IN_PROCESS.contains(&"skill_import") && !EXECUTOR_ONLY.contains(&"skill_import"));
        assert_eq!(
            route("connector_tools", &json!({ "names": ["a"] }), true),
            Some(Route::Executor)
        );
        assert_eq!(
            route("connector_tools", &json!({}), true),
            Some(Route::InProcess)
        );
    }

    #[test]
    fn documents_file_saves_and_attaches_follow_the_executor() {
        let cases = [
            ("file_read", json!({ "file": "bot/files/Report.PDF" })),
            (
                "file_read",
                json!({ "file": { "app_id": "a", "path": "x.docx" } }),
            ),
            (
                "knowledge_add",
                json!({ "index": "i", "files": ["a.md", "b.xlsx"] }),
            ),
            (
                "data_query",
                json!({ "sql": "select 1", "save_as": { "file": "out.csv" } }),
            ),
            ("data_sources", json!({ "operation": "attach" })),
        ];
        for (tool, args) in cases {
            assert_eq!(
                route(tool, &args, true),
                Some(Route::Executor),
                "{tool} {args}"
            );
            assert!(matches!(route(tool, &args, false), Some(Route::Refuse(_))));
        }
        for (tool, args) in [
            ("file_read", json!({ "file": "notes.md" })),
            ("file_read", json!({ "file": "#12.a1" })),
            ("knowledge_add", json!({ "index": "i", "files": ["a.md"] })),
            (
                "data_query",
                json!({ "sql": "select 1", "save_as": { "table": "t" } }),
            ),
            ("data_sources", json!({ "operation": "describe" })),
        ] {
            assert_eq!(
                route(tool, &args, true),
                Some(Route::InProcess),
                "{tool} {args}"
            );
        }
    }

    #[test]
    fn without_an_executor_only_the_in_process_set_is_offered() {
        let tools = host_tools(false, &["python".to_string()]);
        let names: Vec<&str> = tools.names().collect();
        for tool in IN_PROCESS {
            assert!(names.contains(&tool), "{tool}");
        }
        for tool in EXECUTOR_ONLY {
            assert!(!tools.runs(tool), "{tool}");
        }
        assert!(tools.code_languages().is_empty());
    }

    #[test]
    fn an_executor_adds_its_tools_and_code_needs_a_language() {
        let tools = host_tools(true, &[]);
        assert!(tools.runs("file_summarize") && tools.runs("file_write"));
        assert!(!tools.runs("code_run"));
        let with_code = host_tools(true, &["python".to_string()]);
        assert!(with_code.runs("code_run"));
        assert_eq!(with_code.code_languages(), ["python".to_string()]);
    }

    #[test]
    fn languages_parse_from_a_comma_list() {
        assert_eq!(
            parse_languages(Some(" Python, ,js, JavaScript ")),
            ["python", "javascript"]
        );
        assert!(parse_languages(None).is_empty());
        assert!(parse_languages(Some("")).is_empty());
        assert_eq!(
            parse_languages(Some(DEFAULT_CODE_LANGUAGES)),
            ["python", "javascript"]
        );
    }
}
