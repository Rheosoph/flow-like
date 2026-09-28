//! Server-side, flow-supplied destinations must go through the egress guard
//! (`flow_like::flow::execution::egress::GuardedHttpClient`), which refuses
//! cloud metadata, loopback and link-local hosts. A raw reqwest client skips
//! it, so catalog production code may only build one for the first-party,
//! fixed-host destinations listed here.

use std::path::{Path, PathBuf};

const RAW_CLIENT_CONSTRUCTORS: &[&str] = &[
    "reqwest::Client::new",
    "reqwest::Client::builder",
    "reqwest::Client::default",
    "reqwest::ClientBuilder::new",
    "reqwest::blocking::",
    "reqwest::get(",
];

/// Constructors called without the `reqwest::` path, flagged in files whose
/// `use` statements bring reqwest's client types into scope.
const UNQUALIFIED_CLIENT_CONSTRUCTORS: &[&str] = &[
    "Client::new(",
    "Client::builder(",
    "Client::default(",
    "ClientBuilder::new(",
    "blocking::",
];

const TEST_CFG_MARKERS: &[&str] = &["#[cfg(test)]", "#[cfg(all(test"];

/// Path prefixes (relative to `packages/catalog`) allowed to build raw clients.
const FIXED_HOST_CLIENTS: &[(&str, &str)] = &[
    (
        "automation/src/browser/",
        "WebDriver / DevTools endpoints of browsers the RPA nodes drive on the host",
    ),
    (
        "data/atlassian/src/data/atlassian/me.rs",
        "api.atlassian.com",
    ),
    (
        "data/atlassian/src/data/atlassian/provider.rs",
        "api.atlassian.com OAuth resources",
    ),
    (
        "data/google/src/",
        "Google APIs on fixed googleapis.com hosts",
    ),
    ("data/linkedin/src/", "api.linkedin.com"),
    (
        "data/microsoft/src/data/microsoft/copilot_studio/",
        "hosts validated against the Microsoft Direct Line / environment host allowlists",
    ),
    ("data/notion/src/", "api.notion.com"),
    (
        "data/src/events/teams.rs",
        "the executor's own API (Teams relay)",
    ),
    (
        "data/support/src/remote_util.rs",
        "the profile's own hub API",
    ),
    (
        "geo/src/geo/map/get_map_image.rs",
        "fixed OpenStreetMap / ArcGIS / Stadia tile hosts",
    ),
    (
        "geo/src/geo/routing/plan_route.rs",
        "router.project-osrm.org",
    ),
    ("geo/src/geo/search/", "nominatim.openstreetmap.org"),
    ("std-runtime/src/", "the profile's own hub API"),
    (
        "web-mail/src/mail/platform_send.rs",
        "the profile's own hub API (platform mail relay)",
    ),
];

fn catalog_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn production_sources(dir: &Path, sources: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !matches!(
                name.as_ref(),
                "target" | "tests" | "benches" | "examples" | "node_modules"
            ) {
                production_sources(&path, sources);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" {
            sources.push(path);
        }
    }
}

/// Drops inline `#[cfg(test)]` / `#[cfg(all(test, …))]` modules. Out-of-line
/// `mod x;` declarations and code after a test module are still scanned.
fn without_test_modules(source: &str) -> String {
    let mut kept = String::with_capacity(source.len());
    let mut rest = source;
    while let Some((start, body)) = next_inline_test_module(rest) {
        kept.push_str(&rest[..start]);
        rest = &rest[block_end(rest, body)..];
    }
    kept.push_str(rest);
    kept
}

/// Byte offsets of the next inline test module's attribute and opening brace.
fn next_inline_test_module(source: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    loop {
        let start = TEST_CFG_MARKERS
            .iter()
            .filter_map(|marker| source[from..].find(marker).map(|offset| from + offset))
            .min()?;
        let mut line_start = start + source[start..].find('\n')? + 1;
        for line in source[line_start..].split_inclusive('\n') {
            let item = line.trim_start();
            if item.trim().is_empty() || item.starts_with("#[") || item.starts_with("//") {
                line_start += line.len();
                continue;
            }
            let is_module = ["mod ", "pub mod ", "pub(crate) mod "]
                .iter()
                .any(|prefix| item.starts_with(prefix));
            let body = item
                .find('{')
                .filter(|&brace| item.find(';').is_none_or(|semicolon| brace < semicolon));
            if is_module && let Some(brace) = body {
                return Some((start, line_start + line.len() - item.len() + brace));
            }
            break;
        }
        from = start + 1;
    }
}

/// Offset just past the brace closing the block opened at `open`, skipping
/// braces inside comments, string and char literals.
fn block_end(source: &str, open: usize) -> usize {
    let bytes = source.as_bytes();
    let is_ident = |at: usize| bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_';
    let starts_raw_string = |at: usize| {
        at == 0 || !is_ident(at - 1) || (bytes[at - 1] == b'b' && (at == 1 || !is_ident(at - 2)))
    };
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i = source[i..].find('\n').map_or(bytes.len(), |end| i + end);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = source[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |end| i + 2 + end + 2);
                continue;
            }
            b'r' if matches!(bytes.get(i + 1), Some(b'"' | b'#')) && starts_raw_string(i) => {
                let hashes = bytes[i + 1..].iter().take_while(|&&b| b == b'#').count();
                let body = i + 1 + hashes;
                if bytes.get(body) == Some(&b'"') {
                    let terminator = format!("\"{}", "#".repeat(hashes));
                    i = source[body + 1..]
                        .find(&terminator)
                        .map_or(bytes.len(), |end| body + 1 + end + terminator.len());
                    continue;
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'\'' => {
                if bytes.get(i + 1) == Some(&b'\\') {
                    i = source
                        .get(i + 3..)
                        .and_then(|rest| rest.find('\''))
                        .map_or(bytes.len(), |end| i + 3 + end);
                } else if let Some(c) = source[i + 1..].chars().next()
                    && bytes.get(i + 1 + c.len_utf8()) == Some(&b'\'')
                {
                    i += 1 + c.len_utf8();
                }
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

fn imports_reqwest_client(source: &str) -> bool {
    let mut offset = 0;
    source.split_inclusive('\n').any(|line| {
        let item = line.trim_start();
        let start = offset + line.len() - item.len();
        offset += line.len();
        ["use ", "pub use ", "pub(crate) use "]
            .iter()
            .any(|prefix| item.starts_with(prefix))
            && source[start..].split(';').next().is_some_and(|statement| {
                statement.contains("reqwest")
                    && (statement.contains("Client") || statement.contains("blocking"))
            })
    })
}

/// `pattern` not preceded by an identifier character or a path separator.
fn contains_unqualified(source: &str, pattern: &str) -> bool {
    source.match_indices(pattern).any(|(at, _)| {
        source[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == ':'))
    })
}

fn builds_raw_client(source: &str) -> bool {
    let source = without_test_modules(source);
    RAW_CLIENT_CONSTRUCTORS
        .iter()
        .any(|constructor| source.contains(constructor))
        || (imports_reqwest_client(&source)
            && UNQUALIFIED_CLIENT_CONSTRUCTORS
                .iter()
                .any(|constructor| contains_unqualified(&source, constructor)))
}

fn raw_client_files() -> Vec<String> {
    let root = catalog_root();
    let mut sources = Vec::new();
    production_sources(&root, &mut sources);
    let mut files: Vec<String> = sources
        .into_iter()
        .filter(|path| path.components().any(|part| part.as_os_str() == "src"))
        .filter(|path| std::fs::read_to_string(path).is_ok_and(|source| builds_raw_client(&source)))
        .map(|path| {
            path.strip_prefix(&root)
                .expect("sources are collected below the catalog root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    files.sort();
    files
}

#[test]
fn raw_http_clients_only_reach_fixed_first_party_hosts() {
    let files = raw_client_files();
    let unguarded: Vec<&String> = files
        .iter()
        .filter(|file| {
            !FIXED_HOST_CLIENTS
                .iter()
                .any(|(prefix, _)| file.starts_with(prefix))
        })
        .collect();
    assert!(
        unguarded.is_empty(),
        "These catalog files build a raw reqwest client. Requests to flow-supplied URLs must use \
         `egress::GuardedHttpClient::new(context.execution_environment())`; only add a file to \
         FIXED_HOST_CLIENTS when every destination is a fixed first-party host:\n  {}",
        unguarded
            .iter()
            .map(|file| file.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

#[test]
fn fixed_host_allowlist_has_no_stale_entries() {
    let files = raw_client_files();
    let stale: Vec<&str> = FIXED_HOST_CLIENTS
        .iter()
        .map(|(prefix, _)| *prefix)
        .filter(|prefix| !files.iter().any(|file| file.starts_with(prefix)))
        .collect();
    assert!(
        stale.is_empty(),
        "FIXED_HOST_CLIENTS entries no longer build a raw client; remove them: {stale:?}"
    );
}

#[test]
fn test_modules_are_not_scanned() {
    let source = "fn run() {}\n#[cfg(test)]\nmod tests {\n    fn f() { reqwest::get(\"x\"); }\n}\n";
    assert_eq!(without_test_modules(source), "fn run() {}\n\n");
    assert!(!builds_raw_client(source));
    let helper = "#[cfg(test)]\nfn helper() {}\nfn run() { reqwest::Client::new(); }\n";
    assert_eq!(without_test_modules(helper), helper);
}

#[test]
fn code_around_test_modules_is_scanned() {
    let declared =
        "#[cfg(test)]\nmod compatibility_tests;\n\nfn run() { reqwest::Client::new(); }\n";
    assert!(builds_raw_client(declared));
    let after = concat!(
        "#[cfg(test)]\nmod tests {\n",
        "    const OPEN: &str = \"{\";\n",
        "    const RAW: &str = r#\"}\"#;\n",
        "    const BRACE: char = '}';\n",
        "    const QUOTE: char = '\\'';\n",
        "    const ESCAPED: char = '\\u{7d}';\n",
        "    // }\n",
        "    fn f<'a>(s: &'a str) -> &'a str { s }\n",
        "}\n",
        "fn run() { reqwest::Client::builder(); }\n",
    );
    assert!(builds_raw_client(after));
}

#[test]
fn constructor_spellings_are_detected() {
    for source in [
        "fn run() { reqwest::Client::default(); }",
        "fn run() { reqwest::blocking::get(\"x\"); }",
        "use reqwest::Client;\nfn run() { Client::new(); }",
        "use flow_like_types::reqwest::{Client, Url};\nfn run() { Client::builder(); }",
        "use reqwest::blocking;\nfn run() { blocking::get(\"x\"); }",
    ] {
        assert!(builds_raw_client(source), "{source}");
    }
    for source in [
        "use reqwest::Url;\nfn run() { GuardedHttpClient::new(env); }",
        "use flow_like_types::reqwest::Client;\nfn run() { aws_sdk_s3::Client::new(&config); }",
        "fn run() { Client::builder().use_stdio(true); }",
    ] {
        assert!(!builds_raw_client(source), "{source}");
    }
}
