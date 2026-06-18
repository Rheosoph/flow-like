//! Node signature stubs for the ~1200-function problem (see `todo/ast.md` §5).
//!
//! The AST crate owns the *shape* and TS-flavoured *formatting* of a signature; core builds
//! `Signature` values from catalog node metadata.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::model::TypeRef;
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
    /// JS-flavoured display name.
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
}

impl Signature {
    /// Render this signature as a single TS-style declaration line (plus optional doc).
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
        out.push_str("function ");
        out.push_str(&self.display);
        out.push('(');
        if !self.inputs.is_empty() {
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
        out.push_str("): ");
        match self.outputs.as_slice() {
            [] => out.push_str("void"),
            [single] => out.push_str(&render_type_ref(&single.ty)),
            many => {
                let outs: Vec<String> = many
                    .iter()
                    .map(|p| format!("{}: {}", to_camel_case(&p.name), render_type_ref(&p.ty)))
                    .collect();
                out.push('{');
                out.push(' ');
                out.push_str(&outs.join(", "));
                out.push_str(" }");
            }
        }
        if self.impure {
            out.push_str("  // impure");
        }
        out
    }

    /// Render this signature as a fully-documented `.flow.d` declaration: a JSDoc block
    /// (description, `@param`, `@returns`, `@impure`) followed by a `declare function` line.
    pub fn render_declaration(&self) -> String {
        let mut out = String::new();
        out.push_str("/**\n");
        if let Some(doc) = self.doc.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            for line in doc.lines() {
                out.push_str(" * ");
                out.push_str(line.trim_end());
                out.push('\n');
            }
        } else if let Some(friendly) = &self.friendly {
            out.push_str(" * ");
            out.push_str(friendly);
            out.push('\n');
        }
        for p in &self.inputs {
            out.push_str(" * @param ");
            out.push_str(&to_camel_case(&p.name));
            if p.optional {
                out.push_str(" (optional)");
            }
            if let Some(d) = p.doc.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                out.push_str(" — ");
                out.push_str(&d.replace('\n', " "));
            }
            out.push('\n');
        }
        for p in &self.outputs {
            out.push_str(" * @returns ");
            out.push_str(&to_camel_case(&p.name));
            if let Some(d) = p.doc.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                out.push_str(" — ");
                out.push_str(&d.replace('\n', " "));
            }
            out.push('\n');
        }
        if self.impure {
            out.push_str(" * @impure has side effects / drives control flow\n");
        }
        out.push_str(" */\n");

        out.push_str("declare function ");
        out.push_str(&self.display);
        out.push('(');
        if !self.inputs.is_empty() {
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
        out.push_str("): ");
        match self.outputs.as_slice() {
            [] => out.push_str("void"),
            [single] => out.push_str(&render_type_ref(&single.ty)),
            many => {
                let outs: Vec<String> = many
                    .iter()
                    .map(|p| format!("{}: {}", to_camel_case(&p.name), render_type_ref(&p.ty)))
                    .collect();
                out.push_str("{ ");
                out.push_str(&outs.join(", "));
                out.push_str(" }");
            }
        }
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

/// Build the schema sidecar: display name -> per-pin JSON Schema strings.
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
        map.insert(sig.display.clone(), NodeSchemas { inputs, outputs });
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
/// Each file is sorted by display name and sub-grouped by full category path so a coding agent
/// can scan one domain (`ai.flow.d`, `structs.flow.d`, …) at a time. Returns files sorted by stem.
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
            let mut sigs = buckets.remove(&top).unwrap_or_default();
            sigs.sort_by(|a, b| {
                full_category(a)
                    .cmp(&full_category(b))
                    .then_with(|| a.display.cmp(&b.display))
            });
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
/// sub-grouped by full category path. Signatures without a `package` fall into `misc`. Returns
/// files sorted by stem. This is the package-scoped counterpart to [`declarations_by_category`]
/// and is what FlowPilot loads as type reference for the packages a project actually uses.
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
            let mut sigs = buckets.remove(&label).unwrap_or_default();
            sigs.sort_by(|a, b| {
                full_category(a)
                    .cmp(full_category(b))
                    .then_with(|| a.display.cmp(&b.display))
            });
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

/// Render one `.flow.d` file: a header, then declarations grouped by full category path.
fn render_declaration_file(top: &str, sigs: &[&Signature]) -> String {
    let mut out = String::new();
    out.push_str("// ");
    out.push_str(top);
    out.push_str(" — FlowScript node declarations (generated, do not edit).\n");
    out.push_str("// One declare-function per catalog node. Names are camelCase node types.\n\n");

    let mut current: Option<&str> = None;
    for sig in sigs {
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
