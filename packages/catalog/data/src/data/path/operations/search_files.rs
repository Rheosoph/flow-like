use super::list_paths::list_files;
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_storage::{
    Path,
    object_store::{ObjectStore, ObjectStoreExt},
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

const BINARY_SNIFF_BYTES: usize = 8 * 1024;
const MAX_LINE_CHARS: usize = 1_000;
const DEFAULT_MAX_FILE_SIZE: i64 = 1024 * 1024;
const DEFAULT_MAX_RESULTS: i64 = 200;
const DEFAULT_CONTEXT_LINES: i64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileSearchMatch {
    /// Path relative to the searched directory
    pub path: String,
    /// The matching file, ready for Read to String and other file nodes
    pub file: FlowPath,
    /// 1-based line number
    pub line_number: u64,
    pub line: String,
    pub before: Vec<String>,
    pub after: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LineMatch {
    pub path: String,
    pub location: String,
    pub line_number: u64,
    pub line: String,
    pub before: Vec<String>,
    pub after: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct SearchOutcome {
    pub matches: Vec<LineMatch>,
    pub truncated: bool,
    pub files_scanned: usize,
}

pub(crate) struct SearchOptions {
    pub matcher: Regex,
    pub include: Vec<PathGlob>,
    pub exclude: Vec<PathGlob>,
    pub max_file_size: u64,
    pub max_results: usize,
    pub context_lines: usize,
}

/// A glob over `/`-separated relative paths. A pattern without `/` matches any single path
/// segment (so `*.rs` matches files and `node_modules` matches that folder at any depth);
/// a pattern with `/` matches the whole path, where `**` spans folders.
#[derive(Debug, Clone)]
pub(crate) struct PathGlob {
    regex: Regex,
    segment: bool,
}

impl PathGlob {
    pub fn new(pattern: &str) -> flow_like_types::Result<Self> {
        let pattern = pattern.trim();
        let segment = !pattern.trim_matches('/').contains('/');
        let source = glob_to_regex(pattern.trim_start_matches('/').trim_end_matches('/'));
        let regex = Regex::new(&source)
            .map_err(|e| flow_like_types::anyhow!("Invalid glob '{}': {}", pattern, e))?;
        Ok(Self { regex, segment })
    }

    pub fn matches(&self, relative_path: &str) -> bool {
        if self.segment {
            relative_path
                .split('/')
                .any(|segment| self.regex.is_match(segment))
        } else {
            self.regex.is_match(relative_path)
        }
    }
}

pub(crate) fn glob_to_regex(glob: &str) -> String {
    let chars: Vec<char> = glob.chars().collect();
    let mut out = String::from("^");
    let mut i = 0;
    let mut in_braces = false;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '*' if chars.get(i + 1) == Some(&'*') => {
                let at_segment_start = i == 0 || chars[i - 1] == '/';
                if at_segment_start && chars.get(i + 2) == Some(&'/') {
                    out.push_str("(?:.*/)?");
                    i += 3;
                    continue;
                }
                out.push_str(".*");
                i += 2;
                continue;
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '{' if !in_braces => {
                in_braces = true;
                out.push_str("(?:");
            }
            '}' if in_braces => {
                in_braces = false;
                out.push(')');
            }
            ',' if in_braces => out.push('|'),
            '[' => {
                if let Some(end) = chars[i + 1..].iter().position(|&ch| ch == ']') {
                    let class: String = chars[i + 1..i + 1 + end].iter().collect();
                    let class = class
                        .strip_prefix('!')
                        .map(|rest| format!("^{rest}"))
                        .unwrap_or(class);
                    out.push('[');
                    out.push_str(&class.replace('\\', "\\\\"));
                    out.push(']');
                    i += end + 2;
                    continue;
                }
                out.push_str("\\[");
            }
            other => out.push_str(&regex::escape(&other.to_string())),
        }
        i += 1;
    }
    out.push('$');
    out
}

pub(crate) fn build_matcher(
    pattern: &str,
    is_regex: bool,
    case_sensitive: bool,
) -> flow_like_types::Result<Regex> {
    if pattern.is_empty() {
        return Err(flow_like_types::anyhow!("Search pattern must not be empty"));
    }
    let source = if is_regex {
        pattern.to_string()
    } else {
        regex::escape(pattern)
    };
    RegexBuilder::new(&source)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|e| flow_like_types::anyhow!("Invalid search pattern '{}': {}", pattern, e))
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0)
}

fn clip_line(line: &str) -> String {
    let line = line.strip_suffix('\r').unwrap_or(line);
    match line.char_indices().nth(MAX_LINE_CHARS) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

/// Searches `text` and appends matches until `max_results` is reached. Returns `true` when
/// matches were left out because of the limit.
pub(crate) fn search_text(
    text: &str,
    path: &str,
    location: &str,
    options: &SearchOptions,
    matches: &mut Vec<LineMatch>,
) -> bool {
    let lines: Vec<&str> = text.split('\n').collect();
    for (index, line) in lines.iter().enumerate() {
        if !options.matcher.is_match(line) {
            continue;
        }
        if matches.len() >= options.max_results {
            return true;
        }
        let start = index.saturating_sub(options.context_lines);
        let end = (index + 1 + options.context_lines).min(lines.len());
        matches.push(LineMatch {
            path: path.to_string(),
            location: location.to_string(),
            line_number: index as u64 + 1,
            line: clip_line(line),
            before: lines[start..index].iter().map(|l| clip_line(l)).collect(),
            after: lines[index + 1..end].iter().map(|l| clip_line(l)).collect(),
        });
    }
    false
}

pub(crate) fn relative_path(location: &str, prefix: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return location.to_string();
    }
    location
        .strip_prefix(prefix)
        .map(|rest| rest.trim_start_matches('/').to_string())
        .unwrap_or_else(|| location.to_string())
}

pub(crate) fn is_selected(relative: &str, options: &SearchOptions) -> bool {
    let included =
        options.include.is_empty() || options.include.iter().any(|g| g.matches(relative));
    included && !options.exclude.iter().any(|g| g.matches(relative))
}

pub(crate) async fn search_store(
    store: &dyn ObjectStore,
    prefix: &Path,
    options: &SearchOptions,
) -> flow_like_types::Result<SearchOutcome> {
    let mut files = list_files(store, prefix, true).await?;
    files.sort_by(|a, b| a.location.as_ref().cmp(b.location.as_ref()));

    let mut outcome = SearchOutcome::default();
    for file in files {
        let location = file.location.as_ref().to_string();
        let relative = relative_path(&location, prefix.as_ref());
        if file.size > options.max_file_size || !is_selected(&relative, options) {
            continue;
        }
        let bytes = store.get(&file.location).await?.bytes().await?;
        if is_binary(&bytes) {
            continue;
        }
        outcome.files_scanned += 1;
        let text = String::from_utf8_lossy(&bytes);
        if search_text(&text, &relative, &location, options, &mut outcome.matches) {
            outcome.truncated = true;
            break;
        }
    }
    Ok(outcome)
}

fn parse_globs(patterns: Vec<String>) -> flow_like_types::Result<Vec<PathGlob>> {
    patterns
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(PathGlob::new)
        .collect()
}

#[crate::register_node]
#[derive(Default)]
pub struct SearchFilesNode {}

impl SearchFilesNode {
    pub fn new() -> Self {
        SearchFilesNode {}
    }
}

#[async_trait]
impl NodeLogic for SearchFilesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_search_files",
            "Search Files",
            "Searches the text files below a directory for a literal string or regular expression and returns each matching line with its line number and surrounding lines. Works on every store type. Binary files and files above the size limit are skipped, and lines longer than 1,000 characters are shortened.",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "searchFiles");
        node.set_receiver("directory");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "directory",
            "Directory",
            "Directory to search recursively",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "pattern",
            "Pattern",
            "Text or regular expression to search for",
            VariableType::String,
        );

        node.add_input_pin(
            "is_regex",
            "Regex",
            "Treat the pattern as a regular expression",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "case_sensitive",
            "Case Sensitive",
            "Match upper and lower case exactly",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "include",
            "Include",
            "Only search paths matching one of these globs, e.g. *.rs or src/**/*.{ts,tsx}. A glob without / matches any path segment. Empty searches all files",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "exclude",
            "Exclude",
            "Skip paths matching one of these globs, e.g. node_modules or **/*.min.js",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "max_file_size",
            "Max File Size",
            "Skip files larger than this many bytes",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(DEFAULT_MAX_FILE_SIZE)));

        node.add_input_pin(
            "max_results",
            "Max Results",
            "Stop after this many matching lines",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(DEFAULT_MAX_RESULTS)));

        node.add_input_pin(
            "context_lines",
            "Context Lines",
            "Lines to include before and after each match",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(DEFAULT_CONTEXT_LINES)))
        .set_options(PinOptions::new().set_range((0.0, 20.0)).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "matches",
            "Matches",
            "Matching lines with path, line number and context",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<FileSearchMatch>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "truncated",
            "Truncated",
            "True when more matches exist than Max Results allowed",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "files_scanned",
            "Files Scanned",
            "Number of text files that were searched",
            VariableType::Integer,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(6)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let directory: FlowPath = context.evaluate_pin("directory").await?;
        let pattern: String = context.evaluate_pin("pattern").await?;
        let is_regex: bool = context.evaluate_pin("is_regex").await.unwrap_or(false);
        let case_sensitive: bool = context
            .evaluate_pin("case_sensitive")
            .await
            .unwrap_or(false);
        let include: Vec<String> = context.evaluate_pin("include").await.unwrap_or_default();
        let exclude: Vec<String> = context.evaluate_pin("exclude").await.unwrap_or_default();
        let max_file_size: i64 = context
            .evaluate_pin("max_file_size")
            .await
            .unwrap_or(DEFAULT_MAX_FILE_SIZE);
        let max_results: i64 = context
            .evaluate_pin("max_results")
            .await
            .unwrap_or(DEFAULT_MAX_RESULTS);
        let context_lines: i64 = context
            .evaluate_pin("context_lines")
            .await
            .unwrap_or(DEFAULT_CONTEXT_LINES);

        let options = SearchOptions {
            matcher: build_matcher(&pattern, is_regex, case_sensitive)?,
            include: parse_globs(include)?,
            exclude: parse_globs(exclude)?,
            max_file_size: max_file_size.max(0) as u64,
            max_results: max_results.max(1) as usize,
            context_lines: context_lines.clamp(0, 20) as usize,
        };

        let runtime = directory.to_runtime(context).await?;
        let store = runtime.store.as_generic();
        let outcome = search_store(store.as_ref(), &runtime.path, &options).await?;

        let matches: Vec<FileSearchMatch> = outcome
            .matches
            .into_iter()
            .map(|m| {
                let mut file = directory.clone();
                file.path = m.location;
                FileSearchMatch {
                    path: m.path,
                    file,
                    line_number: m.line_number,
                    line: m.line,
                    before: m.before,
                    after: m.after,
                }
            })
            .collect();

        context.set_pin_value("matches", json!(matches)).await?;
        context
            .set_pin_value("truncated", json!(outcome.truncated))
            .await?;
        context
            .set_pin_value("files_scanned", json!(outcome.files_scanned))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::{PutPayload, memory::InMemory};
    use std::sync::Arc;

    fn options(pattern: &str) -> SearchOptions {
        SearchOptions {
            matcher: build_matcher(pattern, false, false).unwrap(),
            include: vec![],
            exclude: vec![],
            max_file_size: 1024 * 1024,
            max_results: 100,
            context_lines: 1,
        }
    }

    fn globs(patterns: &[&str]) -> Vec<PathGlob> {
        patterns.iter().map(|p| PathGlob::new(p).unwrap()).collect()
    }

    async fn store_with(files: &[(&str, &[u8])]) -> Arc<dyn ObjectStore> {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        for (path, bytes) in files {
            store
                .put(&Path::from(*path), PutPayload::from(bytes.to_vec()))
                .await
                .unwrap();
        }
        store
    }

    #[test]
    fn globs_without_slash_match_any_segment() {
        let glob = PathGlob::new("*.rs").unwrap();
        assert!(glob.matches("src/main.rs"));
        assert!(glob.matches("lib.rs"));
        assert!(!glob.matches("src/main.rs.bak"));

        let glob = PathGlob::new("node_modules").unwrap();
        assert!(glob.matches("web/node_modules/react/index.js"));
        assert!(!glob.matches("web/src/node_modules.ts"));
    }

    #[test]
    fn globs_with_slash_match_whole_path() {
        let glob = PathGlob::new("src/**/*.{ts,tsx}").unwrap();
        assert!(glob.matches("src/a.ts"));
        assert!(glob.matches("src/ui/deep/b.tsx"));
        assert!(!glob.matches("web/src/a.ts"));
        assert!(!glob.matches("src/a.js"));

        let glob = PathGlob::new("**/*.min.js").unwrap();
        assert!(glob.matches("dist/app.min.js"));
        assert!(glob.matches("app.min.js"));

        let glob = PathGlob::new("config/[!d]*.yml").unwrap();
        assert!(glob.matches("config/prod.yml"));
        assert!(!glob.matches("config/dev.yml"));
    }

    #[test]
    fn literal_patterns_escape_regex_syntax_and_ignore_case_by_default() {
        let matcher = build_matcher("ValidateAudience = false", false, false).unwrap();
        assert!(matcher.is_match("  validateaudience = FALSE,"));
        let matcher = build_matcher("a.b(", false, true).unwrap();
        assert!(matcher.is_match("x a.b( y"));
        assert!(!matcher.is_match("axb("));
        assert!(build_matcher("", false, false).is_err());
        assert!(build_matcher("(", true, false).is_err());
    }

    #[test]
    fn context_lines_and_long_lines() {
        let mut matches = Vec::new();
        let long = "x".repeat(MAX_LINE_CHARS + 10);
        let text = format!("one\r\ntwo needle\r\nthree\n{long}needle");
        let truncated = search_text(
            &text,
            "a.txt",
            "root/a.txt",
            &options("needle"),
            &mut matches,
        );
        assert!(!truncated);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_number, 2);
        assert_eq!(matches[0].line, "two needle");
        assert_eq!(matches[0].before, vec!["one"]);
        assert_eq!(matches[0].after, vec!["three"]);
        assert_eq!(matches[1].line.chars().count(), MAX_LINE_CHARS + 1);
        assert!(matches[1].after.is_empty());
    }

    #[test]
    fn relative_paths_strip_the_searched_directory() {
        assert_eq!(relative_path("repo/src/a.rs", "repo"), "src/a.rs");
        assert_eq!(relative_path("repo/src/a.rs", "repo/"), "src/a.rs");
        assert_eq!(relative_path("src/a.rs", ""), "src/a.rs");
    }

    #[tokio::test]
    async fn searches_store_skipping_binary_large_and_excluded_files() {
        let store = store_with(&[
            (
                "repo/src/auth.ts",
                b"verify(token, { audience: aud })\nconst x = 1;\n",
            ),
            (
                "repo/src/legacy.ts",
                b"jwt.decode(token) // no audience check\n",
            ),
            ("repo/node_modules/lib/index.js", b"audience\n"),
            ("repo/assets/logo.png", b"\x89PNG\0\0audience"),
            ("repo/big.log", &[b'a'; 64]),
            ("other/src/auth.ts", b"audience\n"),
        ])
        .await;
        let mut opts = options("audience");
        opts.exclude = globs(&["node_modules"]);
        opts.max_file_size = 63;

        let outcome = search_store(store.as_ref(), &Path::from("repo"), &opts)
            .await
            .unwrap();
        let paths: Vec<_> = outcome.matches.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, vec!["src/auth.ts", "src/legacy.ts"]);
        assert_eq!(outcome.files_scanned, 2);
        assert!(!outcome.truncated);
        assert_eq!(outcome.matches[0].location, "repo/src/auth.ts");
    }

    #[tokio::test]
    async fn include_globs_and_result_limit() {
        let store = store_with(&[
            ("repo/a.rs", b"needle\nneedle\n"),
            ("repo/b.rs", b"needle\n"),
            ("repo/c.md", b"needle\n"),
        ])
        .await;
        let mut opts = options("needle");
        opts.include = globs(&["*.rs"]);
        opts.max_results = 2;

        let outcome = search_store(store.as_ref(), &Path::from("repo"), &opts)
            .await
            .unwrap();
        assert_eq!(outcome.matches.len(), 2);
        assert!(outcome.truncated);
        assert!(outcome.matches.iter().all(|m| m.path == "a.rs"));
    }
}
