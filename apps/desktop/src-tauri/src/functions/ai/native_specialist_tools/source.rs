//! Bounded workspace discovery with profile-scoped, revision-checked reads.
use super::*;
use crate::functions::{
    app::tables,
    flow::{board as boards, event as events},
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Mutex as StdMutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

const KINDS: [&str; 5] = ["workflow", "event", "table", "page", "doc"];
const DOCS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../packages/ui/lib/flowpilot/generated/workspace-docs.json"
));
#[derive(Clone, Serialize, Deserialize)]
struct Target {
    kind: String,
    app_id: Option<String>,
    board_id: Option<String>,
    id: String,
}
#[derive(Clone)]
struct Document {
    target: Target,
    revision: String,
    title: String,
    content: String,
    extra: Value,
    truncated: bool,
}
#[derive(Clone)]
struct Snapshot {
    created: Instant,
    observed: u128,
    scope: String,
    hits: Vec<Value>,
    coverage: Value,
    mode: String,
}
static SNAPSHOTS: LazyLock<StdMutex<HashMap<String, Snapshot>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

pub(super) fn supports(tool: &str) -> bool {
    matches!(
        tool,
        "read_flowscript_source" | "search_workspace" | "read_symbol"
    )
}
pub(super) async fn execute(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    match tool {
        "read_flowscript_source" => read_source(session, args).await,
        "search_workspace" => search(session, args).await,
        _ => read_symbol(session, args).await,
    }
}
fn failure(code: &str, message: &str) -> Value {
    json!({"status":"error","code":code,"message":message})
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn encoded(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char)
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
fn resource_id(target: &Target) -> String {
    format!(
        "workspace:v1:{}",
        encoded(&json!([target.kind, target.app_id, target.board_id, target.id]).to_string())
    )
}
fn parse_resource(value: &str) -> Option<Target> {
    if value.len() > 4096 {
        return None;
    }
    let raw = urlencoding::decode(value.strip_prefix("workspace:v1:")?).ok()?;
    let parts: Vec<Value> = serde_json::from_str(&raw).ok()?;
    if parts.len() != 4 {
        return None;
    }
    let target = Target {
        kind: parts[0].as_str()?.to_owned(),
        app_id: parts[1].as_str().map(str::to_owned),
        board_id: parts[2].as_str().map(str::to_owned),
        id: parts[3].as_str()?.to_owned(),
    };
    if !KINDS.contains(&target.kind.as_str())
        || target.id.is_empty()
        || target.id.len() > 512
        || target
            .app_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
        || target
            .board_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
        || if target.kind == "doc" {
            target.app_id.is_some() || target.board_id.is_some()
        } else {
            target.app_id.is_none()
        }
        || target.kind == "workflow" && target.board_id.is_none()
    {
        return None;
    }
    (resource_id(&target) == value).then_some(target)
}
fn authorize(session: &NativePlatformSession, app_id: &str) -> Result<(), String> {
    if session.context.as_ref().and_then(|c| c.app_id.as_deref()) == Some(app_id) {
        Ok(())
    } else {
        session.require_visible(app_id)
    }
}
async fn flow_source(
    session: &NativePlatformSession,
    app_id: &str,
    board_id: &str,
) -> Result<String, String> {
    authorize(session, app_id)?;
    if session.is_local(app_id).await? {
        let app = crate::functions::app::get_app(session.app_handle.clone(), app_id.to_owned())
            .await
            .map_err(|e| e.to_string())?;
        if !app.boards.iter().any(|id| id == board_id) {
            return Err("The board does not belong to this app.".into());
        }
        boards::get_flowscript(
            session.app_handle.clone(),
            app_id.to_owned(),
            board_id.to_owned(),
            None,
            Some(true),
        )
        .await
        .map_err(|e| e.to_string())
    } else {
        session
            .request(
                Method::GET,
                &format!(
                    "apps/{}/board/{}/flowscript?anchors=true",
                    segment(app_id),
                    segment(board_id)
                ),
                None,
            )
            .await?["flowscript"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "The hub returned no canonical source.".into())
    }
}
fn utf16_byte(value: &str, offset: usize) -> Option<usize> {
    let mut n = 0;
    for (byte, c) in value.char_indices() {
        if n == offset {
            return Some(byte);
        }
        n += c.len_utf16();
        if n > offset {
            return None;
        }
    }
    (n == offset).then_some(value.len())
}
fn bound_json_text(value: &str, budget: usize) -> String {
    let mut end = value.len().min(budget);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    while serde_json::to_vec(&value[..end]).map_or(usize::MAX, |v| v.len()) > budget {
        end -= 1;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
    }
    value[..end].to_owned()
}
fn bound_source(source: &str, locator: Option<&str>) -> Value {
    let source_chars = source.encode_utf16().count();
    let mut hit = locator.and_then(|locator| source.find(locator));
    if hit.is_none() {
        if let Some(locator) = locator {
            let mut parts = locator
                .split(|c: char| !c.is_ascii_alphanumeric() && !"_:@./-".contains(c))
                .filter(|s| s.len() >= 8)
                .collect::<Vec<_>>();
            parts.sort_by_key(|part| std::cmp::Reverse(part.len()));
            hit = parts.into_iter().find_map(|part| source.find(part));
        }
    }
    let hit_offset = hit.map(|byte| source[..byte].encode_utf16().count());
    let mut start = if source_chars > 60_000 {
        hit_offset.unwrap_or(0).saturating_sub(20_000)
    } else {
        0
    };
    let mut start_byte = utf16_byte(source, start)
        .unwrap_or_else(|| utf16_byte(source, start.saturating_sub(1)).unwrap_or(0));
    if start > 0 {
        if let Some(next) = source[start_byte..].find('\n') {
            if source[start_byte..start_byte + next].encode_utf16().count() < 15_000 {
                start_byte += next + 1;
                start = source[..start_byte].encode_utf16().count();
            }
        }
    }
    let mut end = (start + 60_000).min(source_chars);
    let mut end_byte =
        utf16_byte(source, end).unwrap_or_else(|| utf16_byte(source, end - 1).unwrap());
    if end < source_chars {
        if let Some(previous) = source[..end_byte].rfind('\n') {
            if previous > start_byte {
                end_byte = previous + 1;
                end = source[..end_byte].encode_utf16().count();
            }
        }
    }
    let mut result = json!({"source":&source[start_byte..end_byte],"source_chars":source_chars,"returned_chars":end-start,"start_offset":start,"end_offset":end,"truncated":start>0||end<source_chars});
    if locator.is_some() {
        result["locator_matched"] = json!(hit.is_some());
    }
    result
}
async fn read_source(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let app_id = string(args, "app_id", "appId")
        .or_else(|| session.context.as_ref().and_then(|c| c.app_id.as_deref()));
    let board_id = string(args, "board_id", "boardId")
        .or_else(|| session.context.as_ref().and_then(|c| c.board_id.as_deref()));
    let (Some(app_id), Some(board_id)) = (app_id, board_id) else {
        return Ok(failure(
            "FLOWSCRIPT_SOURCE_TARGET_REQUIRED",
            "read_flowscript_source requires app_id and board_id.",
        ));
    };
    if authorize(session, app_id).is_err() {
        return Ok(
            json!({"status":"forbidden","code":"FLOWSCRIPT_SOURCE_APP_NOT_ACCESSIBLE","app_id":app_id,"board_id":board_id}),
        );
    }
    let source = match flow_source(session, app_id, board_id).await {
        Ok(source) => source,
        Err(_) => {
            return Ok(failure(
                "FLOWSCRIPT_SOURCE_NOT_READABLE",
                "The requested board could not be read. Verify its identity and access.",
            ));
        }
    };
    let locator = string(args, "locator", "locator");
    let mut result = bound_source(&source, locator);
    result["status"] = json!("ok");
    result["app_id"] = json!(app_id);
    result["board_id"] = json!(board_id);
    if let Some(locator) = locator {
        result["locator"] = json!(locator);
        if result["locator_matched"] == false {
            result["note"] = json!(
                "The locator was not found verbatim or by stable identifier; the bounded source is returned for inspection."
            );
        }
    }
    Ok(result)
}

#[derive(Clone)]
struct Token {
    start: usize,
    end: usize,
    text: String,
    pair: Option<usize>,
}
fn skip_literal(bytes: &[u8], mut i: usize, depth: usize) -> Result<usize, String> {
    if depth > 128 {
        return Err("Source nesting exceeds the index limit.".into());
    }
    let quote = bytes[i];
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] == quote {
            return Ok(i + 1);
        }
        if quote == b'`' && bytes[i..].starts_with(b"${") {
            i += 2;
            let mut braces = 1;
            while i < bytes.len() && braces > 0 {
                match bytes[i] {
                    b'\'' | b'"' | b'`' => {
                        i = skip_literal(bytes, i, depth + 1)?;
                        continue;
                    }
                    b'{' => braces += 1,
                    b'}' => braces -= 1,
                    _ => {}
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    Err("Unterminated source string.".into())
}
fn tokens(source: &str) -> Result<Vec<Token>, String> {
    if source.len() > 1_048_576 {
        return Err("Source exceeds the index limit.".into());
    }
    let bytes = source.as_bytes();
    let mut result: Vec<Token> = Vec::new();
    let mut stack = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        if b"\"'`".contains(&bytes[i]) {
            i = skip_literal(bytes, i, 0)?;
        } else if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else {
            let c = source[i..].chars().next().unwrap();
            i += c.len_utf8();
        }
        let text = source[start..i].to_owned();
        let index = result.len();
        let mut token = Token {
            start,
            end: i,
            text: text.clone(),
            pair: None,
        };
        if matches!(text.as_str(), "{" | "(" | "[") {
            stack.push(index);
            if stack.len() > 128 {
                return Err("Source nesting exceeds the index limit.".into());
            }
        }
        if matches!(text.as_str(), "}" | ")" | "]") {
            let open = stack.pop().ok_or("Unbalanced source.")?;
            let expected = match result[open].text.as_str() {
                "{" => "}",
                "(" => ")",
                _ => "]",
            };
            if text != expected {
                return Err("Unbalanced source.".into());
            }
            result[open].pair = Some(index);
            token.pair = Some(open);
        }
        result.push(token);
        if result.len() > 131_072 {
            return Err("Source token limit exceeded.".into());
        }
    }
    if !stack.is_empty() {
        return Err("Unbalanced source.".into());
    }
    Ok(result)
}
fn workflow_documents(source: &str, app_id: &str, board_id: &str) -> Result<Vec<Document>, String> {
    flow_like::flow::ast::parse(source)
        .map_err(|_| "The canonical source is not indexable.".to_owned())?;
    let tokens = tokens(source)?;
    let revision = hash(source);
    let mut result = Vec::new();
    fn scope(
        source: &str,
        tokens: &[Token],
        mut cursor: usize,
        stop: usize,
        path: &[String],
        app: &str,
        board: &str,
        revision: &str,
        result: &mut Vec<Document>,
    ) -> Result<(), String> {
        while cursor < stop {
            let first = cursor;
            let head = &tokens[cursor];
            if matches!(head.text.as_str(), ";" | "}") {
                cursor += 1;
                continue;
            }
            if head.text == "@" {
                cursor += 1;
                while cursor < stop
                    && source[tokens[first].end..tokens[cursor].start]
                        .find('\n')
                        .is_none()
                {
                    if let Some(close) = tokens[cursor].pair {
                        cursor = close + 1;
                    } else {
                        cursor += 1;
                    }
                }
                continue;
            }
            if matches!(head.text.as_str(), "use" | "const" | "let") {
                cursor += 1;
                while cursor < stop {
                    if source[tokens[cursor - 1].end..tokens[cursor].start].contains('\n') {
                        break;
                    }
                    if let Some(close) = tokens[cursor].pair {
                        if close > cursor {
                            cursor = close + 1;
                            continue;
                        }
                    }
                    cursor += 1;
                }
                continue;
            }
            let Some(open) = (cursor..stop).find(|&i| tokens[i].text == "{") else {
                break;
            };
            let close = tokens[open].pair.ok_or("Missing source body.")?;
            if close >= stop {
                return Err("Declaration escapes its module.".into());
            }
            if head.text == "detached" {
                cursor = close + 1;
                continue;
            }
            let module = head.text == "module";
            let name = if matches!(head.text.as_str(), "function" | "interface" | "module") {
                tokens
                    .get(cursor + 1)
                    .ok_or("Missing declaration name.")?
                    .text
                    .clone()
            } else {
                let paren = (cursor..open)
                    .find(|&i| tokens[i].text == "(")
                    .ok_or("Missing event signature.")?;
                tokens[paren.saturating_sub(1)].text.clone()
            };
            let mut qualified = path.to_vec();
            qualified.push(name);
            if module {
                scope(
                    source,
                    tokens,
                    open + 1,
                    close,
                    &qualified,
                    app,
                    board,
                    revision,
                    result,
                )?;
            } else {
                let name = qualified.join("::");
                if result.iter().any(|d| d.target.id == name) {
                    return Err("The source has duplicate qualified symbols.".into());
                }
                let mut start = head.start;
                let line_start = source[..start].rfind('\n').map_or(0, |at| at + 1);
                let mut scan = line_start;
                while scan > 0 {
                    let before = &source[..scan - 1];
                    let begin = before.rfind('\n').map_or(0, |at| at + 1);
                    let line = source[begin..scan].trim();
                    if !line.starts_with("//") || line.starts_with("//@") {
                        break;
                    }
                    start = begin;
                    scan = begin;
                }
                let target = Target {
                    kind: "workflow".into(),
                    app_id: Some(app.into()),
                    board_id: Some(board.into()),
                    id: name.clone(),
                };
                let anchor_line = source[tokens[open].end..]
                    .split('\n')
                    .next()
                    .unwrap_or("")
                    .trim();
                let anchor = anchor_line
                    .strip_prefix("//@")
                    .and_then(|a| a.split_once(':'))
                    .map(|(_, id)| id.trim());
                result.push(Document{target,revision:revision.into(),title:name.clone(),content:source[start..tokens[close].end].into(),truncated:false,extra:json!({"symbol":name,"signature":source[head.start..tokens[open].start].trim(),"start_line":source[..start].bytes().filter(|&b|b==b'\n').count()+1,"end_line":source[..tokens[close].start].bytes().filter(|&b|b==b'\n').count()+1,"anchor_id":anchor})});
                if result.len() > 2048 {
                    return Err("Source symbol limit exceeded.".into());
                }
            }
            cursor = close + 1;
        }
        Ok(())
    }
    scope(
        source,
        &tokens,
        0,
        tokens.len(),
        &[],
        app_id,
        board_id,
        &revision,
        &mut result,
    )?;
    Ok(result)
}
fn projection(value: &Value, keys: &[&str]) -> Value {
    let mut map = serde_json::Map::new();
    for key in keys {
        if let Some(value) = value.get(key) {
            map.insert((*key).into(), value.clone());
        }
    }
    Value::Object(map)
}
fn string_projection(value: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|key| {
                value
                    .get(key)
                    .and_then(Value::as_str)
                    .map(|value| ((*key).to_owned(), json!(value)))
            })
            .collect(),
    )
}
fn strip_schema_values(value: &Value, depth: usize) -> Value {
    if depth > 16 {
        return Value::Null;
    }
    match value {
        Value::Array(items) => json!(
            items
                .iter()
                .take(256)
                .map(|item| strip_schema_values(item, depth + 1))
                .collect::<Vec<_>>()
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "default"
                            | "examples"
                            | "example"
                            | "metadata"
                            | "$comment"
                            | "$schema"
                            | "$id"
                    )
                })
                .take(256)
                .map(|(key, value)| (key.clone(), strip_schema_values(value, depth + 1)))
                .collect(),
        ),
        _ => value.clone(),
    }
}
fn document(
    target: Target,
    title: String,
    mut value: Value,
    truncated: bool,
) -> Result<Document, String> {
    value.sort_all_objects();
    let content = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    if content.len() > 1_048_576 {
        return Err("Workspace resource exceeds its size limit.".into());
    }
    Ok(Document {
        target,
        revision: hash(&content),
        title,
        content,
        extra: json!({}),
        truncated,
    })
}
fn docs() -> Result<Vec<Document>, String> {
    let corpus: Value = serde_json::from_str(DOCS).map_err(|e| e.to_string())?;
    corpus["entries"].as_array().ok_or("The bundled docs corpus is invalid.")?.iter().map(|entry|Ok(Document{target:Target{kind:"doc".into(),app_id:None,board_id:None,id:required(entry,"resource_id")?.into()},revision:required(entry,"revision")?.into(),title:required(entry,"title")?.into(),content:required(entry,"content")?.into(),extra:json!({"source_path":entry["source_path"],"start_line":entry["line_start"],"end_line":entry["line_end"]}),truncated:false})).collect()
}
async fn read_document(
    session: &NativePlatformSession,
    target: &Target,
) -> Result<Document, String> {
    if target.kind == "doc" {
        return docs()?
            .into_iter()
            .find(|d| d.target.id == target.id)
            .ok_or_else(|| "Workspace document not found.".into());
    }
    let app = target.app_id.as_deref().ok_or("Missing resource app.")?;
    authorize(session, app)?;
    let local = session.is_local(app).await?;
    match target.kind.as_str() {
        "workflow" => {
            let board = target
                .board_id
                .as_deref()
                .ok_or("Missing resource board.")?;
            workflow_documents(&flow_source(session, app, board).await?, app, board)?
                .into_iter()
                .find(|d| d.target.id == target.id)
                .ok_or_else(|| "Workspace symbol not found.".into())
        }
        "event" => {
            let value = if local {
                serde_json::to_value(
                    events::get_event(
                        session.app_handle.clone(),
                        app.into(),
                        target.id.clone(),
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?
            } else {
                session
                    .request(
                        Method::GET,
                        &format!("apps/{}/events/{}", segment(app), segment(&target.id)),
                        None,
                    )
                    .await?
            };
            if value["id"] != target.id
                || target
                    .board_id
                    .as_ref()
                    .is_some_and(|board| value["board_id"] != *board)
            {
                return Err("The resource identity changed.".into());
            }
            let mut contract = projection(
                &value,
                &[
                    "id",
                    "name",
                    "description",
                    "event_type",
                    "board_id",
                    "board_version",
                    "node_id",
                    "route",
                    "default_page_id",
                    "active",
                    "execution_mode",
                    "exposure",
                ],
            );
            let inputs = value["inputs"].as_array().cloned().unwrap_or_default();
            contract["inputs"] = json!(
                inputs
                    .iter()
                    .take(128)
                    .map(|input| {
                        let mut projected = projection(
                            input,
                            &[
                                "id",
                                "name",
                                "friendly_name",
                                "description",
                                "data_type",
                                "value_type",
                                "optional",
                            ],
                        );
                        if let Some(schema) = input.get("schema") {
                            projected["schema"] = strip_schema_values(schema, 0);
                        }
                        projected
                    })
                    .collect::<Vec<_>>()
            );
            document(
                target.clone(),
                value["name"].as_str().unwrap_or(&target.id).into(),
                contract,
                inputs.len() > 128,
            )
        }
        "page" => {
            let page =
                super::widget::load_page(session, app, &target.id, target.board_id.as_deref())
                    .await?;
            let value = serde_json::to_value(&page).map_err(|e| e.to_string())?;
            let mut contract = projection(
                &value,
                &[
                    "id",
                    "name",
                    "title",
                    "route",
                    "boardId",
                    "onLoadEventId",
                    "onUnloadEventId",
                    "onIntervalEventId",
                    "onIntervalSeconds",
                ],
            );
            contract["components"] = json!(
                page.components
                    .iter()
                    .take(256)
                    .map(|c| json!({"id":c.id,"types":[c.get_component_type_name()]}))
                    .collect::<Vec<_>>()
            );
            let mut bindings = Vec::new();
            let mut visited = 0;
            fn visit(
                value: &Value,
                path: &str,
                depth: usize,
                visited: &mut usize,
                bindings: &mut Vec<Value>,
            ) {
                *visited += 1;
                if *visited > 4096 || depth > 16 {
                    return;
                }
                match value {
                    Value::Array(list) => {
                        for (index, item) in list.iter().take(256).enumerate() {
                            visit(
                                item,
                                &format!("{path}/{index}"),
                                depth + 1,
                                visited,
                                bindings,
                            )
                        }
                    }
                    Value::Object(map) => {
                        if let Some(bound) = map.get("path").and_then(Value::as_str) {
                            bindings.push(json!({"location":path,"binding_path":bound}));
                        }
                        if let Some(action) = map.get("name").and_then(Value::as_str) {
                            if let Some(context) = map.get("context").filter(|v| v.is_object()) {
                                let mut entry = string_projection(
                                    context,
                                    &[
                                        "eventId", "event_id", "boardId", "board_id", "nodeId",
                                        "node_id",
                                    ],
                                );
                                entry["location"] = json!(path);
                                entry["action"] = json!(action);
                                entry["page_action"] = string_projection(
                                    map.get("pageAction").unwrap_or(&Value::Null),
                                    &["actionId", "manifestRevision"],
                                );
                                bindings.push(entry);
                            }
                        }
                        for (key, item) in map.iter().take(128) {
                            if ![
                                "context",
                                "pageAction",
                                "defaultValue",
                                "literalJson",
                                "dataModel",
                                "widgetRefs",
                            ]
                            .contains(&key.as_str())
                            {
                                visit(item, &format!("{path}/{key}"), depth + 1, visited, bindings);
                            }
                        }
                    }
                    _ => {}
                }
            }
            for c in page.components.iter().take(256) {
                visit(&c.component, &c.id, 0, &mut visited, &mut bindings);
            }
            contract["bindings"] = json!(bindings);
            let truncated = page.components.len() > 256 || visited > 4096;
            contract["truncated"] = json!(truncated);
            document(target.clone(), page.name, contract, truncated)
        }
        "table" => {
            let schema = if local {
                serde_json::to_value(
                    tables::db_schema(
                        session.app_handle.clone(),
                        app.into(),
                        target.id.clone(),
                        None,
                        Some(false),
                        None,
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?
            } else {
                session
                    .request(
                        Method::GET,
                        &format!("apps/{}/db/{}/schema", segment(app), segment(&target.id)),
                        None,
                    )
                    .await?
            };
            let fields = schema["fields"]
                .as_array()
                .ok_or("The table schema is unreadable.")?;
            let safe = fields
                .iter()
                .take(256)
                .map(|field| {
                    strip_schema_values(&projection(field, &["name", "data_type", "nullable"]), 0)
                })
                .collect::<Vec<_>>();
            document(
                target.clone(),
                target.id.clone(),
                json!({"table_name":target.id,"fields":safe}),
                fields.len() > 256,
            )
        }
        _ => Err("Unknown workspace resource kind.".into()),
    }
}
async fn inventory(
    session: &NativePlatformSession,
    app: &str,
    kind: &str,
    board: Option<&str>,
) -> Result<Vec<Target>, String> {
    let local = session.is_local(app).await?;
    let values = match kind {
        "workflow" => {
            let app = super::board::load_app(session, app).await?;
            return Ok(app
                .boards
                .into_iter()
                .filter(|id| board.is_none_or(|board| id == board))
                .map(|id| Target {
                    kind: kind.into(),
                    app_id: Some(app.id.clone()),
                    board_id: Some(id.clone()),
                    id,
                })
                .collect());
        }
        "event" => {
            if local {
                serde_json::to_value(
                    events::get_events(session.app_handle.clone(), app.into())
                        .await
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?
            } else {
                session
                    .request(Method::GET, &format!("apps/{}/events", segment(app)), None)
                    .await?
            }
        }
        "page" => json!(super::widget::page_list(session, app).await?),
        "table" => {
            if local {
                json!(
                    tables::db_table_names(session.app_handle.clone(), app.into(), None, None)
                        .await
                        .map_err(|e| e.to_string())?
                )
            } else {
                session
                    .request(Method::GET, &format!("apps/{}/db", segment(app)), None)
                    .await?
            }
        }
        _ => return Err("Unknown workspace kind.".into()),
    };
    Ok(values
        .as_array()
        .ok_or("Invalid workspace inventory.")?
        .iter()
        .filter_map(|value| {
            let id = if kind == "table" {
                value.as_str()
            } else {
                value
                    .get("id")
                    .or_else(|| value.get("pageId"))
                    .and_then(Value::as_str)
            }?;
            let board_id = value
                .get("board_id")
                .or_else(|| value.get("boardId"))
                .and_then(Value::as_str);
            if board.is_some_and(|board| kind != "table" && board_id != Some(board)) {
                return None;
            }
            Some(Target {
                kind: kind.into(),
                app_id: Some(app.into()),
                board_id: board_id.map(str::to_owned),
                id: id.into(),
            })
        })
        .collect::<Vec<_>>())
}

async fn read_symbol(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let Some(target) = args["resource_id"].as_str().and_then(parse_resource) else {
        return Ok(failure(
            "WORKSPACE_REFERENCE_INVALID",
            "Use an exact resource_id and revision returned by search_workspace.",
        ));
    };
    let Some(revision) = args["revision"].as_str().filter(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }) else {
        return Ok(failure(
            "WORKSPACE_REFERENCE_INVALID",
            "Use the exact revision returned by search_workspace.",
        ));
    };
    let offset = match args.get("offset") {
        None => 0,
        Some(value) => match value
            .as_u64()
            .filter(|&n| n <= 9_007_199_254_740_991)
            .and_then(|n| usize::try_from(n).ok())
        {
            Some(n) => n,
            None => {
                return Ok(failure(
                    "WORKSPACE_OFFSET_INVALID",
                    "Use next_offset from the previous read.",
                ));
            }
        },
    };
    let document = match read_document(session, &target).await {
        Ok(document) => document,
        Err(_) => {
            return Ok(failure(
                "WORKSPACE_RESOURCE_UNREADABLE",
                "The exact resource is missing or unreadable. Verify identity and access.",
            ));
        }
    };
    if document.revision != revision {
        return Ok(
            json!({"status":"stale","code":"WORKSPACE_REVISION_CHANGED","message":"The resource changed after discovery. Search again before reusing it."}),
        );
    }
    let Some(byte) = utf16_byte(&document.content, offset) else {
        return Ok(failure(
            "WORKSPACE_OFFSET_INVALID",
            "Use next_offset from the previous read of this revision.",
        ));
    };
    let content = bound_json_text(&document.content[byte..], 16_000);
    let count = content.encode_utf16().count();
    let total = document.content.encode_utf16().count();
    let mut result = json!({"status":"ok","resource_id":resource_id(&target),"revision":revision,"kind":target.kind,"app_id":target.app_id,"board_id":target.board_id,"title":bound_json_text(&document.title,512),"content":content,"content_is_untrusted":true,"offset":offset,"next_offset":(offset+count<total).then_some(offset+count),"content_chars":total,"contract_truncated":document.truncated});
    result
        .as_object_mut()
        .unwrap()
        .extend(document.extra.as_object().cloned().unwrap_or_default());
    session.assert_current_profile().await?;
    Ok(result)
}

fn tokenize(value: &str) -> Vec<String> {
    let originals = value
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut parts = Vec::new();
    for original in &originals {
        let mut previous = None;
        let mut start = 0;
        let mut split = Vec::new();
        for (at, c) in original.char_indices() {
            if previous.is_some_and(|p: char| p.is_ascii_lowercase() || p.is_ascii_digit())
                && c.is_ascii_uppercase()
            {
                split.push(original[start..at].to_owned());
                start = at;
            }
            previous = Some(c);
        }
        if start > 0 {
            split.push(original[start..].to_owned());
            parts.extend(split);
        }
    }
    originals.into_iter().chain(parts).collect()
}
fn normalize(raw: &str) -> Option<String> {
    let stop = "a an the and or for with to of in on at by from as is are was were be been being do does did can could should would will may might how what which where when who why that this these those it its then than while before after please find look looking show get give existing example examples helper helpers function functions workflow workflows flow flows code implementation implements implement use using used keep appropriate new also about into over under";
    let mut term = raw
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase();
    if term.is_empty() || stop.split_whitespace().any(|word| word == term) {
        return None;
    }
    if term.len() >= 5 && term.bytes().all(|c| c.is_ascii_lowercase()) {
        if term.ends_with("ies") {
            term.truncate(term.len() - 3);
            term.push('y');
        } else if ["sses", "shes", "ches", "xes", "zes"]
            .iter()
            .any(|end| term.ends_with(end))
        {
            term.truncate(term.len() - 2);
        } else if term.ends_with('s') && !["ss", "us", "is"].iter().any(|end| term.ends_with(end)) {
            term.pop();
        }
        let strip = if term.len() > 6 && term.ends_with("ing") {
            3
        } else if term.len() > 5 && term.ends_with("ed") {
            2
        } else {
            0
        };
        if strip > 0 {
            term.truncate(term.len() - strip);
            let bytes = term.as_bytes();
            if bytes.len() > 1
                && bytes[bytes.len() - 1] == bytes[bytes.len() - 2]
                && b"bdgmnprt".contains(&bytes[bytes.len() - 1])
            {
                term.pop();
            }
        } else if term.len() > 6 && term.ends_with("ly") {
            term.truncate(term.len() - 2);
        }
        if term.len() > 4 && term.ends_with('e') {
            term.pop();
        }
    }
    Some(term)
}
fn terms(value: &str) -> BTreeSet<String> {
    tokenize(value)
        .iter()
        .filter_map(|s| normalize(s))
        .collect()
}
fn edit_distance(a: &str, b: &str) -> usize {
    let a = a.encode_utf16().collect::<Vec<_>>();
    let b = b.encode_utf16().collect::<Vec<_>>();
    if a.len().abs_diff(b.len()) > 2 {
        return 3;
    }
    let mut previous = (0..=b.len()).collect::<Vec<_>>();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            current.push(
                (previous[j + 1] + 1)
                    .min(current[j] + 1)
                    .min(previous[j] + usize::from(ca != cb)),
            );
        }
        previous = current;
    }
    previous[b.len()]
}
struct IndexedField {
    frequencies: HashMap<String, usize>,
    length: usize,
}
#[derive(Default, Clone)]
struct Match {
    score: f64,
    terms: BTreeSet<String>,
    fields: BTreeSet<&'static str>,
    derived: BTreeSet<String>,
}
fn rank(documents: &[Document], query: &str) -> (Vec<Value>, String) {
    let query = query.trim();
    let exact = documents
        .iter()
        .map(|doc| {
            doc.title.trim().eq_ignore_ascii_case(query)
                || doc.extra["symbol"].as_str().is_some_and(|symbol| {
                    symbol.eq_ignore_ascii_case(query)
                        || symbol
                            .rsplit("::")
                            .next()
                            .is_some_and(|s| s.eq_ignore_ascii_case(query))
                })
        })
        .collect::<Vec<_>>();
    let mut query_terms = terms(query);
    if query_terms.is_empty() && exact.iter().any(|&e| e) {
        query_terms.insert(query.to_lowercase());
    }
    if query_terms.is_empty() {
        return (vec![], "no_meaningful_terms".into());
    }
    let acronyms = tokenize(query)
        .into_iter()
        .filter(|s| {
            s.len() > 1
                && s.as_bytes()[0].is_ascii_uppercase()
                && s.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        })
        .filter_map(|s| normalize(&s))
        .collect::<BTreeSet<_>>();
    let fields = documents
        .iter()
        .map(|doc| {
            [
                doc.title.as_str(),
                doc.extra["symbol"].as_str().unwrap_or(""),
                doc.extra["signature"].as_str().unwrap_or(""),
                doc.content.as_str(),
            ]
            .map(|text| {
                let tokens = tokenize(text);
                let length = tokens.iter().collect::<HashSet<_>>().len();
                let mut frequencies = HashMap::new();
                for term in tokens.iter().filter_map(|t| normalize(t)) {
                    *frequencies.entry(term).or_insert(0) += 1;
                }
                IndexedField {
                    frequencies,
                    length,
                }
            })
        })
        .collect::<Vec<_>>();
    let mut averages = [0.0; 4];
    let mut frequency: HashMap<String, usize> = HashMap::new();
    let mut field_frequency: HashMap<(usize, String), usize> = HashMap::new();
    for fieldset in &fields {
        let mut seen = HashSet::new();
        for (i, field) in fieldset.iter().enumerate() {
            averages[i] += field.length as f64 / documents.len().max(1) as f64;
            for term in field.frequencies.keys() {
                seen.insert(term);
                *field_frequency.entry((i, term.clone())).or_insert(0) += 1;
            }
        }
        for term in seen {
            *frequency.entry(term.clone()).or_insert(0) += 1;
        }
    }
    let evaluate = |fuzzy: bool| -> Vec<Match> {
        fields
            .iter()
            .map(|fieldset| {
                let mut result = Match::default();
                for term in &query_terms {
                    for (i, field) in fieldset.iter().enumerate() {
                        for (held, &tf) in &field.frequencies {
                            let len = held.encode_utf16().count() as f64;
                            let weight = if term == held {
                                1.0
                            } else if held.starts_with(term) {
                                0.375 * len
                                    / (len + 0.3 * (len - term.encode_utf16().count() as f64))
                            } else if fuzzy && term.encode_utf16().count() >= 5 {
                                let distance = edit_distance(term, held);
                                if distance
                                    <= 2.min(
                                        (term.encode_utf16().count() as f64 * 0.2).round() as usize
                                    )
                                {
                                    0.45 * len / (len + distance as f64)
                                } else {
                                    continue;
                                }
                            } else {
                                continue;
                            };
                            let matching =
                                *field_frequency.get(&(i, held.clone())).unwrap_or(&0) as f64;
                            let idf = (1.0
                                + (documents.len() as f64 - matching + 0.5) / (matching + 0.5))
                                .ln();
                            let tf = tf as f64;
                            let bm25 = idf
                                * (0.5
                                    + tf * 2.2
                                        / (tf
                                            + 1.2
                                                * (0.3
                                                    + 0.7 * field.length as f64
                                                        / averages[i].max(1.0))));
                            result.score += weight * [3.0, 4.0, 2.0, 1.0][i] * bm25;
                            result.terms.insert(term.clone());
                            result
                                .fields
                                .insert(["title", "symbol", "signature", "content"][i]);
                            result.derived.insert(held.clone());
                        }
                    }
                }
                result.score *= result.terms.len().max(1) as f64;
                result
            })
            .collect()
    };
    let prefix = evaluate(false);
    let prefix_exists = prefix.iter().any(|r| r.terms.len() == query_terms.len());
    let strict = if prefix_exists {
        prefix.clone()
    } else {
        evaluate(true)
    };
    let strict_exists = strict.iter().any(|r| r.terms.len() == query_terms.len());
    let strict_max = strict
        .iter()
        .filter(|r| r.terms.len() == query_terms.len())
        .map(|r| r.score)
        .fold(1.0, f64::max);
    let identifier = !query.chars().any(char::is_whitespace)
        && (query.contains('_')
            || query.contains("::")
            || query.chars().any(|c| c.is_ascii_digit())
            || query
                .as_bytes()
                .windows(2)
                .any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase()));
    let weights = query_terms
        .iter()
        .map(|term| {
            (
                term.clone(),
                (1.0 + (1.0
                    + documents.len() as f64 / (1.0 + *frequency.get(term).unwrap_or(&0) as f64))
                    .ln())
                .min(4.0),
            )
        })
        .collect::<HashMap<_, _>>();
    let total: f64 = weights.values().sum();
    let mut hits = Vec::new();
    let mut partial = false;
    for (i, doc) in documents.iter().enumerate() {
        let is_strict = strict[i].terms.len() == query_terms.len();
        let mut result = if is_strict {
            strict[i].clone()
        } else if exact[i] {
            Match {
                score: strict_max,
                terms: query_terms.clone(),
                fields: if doc.extra["symbol"].is_string() {
                    BTreeSet::from(["title", "symbol"])
                } else {
                    BTreeSet::from(["title"])
                },
                derived: query_terms.clone(),
            }
        } else if !identifier && query_terms.len() >= 2 {
            prefix[i].clone()
        } else {
            continue;
        };
        let coverage = result
            .terms
            .iter()
            .map(|term| weights.get(term).copied().unwrap_or(0.0))
            .sum::<f64>()
            / total;
        if !exact[i] && acronyms.iter().any(|term| !result.terms.contains(term)) {
            continue;
        }
        if !is_strict && !exact[i] && (result.terms.len() < 2 || coverage < 0.5) {
            continue;
        }
        let quality = if exact[i] {
            "identifier"
        } else if is_strict {
            "strict"
        } else {
            partial = true;
            "partial"
        };
        result.score *= coverage
            * coverage
            * if exact[i] {
                8.0
            } else if is_strict {
                1.25
            } else {
                1.0
            };
        let lower = doc.content.to_lowercase();
        let position = result
            .derived
            .iter()
            .filter_map(|term| lower.find(term))
            .min()
            .unwrap_or(0);
        let mut start = position.saturating_sub(100).min(doc.content.len());
        while !doc.content.is_char_boundary(start) {
            start -= 1;
        }
        let snippet = format!(
            "{}{}",
            if start > 0 { "…" } else { "" },
            bound_json_text(&doc.content[start..], 600)
        );
        let mut hit = json!({"resource_id":resource_id(&doc.target),"revision":doc.revision,"kind":doc.target.kind,"app_id":doc.target.app_id,"board_id":doc.target.board_id,"title":bound_json_text(&doc.title,512),"snippet":snippet,"matched_fields":result.fields,"score":(result.score*1000.0).round()/1000.0,"match_quality":quality,"query_coverage":(coverage*1000.0).round()/1000.0});
        hit.as_object_mut()
            .unwrap()
            .extend(doc.extra.as_object().cloned().unwrap_or_default());
        if let Some(signature) = hit["signature"].as_str() {
            hit["signature"] = json!(bound_json_text(signature, 1024));
        }
        hits.push(hit);
    }
    hits.sort_by(|a, b| {
        b["score"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
            .then_with(|| a["resource_id"].as_str().cmp(&b["resource_id"].as_str()))
    });
    (
        hits,
        if partial {
            if strict_exists { "mixed" } else { "partial" }
        } else if prefix_exists {
            "prefix"
        } else {
            "fuzzy"
        }
        .into(),
    )
}

async fn search(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let query = string(args, "query", "query").unwrap_or("").trim();
    let app = string(args, "app_id", "appId")
        .or_else(|| session.context.as_ref().and_then(|c| c.app_id.as_deref()));
    let board = string(args, "board_id", "boardId");
    let limit = args
        .get("limit")
        .map(|value| value.as_u64().unwrap_or(0))
        .unwrap_or(10) as usize;
    let kinds = match args.get("kinds") {
        None => KINDS.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    };
    if args
        .get("app_id")
        .or_else(|| args.get("appId"))
        .is_some_and(|value| {
            value
                .as_str()
                .is_none_or(|s| s.is_empty() || s.encode_utf16().count() > 256)
        })
        || args
            .get("board_id")
            .or_else(|| args.get("boardId"))
            .is_some_and(|value| {
                value
                    .as_str()
                    .is_none_or(|s| s.is_empty() || s.encode_utf16().count() > 256)
            })
        || args
            .get("kinds")
            .and_then(Value::as_array)
            .is_some_and(|list| list.iter().any(|v| !v.is_string()))
        || query.is_empty()
        || query.encode_utf16().count() > 256
        || limit == 0
        || limit > 20
        || kinds.is_empty()
        || kinds.len() > 5
        || kinds.iter().any(|kind| !KINDS.contains(&kind.as_str()))
        || board.is_some() && app.is_none()
    {
        return Ok(failure(
            "WORKSPACE_QUERY_INVALID",
            "Use a nonempty query up to 256 characters, valid kinds and a limit of 1 to 20. A board filter requires an app.",
        ));
    }
    let kinds = kinds
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let identity = json!([
        session.profile.id,
        session.hub,
        session.subject,
        session
            .profile_app_ids()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        session.context.as_ref().and_then(|c| c.run_id.as_ref()),
        query,
        app,
        board,
        kinds
    ])
    .to_string();
    let (id, snapshot, offset, cache) = if let Some(cursor) = args.get("cursor") {
        let parsed = cursor
            .as_str()
            .and_then(|cursor| cursor.split_once(':'))
            .filter(|(id, offset)| {
                id.len() == 36
                    && uuid::Uuid::parse_str(id).is_ok()
                    && !offset.is_empty()
                    && offset.len() <= 5
                    && offset.bytes().all(|b| b.is_ascii_digit())
            })
            .and_then(|(id, offset)| {
                offset
                    .parse::<usize>()
                    .ok()
                    .map(|offset| (id.to_owned(), offset))
            });
        let Some((id, offset)) = parsed else {
            return Ok(failure(
                "WORKSPACE_CURSOR_INVALID",
                "Use the next_cursor returned by this search.",
            ));
        };
        let retained = {
            let snapshots = SNAPSHOTS.lock().unwrap_or_else(|p| p.into_inner());
            snapshots
                .get(&id)
                .filter(|s| {
                    s.scope == identity
                        && s.created.elapsed() < Duration::from_secs(120)
                        && offset <= s.hits.len()
                })
                .cloned()
        };
        let Some(snapshot) = retained else {
            return Ok(failure(
                "WORKSPACE_CURSOR_INVALID",
                "The cursor expired or its query/scope changed. Search again without a cursor.",
            ));
        };
        (id, snapshot, offset, "cursor")
    } else {
        let mut documents = Vec::new();
        let mut issues = Vec::new();
        let mut app_ids = if kinds.iter().all(|k| k == "doc") {
            Vec::new()
        } else if let Some(app) = app {
            authorize(session, app)?;
            vec![app.to_owned()]
        } else {
            let mut ids = session.profile_app_ids().into_iter().collect::<Vec<_>>();
            ids.sort();
            ids
        };
        if app_ids.len() > 4 {
            issues.push(json!({"code":"APP_LIMIT"}));
            app_ids.truncate(4);
        }
        if kinds.iter().any(|kind| kind == "doc") {
            documents.extend(docs()?);
        }
        for app in &app_ids {
            for kind in kinds.iter().filter(|kind| kind.as_str() != "doc") {
                let listed = inventory(session, app, kind, board).await;
                let mut targets = match listed {
                    Ok(targets) => targets,
                    Err(_) => {
                        issues
                            .push(json!({"code":"INVENTORY_UNREADABLE","kind":kind,"app_id":app}));
                        continue;
                    }
                };
                targets.sort_by(|a, b| a.id.cmp(&b.id));
                if targets.len() > 16 {
                    issues.push(json!({"code":"RESOURCE_LIMIT","kind":kind,"app_id":app}));
                    targets.truncate(16);
                }
                for target in targets {
                    let loaded = if kind == "workflow" {
                        match flow_source(
                            session,
                            app,
                            target.board_id.as_deref().unwrap_or(&target.id),
                        )
                        .await
                        {
                            Ok(source) => workflow_documents(
                                &source,
                                app,
                                target.board_id.as_deref().unwrap_or(&target.id),
                            ),
                            Err(error) => Err(error),
                        }
                    } else {
                        read_document(session, &target).await.map(|doc| vec![doc])
                    };
                    match loaded{Ok(loaded)=>documents.extend(loaded),Err(_)=>issues.push(json!({"code":"RESOURCE_UNREADABLE","kind":kind,"app_id":app,"resource":target.id}))}
                }
            }
        }
        let mut size = 0;
        documents.retain(|doc| {
            size += doc.content.len();
            size <= 8_388_608
        });
        if size > 8_388_608 || documents.len() > 2000 {
            issues.push(json!({"code":"CORPUS_LIMIT"}));
            documents.truncate(2000);
        }
        for doc in documents.iter().filter(|doc| doc.truncated) {
            issues.push(json!({"code":"CONTRACT_TRUNCATED","kind":doc.target.kind,"app_id":doc.target.app_id,"resource":doc.target.id}));
        }
        let coverage = json!({"complete":issues.is_empty(),"indexed_resources":documents.len(),"app_ids":app_ids,"kinds":kinds,"issues":issues.into_iter().take(64).collect::<Vec<_>>()});
        let (hits, mode) = rank(&documents, query);
        let id = uuid::Uuid::new_v4().to_string();
        let snapshot = Snapshot {
            created: Instant::now(),
            observed: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            scope: identity,
            hits,
            coverage,
            mode,
        };
        {
            let mut snapshots = SNAPSHOTS.lock().unwrap_or_else(|p| p.into_inner());
            snapshots.retain(|_, s| s.created.elapsed() < Duration::from_secs(120));
            if snapshots.len() >= 128 {
                if let Some(oldest) = snapshots
                    .iter()
                    .min_by_key(|(_, s)| s.created)
                    .map(|(id, _)| id.clone())
                {
                    snapshots.remove(&oldest);
                }
            }
            snapshots.insert(id.clone(), snapshot.clone());
        }
        (id, snapshot, 0, "miss")
    };
    session.assert_current_profile().await?;
    let mut hits = Vec::new();
    for hit in snapshot.hits.iter().skip(offset).take(limit) {
        if serde_json::to_vec(&json!({"coverage":snapshot.coverage,"hits":[&hits,hit]}))
            .map_err(|e| e.to_string())?
            .len()
            > 22_000
        {
            break;
        }
        hits.push(hit.clone());
    }
    if hits.is_empty() && offset < snapshot.hits.len() {
        return Ok(failure(
            "WORKSPACE_REPLY_TOO_LARGE",
            "A result exceeds the bounded reply. Refine the query or scope.",
        ));
    }
    if cache == "cursor" {
        for hit in &hits {
            let target = parse_resource(hit["resource_id"].as_str().unwrap_or(""))
                .ok_or("Invalid retained workspace resource.")?;
            if read_document(session, &target)
                .await
                .map(|doc| doc.revision != hit["revision"].as_str().unwrap_or(""))
                .unwrap_or(true)
            {
                return Ok(
                    json!({"status":"stale","code":"WORKSPACE_SNAPSHOT_CHANGED","message":"A result changed or became unreadable. Search again without a cursor."}),
                );
            }
        }
    }
    session.assert_current_profile().await?;
    let next = (offset + hits.len() < snapshot.hits.len())
        .then(|| format!("{id}:{}", offset + hits.len()));
    Ok(
        json!({"status":"ok","query":query,"match_mode":snapshot.mode,"matched_resources":snapshot.hits.len(),"coverage":snapshot.coverage,"freshness":{"coverage_basis":"observed_snapshot","corpus_observed_at":snapshot.observed,"corpus_age_ms":snapshot.created.elapsed().as_millis(),"cache":cache,"inventory_revalidated":false,"hits_revalidated":if cache=="cursor"{hits.len()}else{0},"validation_reads":if cache=="cursor"{hits.len()}else{0}},"content_is_untrusted":true,"hits":hits,"next_cursor":next}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ranked_document(id: &str, symbol: &str, content: &str) -> Document {
        Document {
            target: Target {
                kind: "workflow".into(),
                app_id: Some("app".into()),
                board_id: Some("board".into()),
                id: id.into(),
            },
            revision: "a".repeat(64),
            title: symbol.into(),
            content: content.into(),
            extra: json!({"symbol":symbol,"signature":format!("function {symbol}()")}),
            truncated: false,
        }
    }
    #[test]
    fn search_matches_web_relevance_guards() {
        let shipping = ranked_document(
            "shipping",
            "dispatch",
            "orders are shipped using carrier labels",
        );
        let (hits, _) = rank(&[shipping.clone()], "How does it track shipped orders?");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["match_quality"], "partial");
        assert!(rank(&[shipping], "SAML shipped orders").0.is_empty());
        let lease = ranked_document("lease", "lease", "renew the database connection");
        assert!(
            rank(&[lease.clone()], "Redis distributed lease renewal")
                .0
                .is_empty()
        );
        assert!(rank(&[lease], "database satellite").0.is_empty());
        assert_eq!(
            rank(&[ranked_document("getter", "get", "return receipt")], "get")
                .0
                .len(),
            1
        );
        let (hits, mode) = rank(
            &[ranked_document(
                "amount",
                "calculateAmount",
                "return amount",
            )],
            "calcluateAmount",
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(mode, "fuzzy");
        assert_eq!(
            rank(
                &[ranked_document(
                    "audit",
                    "audit",
                    "store records and process updates"
                )],
                "stored records processed updates"
            )
            .0
            .len(),
            1
        );
    }
    #[test]
    fn exact_resource_ids_round_trip() {
        let target = Target {
            kind: "workflow".into(),
            app_id: Some("a".into()),
            board_id: Some("b".into()),
            id: "module::symbol".into(),
        };
        let id = resource_id(&target);
        assert_eq!(parse_resource(&id).unwrap().id, "module::symbol");
        assert!(
            parse_resource("workspace:v1:%5B%22doc%22,%22foreign%22,null,%22id%22%5D").is_none()
        );
    }
    #[test]
    fn utf16_offsets_do_not_split_unicode() {
        assert_eq!(utf16_byte("a🦀b", 1), Some(1));
        assert_eq!(utf16_byte("a🦀b", 2), None);
        assert_eq!(utf16_byte("a🦀b", 3), Some(5));
    }
    #[test]
    fn source_window_centers_a_long_locator() {
        let source = format!(
            "{}\nstable_identifier\n{}",
            "a\n".repeat(40_000),
            "z\n".repeat(40_000)
        );
        let result = bound_source(&source, Some("Find stable_identifier"));
        assert_eq!(result["locator_matched"], true);
        assert!(
            result["source"]
                .as_str()
                .unwrap()
                .contains("stable_identifier")
        );
        assert_eq!(result["truncated"], true);
    }
    #[test]
    fn symbol_extraction_ignores_braces_in_literals() {
        let source = "function one() {\n  const x = \"{ not a body }\"\n}\nfunction two() {}\n";
        let docs = workflow_documents(source, "app", "board").unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].target.id, "one");
        assert!(!docs[0].content.contains("function two"));
    }
    #[test]
    fn discovery_projection_does_not_retain_defaults() {
        let schema = json!({"type":"object","properties":{"secret":{"type":"string","default":"token","examples":["private"]}},"metadata":{"sensitive":"value"}});
        let safe = strip_schema_values(&schema, 0).to_string();
        assert!(!safe.contains("token"));
        assert!(!safe.contains("private"));
        assert!(!safe.contains("sensitive"));
    }
}
