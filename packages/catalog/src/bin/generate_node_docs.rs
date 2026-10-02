use flow_like_catalog::labeled_catalog;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocScores {
    privacy: u8,
    security: u8,
    performance: u8,
    governance: u8,
    reliability: u8,
    cost: u8,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocPinOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    sensitive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_values: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    range: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enforce_schema: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enforce_generic_value_type: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocPin {
    name: String,
    friendly_name: String,
    description: String,
    pin_type: String,
    data_type: String,
    value_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_value: Option<Value>,
    index: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<DocPinOptions>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocFnRefs {
    fn_refs: Vec<String>,
    can_reference_fns: bool,
    can_be_referenced_by_fns: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocNode {
    slug: String,
    package_name: String,
    name: String,
    friendly_name: String,
    description: String,
    category: String,
    category_path: Vec<String>,
    category_slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scores: Option<DocScores>,
    pins: Vec<DocPin>,
    input_count: usize,
    output_count: usize,
    flags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    docs: Option<String>,
    oauth_providers: Vec<String>,
    required_oauth_scopes: BTreeMap<String, Vec<String>>,
    permissions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fn_refs: Option<DocFnRefs>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocCategory {
    label: String,
    path: String,
    slug: String,
    depth: usize,
    count: usize,
    description: String,
}

#[derive(Default)]
struct CategoryAccumulator {
    label: String,
    path: String,
    slug: String,
    depth: usize,
    nodes: BTreeSet<String>,
}

#[derive(Debug, Serialize)]
struct SidebarItem {
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    collapsed: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    items: Vec<SidebarItem>,
}

#[derive(Default)]
struct SidebarCategory {
    label: String,
    path: String,
    slug: String,
    children: BTreeMap<String, SidebarCategory>,
    nodes: Vec<SidebarNode>,
}

struct SidebarNode {
    label: String,
    slug: String,
}

#[derive(Default)]
struct GeneratedOutput {
    check: bool,
    expected: BTreeSet<PathBuf>,
    differences: BTreeMap<PathBuf, &'static str>,
}

impl GeneratedOutput {
    fn write(&mut self, path: PathBuf, content: String) -> Result<(), Box<dyn Error>> {
        self.expected.insert(path.clone());
        if self.check {
            match fs::read(&path) {
                Ok(actual) if actual == content.as_bytes() => {}
                Ok(_) => {
                    self.differences.insert(path, "changed");
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    self.differences.insert(path, "missing");
                }
                Err(error) => return Err(error.into()),
            }
        } else {
            fs::create_dir_all(path.parent().ok_or("generated file has no parent")?)?;
            fs::write(path, content)?;
        }
        Ok(())
    }

    fn find_obsolete(&mut self, directory: &Path, nodes_root: &Path) -> Result<(), Box<dyn Error>> {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if directory == nodes_root
                && matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("overview.md" | "overview.mdx")
                )
            {
                continue;
            }
            if entry.file_type()?.is_dir() {
                self.find_obsolete(&path, nodes_root)?;
            } else if !self.expected.contains(&path) {
                self.differences.insert(path, "obsolete");
            }
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut output = GeneratedOutput::default();
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--check" => output.check = true,
            _ => {
                return Err(format!(
                    "Unknown argument: {argument}. Usage: generate_node_docs [--check]"
                )
                .into());
            }
        }
    }
    let root = workspace_root()?;
    let docs_src = root.join("apps/docs/src");
    let docs_root = docs_src.join("content/docs");
    let nodes_dir = docs_root.join("nodes");

    if !output.check {
        fs::create_dir_all(&nodes_dir)?;
        clean_generated_node_docs(&nodes_dir)?;
    }

    let mut nodes = build_doc_nodes()?;
    nodes.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.friendly_name.cmp(&b.friendly_name))
            .then_with(|| a.name.cmp(&b.name))
    });

    let categories = build_categories(&nodes);

    write_catalog_data(&docs_src, &nodes, &categories, &mut output)?;
    write_sidebar(&docs_src, &categories, &nodes, &mut output)?;
    write_category_pages(&docs_src, &docs_root, &categories, &mut output)?;
    write_node_pages(&docs_src, &docs_root, &nodes, &mut output)?;

    if output.check {
        output.find_obsolete(&nodes_dir, &nodes_dir)?;
        if !output.differences.is_empty() {
            for (path, kind) in &output.differences {
                eprintln!(
                    "{kind}: {}",
                    path.strip_prefix(&root).unwrap_or(path).display()
                );
            }
            return Err(format!(
                "{} generated documentation files are out of date. Run `bun run docs:nodes` and review the changes.",
                output.differences.len()
            ).into());
        }
    }

    println!(
        "{} {} node pages and {} category pages in {}",
        if output.check { "Checked" } else { "Generated" },
        nodes.len(),
        categories.len(),
        nodes_dir.display()
    );

    Ok(())
}

fn workspace_root() -> Result<PathBuf, Box<dyn Error>> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?)
}

fn clean_generated_node_docs(nodes_dir: &Path) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(nodes_dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();

        if file_name == "overview.md" || file_name == "overview.mdx" {
            continue;
        }

        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }

    Ok(())
}

fn build_doc_nodes() -> Result<Vec<DocNode>, Box<dyn Error>> {
    let mut by_name: BTreeMap<String, DocNode> = BTreeMap::new();

    for labeled in labeled_catalog() {
        let raw_node = labeled.node.get_node();
        let raw = serde_json::to_value(raw_node)?;
        let obj = raw.as_object().ok_or("serialized node is not an object")?;
        let name = string_field(obj, "name");
        let friendly_name = string_field(obj, "friendly_name");
        let description = string_field(obj, "description");
        let category = non_empty_or(string_field(obj, "category"), "Uncategorized");
        let category_path = category_segments(&category);
        let category_slug = format!("nodes/{}", safe_segments(&category_path).join("/"));
        let slug = format!("{}/{}", category_slug, safe_segment(&name));
        let pins = pin_list(obj.get("pins"));
        let input_count = pins.iter().filter(|pin| pin.pin_type == "Input").count();
        let output_count = pins.iter().filter(|pin| pin.pin_type == "Output").count();
        let oauth_providers = string_array(obj.get("oauth_providers")).unwrap_or_default();
        let required_oauth_scopes = string_array_map(obj.get("required_oauth_scopes"));
        let permissions = permissions(obj);
        let flags = node_flags(obj, !oauth_providers.is_empty(), !permissions.is_empty());

        by_name.insert(
            name.clone(),
            DocNode {
                slug,
                package_name: labeled.package,
                name,
                friendly_name,
                description,
                category,
                category_path,
                category_slug,
                icon: opt_string_field(obj, "icon"),
                scores: scores(obj.get("scores")),
                pins,
                input_count,
                output_count,
                flags,
                version: opt_u64_field(obj, "version"),
                docs: opt_string_field(obj, "docs"),
                oauth_providers,
                required_oauth_scopes,
                permissions,
                fn_refs: fn_refs(obj.get("fn_refs")),
            },
        );
    }

    Ok(by_name.into_values().collect())
}

fn build_categories(nodes: &[DocNode]) -> Vec<DocCategory> {
    let mut categories: BTreeMap<String, CategoryAccumulator> = BTreeMap::new();

    for node in nodes {
        for depth in 1..=node.category_path.len() {
            let path_parts = &node.category_path[..depth];
            let path = path_parts.join("/");
            let slug = format!("nodes/{}", safe_segments(path_parts).join("/"));
            let label = path_parts
                .last()
                .map(|segment| display_label(segment))
                .unwrap_or_else(|| display_label(&path));
            let entry = categories
                .entry(path.clone())
                .or_insert_with(|| CategoryAccumulator {
                    label,
                    path,
                    slug,
                    depth,
                    nodes: BTreeSet::new(),
                });
            entry.nodes.insert(node.name.clone());
        }
    }

    categories
        .into_values()
        .map(|category| DocCategory {
            description: format!(
                "Browse {} generated Flow-Like node reference{} in {} with pin details and available schema, package, and risk-rating metadata.",
                category.nodes.len(),
                if category.nodes.len() == 1 { "" } else { "s" },
                display_category_path(&category.path)
            ),
            label: category.label,
            path: category.path,
            slug: category.slug,
            depth: category.depth,
            count: category.nodes.len(),
        })
        .collect()
}

fn write_catalog_data(
    docs_src: &Path,
    nodes: &[DocNode],
    categories: &[DocCategory],
    output: &mut GeneratedOutput,
) -> Result<(), Box<dyn Error>> {
    let generated_dir = docs_src.join("generated");

    let nodes_json = serde_json::to_string_pretty(nodes)?;
    let categories_json = serde_json::to_string_pretty(categories)?;
    let content = format!(
        "{}\n\n{}\n\nexport const catalogNodes: CatalogNode[] = {};\n\n\
         export const catalogCategories: CatalogCategory[] = {};\n\n\
         export const nodesBySlug = Object.fromEntries(\n\
         \tcatalogNodes.map((node) => [node.slug, node]),\n\
         ) as Record<string, CatalogNode | undefined>;\n\n\
         export const catalogSummaries = catalogNodes.map(toCatalogSummary);\n\n\
         export const nodesByCategory = Object.fromEntries(\n\
         \tcatalogCategories.map((category) => [\n\
         \t\tcategory.path,\n\
         \t\tcatalogSummaries.filter(\n\
         \t\t\t(node) =>\n\
         \t\t\t\tnode.category === category.path ||\n\
         \t\t\t\tnode.category.startsWith(`${{category.path}}/`),\n\
         \t\t),\n\
         \t]),\n\
         ) as Record<string, CatalogSummary[] | undefined>;\n",
        generated_header(),
        "import type {\n\tCatalogCategory,\n\tCatalogNode,\n} from \"../components/node-docs/NodeReference\";\nimport { toCatalogSummary, type CatalogSummary } from \"../components/node-docs/catalog-summary\";",
        nodes_json,
        categories_json
    );

    output.write(generated_dir.join("catalog-nodes.ts"), content)?;
    Ok(())
}

fn write_sidebar(
    docs_src: &Path,
    categories: &[DocCategory],
    nodes: &[DocNode],
    output: &mut GeneratedOutput,
) -> Result<(), Box<dyn Error>> {
    let generated_dir = docs_src.join("generated");

    let mut root = SidebarCategory::default();
    for category in categories {
        insert_sidebar_category(&mut root, category);
    }

    for node in nodes {
        let category = ensure_sidebar_category(&mut root, &node.category_path);
        category.nodes.push(SidebarNode {
            label: node.friendly_name.clone(),
            slug: node.slug.clone(),
        });
    }

    let items = root
        .children
        .into_values()
        .map(sidebar_category_item)
        .collect::<Vec<_>>();

    let content = format!(
        "{}\n\nexport const generatedNodeSidebar = {};\n",
        generated_header(),
        serde_json::to_string_pretty(&items)?
    );
    output.write(generated_dir.join("node-sidebar.mjs"), content)?;
    Ok(())
}

fn insert_sidebar_category(root: &mut SidebarCategory, category: &DocCategory) {
    let entry = ensure_sidebar_category(root, &category_segments(&category.path));
    entry.label = category.label.clone();
    entry.path = category.path.clone();
    entry.slug = category.slug.clone();
}

fn ensure_sidebar_category<'a>(
    root: &'a mut SidebarCategory,
    segments: &[String],
) -> &'a mut SidebarCategory {
    let mut current = root;
    let mut path_parts = Vec::new();

    for segment in segments {
        path_parts.push(segment.clone());
        let path = path_parts.join("/");
        let slug = format!("nodes/{}", safe_segments(&path_parts).join("/"));

        current = current
            .children
            .entry(segment.clone())
            .or_insert_with(|| SidebarCategory {
                label: display_label(segment),
                path,
                slug,
                children: BTreeMap::new(),
                nodes: Vec::new(),
            });
    }

    current
}

fn sidebar_category_item(category: SidebarCategory) -> SidebarItem {
    let mut items = Vec::with_capacity(category.children.len() + category.nodes.len() + 1);
    items.push(sidebar_link("Overview".to_string(), category.slug));

    let child_items = category
        .children
        .into_values()
        .map(sidebar_category_item)
        .collect::<Vec<_>>();
    items.extend(child_items);

    let mut node_items = category
        .nodes
        .into_iter()
        .map(|node| sidebar_link(node.label, node.slug))
        .collect::<Vec<_>>();
    node_items.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.slug.cmp(&b.slug)));
    items.extend(node_items);

    SidebarItem {
        label: category.label,
        slug: None,
        collapsed: Some(true),
        items,
    }
}

fn sidebar_link(label: String, slug: String) -> SidebarItem {
    SidebarItem {
        label,
        slug: Some(slug),
        collapsed: None,
        items: Vec::new(),
    }
}

fn write_category_pages(
    docs_src: &Path,
    docs_root: &Path,
    categories: &[DocCategory],
    output: &mut GeneratedOutput,
) -> Result<(), Box<dyn Error>> {
    for category in categories {
        let file_path = docs_root.join(format!("{}.mdx", category.slug));

        let component_import = relative_import(
            file_path.parent().unwrap(),
            &docs_src.join("components/node-docs/NodeReference"),
        );
        let data_import = relative_import(
            file_path.parent().unwrap(),
            &docs_src.join("generated/catalog-nodes"),
        );
        let title = format!("{} Node Catalog", category.label);
        let description = category_seo_description(category);
        let head = frontmatter_head(
            &category_keywords(category),
            Some(&display_category_path(&category.path)),
            &collection_page_schema(
                &title,
                &description,
                &format!("https://docs.flow-like.com/{}/", category.slug),
                category,
            ),
        );
        let category_prop = format!("{{{}}}", json_string(&category.path));
        let content = format!(
            "---\ntitle: {}\ndescription: {}\nsidebar:\n  order: 0\n  label: {}\n{}---\n\n\
             import {{ NodeCategoryOverview }} from {};\n\
             import {{ nodesByCategory }} from {};\n\n\
             <NodeCategoryOverview\n\
             \tcategory={}\n\
             \tlabel={}\n\
             \tnodes={{nodesByCategory[{}] ?? []}}\n\
             \tclient:load\n\
             />\n",
            json_string(&title),
            json_string(&description),
            json_string(&category.label),
            head,
            json_string(&component_import),
            json_string(&data_import),
            category_prop,
            json_string(&category.label),
            json_string(&category.path),
        );

        output.write(file_path, content)?;
    }

    Ok(())
}

// Keep the setup guide in generated references after catalog regeneration.
const TYPED_DECISION_GUIDE: &str = r#"Use Typed Decision to choose a label, score text against a rubric, or estimate whether a statement holds. **Model** selects the weights while the question inputs and result shape stay the same. Existing Laya nodes keep Laya as their default model.

## Choose model weights

This node runs ONNX models. The dropdown includes these models:

| Model | First use |
| --- | --- |
| `mizchi/laya-multilingual-onnx` | Downloads missing files, about 681 MB for the complete bundle. Default for existing flows. |
| `fastino/GLiNER2.5-Decide` | Downloads a pinned community ONNX conversion, about 1.76 GB. |
| `fastino/GLiNER2.5-multi-Decide` | Downloads a verified ONNX export from the Flow-Like CDN, about 1.13 GB. |
| `fastino/GLiNER2.5-Decide-1B` | Downloads a verified ONNX export from the Flow-Like CDN, about 4.14 GB. |
| `fastino/gliner2.5-multi-v1` | Downloads a pinned community ONNX conversion, about 1.13 GB. |
| `fastino/gliner2.5-base-v1` | Downloads a pinned community ONNX conversion, about 749 MB. |
| `fastino/gliner2.5-small-v1` | Downloads a pinned community ONNX conversion, about 293 MB. |
| `custom` | Loads a complete bundle that you provide. Missing files produce an error. |

The Decide preset uses the `nishparadox/gliner2.5-decide-onnx` float32 conversion. The small, base, and multilingual v1 presets use the encoder and classifier from `codesoda/gliner2-onnx`. Multi-Decide and Decide-1B use Flow-Like ONNX exports from the CDN. Download URLs use fixed revisions, and downloaded files are checked against SHA-256 hashes.

Connect **Model Directory** to a `FlowPath`, a reference to a directory in your flow's storage. Downloadable GLiNER models use a separate `.decision-cache/<model-name>` directory for each preset. Existing cached files are reused. Laya keeps its `.laya-cache` layout and also recognizes its older cache layout. Loaded model sessions are reused within an execution cache.

## Ask a question

1. Set **Text** to the content or serialized JSON state to evaluate.
2. Write the question in **Instructions**.
3. Select **Question Type** and fill in its inputs:

| Question type | Inputs | Answer |
| --- | --- | --- |
| `choice` | **Choices**: unique labels, such as `billing` and `technical support`. | **Choice**: the selected label. |
| `score` | **Levels**: rubric descriptions in order, starting at level zero. | **Score**: the expected rubric level, which can be fractional. |
| `noul` | Optional **False Description** and **True Description**. | **P(True)**: the probability that the statement holds. |

The result also contains option probabilities, confidence, and the number of input tokens. `act_probability` contains Laya's separate action-head probability and is `null` for GLiNER. Confidence summarizes the option distribution; it does not measure real-world accuracy.

## Export weights

All presets download automatically. To export a different checkpoint or your own fine-tuned weights, use the export tool from the Flow-Like repository. With [uv installed](https://docs.astral.sh/uv/getting-started/installation/), run this from the repository root:

```bash
uv run --python 3.12 tools/export-decision-model.py \
  --model fastino/GLiNER2.5-multi-Decide \
  --output ./decision-model
```

Use any supported GLiNER2 checkpoint or local checkpoint directory as `--model`. The output directory must not already exist. The tool installs its Python dependencies, exports on CPU in float32, and checks the exported result before publishing the bundle. Exporting needs several times the checkpoint size in memory and disk space; the 1B model needs multiple gigabytes.

Place the resulting directory in storage accessible to the flow and connect its `FlowPath` to **Model Directory**. Choose `custom` in **Model**. A local bundle can also override a preset when its `source_model` matches that preset.

## Use custom weights

Select `custom` and point **Model Directory** at your complete bundle. The node detects the format from its configuration and performs no model downloads.

- A GLiNER classification export contains `decision_config.json`, `model.onnx`, `tokenizer.json`, and `model.onnx_data` when the export uses external tensor data. The export tool also accepts a local GLiNER2 checkpoint directory as `--model`.
- A Laya bundle contains `model.onnx`, `rl_agent_config.json`, and either `tokenizer/tokenizer.json` or `tokenizer.json`.

For compatibility, saved flows retain the internal node name `onnx_laya` and the FlowScript name `onnx::laya`.

"#;

fn write_node_pages(
    docs_src: &Path,
    docs_root: &Path,
    nodes: &[DocNode],
    output: &mut GeneratedOutput,
) -> Result<(), Box<dyn Error>> {
    for node in nodes {
        let file_path = docs_root.join(format!("{}.mdx", node.slug));

        let component_import = relative_import(
            file_path.parent().unwrap(),
            &docs_src.join("components/node-docs/NodeReference"),
        );
        let data_import = relative_import(
            file_path.parent().unwrap(),
            &docs_src.join("generated/catalog-nodes"),
        );
        let node_prop = format!("{{nodesBySlug[{}]}}", json_string(&node.slug));
        let guide = if node.name == "onnx_laya" {
            TYPED_DECISION_GUIDE
        } else {
            ""
        };
        let title = node_page_title(&node.friendly_name);
        let description = node_seo_description(node);
        let head = frontmatter_head(
            &node_keywords(node),
            Some(&display_category_path(&node.category)),
            &tech_article_schema(
                &title,
                &description,
                &format!("https://docs.flow-like.com/{}/", node.slug),
                node,
            ),
        );
        let content = format!(
            "---\ntitle: {}\ndescription: {}\nsidebar:\n  label: {}\n{}---\n\n\
             import {{ NodeReference }} from {};\n\
             import {{ nodesBySlug }} from {};\n\n\
             {guide}<NodeReference node={} />\n",
            json_string(&title),
            json_string(&description),
            json_string(&node.friendly_name),
            head,
            json_string(&component_import),
            json_string(&data_import),
            node_prop,
        );

        output.write(file_path, content)?;
    }

    Ok(())
}

fn generated_header() -> &'static str {
    "// Generated by `cargo run -p flow-like-catalog --bin generate_node_docs`.\n// Do not edit by hand."
}

fn string_field(obj: &Map<String, Value>, field: &str) -> String {
    opt_string_field(obj, field).unwrap_or_default()
}

fn opt_string_field(obj: &Map<String, Value>, field: &str) -> Option<String> {
    obj.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn opt_u64_field(obj: &Map<String, Value>, field: &str) -> Option<u64> {
    obj.get(field).and_then(Value::as_u64)
}

fn opt_bool_field(obj: &Map<String, Value>, field: &str) -> bool {
    obj.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn non_empty_or(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn node_seo_description(node: &DocNode) -> String {
    let description = with_terminal_punctuation(&seo_detail(&node.description));
    if description.is_empty() {
        return bounded_description(
            &[
                format!("{} Flow-Like node reference.", node.friendly_name),
                node_metadata_summary(node),
            ],
            158,
        );
    }

    bounded_description(
        &[
            format!("{} Flow-Like node: {}", node.friendly_name, description),
            node_metadata_summary(node),
        ],
        158,
    )
}

fn node_metadata_summary(node: &DocNode) -> String {
    let has_schemas = node.pins.iter().any(|pin| pin.schema.is_some());
    match (has_schemas, node.scores.is_some()) {
        (true, true) => {
            "Includes pin details, schemas, package metadata, and risk ratings.".to_string()
        }
        (true, false) => "Includes pin details, schemas, and package metadata.".to_string(),
        (false, true) => "Includes pin details, package metadata, and risk ratings.".to_string(),
        (false, false) => "Includes pin details and package metadata.".to_string(),
    }
}

fn seo_detail(value: &str) -> String {
    let value = compact_whitespace(value);
    if value.is_empty() {
        return value;
    }

    for (index, delimiter) in value.char_indices() {
        if !matches!(delimiter, '.' | '!' | '?') || !is_sentence_boundary(&value, index, delimiter)
        {
            continue;
        }

        let sentence = value[..=index].trim();
        if sentence.chars().count() >= 24 {
            return sentence.to_string();
        }
    }

    value
}

fn is_sentence_boundary(value: &str, index: usize, delimiter: char) -> bool {
    if delimiter == '.' {
        let prefix = value[..=index].to_ascii_lowercase();
        if prefix.ends_with("e.g.") || prefix.ends_with("i.e.") {
            return false;
        }
    }

    let remainder = &value[index + delimiter.len_utf8()..];
    remainder.is_empty() || remainder.chars().next().is_some_and(char::is_whitespace)
}

fn node_page_title(friendly_name: &str) -> String {
    let friendly_name = compact_whitespace(friendly_name);
    if friendly_name
        .split_whitespace()
        .last()
        .is_some_and(|word| word.eq_ignore_ascii_case("node"))
    {
        friendly_name
    } else {
        format!("{friendly_name} Node")
    }
}

fn with_terminal_punctuation(value: &str) -> String {
    let value = value.trim();
    if value.is_empty()
        || value.ends_with('.')
        || value.ends_with('!')
        || value.ends_with('?')
        || value.ends_with(':')
    {
        value.to_string()
    } else {
        format!("{}.", value)
    }
}

fn category_seo_description(category: &DocCategory) -> String {
    bounded_description(
        &[
            format!(
                "{} Flow-Like node category with {} generated node reference{}.",
                display_category_path(&category.path),
                category.count,
                if category.count == 1 { "" } else { "s" }
            ),
            "Browse pin details plus available schema, package, and risk-rating metadata for workflow automation."
                .to_string(),
        ],
        158,
    )
}

fn bounded_description(parts: &[String], max_chars: usize) -> String {
    let mut out = String::new();

    for part in parts {
        let part = compact_whitespace(part);
        if part.is_empty() {
            continue;
        }

        let candidate = if out.is_empty() {
            part.clone()
        } else {
            format!("{} {}", out, part)
        };

        if candidate.chars().count() <= max_chars {
            out = candidate;
        } else if out.is_empty() {
            out = truncate_at_word(&part, max_chars);
            break;
        }
    }

    if out.is_empty() {
        "Generated Flow-Like node catalog documentation with pin details and available schema, package, and risk-rating metadata."
            .to_string()
    } else {
        out
    }
}

fn truncate_at_word(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    if max_chars <= 3 {
        return value.chars().take(max_chars).collect();
    }

    let mut out = value.chars().take(max_chars - 3).collect::<String>();
    if let Some(index) = out.rfind(|ch: char| ch.is_whitespace())
        && index > max_chars / 2
    {
        out.truncate(index);
    }

    format!(
        "{}...",
        out.trim_end_matches(|ch: char| {
            ch.is_whitespace() || ch == ',' || ch == ';' || ch == ':' || ch == '.' || ch == '-'
        })
    )
}

fn compact_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn node_keywords(node: &DocNode) -> Vec<String> {
    let mut keywords = Vec::new();
    push_keyword(&mut keywords, &node.friendly_name);
    push_keyword(&mut keywords, &format!("{} node", node.friendly_name));
    push_keyword(&mut keywords, "Flow-Like node");
    push_keyword(&mut keywords, "Flow-Like node catalog");
    push_keyword(&mut keywords, "workflow automation node");
    push_keyword(&mut keywords, &display_category_path(&node.category));
    push_keyword(&mut keywords, &node.package_name);
    push_keyword(&mut keywords, &node.name);

    for segment in &node.category_path {
        push_keyword(&mut keywords, &display_label(segment));
    }

    push_keyword(&mut keywords, "pins");
    push_keyword(&mut keywords, "schemas");
    push_keyword(&mut keywords, "security rating");
    keywords.truncate(18);
    keywords
}

fn category_keywords(category: &DocCategory) -> Vec<String> {
    let mut keywords = Vec::new();
    push_keyword(&mut keywords, &format!("{} nodes", category.label));
    push_keyword(&mut keywords, &display_category_path(&category.path));
    push_keyword(&mut keywords, "Flow-Like node catalog");
    push_keyword(&mut keywords, "Flow-Like nodes");
    push_keyword(&mut keywords, "workflow automation");
    push_keyword(&mut keywords, "node reference");
    push_keyword(&mut keywords, "pins");
    push_keyword(&mut keywords, "schemas");
    push_keyword(&mut keywords, "security ratings");
    keywords
}

fn push_keyword(keywords: &mut Vec<String>, value: &str) {
    let value = compact_whitespace(value);
    if value.is_empty() {
        return;
    }

    if !keywords
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&value))
    {
        keywords.push(value);
    }
}

fn frontmatter_head(keywords: &[String], section: Option<&str>, structured_data: &Value) -> String {
    let mut head = String::from("head:\n");

    if !keywords.is_empty() {
        head.push_str("  - tag: meta\n    attrs:\n      name: keywords\n      content: ");
        head.push_str(&json_string(&keywords.join(", ")));
        head.push('\n');
    }

    if let Some(section) = section {
        head.push_str(
            "  - tag: meta\n    attrs:\n      property: article:section\n      content: ",
        );
        head.push_str(&json_string(section));
        head.push('\n');
    }

    head.push_str("  - tag: script\n    attrs:\n      type: application/ld+json\n    content: ");
    head.push_str(&json_string(&structured_data.to_string()));
    head.push('\n');
    head
}

fn tech_article_schema(title: &str, description: &str, url: &str, node: &DocNode) -> Value {
    json!({
        "@context": "https://schema.org",
        "@type": "TechArticle",
        "headline": title,
        "name": node.friendly_name,
        "description": description,
        "url": url,
        "isPartOf": {
            "@type": "CollectionPage",
            "name": "Flow-Like Node Catalog",
            "url": "https://docs.flow-like.com/nodes/overview/"
        },
        "about": display_category_path(&node.category),
        "keywords": node_keywords(node).join(", "),
        "programmingLanguage": "Rust",
        "mainEntity": {
            "@type": "SoftwareSourceCode",
            "name": node.friendly_name,
            "description": compact_whitespace(&node.description),
            "codeSampleType": "Flow-Like node",
            "runtimePlatform": "Flow-Like",
            "programmingLanguage": "Rust"
        }
    })
}

fn collection_page_schema(
    title: &str,
    description: &str,
    url: &str,
    category: &DocCategory,
) -> Value {
    json!({
        "@context": "https://schema.org",
        "@type": "CollectionPage",
        "name": title,
        "description": description,
        "url": url,
        "isPartOf": {
            "@type": "CollectionPage",
            "name": "Flow-Like Node Catalog",
            "url": "https://docs.flow-like.com/nodes/overview/"
        },
        "about": display_category_path(&category.path),
        "keywords": category_keywords(category).join(", ")
    })
}

fn display_category_path(path: &str) -> String {
    category_segments(path)
        .into_iter()
        .map(|segment| display_label(&segment))
        .collect::<Vec<_>>()
        .join("/")
}

fn display_label(value: &str) -> String {
    let words = value
        .trim()
        .split(|ch: char| ch == '-' || ch == '_' || ch == '.' || ch.is_whitespace())
        .filter(|word| !word.is_empty())
        .map(display_word)
        .collect::<Vec<_>>();

    if words.is_empty() {
        "Nodes".to_string()
    } else {
        words.join(" ")
    }
}

fn display_word(word: &str) -> String {
    match word.to_ascii_lowercase().as_str() {
        "ai" => return "AI".to_string(),
        "api" => return "API".to_string(),
        "csv" => return "CSV".to_string(),
        "datafusion" => return "DataFusion".to_string(),
        "github" => return "GitHub".to_string(),
        "h3" => return "H3".to_string(),
        "html" => return "HTML".to_string(),
        "http" => return "HTTP".to_string(),
        "imap" => return "IMAP".to_string(),
        "json" => return "JSON".to_string(),
        "kg" => return "KG".to_string(),
        "llm" => return "LLM".to_string(),
        "mcp" => return "MCP".to_string(),
        "ml" => return "ML".to_string(),
        "oauth" => return "OAuth".to_string(),
        "ocr" => return "OCR".to_string(),
        "onnx" => return "ONNX".to_string(),
        "pdf" => return "PDF".to_string(),
        "rest" => return "REST".to_string(),
        "smtp" => return "SMTP".to_string(),
        "sql" => return "SQL".to_string(),
        "stt" => return "STT".to_string(),
        "tcp" => return "TCP".to_string(),
        "tls" => return "TLS".to_string(),
        "tsv" => return "TSV".to_string(),
        "tts" => return "TTS".to_string(),
        "udp" => return "UDP".to_string(),
        "ui" => return "UI".to_string(),
        "url" => return "URL".to_string(),
        "websocket" => return "WebSocket".to_string(),
        "xai" => return "xAI".to_string(),
        _ => {}
    }

    if word.chars().any(|ch| ch.is_ascii_uppercase()) {
        return word.to_string();
    }

    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn category_segments(category: &str) -> Vec<String> {
    let segments = category
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if segments.is_empty() {
        vec!["Uncategorized".to_string()]
    } else {
        segments
    }
}

fn safe_segments(segments: &[String]) -> Vec<String> {
    segments
        .iter()
        .map(|segment| safe_segment(segment))
        .collect()
}

fn safe_segment(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;

    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if ch == ' ' || ch == '_' || ch == '-' || ch == '.' {
            if !last_was_dash && !out.is_empty() {
                out.push('-');
                last_was_dash = true;
            }
        } else if !last_was_dash && !out.is_empty() {
            out.push('-');
            last_was_dash = true;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }

    if out.is_empty() {
        "node".to_string()
    } else {
        out
    }
}

fn pin_list(value: Option<&Value>) -> Vec<DocPin> {
    let mut pins = value
        .and_then(Value::as_object)
        .map(|pins| pins.values().filter_map(doc_pin).collect::<Vec<_>>())
        .unwrap_or_default();

    pins.sort_by(|a, b| {
        pin_type_order(&a.pin_type)
            .cmp(&pin_type_order(&b.pin_type))
            .then_with(|| a.index.cmp(&b.index))
            .then_with(|| a.friendly_name.cmp(&b.friendly_name))
            .then_with(|| a.name.cmp(&b.name))
    });

    pins
}

fn doc_pin(value: &Value) -> Option<DocPin> {
    let obj = value.as_object()?;
    Some(DocPin {
        name: string_field(obj, "name"),
        friendly_name: string_field(obj, "friendly_name"),
        description: string_field(obj, "description"),
        pin_type: string_field(obj, "pin_type"),
        data_type: string_field(obj, "data_type"),
        value_type: string_field(obj, "value_type"),
        schema: opt_string_field(obj, "schema"),
        default_value: decoded_default_value(obj.get("default_value")),
        index: opt_u64_field(obj, "index").unwrap_or(0),
        options: pin_options(obj.get("options")),
    })
}

fn pin_type_order(value: &str) -> u8 {
    match value {
        "Input" => 0,
        "Output" => 1,
        _ => 2,
    }
}

fn decoded_default_value(value: Option<&Value>) -> Option<Value> {
    let bytes = value?.as_array()?;
    if bytes.is_empty() {
        return None;
    }

    let bytes = bytes
        .iter()
        .map(|value| value.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect::<Option<Vec<_>>>()?;

    serde_json::from_slice(&bytes)
        .ok()
        .or_else(|| Some(json!("<binary default>")))
}

fn pin_options(value: Option<&Value>) -> Option<DocPinOptions> {
    let obj = value?.as_object()?;
    let options = DocPinOptions {
        sensitive: obj.get("sensitive").and_then(Value::as_bool),
        valid_values: string_array(obj.get("valid_values")),
        range: range(obj.get("range")),
        step: obj.get("step").and_then(Value::as_f64),
        enforce_schema: obj.get("enforce_schema").and_then(Value::as_bool),
        enforce_generic_value_type: obj
            .get("enforce_generic_value_type")
            .and_then(Value::as_bool),
    };

    if options.sensitive.is_none()
        && options.valid_values.is_none()
        && options.range.is_none()
        && options.step.is_none()
        && options.enforce_schema.is_none()
        && options.enforce_generic_value_type.is_none()
    {
        None
    } else {
        Some(options)
    }
}

fn range(value: Option<&Value>) -> Option<[f64; 2]> {
    let values = value?.as_array()?;
    if values.len() != 2 {
        return None;
    }

    Some([values[0].as_f64()?, values[1].as_f64()?])
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    let values = value?.as_array()?;
    let out = values
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if out.is_empty() { None } else { Some(out) }
}

fn string_array_map(value: Option<&Value>) -> BTreeMap<String, Vec<String>> {
    value
        .and_then(Value::as_object)
        .map(|obj| {
            obj.iter()
                .filter_map(|(key, value)| {
                    string_array(Some(value)).map(|scopes| (key.clone(), scopes))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn scores(value: Option<&Value>) -> Option<DocScores> {
    let obj = value?.as_object()?;
    Some(DocScores {
        privacy: u8_field(obj, "privacy"),
        security: u8_field(obj, "security"),
        performance: u8_field(obj, "performance"),
        governance: u8_field(obj, "governance"),
        reliability: u8_field(obj, "reliability"),
        cost: u8_field(obj, "cost"),
    })
}

fn u8_field(obj: &Map<String, Value>, field: &str) -> u8 {
    obj.get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .unwrap_or(0)
}

fn fn_refs(value: Option<&Value>) -> Option<DocFnRefs> {
    let obj = value?.as_object()?;
    Some(DocFnRefs {
        fn_refs: string_array(obj.get("fn_refs")).unwrap_or_default(),
        can_reference_fns: opt_bool_field(obj, "can_reference_fns"),
        can_be_referenced_by_fns: opt_bool_field(obj, "can_be_referenced_by_fns"),
    })
}

fn permissions(obj: &Map<String, Value>) -> Vec<String> {
    obj.get("wasm")
        .and_then(Value::as_object)
        .and_then(|wasm| string_array(wasm.get("permissions")))
        .unwrap_or_default()
}

fn node_flags(obj: &Map<String, Value>, has_oauth: bool, has_permissions: bool) -> Vec<String> {
    let mut flags = Vec::new();
    if opt_bool_field(obj, "only_offline") {
        flags.push("Local only".to_string());
    }
    if opt_bool_field(obj, "long_running") {
        flags.push("Long running".to_string());
    }
    if opt_bool_field(obj, "event_callback") {
        flags.push("Event callback".to_string());
    }
    if opt_bool_field(obj, "start") {
        flags.push("Start node".to_string());
    }
    if has_oauth {
        flags.push("OAuth".to_string());
    }
    if has_permissions {
        flags.push("Sandbox permissions".to_string());
    }
    if obj.get("wasm").is_some_and(|value| !value.is_null()) {
        flags.push("WASM".to_string());
    }
    flags
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("serialize string")
}

fn relative_import(from_dir: &Path, to: &Path) -> String {
    let from = normalized_components(from_dir);
    let to = normalized_components(to);
    let shared = from
        .iter()
        .zip(to.iter())
        .take_while(|(left, right)| left == right)
        .count();

    let mut parts = Vec::new();
    for _ in shared..from.len() {
        parts.push("..".to_string());
    }
    for part in &to[shared..] {
        parts.push(part.clone());
    }

    let path = if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    };

    if path.starts_with('.') {
        path
    } else {
        format!("./{path}")
    }
}

fn normalized_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".to_string()),
            Component::CurDir => None,
            Component::RootDir | Component::Prefix(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{GeneratedOutput, node_page_title, seo_detail};
    use std::{fs, path::PathBuf, time::SystemTime};

    struct OutputFixture(PathBuf);

    impl OutputFixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "flow-like-node-docs-{}-{}",
                std::process::id(),
                SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()
            )))
        }
    }

    impl Drop for OutputFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn check_reports_drift_without_changing_files() {
        let fixture = OutputFixture::new();
        let root = &fixture.0;
        let mut writer = GeneratedOutput::default();
        for (name, content) in [
            ("same.mdx", "same"),
            ("changed.mdx", "before"),
            ("old/removed.mdx", "obsolete"),
            ("overview.mdx", "authored overview"),
        ] {
            writer.write(root.join(name), content.to_string()).unwrap();
        }

        let mut check = GeneratedOutput {
            check: true,
            ..Default::default()
        };
        check
            .write(root.join("same.mdx"), "same".to_string())
            .unwrap();
        check
            .write(root.join("changed.mdx"), "after".to_string())
            .unwrap();
        check
            .write(root.join("missing/new.mdx"), "new".to_string())
            .unwrap();
        check.find_obsolete(root, root).unwrap();
        assert_eq!(check.differences.len(), 3);
        assert_eq!(check.differences[&root.join("changed.mdx")], "changed");
        assert_eq!(check.differences[&root.join("missing/new.mdx")], "missing");
        assert_eq!(check.differences[&root.join("old/removed.mdx")], "obsolete");
        assert_eq!(
            fs::read_to_string(root.join("changed.mdx")).unwrap(),
            "before"
        );
        assert_eq!(
            fs::read_to_string(root.join("old/removed.mdx")).unwrap(),
            "obsolete"
        );
        assert_eq!(
            fs::read_to_string(root.join("overview.mdx")).unwrap(),
            "authored overview"
        );
        assert!(!root.join("missing").exists());
    }

    #[test]
    fn check_accepts_identical_output_and_never_creates_a_missing_root() {
        let fixture = OutputFixture::new();
        let mut check = GeneratedOutput {
            check: true,
            ..Default::default()
        };
        check
            .write(fixture.0.join("new.mdx"), "new".to_string())
            .unwrap();
        check.find_obsolete(&fixture.0, &fixture.0).unwrap();
        assert!(!fixture.0.exists());

        GeneratedOutput::default()
            .write(fixture.0.join("new.mdx"), "new".to_string())
            .unwrap();
        let mut check = GeneratedOutput {
            check: true,
            ..Default::default()
        };
        check
            .write(fixture.0.join("new.mdx"), "new".to_string())
            .unwrap();
        check.find_obsolete(&fixture.0, &fixture.0).unwrap();
        assert!(check.differences.is_empty());
    }

    #[test]
    fn seo_detail_ignores_non_sentence_periods() {
        assert_eq!(
            seo_detail("Convert Image Color/Pixel Type (e.g. to Grayscale)"),
            "Convert Image Color/Pixel Type (e.g. to Grayscale)"
        );
        assert_eq!(
            seo_detail("Creates typed video options for fal.ai video models."),
            "Creates typed video options for fal.ai video models."
        );
        assert_eq!(
            seo_detail(
                "Connect to Jira and Confluence using OAuth 2.0. Requires OAuth provider configuration."
            ),
            "Connect to Jira and Confluence using OAuth 2.0."
        );
    }

    #[test]
    fn node_page_title_does_not_repeat_node_suffix() {
        assert_eq!(node_page_title("Delay"), "Delay Node");
        assert_eq!(node_page_title("Upsert Graph Node"), "Upsert Graph Node");
        assert_eq!(node_page_title("Custom NODE"), "Custom NODE");
    }
}
