//! FlowScript names for catalog nodes: `namespace::alias` derivation and the collision contract.
//!
//! A node's FlowScript name is presentation only — the immutable `node_type` stays the identity
//! and the legacy flat spelling ([`legacy_display`]) is derived from it forever. Explicit
//! `namespace`/`alias` fields on a node win; everything else falls back to the derivation here,
//! which is what third-party/WASM nodes that never declare names get.
//!
//! Derivation: the longest category-path prefix found in [`NAMESPACES`] names the namespace
//! (`Utils/String` → `string`, `Web/API` → `http`). Unless the row is `flatten`ed, the remaining
//! sub-category segments are appended as nested namespace segments (`Utils/Faker/Name` →
//! `faker.name`). The alias is the node type minus the leading segments that repeat the
//! namespace's own tokens (matched category tokens, consumed prefixes, the namespace name and —
//! only when not flattened — the appended sub-category tokens), camelCased; the last segment is
//! never stripped (`string_trim` → `trim`, `child` → `child`).
//!
//! Resolution order ([`effective_names`]): explicit `Node` fields (every first-party node has
//! them since the bake-in), then the per-node [`NAME_OVERRIDES`] residue table, then the
//! derivation above. The receiver follows the same order, falling back to
//! [`default_receiver_pin`].
//!
//! The collision contract ([`check_names`]) keeps one case-insensitive key space across flat
//! names, aliases, qualified names and rename keys: every key maps to exactly one node type,
//! none is a keyword. Namespace segments are not keys: `::` paths and members are syntactically
//! disjoint (`set::insert(…)` vs `map::set(…)`), so a segment may repeat a member name.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::text::{is_valid_identifier, to_camel_case};

/// How one category-path prefix maps onto a FlowScript namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamespaceSpec {
    /// The namespace name (single segment; nesting comes from unflattened sub-categories).
    pub namespace: &'static str,
    /// Extra leading node-type segments that repeat the category but are not in its tokens
    /// (`utils` for `Utils/Hash` nodes named `utils_hash_*`, `a2ui` for `UI`).
    pub consumed_prefixes: &'static [&'static str],
    /// When set, sub-categories below the prefix neither nest the namespace nor strip tokens
    /// (`Web/API/Response` stays `http`, `http_response_to_text` → `http::responseToText`).
    pub flatten: bool,
}

const fn spec(
    namespace: &'static str,
    consumed_prefixes: &'static [&'static str],
    flatten: bool,
) -> NamespaceSpec {
    NamespaceSpec {
        namespace,
        consumed_prefixes,
        flatten,
    }
}

/// Category-path prefix → namespace table. Lookup takes the longest matching prefix.
///
/// A dotted `namespace` nests under an existing root without relying on sub-category
/// derivation (`db.graph`, `github.copilot`). Roots are chosen so that common pin/binding
/// names (`response`, `session`, `vector`, `markdown`, `selector`) never become a path root,
/// because lower renames a binding that spells a root used by a static call in the same text.
pub const NAMESPACES: &[(&str, NamespaceSpec)] = &[
    // Value types
    ("Utils/String", spec("string", &[], true)),
    ("Utils/Array", spec("array", &[], true)),
    ("Utils/Bool", spec("bool", &[], true)),
    ("Utils/Set", spec("set", &[], true)),
    ("Utils/Map", spec("map", &[], true)),
    ("Utils/Bytes", spec("bytes", &[], true)),
    ("Utils/DateTime", spec("datetime", &["utils"], true)),
    ("Math/Int", spec("int", &["math"], true)),
    ("Math/Float", spec("float", &["math"], true)),
    ("Math", spec("math", &[], true)),
    (
        "Utils/Math/Vector",
        spec("math.vector", &["utils", "float"], true),
    ),
    ("Structs", spec("struct", &[], true)),
    // Utilities
    ("Utils/Hash", spec("hash", &["utils"], true)),
    ("Utils/Crypto", spec("crypto", &["utils"], true)),
    ("Utils/Encoding", spec("encoding", &["utils"], true)),
    ("Utils/Types", spec("types", &["utils"], true)),
    ("Utils/Conversions", spec("json", &["val", "utils"], true)),
    ("Utils/JSON", spec("json", &["utils"], true)),
    ("Utils/Markdown", spec("md", &["utils"], true)),
    ("Utils/Format", spec("fmt", &["format"], true)),
    ("Utils/Random", spec("random", &["utils"], true)),
    ("Utils/CSV", spec("files", &["utils"], true)),
    ("Utils/User", spec("user", &["utils"], true)),
    ("Utils/Faker", spec("faker", &["utils"], false)),
    ("Utils/Execution", spec("execution", &["utils"], true)),
    ("Utils", spec("utils", &[], false)),
    // Runtime
    ("Control", spec("control", &[], true)),
    ("Variable", spec("variable", &[], true)),
    ("Logging", spec("log", &[], true)),
    (
        "Events/Chat",
        spec("chat", &["events", "interaction"], true),
    ),
    ("Events/Remote", spec("remote", &[], true)),
    ("Events", spec("events", &[], true)),
    ("Notifications", spec("notify", &[], true)),
    ("UI", spec("ui", &["a2ui"], true)),
    // Web
    ("Web/API", spec("http", &["web"], true)),
    ("Web/Auth", spec("auth", &["web"], true)),
    ("Web/Geo/H3", spec("h3", &["web", "geo"], true)),
    ("Web/Geo", spec("geo", &["web"], true)),
    ("Web/MCP", spec("mcp", &["web"], true)),
    ("Web/REST", spec("rest", &["web"], true)),
    ("Web/MQTT", spec("mqtt", &["web"], true)),
    ("Web/TCP", spec("tcp", &["web"], true)),
    ("Web/UDP", spec("udp", &["web"], true)),
    ("Web/WebSocket", spec("websocket", &["web"], true)),
    ("Web/TLS", spec("tls", &["web"], true)),
    ("Web/Camera", spec("camera", &["web"], true)),
    ("Web", spec("web", &[], true)),
    // Data
    ("Data/Files/Path", spec("path", &["data", "files"], true)),
    (
        "Data/Files",
        spec("files", &["data", "path", "storage", "external"], true),
    ),
    ("Data/DataFusion", spec("df", &["data"], true)),
    ("Data/Database/Graph", spec("db.graph", &["data"], true)),
    ("Data/Database", spec("db", &["data"], true)),
    ("Data/Cache", spec("data.cache", &[], true)),
    ("Data/GitHub", spec("github", &["data"], true)),
    (
        "Data/Atlassian/Jira",
        spec("jira", &["data", "atlassian"], true),
    ),
    (
        "Data/Atlassian/Confluence",
        spec("confluence", &["data", "atlassian"], true),
    ),
    ("Data/Atlassian", spec("atlassian", &["data"], true)),
    ("Data/Notion", spec("notion", &["data"], true)),
    ("Data/LinkedIn", spec("linkedin", &["data"], true)),
    (
        "Data/Microsoft/To Do",
        spec("microsoft.todo", &["data"], true),
    ),
    ("Data/Microsoft", spec("microsoft", &["data"], false)),
    ("Data/Google", spec("google", &["data"], false)),
    ("Data/Databricks", spec("databricks", &["data"], true)),
    (
        "Data/Excel",
        spec("excel", &["data", "files", "spreadsheet", "tables"], true),
    ),
    ("Data Studio", spec("ontology", &[], true)),
    ("Data", spec("data", &[], true)),
    // AI
    (
        "AI/Generative/History",
        spec("history", &["ai", "generative"], true),
    ),
    (
        "AI/Generative/Provider",
        spec("ai.provider", &["ai", "generative", "build"], true),
    ),
    (
        "AI/Generative/Preferences",
        spec("ai.preferences", &["ai", "generative"], true),
    ),
    (
        "AI/Generative/Response",
        spec("ai.response", &["ai", "generative", "llm"], true),
    ),
    (
        "AI/Generative/Audio",
        spec("ai.audio", &["ai", "generative"], false),
    ),
    (
        "AI/Generative/Image",
        spec("ai.image", &["ai", "generative"], false),
    ),
    (
        "AI/Generative/Video/Provider",
        spec("ai.video.provider", &["ai", "generative", "build"], true),
    ),
    (
        "AI/Generative/Video",
        spec("ai.video", &["ai", "generative"], false),
    ),
    (
        "AI/Generative",
        spec("ai", &["ai", "generative", "llm"], true),
    ),
    ("AI/Embedding", spec("ai.embedding", &["ai"], true)),
    ("AI/Memory", spec("ai.memory", &["ai"], true)),
    ("AI/Processing", spec("ai.processing", &["ai"], true)),
    ("AI/Preprocessing", spec("ai.processing", &["ai"], true)),
    (
        "Processing/Privacy",
        spec("ai.processing", &["processing"], true),
    ),
    (
        "AI/GitHub/Copilot",
        spec("github.copilot", &["ai", "copilot"], true),
    ),
    ("AI/Agents", spec("agent", &["ai"], true)),
    ("AI/ML/ONNX", spec("onnx", &["ai", "ml"], true)),
    ("AI/ML", spec("ml", &["ai", "tuning"], true)),
    ("AI", spec("ai", &[], false)),
    ("Bit", spec("ai", &[], true)),
    // Automation
    ("Automation/RPA", spec("rpa", &["automation"], true)),
    ("Automation/Browser", spec("browser", &["automation"], true)),
    (
        "Automation/Computer",
        spec("computer", &["automation"], true),
    ),
    (
        "Automation/Fingerprint",
        spec("automation.fingerprint", &[], true),
    ),
    (
        "Automation/Selector",
        spec("automation.selector", &[], true),
    ),
    ("Automation/Vision", spec("automation.vision", &[], true)),
    ("Automation/LLM", spec("automation.llm", &[], true)),
    ("Automation", spec("automation", &[], true)),
    // Documents and media
    ("Document/DOCX", spec("docx", &["document"], true)),
    ("Document/PDF", spec("pdf", &["document"], true)),
    ("Document/PPTX", spec("pptx", &["document"], true)),
    ("Document", spec("document", &[], false)),
    ("Image/PDF", spec("pdf", &["image"], true)),
    ("Image", spec("image", &["video"], true)),
    ("Video", spec("video", &[], true)),
    ("Audio", spec("audio", &["video"], true)),
    ("Subtitles", spec("video", &["video"], true)),
    ("Diagnostics", spec("video", &["video"], true)),
    ("Streaming", spec("video", &["video"], true)),
    (
        "Email/Access",
        spec("email", &["mail", "imap", "inbox"], true),
    ),
    ("Email/IMAP", spec("imap", &["email", "mail"], true)),
    ("Email/SMTP", spec("smtp", &["email"], true)),
    ("Email", spec("email", &[], false)),
];

/// Value-type namespaces and the pin type that makes a node in them a method of that value:
/// `(namespace, data_type, value_type)` in the `Debug` spelling of core's `VariableType` /
/// `ValueType`. `*` matches any data type (containers are classed by their container). Lookup
/// takes the first matching row, so byte buffers (`Byte` pins are always `Array` in the
/// catalog) class as `bytes` before the generic `array` row claims them.
pub const VALUE_TYPE_NAMESPACES: &[(&str, &str, &str)] = &[
    ("string", "String", "Normal"),
    ("int", "Integer", "Normal"),
    ("float", "Float", "Normal"),
    ("bool", "Boolean", "Normal"),
    ("bytes", "Byte", "Array"),
    ("array", "*", "Array"),
    ("map", "*", "HashMap"),
    ("set", "*", "HashSet"),
    ("struct", "Struct", "Normal"),
    // A SCALAR byte is its own class. Sharing the `bytes` class with `Byte/Array` meant a single
    // byte dispatched to buffer methods (`sentinel.toHex()`) and only failed later at connection
    // validation with a pin-type error, instead of "no method `toHex` on `byte`" up front. No
    // catalog node takes a scalar-byte receiver, so nothing resolves through this today.
    ("byte", "Byte", "Normal"),
    ("path", "PathBuf", "Normal"),
    ("datetime", "Date", "Normal"),
];

/// Reserved words a FlowScript identifier can never be.
pub const KEYWORDS: &[&str] = &[
    "function",
    "const",
    "let",
    "for",
    "of",
    "if",
    "else",
    "while",
    "return",
    "true",
    "false",
    "null",
    "interface",
    "use",
    "as",
];

/// Whether `ident` is a FlowScript keyword (case-insensitive).
pub fn is_keyword(ident: &str) -> bool {
    KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(ident))
}

/// The legacy flat spelling of a node type (`string_trim` → `stringTrim`). Accepted forever.
pub fn legacy_display(node_type: &str) -> String {
    to_camel_case(node_type)
}

/// Join a (possibly dotted) namespace and an alias into the static call spelling
/// (`utils.markdown` + `mdHtmlToMd` → `utils::markdown::mdHtmlToMd`).
pub fn qualified_name(namespace: &str, alias: &str) -> String {
    let mut out = String::with_capacity(namespace.len() + alias.len() + 2);
    for segment in namespace_segments(namespace) {
        out.push_str(segment);
        out.push_str("::");
    }
    out.push_str(alias);
    out
}

/// The segments of a dotted namespace path, skipping empties.
pub fn namespace_segments(namespace: &str) -> impl Iterator<Item = &str> {
    namespace
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The value-type namespace a receiver pin of this type belongs to (its method *class*), or
/// `None` for types that have no value-type namespace (`Generic`, `Execution`).
pub fn receiver_class(data_type: &str, value_type: &str) -> Option<&'static str> {
    VALUE_TYPE_NAMESPACES
        .iter()
        .find(|(_, dt, vt)| *vt == value_type && (*dt == "*" || *dt == data_type))
        .map(|(namespace, _, _)| *namespace)
}

/// Whether a pin of `(data_type, value_type)` is the namespace's own value type, i.e. whether
/// a node in `namespace` (or nested below it) takes that pin as its default method receiver.
pub fn namespace_accepts_receiver(namespace: &str, data_type: &str, value_type: &str) -> bool {
    let Some(root) = namespace_segments(namespace).next() else {
        return false;
    };
    receiver_class(data_type, value_type).is_some_and(|class| class == root)
}

/// The root `title` of a JSON Schema: the method class identity of a titled struct receiver.
pub fn schema_title(schema: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(schema)
        .ok()?
        .get("title")?
        .as_str()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
}

/// The method class of a receiver pin: its value-type namespace, or the schema title for a
/// titled struct. `None` for types without a class (`Generic`, the universal receiver, and
/// `Execution`).
pub fn receiver_class_of(
    data_type: &str,
    value_type: &str,
    schema: Option<&str>,
) -> Option<String> {
    let class = receiver_class(data_type, value_type)?;
    if class == "struct"
        && let Some(title) = schema.and_then(schema_title)
    {
        return Some(title);
    }
    Some(class.to_string())
}

/// The default method-receiver rule shared by core's `Node` and `NodeMetadata`. `inputs` are
/// `(name, data_type, value_type)` in pin order (execution pins are skipped): the first data
/// input is the receiver iff its type is the namespace's own value type.
pub fn default_receiver_pin<'a>(
    namespace: &str,
    inputs: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> Option<String> {
    let (name, data_type, value_type) = inputs
        .into_iter()
        .find(|(_, data_type, _)| *data_type != "Execution")?;
    namespace_accepts_receiver(namespace, data_type, value_type).then(|| name.to_string())
}

/// The effective receiver pin of a node: an explicit `receiver` field wins (`Some("")` opts out
/// of the method form), otherwise [`default_receiver_pin`] decides.
pub fn effective_receiver_pin<'a>(
    explicit: Option<&str>,
    namespace: &str,
    inputs: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> Option<String> {
    match explicit {
        Some("") => None,
        Some(pin) => Some(pin.to_string()),
        None => default_receiver_pin(namespace, inputs),
    }
}

/// One [`NAME_OVERRIDES`] row: `(node_type, namespace, alias, receiver)`.
pub type NameOverride = (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
);

/// Per-node `(node_type, namespace, alias, receiver)` rows for first-party nodes that cannot
/// carry explicit `set_flowscript_name(…)` / `set_receiver(…)` calls in their source
/// (`Some("")` as the receiver makes the node static-only; `None` keeps the table derivation /
/// default rule for that field).
///
/// Empty since the bake-in (2026-08-23): every catalog node declares its names explicitly and
/// `lint_catalog::flowscript_names_are_explicit_on_first_party_nodes` requires that, treating
/// this table as the only allowlist. A row here is the staging step for a node whose
/// `Node::new` id is not a literal; it is removed again once the source carries the calls.
/// Explicit `Node` fields always win over this table. Third-party/WASM nodes are never listed.
pub const NAME_OVERRIDES: &[NameOverride] = &[];

fn override_row<'a>(overrides: &'a [NameOverride], node_type: &str) -> Option<&'a NameOverride> {
    overrides.iter().find(|(name, ..)| *name == node_type)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// The explicit naming fields of a node (`Node.namespace/alias/receiver` or their metadata
/// mirror). Empty `namespace`/`alias` count as unset; an empty `receiver` is the static-only
/// opt-out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NameFields<'a> {
    pub namespace: Option<&'a str>,
    pub alias: Option<&'a str>,
    pub receiver: Option<&'a str>,
}

/// A node's effective FlowScript names: explicit fields, else [`NAME_OVERRIDES`], else the
/// table derivation (and the default receiver rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveNames {
    pub namespace: String,
    pub alias: String,
    /// Receiver pin in method form; `None` when the node is static-only.
    pub receiver: Option<String>,
}

/// The effective `(namespace, alias)` of a node without looking at its pins.
pub fn effective_spelling(
    node_type: &str,
    category: &str,
    fields: NameFields<'_>,
) -> (String, String) {
    spelling_with(NAME_OVERRIDES, node_type, category, fields)
}

fn spelling_with(
    overrides: &[NameOverride],
    node_type: &str,
    category: &str,
    fields: NameFields<'_>,
) -> (String, String) {
    let row = override_row(overrides, node_type);
    let namespace = non_empty(fields.namespace)
        .or_else(|| row.and_then(|(_, namespace, _, _)| non_empty(*namespace)))
        .map(str::to_string)
        .unwrap_or_else(|| derive_namespace(category));
    let alias = non_empty(fields.alias)
        .or_else(|| row.and_then(|(_, _, alias, _)| non_empty(*alias)))
        .map(str::to_string)
        .unwrap_or_else(|| derive_alias(node_type, category));
    (namespace, alias)
}

/// The one source of truth for a node's FlowScript names, shared by core's `Node` accessors
/// and the reconcile catalog index. `inputs` are `(name, data_type, value_type)` in pin order
/// (execution pins are skipped) and only matter for the default receiver rule.
pub fn effective_names<'a>(
    node_type: &str,
    category: &str,
    fields: NameFields<'_>,
    inputs: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> EffectiveNames {
    names_with(NAME_OVERRIDES, node_type, category, fields, inputs)
}

fn names_with<'a>(
    overrides: &[NameOverride],
    node_type: &str,
    category: &str,
    fields: NameFields<'_>,
    inputs: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> EffectiveNames {
    let (namespace, alias) = spelling_with(overrides, node_type, category, fields);
    let explicit_receiver = fields
        .receiver
        .or_else(|| override_row(overrides, node_type).and_then(|(_, _, _, receiver)| *receiver));
    let receiver = effective_receiver_pin(explicit_receiver, &namespace, inputs);
    EffectiveNames {
        namespace,
        alias,
        receiver,
    }
}

/// Whether `name` is one of the value-type namespaces (`string`, `int`, `array`, …) — the
/// classes whose method tables come from pin types rather than schema titles.
pub fn is_value_type_namespace(name: &str) -> bool {
    VALUE_TYPE_NAMESPACES
        .iter()
        .any(|(namespace, _, _)| namespace.eq_ignore_ascii_case(name))
}

fn category_segments(category: &str) -> Vec<&str> {
    category
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Lowercase alphanumeric runs of a category segment or namespace name.
fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
}

/// camelCase a category segment for use as a namespace segment. Unlike [`to_camel_case`] the
/// whole first word is lowercased, so acronym segments (`DOCX`, `ONNX`, `H3`) read as
/// `docx`/`onnx`/`h3` rather than `dOCX`.
fn segment_name(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for (i, word) in segment
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .enumerate()
    {
        if i == 0 {
            out.push_str(&word.to_lowercase());
        } else {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        }
    }
    if out.chars().next().is_some_and(|c| c.is_numeric()) {
        out.insert(0, '_');
    }
    out
}

/// Longest [`NAMESPACES`] prefix of `segments`: the number of segments it covers and its spec.
fn longest_match(segments: &[&str]) -> Option<(usize, &'static NamespaceSpec)> {
    (1..=segments.len()).rev().find_map(|len| {
        let key = segments[..len].join("/");
        NAMESPACES
            .iter()
            .find(|(prefix, _)| *prefix == key)
            .map(|(_, spec)| (len, spec))
    })
}

/// The FlowScript namespace for a catalog category path (dotted when nested).
pub fn derive_namespace(category: &str) -> String {
    let segments = category_segments(category);
    let path: Vec<String> = match longest_match(&segments) {
        Some((matched, spec)) => {
            let mut path = vec![spec.namespace.to_string()];
            if !spec.flatten {
                path.extend(segments[matched..].iter().map(|s| segment_name(s)));
            }
            path
        }
        None => segments.iter().map(|s| segment_name(s)).collect(),
    };
    path.join(".")
}

/// The FlowScript member name of `node_type` inside the namespace derived from `category`.
pub fn derive_alias(node_type: &str, category: &str) -> String {
    let segments = category_segments(category);
    let mut strip: BTreeSet<String> = BTreeSet::new();
    match longest_match(&segments) {
        Some((matched, spec)) => {
            strip.extend(segments[..matched].iter().flat_map(|s| tokens(s)));
            strip.extend(spec.consumed_prefixes.iter().map(|p| p.to_lowercase()));
            strip.extend(tokens(spec.namespace));
            if !spec.flatten {
                strip.extend(segments[matched..].iter().flat_map(|s| tokens(s)));
            }
        }
        None => strip.extend(segments.iter().flat_map(|s| tokens(s))),
    }

    let parts: Vec<&str> = node_type
        .split(|c: char| !c.is_alphanumeric())
        .filter(|p| !p.is_empty())
        .collect();
    let mut start = 0;
    while start + 1 < parts.len() && strip.contains(&parts[start].to_lowercase()) {
        start += 1;
    }
    to_camel_case(&parts[start..].join("_"))
}

/// One node's effective FlowScript names, as seen by the collision checker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameEntry {
    pub node_type: String,
    /// Legacy flat spelling ([`legacy_display`]).
    pub flat: String,
    pub namespace: String,
    pub alias: String,
    /// Method class of the receiver pin when the node is callable in method form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_class: Option<String>,
}

impl NameEntry {
    /// Build an entry from the derived defaults (no explicit fields, no receiver).
    pub fn derived(node_type: &str, category: &str) -> Self {
        Self {
            node_type: node_type.to_string(),
            flat: legacy_display(node_type),
            namespace: derive_namespace(category),
            alias: derive_alias(node_type, category),
            receiver_class: None,
        }
    }

    pub fn qualified(&self) -> String {
        qualified_name(&self.namespace, &self.alias)
    }
}

/// One node's complete FlowScript naming record, as written to `flow.d/names.json` for review
/// and consumed by editor tooling (completion after `x.` is `methods[class]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeNames {
    /// `namespace::alias`.
    pub qualified: String,
    pub namespace: String,
    pub alias: String,
    /// Legacy flat spelling.
    pub flat: String,
    /// Receiver pin name in method form, `None` when static only.
    pub receiver: Option<String>,
    /// Method class of the receiver (`string`, `array`, a schema title), `None` when static only.
    pub class: Option<String>,
    pub category: String,
}

impl NodeNames {
    pub fn entry(&self, node_type: &str) -> NameEntry {
        NameEntry {
            node_type: node_type.to_string(),
            flat: self.flat.clone(),
            namespace: self.namespace.clone(),
            alias: self.alias.clone(),
            receiver_class: self.class.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollisionKind {
    /// Rule 1: two nodes share `(namespace, alias)`.
    DuplicateQualified,
    /// Rule 2: an alias equals another node's flat name or raw node type.
    AliasShadowsFlat,
    /// Rule 4: a `namespace::alias` is itself a namespace path (or a prefix of one).
    QualifiedIsNamespace,
    /// Rule 5: a rename key equals a live flat name, alias or qualified name.
    RenameShadowsLive,
    /// Rule 6: an alias or namespace segment is not a valid identifier.
    InvalidIdentifier,
    /// Rule 6: an alias or namespace segment is a keyword.
    Keyword,
    /// Two nodes are the same method `(class, alias)`.
    DuplicateMethod,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameCollision {
    /// The colliding key (lowercased spelling as it was compared).
    pub key: String,
    pub kind: CollisionKind,
    /// Every node type involved, sorted and deduplicated.
    pub node_types: Vec<String>,
}

impl std::fmt::Display for NameCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} `{}`: {}",
            self.kind,
            self.key,
            self.node_types.join(", ")
        )
    }
}

type Owners = BTreeMap<String, BTreeSet<String>>;

fn add(owners: &mut Owners, key: String, node_type: &str) {
    owners
        .entry(key.to_lowercase())
        .or_default()
        .insert(node_type.to_string());
}

fn others<'a>(owners: &'a Owners, key: &str, except: &str) -> impl Iterator<Item = &'a String> {
    owners
        .get(key)
        .into_iter()
        .flatten()
        .filter(move |n| n.as_str() != except)
}

/// Check the naming contract over one catalog. Entries are one per node type (or one per placed
/// instance — same-type duplicates are never collisions). `renames` are `(key, node_type)`
/// pairs of deprecated spellings that must stay resolvable.
pub fn check_names(entries: &[NameEntry], renames: &[(&str, &str)]) -> Vec<NameCollision> {
    let mut flats = Owners::new();
    let mut raws = Owners::new();
    let mut aliases = Owners::new();
    let mut qualified = Owners::new();
    let mut namespaces = Owners::new();
    let mut methods = Owners::new();
    for e in entries {
        add(&mut flats, e.flat.clone(), &e.node_type);
        add(&mut raws, e.node_type.clone(), &e.node_type);
        add(&mut aliases, e.alias.clone(), &e.node_type);
        add(
            &mut qualified,
            format!("{}.{}", e.namespace, e.alias),
            &e.node_type,
        );
        add(&mut namespaces, e.namespace.clone(), &e.node_type);
        if let Some(class) = &e.receiver_class {
            add(&mut methods, format!("{class}.{}", e.alias), &e.node_type);
        }
    }

    let mut out: BTreeSet<(CollisionKind, String, Vec<String>)> = BTreeSet::new();
    let mut push = |kind: CollisionKind, key: &str, nodes: BTreeSet<String>| {
        out.insert((kind, key.to_lowercase(), nodes.into_iter().collect()));
    };

    for (key, nodes) in &qualified {
        if nodes.len() > 1 {
            push(CollisionKind::DuplicateQualified, key, nodes.clone());
        }
    }

    for (key, nodes) in &methods {
        if nodes.len() > 1 {
            push(CollisionKind::DuplicateMethod, key, nodes.clone());
        }
    }

    for e in entries {
        let alias = e.alias.to_lowercase();
        let mut hit: BTreeSet<String> = others(&flats, &alias, &e.node_type)
            .chain(others(&raws, &alias, &e.node_type))
            .cloned()
            .collect();
        if !hit.is_empty() {
            hit.insert(e.node_type.clone());
            push(CollisionKind::AliasShadowsFlat, &alias, hit);
        }
    }

    for (key, nodes) in &qualified {
        let prefix = format!("{key}.");
        let mut hit: BTreeSet<String> = namespaces
            .iter()
            .filter(|(ns, _)| *ns == key || ns.starts_with(&prefix))
            .flat_map(|(_, owners)| owners.iter().cloned())
            .collect();
        if !hit.is_empty() {
            hit.extend(nodes.iter().cloned());
            push(CollisionKind::QualifiedIsNamespace, key, hit);
        }
    }

    for (key, target) in renames {
        let lower = key.to_lowercase();
        let dotted = lower.replace("::", ".");
        let mut hit: BTreeSet<String> = flats
            .get(&lower)
            .into_iter()
            .chain(aliases.get(&lower))
            .chain(qualified.get(&dotted))
            .flatten()
            .cloned()
            .collect();
        if !hit.is_empty() {
            hit.insert(target.to_string());
            push(CollisionKind::RenameShadowsLive, &lower, hit);
        }
    }

    for e in entries {
        let idents = std::iter::once(e.alias.as_str()).chain(namespace_segments(&e.namespace));
        for ident in idents {
            let nodes: BTreeSet<String> = [e.node_type.clone()].into();
            if !is_valid_identifier(ident) {
                push(CollisionKind::InvalidIdentifier, ident, nodes);
            } else if is_keyword(ident) {
                push(CollisionKind::Keyword, ident, nodes);
            }
        }
    }

    out.into_iter()
        .map(|(kind, key, node_types)| NameCollision {
            key,
            kind,
            node_types,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn derived(node_type: &str, category: &str) -> String {
        format!(
            "{}.{}",
            derive_namespace(category),
            derive_alias(node_type, category)
        )
    }

    #[test]
    fn derivation_matches_the_reviewed_prototype() {
        let cases = [
            ("string_trim", "Utils/String", "string.trim"),
            ("utils_hash_md5", "Utils/Hash", "hash.md5"),
            ("http_fetch", "Web/API", "http.fetch"),
            (
                "http_response_to_text",
                "Web/API/Response",
                "http.responseToText",
            ),
            ("struct_get", "Structs/Fields", "struct.get"),
            (
                "data_atlassian_jira_create_issue",
                "Data/Atlassian/Jira",
                "jira.createIssue",
            ),
            (
                "data_atlassian_jira_create_sprint",
                "Data/Atlassian/Jira/Agile",
                "jira.createSprint",
            ),
            ("array_batch_push", "Utils/Array/Batch", "array.batchPush"),
            ("a2ui_set_element_text", "UI/Elements", "ui.setElementText"),
            ("child", "Data/Files/Path", "path.child"),
            ("utils_md_md_to_html", "Utils/Markdown", "md.toHtml"),
            ("utils_md_html_to_md", "Utils/Markdown", "md.htmlToMd"),
            (
                "browser_click",
                "Automation/Browser/Interact",
                "browser.click",
            ),
            ("val_to_bytes", "Utils/Conversions", "json.toBytes"),
            ("utils_json_make_schema", "Utils/JSON", "json.makeSchema"),
            ("df_sql_query", "Data/DataFusion", "df.sqlQuery"),
            ("int_add", "Math/Int", "int.add"),
            ("eval", "Math", "math.eval"),
            (
                "float_vector_dot_product",
                "Utils/Math/Vector",
                "math.vector.dotProduct",
            ),
            ("control_for_each", "Control", "control.forEach"),
            ("log_info", "Logging", "log.info"),
            ("utils_datetime_now", "Utils/DateTime", "datetime.now"),
            (
                "utils_datetime_after",
                "Utils/DateTime/Comparison",
                "datetime.after",
            ),
            (
                "ai_generative_add_history_message",
                "AI/Generative/History",
                "history.addHistoryMessage",
            ),
            (
                "ai_generative_make_history_message",
                "AI/Generative/History/Message",
                "history.makeHistoryMessage",
            ),
            (
                "ai_generative_build_anthropic",
                "AI/Generative/Provider",
                "ai.provider.anthropic",
            ),
            (
                "ai_generative_llm_response_last_content",
                "AI/Generative/Response",
                "ai.response.lastContent",
            ),
            (
                "ai_image_options_openai",
                "AI/Generative/Image/Options",
                "ai.image.options.openai",
            ),
            (
                "ai_video_build_fal",
                "AI/Generative/Video/Provider",
                "ai.video.provider.fal",
            ),
            ("llm_branch", "AI/Generative", "ai.branch"),
            ("chunk_text", "AI/Preprocessing", "ai.processing.chunkText"),
            (
                "processing_pii_detection_options",
                "Processing/Privacy",
                "ai.processing.piiDetectionOptions",
            ),
            ("kg_extract", "AI/Memory/Graph", "ai.memory.kgExtract"),
            (
                "copilot_get_models",
                "AI/GitHub/Copilot/Utilities",
                "github.copilot.getModels",
            ),
            ("agent_invoke", "AI/Agents", "agent.invoke"),
            ("fit_kmeans", "AI/ML/Clustering", "ml.fitKmeans"),
            ("ai_ml_tuning_grid_search", "AI/ML/Tuning", "ml.gridSearch"),
            ("onnx_vad", "AI/ML/ONNX/Audio", "onnx.vad"),
            ("is_bit_of_type", "Bit", "ai.isBitOfType"),
            ("events_chat_push_step", "Events/Chat", "chat.pushStep"),
            ("interaction_form", "Events/Chat/Interaction", "chat.form"),
            ("events_simple", "Events", "events.simple"),
            ("notify_user", "Notifications", "notify.user"),
            ("h3_grid_disk", "Web/Geo/H3", "h3.gridDisk"),
            (
                "geo_search_location",
                "Web/Geo/Search",
                "geo.searchLocation",
            ),
            ("mcp_register_auth", "Web/MCP", "mcp.registerAuth"),
            ("tcp_send", "Web/TCP", "tcp.send"),
            ("web_camera_grab_frame", "Web/Camera", "camera.grabFrame"),
            ("path_exists", "Data/Files/Operations", "files.exists"),
            ("storage_copy", "Data/Files/Operations", "files.copy"),
            ("external_s3_store", "Data/Files/External", "files.s3Store"),
            (
                "graph_cypher_query",
                "Data/Database/Graph/Query",
                "db.graph.cypherQuery",
            ),
            (
                "insert_local_db",
                "Data/Database/Insert",
                "db.insertLocalDb",
            ),
            ("cache_open", "Data/Cache", "data.cache.open"),
            ("data_atlassian_get_me", "Data/Atlassian", "atlassian.getMe"),
            (
                "data_microsoft_todo_create_task",
                "Data/Microsoft/To Do",
                "microsoft.todo.createTask",
            ),
            (
                "data_microsoft_outlook_get_message",
                "Data/Microsoft/Outlook",
                "microsoft.outlook.getMessage",
            ),
            (
                "data_databricks_list_catalogs",
                "Data/Databricks/Unity Catalog",
                "databricks.listCatalogs",
            ),
            (
                "files_spreadsheet_copy_worksheet",
                "Data/Excel",
                "excel.copyWorksheet",
            ),
            (
                "ontology_query_objects",
                "Data Studio/Objects",
                "ontology.queryObjects",
            ),
            ("data_aws_provider", "Data/Providers", "data.awsProvider"),
            (
                "fingerprint_create",
                "Automation/Fingerprint",
                "automation.fingerprint.create",
            ),
            (
                "llm_heal_selector",
                "Automation/LLM/Healing",
                "automation.llm.healSelector",
            ),
            ("docx_add_image", "Document/DOCX", "docx.addImage"),
            ("pdf_page_count", "Image/PDF", "pdf.pageCount"),
            ("resize_image", "Image/Transform", "image.resizeImage"),
            ("video_remux", "Video/Containers", "video.remux"),
            ("video_audio_to_wav", "Audio", "audio.toWav"),
            ("video_burn_subtitles", "Subtitles", "video.burnSubtitles"),
            (
                "mail_imap_inbox_mail_to_reference",
                "Email/Access",
                "email.toReference",
            ),
            ("email_imap_connect", "Email/IMAP", "imap.connect"),
            (
                "mail_imap_calendar_list",
                "Email/IMAP/Calendar",
                "imap.calendarList",
            ),
            ("email_smtp_send", "Email/SMTP", "smtp.send"),
            ("format_bytes", "Utils/Format", "fmt.bytes"),
            ("random_choice", "Utils/Random", "random.choice"),
            (
                "faker_first_name",
                "Utils/Faker/Name",
                "faker.name.firstName",
            ),
            ("cuid", "Utils", "utils.cuid"),
        ];
        for (node_type, category, expected) in cases {
            assert_eq!(derived(node_type, category), expected, "{node_type}");
        }
    }

    #[test]
    fn unmatched_category_becomes_the_lowercased_path() {
        assert_eq!(derive_namespace("Data Studio/Actions"), "ontology");
        assert_eq!(derive_namespace("Document/DOCX"), "docx");
        assert_eq!(derive_namespace("Document/Other"), "document.other");
        assert_eq!(derive_namespace("AI/ML/ONNX"), "onnx");
        assert_eq!(derive_namespace("AI/Other"), "ai.other");
        assert_eq!(derive_namespace("Web/Geo/H3"), "h3");
        assert_eq!(derive_namespace("Utils/Array/By Reference"), "array");
        assert_eq!(derive_namespace("Utils/Set/By Reference"), "set");
        assert_eq!(derive_namespace("Utils/Other"), "utils.other");
        assert_eq!(derive_namespace("Unknown/Thing"), "unknown.thing");
        assert_eq!(derive_namespace(""), "");
        assert_eq!(derive_alias("node", ""), "node");
    }

    #[test]
    fn overrides_sit_between_explicit_fields_and_derivation() {
        const OVERRIDES: &[NameOverride] = &[
            ("utils_hash_md5", None, Some("digestMd5"), Some("input")),
            ("cuid", Some("id"), None, None),
            ("int_random_in_range", None, None, Some("")),
        ];
        let inputs = [("input", "String", "Normal")];
        let names = names_with(
            OVERRIDES,
            "utils_hash_md5",
            "Utils/Hash",
            NameFields::default(),
            inputs,
        );
        assert_eq!(
            names.namespace, "hash",
            "derived when the row leaves it `None`"
        );
        assert_eq!(names.alias, "digestMd5");
        assert_eq!(names.receiver.as_deref(), Some("input"));

        let names = names_with(
            OVERRIDES,
            "utils_hash_md5",
            "Utils/Hash",
            NameFields {
                namespace: Some("digest"),
                alias: Some(" "),
                receiver: Some(""),
            },
            inputs,
        );
        assert_eq!(names.namespace, "digest");
        assert_eq!(
            names.alias, "digestMd5",
            "blank explicit alias falls through"
        );
        assert_eq!(names.receiver, None);

        let (namespace, alias) = spelling_with(
            OVERRIDES,
            "string_trim",
            "Utils/String",
            NameFields::default(),
        );
        assert_eq!(
            (namespace.as_str(), alias.as_str()),
            ("string", "trim"),
            "no row: derived"
        );
        let (namespace, alias) = spelling_with(OVERRIDES, "cuid", "Utils", NameFields::default());
        assert_eq!((namespace.as_str(), alias.as_str()), ("id", "cuid"));
        let (namespace, alias) = effective_spelling("cuid", "Utils", NameFields::default());
        assert_eq!(
            (namespace, alias),
            (derive_namespace("Utils"), derive_alias("cuid", "Utils")),
            "the live residue table has no row for it"
        );

        let names = names_with(
            OVERRIDES,
            "int_random_in_range",
            "Math/Int/Random",
            NameFields::default(),
            [("min", "Integer", "Normal"), ("max", "Integer", "Normal")],
        );
        assert_eq!(
            names.receiver, None,
            "explicit opt-out in the override table"
        );

        let names = effective_names(
            "string_trim",
            "Utils/String",
            NameFields::default(),
            [("string", "String", "Normal")],
        );
        assert_eq!(names.receiver.as_deref(), Some("string"), "default rule");
    }

    #[test]
    fn override_residue_is_empty_after_the_bake_in() {
        assert!(
            NAME_OVERRIDES.is_empty(),
            "every first-party node carries explicit set_flowscript_name/set_receiver calls; a row here must name a node whose id is not a literal in its source"
        );
    }

    #[test]
    fn override_table_is_well_formed() {
        let mut seen = BTreeSet::new();
        for (node_type, namespace, alias, receiver) in NAME_OVERRIDES {
            assert!(
                seen.insert(*node_type),
                "duplicate override for {node_type}"
            );
            assert!(
                namespace.is_some() || alias.is_some() || receiver.is_some(),
                "{node_type}: empty override row"
            );
            let idents = namespace
                .iter()
                .flat_map(|namespace| namespace_segments(namespace))
                .chain(alias.iter().copied());
            for ident in idents {
                assert!(is_valid_identifier(ident), "{node_type}: `{ident}`");
                assert!(!is_keyword(ident), "{node_type}: `{ident}` is a keyword");
            }
        }
    }

    #[test]
    fn alias_never_strips_the_last_segment() {
        assert_eq!(derive_alias("string", "Utils/String"), "string");
        assert_eq!(derive_alias("utils_hash", "Utils/Hash"), "hash");
        assert_eq!(derive_alias("a2ui", "UI"), "a2ui");
    }

    #[test]
    fn qualified_names_join_with_path_separators() {
        assert_eq!(qualified_name("string", "trim"), "string::trim");
        assert_eq!(
            qualified_name("utils.markdown", "mdHtmlToMd"),
            "utils::markdown::mdHtmlToMd"
        );
        assert_eq!(qualified_name("", "trim"), "trim");
        assert_eq!(legacy_display("string_trim"), "stringTrim");
    }

    #[test]
    fn keywords() {
        for k in [
            "function", "const", "let", "for", "of", "if", "else", "while",
        ] {
            assert!(is_keyword(k), "{k}");
        }
        for k in ["return", "true", "false", "null", "interface", "use", "as"] {
            assert!(is_keyword(k), "{k}");
        }
        assert!(is_keyword("Function"));
        assert!(!is_keyword("trim"));
    }

    #[test]
    fn receiver_classes_follow_the_value_type_table() {
        assert_eq!(receiver_class("String", "Normal"), Some("string"));
        assert_eq!(receiver_class("Integer", "Normal"), Some("int"));
        assert_eq!(receiver_class("Float", "Normal"), Some("float"));
        assert_eq!(receiver_class("Boolean", "Normal"), Some("bool"));
        assert_eq!(receiver_class("String", "Array"), Some("array"));
        assert_eq!(receiver_class("Generic", "Array"), Some("array"));
        assert_eq!(receiver_class("Struct", "HashMap"), Some("map"));
        assert_eq!(receiver_class("Integer", "HashSet"), Some("set"));
        assert_eq!(receiver_class("Struct", "Normal"), Some("struct"));
        assert_eq!(receiver_class("Byte", "Array"), Some("bytes"));
        assert_eq!(receiver_class("Byte", "Normal"), Some("byte"));
        assert_eq!(receiver_class("PathBuf", "Normal"), Some("path"));
        assert_eq!(receiver_class("Date", "Normal"), Some("datetime"));
        assert_eq!(receiver_class("Generic", "Normal"), None);
        assert_eq!(receiver_class("Execution", "Normal"), None);

        assert!(namespace_accepts_receiver("string", "String", "Normal"));
        assert!(namespace_accepts_receiver("array", "Struct", "Array"));
        assert!(namespace_accepts_receiver(
            "datetime.comparison",
            "Date",
            "Normal"
        ));
        assert!(!namespace_accepts_receiver("string", "Integer", "Normal"));
        assert!(!namespace_accepts_receiver("http", "Struct", "Normal"));
        assert!(!namespace_accepts_receiver("", "String", "Normal"));
    }

    fn entry(node_type: &str, namespace: &str, alias: &str) -> NameEntry {
        NameEntry {
            node_type: node_type.to_string(),
            flat: legacy_display(node_type),
            namespace: namespace.to_string(),
            alias: alias.to_string(),
            receiver_class: None,
        }
    }

    fn kinds(collisions: &[NameCollision]) -> Vec<CollisionKind> {
        collisions.iter().map(|c| c.kind).collect()
    }

    #[test]
    fn clean_catalog_has_no_collisions() {
        let entries = [
            entry("string_trim", "string", "trim"),
            entry("string_contains", "string", "contains"),
            entry("http_fetch", "http", "fetch"),
            entry("utils_md_html_to_md", "utils.markdown", "mdHtmlToMd"),
        ];
        assert!(check_names(&entries, &[]).is_empty());
    }

    #[test]
    fn rule1_duplicate_qualified_names_case_insensitive() {
        let entries = [
            entry("string_trim", "string", "trim"),
            entry("string_trim_v2", "string", "Trim"),
        ];
        let found = check_names(&entries, &[]);
        assert_eq!(kinds(&found), vec![CollisionKind::DuplicateQualified]);
        assert_eq!(found[0].key, "string.trim");
        assert_eq!(found[0].node_types, vec!["string_trim", "string_trim_v2"]);
    }

    #[test]
    fn rule2_alias_must_not_equal_another_flat_or_raw_name() {
        let entries = [
            entry("string_trim", "string", "trim"),
            entry("trim", "text", "trimText"),
        ];
        let found = check_names(&entries, &[]);
        assert_eq!(kinds(&found), vec![CollisionKind::AliasShadowsFlat]);
        assert_eq!(found[0].node_types, vec!["string_trim", "trim"]);

        let entries = [
            entry("string_trim", "string", "trim"),
            entry("text_trim", "text", "stringTrim"),
        ];
        let found = check_names(&entries, &[]);
        assert_eq!(kinds(&found), vec![CollisionKind::AliasShadowsFlat]);
        assert_eq!(found[0].key, "stringtrim");
    }

    #[test]
    fn alias_equal_to_its_own_flat_name_is_fine() {
        let entries = [entry("trim", "string", "trim")];
        assert!(check_names(&entries, &[]).is_empty());
    }

    #[test]
    fn same_node_type_placed_twice_is_not_a_collision() {
        let entries = [
            entry("string_trim", "string", "trim"),
            entry("string_trim", "string", "trim"),
        ];
        assert!(check_names(&entries, &[]).is_empty());
    }

    #[test]
    fn rule4_qualified_name_must_not_prefix_a_namespace_path() {
        let entries = [
            entry("http_request", "http", "request"),
            entry("http_request_send", "http.request", "send"),
        ];
        let found = check_names(&entries, &[]);
        let rule4: Vec<&NameCollision> = found
            .iter()
            .filter(|c| c.kind == CollisionKind::QualifiedIsNamespace)
            .collect();
        assert_eq!(rule4.len(), 1);
        assert_eq!(rule4[0].key, "http.request");
        assert_eq!(
            rule4[0].node_types,
            vec!["http_request", "http_request_send"]
        );
    }

    #[test]
    fn rule5_rename_keys_must_not_equal_live_names() {
        let entries = [
            entry("string_trim", "string", "trim"),
            entry("string_strip", "string", "strip"),
        ];
        let found = check_names(&entries, &[("stringTrim", "string_strip")]);
        assert_eq!(kinds(&found), vec![CollisionKind::RenameShadowsLive]);
        assert_eq!(found[0].node_types, vec!["string_strip", "string_trim"]);

        let found = check_names(&entries, &[("string::trim", "string_strip")]);
        assert_eq!(kinds(&found), vec![CollisionKind::RenameShadowsLive]);

        let found = check_names(&entries, &[("Strip", "string_trim")]);
        assert_eq!(kinds(&found), vec![CollisionKind::RenameShadowsLive]);

        assert!(check_names(&entries, &[("stringTrimmed", "string_trim")]).is_empty());
    }

    #[test]
    fn rule6_identifiers_must_be_valid_and_not_keywords() {
        let found = check_names(&[entry("control_for", "control", "for")], &[]);
        assert_eq!(kinds(&found), vec![CollisionKind::Keyword]);
        assert_eq!(found[0].key, "for");

        let found = check_names(&[entry("x", "use.me", "ok")], &[]);
        assert_eq!(kinds(&found), vec![CollisionKind::Keyword]);
        assert_eq!(found[0].key, "use");

        let found = check_names(&[entry("x", "my-ns", "a.b")], &[]);
        assert_eq!(
            kinds(&found),
            vec![
                CollisionKind::InvalidIdentifier,
                CollisionKind::InvalidIdentifier
            ]
        );
    }

    #[test]
    fn methods_are_unique_per_receiver_class() {
        let mut a = entry("string_contains", "string", "contains");
        a.receiver_class = Some("string".to_string());
        let mut b = entry("text_contains", "text", "contains");
        b.receiver_class = Some("String".to_string());
        let mut c = entry("array_contains", "array", "contains");
        c.receiver_class = Some("array".to_string());
        let found = check_names(&[a, b, c], &[]);
        assert_eq!(kinds(&found), vec![CollisionKind::DuplicateMethod]);
        assert_eq!(found[0].key, "string.contains");
        assert_eq!(
            found[0].node_types,
            vec!["string_contains", "text_contains"]
        );
    }

    #[test]
    fn derived_entry_uses_the_defaults() {
        let e = NameEntry::derived("string_trim", "Utils/String");
        assert_eq!(e.flat, "stringTrim");
        assert_eq!(e.namespace, "string");
        assert_eq!(e.alias, "trim");
        assert_eq!(e.qualified(), "string::trim");
    }
}
