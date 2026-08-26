//! Node signature stubs for the ~1200-function problem (see `todo/ast.md` §5).
//!
//! The AST crate owns the *shape* and TS-flavoured *formatting* of a signature; core builds
//! `Signature` values from catalog node metadata.
//!
//! Declarations (`.flow.d` v2) are grouped by FlowScript namespace: every catalog node is one
//! `function alias(this: T, { pin: type, … }): R;` inside a `declare namespace ns { … }` block
//! (nested for dotted namespaces). The `{ … }` object is the complete static call shape — the
//! receiver pin included — while the `this:` parameter marks the pin that binds the value in
//! method form. JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the
//! legacy camelCase spelling (`@alias`), which stays accepted forever.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::model::{Container, TypeRef};
use crate::naming::{namespace_segments, qualified_name, schema_title};
use crate::render::render_type_ref;
use crate::text::to_camel_case;

/// A single input/output of a node signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigParam {
    /// Pin name (raw catalog name).
    pub name: String,
    pub ty: TypeRef,
    /// Whether this input has a default / is optional.
    #[serde(default)]
    pub optional: bool,
    /// Optional human-facing pin description (for declaration docs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// Optional JSON Schema string for struct-typed pins (enables deep type resolution).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

/// A TS-flavoured signature stub for one node type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    /// Catalog node type (e.g. `ai_generative_invoke`).
    pub node_type: String,
    /// Legacy flat display name (`aiGenerativeInvoke`).
    pub display: String,
    /// Human-facing label (e.g. `Invoke Agent`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendly: Option<String>,
    /// Catalog category path (e.g. `AI/Generative`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Source catalog package label this node ships in (e.g. `std`, `data`, `web`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default)]
    pub inputs: Vec<SigParam>,
    #[serde(default)]
    pub outputs: Vec<SigParam>,
    /// True if the node has exec pins (side-effecting).
    #[serde(default)]
    pub impure: bool,
    /// Optional one-line doc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// Effective FlowScript namespace (`string`, `http`, `utils.markdown`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Effective FlowScript member name inside `namespace` (`trim`, `fetch`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Input pin bound to the value in method form (`s.trim()`); `None` = static only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
}

/// Whether a declaration line spells a node signature: the v2 `function ns::alias(…)` form or the
/// legacy `declare function flat(…)` form.
pub fn is_signature_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("declare function ") || line.starts_with("function ")
}

impl Signature {
    /// Namespace path segments (`["ai", "ml"]`); empty for a node without a namespace.
    pub fn namespace_path(&self) -> Vec<&str> {
        self.namespace
            .as_deref()
            .map(|ns| namespace_segments(ns).collect())
            .unwrap_or_default()
    }

    /// Member name inside the namespace; the legacy display when the node has no alias.
    pub fn alias_name(&self) -> &str {
        self.alias
            .as_deref()
            .map(str::trim)
            .filter(|alias| !alias.is_empty())
            .unwrap_or(&self.display)
    }

    /// The static call spelling: `ns::alias`, or the legacy display without a namespace.
    pub fn qualified(&self) -> String {
        match self.namespace.as_deref().map(str::trim) {
            Some(ns) if !ns.is_empty() => qualified_name(ns, self.alias_name()),
            _ => self.display.clone(),
        }
    }

    /// The input pin bound by the receiver in method form, when the node has one.
    pub fn receiver_param(&self) -> Option<&SigParam> {
        let receiver = self.receiver.as_deref()?.trim();
        if receiver.is_empty() {
            return None;
        }
        self.inputs.iter().find(|p| p.name == receiver)
    }

    /// The `this:` type of the receiver: the schema title of a titled struct, else the pin type
    /// text (`string`, `any[]`, `Struct`, `any`).
    pub fn receiver_type(&self) -> Option<String> {
        let param = self.receiver_param()?;
        if param.ty.base == "Struct"
            && param.ty.container == Container::Normal
            && let Some(title) = param.schema.as_deref().and_then(schema_title)
        {
            return Some(title);
        }
        Some(render_type_ref(&param.ty))
    }

    fn render_params(&self) -> String {
        let mut out = String::new();
        if let Some(this) = self.receiver_type() {
            out.push_str("this: ");
            out.push_str(&this);
        }
        if !self.inputs.is_empty() {
            if !out.is_empty() {
                out.push_str(", ");
            }
            let inputs: Vec<String> = self
                .inputs
                .iter()
                .map(|p| {
                    format!(
                        "{}{}: {}",
                        to_camel_case(&p.name),
                        if p.optional { "?" } else { "" },
                        render_type_ref(&p.ty)
                    )
                })
                .collect();
            out.push_str("{ ");
            out.push_str(&inputs.join(", "));
            out.push_str(" }");
        }
        out
    }

    fn render_return(&self) -> String {
        match self.outputs.as_slice() {
            [] => "void".to_string(),
            [single] => render_type_ref(&single.ty),
            many => {
                let outs: Vec<String> = many
                    .iter()
                    .map(|p| format!("{}: {}", to_camel_case(&p.name), render_type_ref(&p.ty)))
                    .collect();
                format!("{{ {} }}", outs.join(", "))
            }
        }
    }

    fn render_head(&self, name: &str) -> String {
        format!(
            "function {name}({}): {}",
            self.render_params(),
            self.render_return()
        )
    }

    /// The bare signature line with the qualified name: `function string::contains(this: string,
    /// { string: string, substring: string }): bool;` (legacy `declare function flat(…);` when
    /// the node has no namespace).
    pub fn signature_line(&self) -> String {
        if self.namespace_path().is_empty() {
            format!("declare {};", self.render_head(&self.display))
        } else {
            format!("{};", self.render_head(&self.qualified()))
        }
    }

    /// Render this signature as a single TS-style stub line (plus optional doc), e.g.
    /// `function string::contains(this: string, { … }): bool  // impure`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if let Some(doc) = &self.doc {
            let doc = doc.trim();
            if !doc.is_empty() {
                out.push_str("/** ");
                out.push_str(&doc.replace('\n', " "));
                out.push_str(" */\n");
            }
        }
        out.push_str(&self.render_head(&self.qualified()));
        if self.impure {
            out.push_str("  // impure");
        }
        out
    }

    fn render_jsdoc(&self, indent: &str) -> String {
        let mut out = String::new();
        out.push_str(indent);
        out.push_str("/**\n");
        if let Some(doc) = self.doc.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            for line in doc.lines() {
                out.push_str(indent);
                out.push_str(" * ");
                out.push_str(line.trim_end());
                out.push('\n');
            }
        } else if let Some(friendly) = &self.friendly {
            out.push_str(indent);
            out.push_str(" * ");
            out.push_str(friendly);
            out.push('\n');
        }
        out.push_str(indent);
        out.push_str(" * @node ");
        out.push_str(&self.node_type);
        if let Some(receiver) = self.receiver_param() {
            out.push_str(" @receiver ");
            out.push_str(&receiver.name);
        }
        out.push_str(" @alias ");
        out.push_str(&self.display);
        out.push('\n');
        let receiver_name = self.receiver_param().map(|p| p.name.as_str());
        for p in &self.inputs {
            out.push_str(indent);
            out.push_str(" * @param ");
            out.push_str(&to_camel_case(&p.name));
            if p.optional {
                out.push_str(" (optional)");
            }
            let doc = p.doc.as_deref().map(str::trim).filter(|d| !d.is_empty());
            let is_receiver = receiver_name == Some(p.name.as_str());
            if doc.is_some() || is_receiver {
                out.push_str(" — ");
            }
            if let Some(d) = doc {
                out.push_str(&d.replace('\n', " "));
            }
            if is_receiver {
                if doc.is_some() {
                    out.push(' ');
                }
                out.push_str(&format!(
                    "(receiver: `this` in `x.{}(...)`)",
                    self.alias_name()
                ));
            }
            out.push('\n');
        }
        for p in &self.outputs {
            out.push_str(indent);
            out.push_str(" * @returns ");
            out.push_str(&to_camel_case(&p.name));
            if let Some(d) = p.doc.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                out.push_str(" — ");
                out.push_str(&d.replace('\n', " "));
            }
            out.push('\n');
        }
        if self.impure {
            out.push_str(indent);
            out.push_str(" * @impure has side effects / drives control flow\n");
        }
        out.push_str(indent);
        out.push_str(" */\n");
        out
    }

    /// Render this signature as a fully-documented standalone declaration: a JSDoc block
    /// (description, `@node`/`@receiver`/`@alias`, `@param`, `@returns`, `@impure`) followed by
    /// the qualified signature line.
    pub fn render_declaration(&self) -> String {
        let mut out = self.render_jsdoc("");
        out.push_str(&self.signature_line());
        out
    }

    /// Render this signature as a member of its `declare namespace` block: the JSDoc block plus
    /// `function alias(…): R;`, each line prefixed with `indent`.
    pub fn render_namespace_member(&self, indent: &str) -> String {
        let mut out = self.render_jsdoc(indent);
        out.push_str(indent);
        out.push_str(&self.render_head(self.alias_name()));
        out.push(';');
        out
    }
}

/// Per-node JSON Schema strings for struct-typed pins, keyed by camelCase pin name.
///
/// Emitted as a sidecar next to the `.flow.d` files so editor tooling can resolve node-call
/// result structs deeply without bloating the human/agent-facing declaration text.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeSchemas {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<String, String>,
}

/// Build the schema sidecar: node type -> per-pin JSON Schema strings.
///
/// Only pins that carry a schema are included; nodes without any schema-bearing pins are
/// omitted entirely, keeping the sidecar compact.
pub fn schema_sidecar(signatures: &[Signature]) -> BTreeMap<String, NodeSchemas> {
    let collect = |params: &[SigParam]| -> BTreeMap<String, String> {
        params
            .iter()
            .filter_map(|p| {
                p.schema
                    .as_ref()
                    .map(|s| (to_camel_case(&p.name), s.clone()))
            })
            .collect()
    };

    let mut map = BTreeMap::new();
    for sig in signatures {
        let inputs = collect(&sig.inputs);
        let outputs = collect(&sig.outputs);
        if inputs.is_empty() && outputs.is_empty() {
            continue;
        }
        map.insert(sig.node_type.clone(), NodeSchemas { inputs, outputs });
    }
    map
}

/// A single `.flow.d` declaration file: a filename stem plus its rendered content.
#[derive(Debug, Clone)]
pub struct DeclarationFile {
    /// Filename stem (sanitized top-level category, e.g. `ai`, `structs`, `control`).
    pub stem: String,
    /// The category label this file covers (e.g. `AI`).
    pub category: String,
    /// Rendered `.flow.d` file content.
    pub content: String,
}

/// Group signatures into per-top-level-category `.flow.d` declaration files.
///
/// Each file is grouped by namespace (nested blocks for dotted paths) and, inside a namespace,
/// by full category path so a coding agent can scan one domain (`ai.flow.d`, `structs.flow.d`,
/// …) at a time. Returns files sorted by stem.
pub fn declarations_by_category(signatures: &[Signature]) -> Vec<DeclarationFile> {
    let mut buckets: HashMap<String, Vec<&Signature>> = HashMap::new();
    for sig in signatures {
        let top = top_category(sig);
        buckets.entry(top).or_default().push(sig);
    }

    let mut stems: Vec<String> = buckets.keys().cloned().collect();
    stems.sort();

    stems
        .into_iter()
        .map(|top| {
            let sigs = buckets.remove(&top).unwrap_or_default();
            let content = render_declaration_file(&top, &sigs);
            DeclarationFile {
                stem: sanitize_stem(&top),
                category: top,
                content,
            }
        })
        .collect()
}

/// Group signatures into per-source-package `.flow.d` declaration files.
///
/// Each file collects every node shipped by one catalog package (`std`, `data`, `web`, …) and is
/// grouped by namespace, then by full category path. Signatures without a `package` fall into
/// `misc`. Returns files sorted by stem. This is the package-scoped counterpart to
/// [`declarations_by_category`] and is what FlowPilot loads as type reference for the packages a
/// project actually uses.
pub fn declarations_by_package(signatures: &[Signature]) -> Vec<DeclarationFile> {
    let mut buckets: HashMap<String, Vec<&Signature>> = HashMap::new();
    for sig in signatures {
        buckets.entry(package_label(sig)).or_default().push(sig);
    }

    let mut labels: Vec<String> = buckets.keys().cloned().collect();
    labels.sort();

    labels
        .into_iter()
        .map(|label| {
            let sigs = buckets.remove(&label).unwrap_or_default();
            let content = render_declaration_file(&label, &sigs);
            DeclarationFile {
                stem: sanitize_stem(&label),
                category: label,
                content,
            }
        })
        .collect()
}

/// Source package label, defaulting to `misc` when a signature is untagged.
fn package_label(sig: &Signature) -> String {
    match &sig.package {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => "misc".to_string(),
    }
}

#[derive(Default)]
struct NamespaceNode<'a> {
    members: Vec<&'a Signature>,
    children: BTreeMap<String, NamespaceNode<'a>>,
}

impl<'a> NamespaceNode<'a> {
    fn insert(&mut self, path: &[&str], sig: &'a Signature) {
        match path.split_first() {
            None => self.members.push(sig),
            Some((head, rest)) => self
                .children
                .entry((*head).to_string())
                .or_default()
                .insert(rest, sig),
        }
    }

    fn render_into(&self, out: &mut String, depth: usize) {
        let indent = "    ".repeat(depth);
        let mut members = self.members.clone();
        members.sort_by(|a, b| {
            full_category(a)
                .cmp(full_category(b))
                .then_with(|| a.alias_name().cmp(b.alias_name()))
                .then_with(|| a.node_type.cmp(&b.node_type))
        });
        let mut current: Option<&str> = None;
        for sig in &members {
            let cat = full_category(sig);
            if current != Some(cat) {
                if !out.ends_with("\n\n") && !out.ends_with("{\n") {
                    out.push('\n');
                }
                out.push_str(&indent);
                out.push_str("// === ");
                out.push_str(cat);
                out.push_str(" ===\n\n");
                current = Some(cat);
            }
            out.push_str(&sig.render_namespace_member(&indent));
            out.push_str("\n\n");
        }
        for (name, child) in &self.children {
            if !out.ends_with("\n\n") && !out.ends_with("{\n") {
                out.push('\n');
            }
            out.push_str(&indent);
            if depth == 0 {
                out.push_str("declare ");
            }
            out.push_str("namespace ");
            out.push_str(name);
            out.push_str(" {\n");
            child.render_into(out, depth + 1);
            while out.ends_with("\n\n") {
                out.pop();
            }
            out.push_str(&indent);
            out.push_str("}\n\n");
        }
    }
}

/// Render one `.flow.d` file: a header, then declarations grouped by namespace and category.
fn render_declaration_file(top: &str, sigs: &[&Signature]) -> String {
    let mut out = String::new();
    out.push_str("// ");
    out.push_str(top);
    out.push_str(" — FlowScript node declarations (generated, do not edit).\n");
    out.push_str(
        "// One `function` per catalog node, grouped by FlowScript namespace. Call a node as\n\
         // `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and\n\
         // call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node\n\
         // is also a method on that value (`x.alias(...)`, remaining inputs positional or named).\n\
         // JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy\n\
         // camelCase spelling (`@alias`), which is still accepted.\n\n",
    );

    let mut root = NamespaceNode::default();
    for sig in sigs {
        let path = sig.namespace_path();
        root.insert(&path, sig);
    }

    let mut flat = root.members.clone();
    flat.sort_by(|a, b| {
        full_category(a)
            .cmp(full_category(b))
            .then_with(|| a.display.cmp(&b.display))
    });
    let mut current: Option<&str> = None;
    for sig in flat {
        let cat = full_category(sig);
        if current != Some(cat) {
            if current.is_some() {
                out.push('\n');
            }
            out.push_str("// === ");
            out.push_str(cat);
            out.push_str(" ===\n\n");
            current = Some(cat);
        }
        out.push_str(&sig.render_declaration());
        out.push_str("\n\n");
    }
    root.members.clear();
    root.render_into(&mut out, 0);
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Top-level category segment (before the first `/`), defaulting to `misc`.
fn top_category(sig: &Signature) -> String {
    match &sig.category {
        Some(c) if !c.trim().is_empty() => c.split('/').next().unwrap_or(c).trim().to_string(),
        _ => "Misc".to_string(),
    }
}

/// Full category path, defaulting to `Misc`.
fn full_category(sig: &Signature) -> &str {
    match &sig.category {
        Some(c) if !c.trim().is_empty() => c.as_str(),
        _ => "Misc",
    }
}

/// Lower-case, filesystem-safe stem from a category label.
fn sanitize_stem(category: &str) -> String {
    let mut out = String::with_capacity(category.len());
    let mut prev_dash = false;
    for ch in category.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "misc".to_string()
    } else {
        out
    }
}

/// Render a set of signatures as a read-only signature header block.
pub fn render_signatures(signatures: &[Signature]) -> String {
    let mut out = String::from("// signatures (generated, read-only)\n");
    for sig in signatures {
        out.push_str(&sig.render());
        out.push('\n');
    }
    out
}

/// Current schema version of the serialized signature registry.
pub const SIGNATURE_SET_VERSION: u32 = 1;

/// A serializable registry of node signatures.
///
/// This is the build-time dump of the catalog's `get_node()` metadata. The parser uses it to
/// recover pin names from positional arguments and to validate calls, without depending on the
/// (heavy) catalog at parse time. Generated by a test in `flow-like-catalog`; consumed here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureSet {
    /// Schema version (`SIGNATURE_SET_VERSION`).
    pub version: u32,
    /// One entry per catalog node type, sorted by `node_type` for stable diffs.
    pub signatures: Vec<Signature>,
}

impl SignatureSet {
    /// Build a set from signatures, sorting by node type for deterministic output.
    pub fn new(mut signatures: Vec<Signature>) -> Self {
        signatures.sort_by(|a, b| a.node_type.cmp(&b.node_type));
        Self {
            version: SIGNATURE_SET_VERSION,
            signatures,
        }
    }

    /// Index signatures by catalog node type for O(1) lookup.
    pub fn by_node_type(&self) -> HashMap<&str, &Signature> {
        self.signatures
            .iter()
            .map(|s| (s.node_type.as_str(), s))
            .collect()
    }

    /// Look up a single signature by catalog node type.
    pub fn get(&self, node_type: &str) -> Option<&Signature> {
        self.signatures.iter().find(|s| s.node_type == node_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &str, base: &str, container: Container, optional: bool) -> SigParam {
        SigParam {
            name: name.to_string(),
            ty: TypeRef::new(base, container),
            optional,
            doc: Some(format!("{name} doc")),
            schema: None,
        }
    }

    fn contains() -> Signature {
        Signature {
            node_type: "string_contains".to_string(),
            display: "stringContains".to_string(),
            friendly: Some("Contains".to_string()),
            category: Some("Utils/String".to_string()),
            package: Some("std".to_string()),
            inputs: vec![
                param("string", "string", Container::Normal, false),
                param("substring", "string", Container::Normal, false),
                param("ignore_case", "bool", Container::Normal, true),
            ],
            outputs: vec![param("contains", "bool", Container::Normal, false)],
            impure: false,
            doc: Some("Checks whether a string contains a substring".to_string()),
            namespace: Some("string".to_string()),
            alias: Some("contains".to_string()),
            receiver: Some("string".to_string()),
        }
    }

    fn model_read() -> Signature {
        Signature {
            node_type: "ai_ml_model_read".to_string(),
            display: "aiMlModelRead".to_string(),
            friendly: Some("Read Model".to_string()),
            category: Some("AI/ML".to_string()),
            package: None,
            inputs: vec![param("path", "string", Container::Normal, false)],
            outputs: vec![param("model", "Struct", Container::Normal, false)],
            impure: true,
            doc: None,
            namespace: Some("ai.ml".to_string()),
            alias: Some("read".to_string()),
            receiver: None,
        }
    }

    fn fetch() -> Signature {
        Signature {
            node_type: "http_fetch".to_string(),
            display: "httpFetch".to_string(),
            friendly: None,
            category: Some("AI/Web".to_string()),
            package: None,
            inputs: vec![param("url", "string", Container::Normal, false)],
            outputs: vec![SigParam {
                name: "response".to_string(),
                ty: TypeRef::new("Struct", Container::Normal),
                optional: false,
                doc: None,
                schema: Some(r#"{"title":"HttpResponse","type":"object"}"#.to_string()),
            }],
            impure: true,
            doc: None,
            namespace: Some("http".to_string()),
            alias: Some("fetch".to_string()),
            receiver: None,
        }
    }

    #[test]
    fn one_line_stub_uses_the_qualified_name_and_this_parameter() {
        assert_eq!(
            contains().render(),
            "/** Checks whether a string contains a substring */\nfunction string::contains(this: string, { string: string, substring: string, ignoreCase?: bool }): bool"
        );
        assert_eq!(
            model_read().render(),
            "function ai::ml::read({ path: string }): Struct  // impure"
        );
        assert_eq!(
            contains().signature_line(),
            "function string::contains(this: string, { string: string, substring: string, ignoreCase?: bool }): bool;"
        );
        assert!(is_signature_line(&contains().signature_line()));
        assert!(is_signature_line(
            "declare function stringTrim({ string: string }): string;"
        ));
        assert!(!is_signature_line("// function stringTrim"));
    }

    #[test]
    fn standalone_declaration_carries_the_tags() {
        let rendered = contains().render_declaration();
        assert!(
            rendered.contains(" * @node string_contains @receiver string @alias stringContains\n")
        );
        assert!(
            rendered.contains(
                " * @param string — string doc (receiver: `this` in `x.contains(...)`)\n"
            )
        );
        assert!(rendered.contains(" * @param ignoreCase (optional) — ignore_case doc\n"));
        assert!(rendered.contains(" * @returns contains — contains doc\n"));
        assert!(rendered.ends_with(
            "function string::contains(this: string, { string: string, substring: string, ignoreCase?: bool }): bool;"
        ));
        let impure = model_read().render_declaration();
        assert!(impure.contains(" * Read Model\n * @node ai_ml_model_read @alias aiMlModelRead\n"));
        assert!(impure.contains(" * @impure has side effects / drives control flow\n"));
        assert!(impure.ends_with("function ai::ml::read({ path: string }): Struct;"));
    }

    #[test]
    fn titled_struct_receivers_use_the_schema_title() {
        let mut to_text = fetch();
        to_text.node_type = "http_response_to_text".to_string();
        to_text.display = "httpResponseToText".to_string();
        to_text.alias = Some("toText".to_string());
        to_text.inputs = vec![SigParam {
            name: "response".to_string(),
            ty: TypeRef::new("Struct", Container::Normal),
            optional: false,
            doc: None,
            schema: Some(r#"{"title":"HttpResponse","type":"object"}"#.to_string()),
        }];
        to_text.outputs = vec![param("text", "string", Container::Normal, false)];
        to_text.receiver = Some("response".to_string());
        to_text.impure = false;
        assert_eq!(to_text.receiver_type().as_deref(), Some("HttpResponse"));
        assert_eq!(
            to_text.signature_line(),
            "function http::toText(this: HttpResponse, { response: Struct }): string;"
        );

        let mut push = contains();
        push.inputs = vec![
            param("array_in", "any", Container::Array, false),
            param("value", "any", Container::Normal, false),
        ];
        push.receiver = Some("array_in".to_string());
        assert_eq!(push.receiver_type().as_deref(), Some("any[]"));

        let mut opted_out = contains();
        opted_out.receiver = Some(String::new());
        assert_eq!(opted_out.receiver_type(), None);
        assert!(
            opted_out
                .signature_line()
                .starts_with("function string::contains({ string")
        );
    }

    #[test]
    fn declaration_files_nest_namespaces_and_group_categories() {
        let files = declarations_by_category(&[fetch(), model_read(), contains()]);
        assert_eq!(
            files.iter().map(|f| f.stem.as_str()).collect::<Vec<_>>(),
            ["ai", "utils"]
        );
        let ai = &files[0].content;
        assert!(ai.starts_with("// AI — FlowScript node declarations (generated, do not edit).\n"));
        assert!(ai.contains("// `ns::alias({ pin: value })`"));
        let expected_ai = "declare namespace ai {\n    namespace ml {\n        // === AI/ML ===\n\n        /**\n         * Read Model\n         * @node ai_ml_model_read @alias aiMlModelRead\n         * @param path — path doc\n         * @returns model — model doc\n         * @impure has side effects / drives control flow\n         */\n        function read({ path: string }): Struct;\n    }\n}\n\ndeclare namespace http {\n    // === AI/Web ===\n\n    /**\n     * @node http_fetch @alias httpFetch\n     * @param url — url doc\n     * @returns response\n     * @impure has side effects / drives control flow\n     */\n    function fetch({ url: string }): Struct;\n}\n";
        assert!(
            ai.ends_with(expected_ai),
            "unexpected ai.flow.d tail:\n{ai}"
        );
        let utils = &files[1].content;
        assert!(utils.contains(
            "declare namespace string {\n    // === Utils/String ===\n\n    /**\n     * Checks whether a string contains a substring\n     * @node string_contains @receiver string @alias stringContains\n"
        ));
        assert!(utils.contains(
            "    function contains(this: string, { string: string, substring: string, ignoreCase?: bool }): bool;\n}"
        ));
        assert!(!utils.contains("declare function"));
    }

    #[test]
    fn namespace_less_signatures_keep_the_legacy_top_level_form() {
        let mut legacy = contains();
        legacy.namespace = None;
        legacy.alias = None;
        legacy.receiver = None;
        assert_eq!(legacy.qualified(), "stringContains");
        assert_eq!(
            legacy.signature_line(),
            "declare function stringContains({ string: string, substring: string, ignoreCase?: bool }): bool;"
        );
        let files = declarations_by_package(&[legacy]);
        assert_eq!(files[0].stem, "std");
        assert!(
            files[0]
                .content
                .contains("// === Utils/String ===\n\n/**\n")
        );
        assert!(files[0].content.ends_with(
            "declare function stringContains({ string: string, substring: string, ignoreCase?: bool }): bool;\n"
        ));
    }

    #[test]
    fn schema_sidecar_is_keyed_by_node_type() {
        let sidecar = schema_sidecar(&[fetch(), contains()]);
        assert_eq!(sidecar.keys().collect::<Vec<_>>(), ["http_fetch"]);
        assert!(sidecar["http_fetch"].outputs.contains_key("response"));
    }
}
