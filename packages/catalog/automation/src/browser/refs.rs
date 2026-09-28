#[cfg(feature = "execute")]
use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(any(feature = "execute", test))]
pub(crate) const STALE_REF_ERROR: &str = "Stale element ref '{ref}' — take a new browser snapshot";

#[cfg(any(feature = "execute", test))]
pub(crate) fn stale_ref_error(reference: &str) -> flow_like_types::Error {
    flow_like_types::anyhow!("{}", STALE_REF_ERROR.replace("{ref}", reference))
}

#[cfg(feature = "execute")]
pub(crate) fn is_stale_ref_error(error: &flow_like_types::Error) -> bool {
    let prefix = STALE_REF_ERROR.split("{ref}").next().unwrap_or_default();
    error
        .chain()
        .any(|cause| cause.to_string().starts_with(prefix))
}

/// One line of a browser snapshot.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct SnapshotElement {
    /// Ref usable as a `Ref` selector until the document changes.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub element_ref: Option<String>,
    /// ARIA role; `text` for plain text lines.
    pub role: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// focused, disabled, checked, checked=mixed, pressed, expanded, collapsed, selected,
    /// required, readonly, invalid, cursor=pointer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
    pub depth: u32,
}

#[cfg(any(feature = "execute", test))]
pub(crate) mod tree {
    use super::SnapshotElement;
    use flow_like_types::Value;
    use serde::Deserialize;
    use std::collections::{HashMap, HashSet};

    const MAX_RAW_DEPTH: usize = 768;
    const NAME_LIMIT: usize = 150;
    const CONTENT_LIMIT: usize = 300;

    #[derive(Deserialize, Debug, Clone, Default)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct AxNode {
        pub node_id: String,
        #[serde(default)]
        pub ignored: bool,
        #[serde(default)]
        pub ignored_reasons: Vec<AxProperty>,
        pub role: Option<AxValue>,
        pub name: Option<AxValue>,
        pub description: Option<AxValue>,
        pub value: Option<AxValue>,
        #[serde(default)]
        pub properties: Vec<AxProperty>,
        pub parent_id: Option<String>,
        #[serde(default)]
        pub child_ids: Vec<String>,
        #[serde(rename = "backendDOMNodeId")]
        pub backend_dom_node_id: Option<i64>,
    }

    #[derive(Deserialize, Debug, Clone, Default)]
    pub(crate) struct AxValue {
        #[serde(default)]
        pub value: Value,
    }

    #[derive(Deserialize, Debug, Clone, Default)]
    pub(crate) struct AxProperty {
        pub name: String,
        #[serde(default)]
        pub value: AxValue,
    }

    impl AxValue {
        fn text(&self) -> String {
            match &self.value {
                Value::String(text) => normalize(text),
                Value::Number(number) => number.to_string(),
                Value::Bool(flag) => flag.to_string(),
                _ => String::new(),
            }
        }
    }

    impl AxNode {
        fn role(&self) -> &str {
            self.role
                .as_ref()
                .and_then(|role| role.value.as_str())
                .unwrap_or_default()
        }

        fn text_of(value: &Option<AxValue>) -> String {
            value.as_ref().map(AxValue::text).unwrap_or_default()
        }

        fn property(&self, name: &str) -> Option<&Value> {
            self.properties
                .iter()
                .find(|property| property.name == name)
                .map(|property| &property.value.value)
        }

        fn hidden(&self) -> bool {
            self.ignored_reasons.iter().any(|reason| {
                matches!(
                    reason.name.as_str(),
                    "ariaHiddenElement"
                        | "ariaHiddenSubtree"
                        | "notRendered"
                        | "notVisible"
                        | "inertElement"
                        | "inertSubtree"
                        | "activeModalDialog"
                )
            })
        }
    }

    pub(crate) struct AxTree {
        nodes: Vec<AxNode>,
        index: HashMap<String, usize>,
    }

    impl AxTree {
        pub fn new(nodes: Vec<AxNode>) -> Self {
            let index = nodes
                .iter()
                .enumerate()
                .map(|(position, node)| (node.node_id.clone(), position))
                .collect();
            Self { nodes, index }
        }

        pub fn from_response(response: &Value) -> flow_like_types::Result<Self> {
            let nodes: Vec<AxNode> =
                flow_like_types::json::from_value(response["nodes"].clone()).map_err(|error| {
                    flow_like_types::anyhow!("Failed to parse accessibility tree: {error}")
                })?;
            Ok(Self::new(nodes))
        }

        pub fn root(&self) -> Option<usize> {
            self.nodes
                .iter()
                .position(|node| node.parent_id.is_none())
                .or((!self.nodes.is_empty()).then_some(0))
        }

        pub fn find_backend(&self, backend: i64) -> Option<usize> {
            self.nodes
                .iter()
                .position(|node| node.role() != "InlineTextBox" && node.backend_dom_node_id == Some(backend))
        }

        pub fn title(&self) -> String {
            self.root()
                .map(|root| AxNode::text_of(&self.nodes[root].name))
                .unwrap_or_default()
        }

        fn subtree_text(&self, index: usize, limit: usize) -> String {
            let mut parts = Vec::new();
            let mut length = 0;
            let mut stack = vec![(index, 0usize)];
            while let Some((current, depth)) = stack.pop() {
                if length >= limit || depth > MAX_RAW_DEPTH {
                    continue;
                }
                let node = &self.nodes[current];
                if node.role() == "StaticText" {
                    let text = AxNode::text_of(&node.name);
                    if !text.is_empty() {
                        length += text.len() + 1;
                        parts.push(text);
                    }
                    continue;
                }
                for child in node.child_ids.iter().rev() {
                    if let Some(&child) = self.index.get(child) {
                        stack.push((child, depth + 1));
                    }
                }
            }
            clip(&parts.join(" "), limit)
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Class {
        Skip,
        Root,
        Text,
        Frame,
        Generic,
        Interactive,
        Structural,
    }

    fn classify(role: &str) -> Class {
        match role {
            "InlineTextBox" | "ListMarker" | "LineBreak" => Class::Skip,
            "RootWebArea" | "WebArea" => Class::Root,
            "StaticText" => Class::Text,
            "Iframe" | "IframePresentational" => Class::Frame,
            "" | "generic" | "none" | "presentation" | "GenericContainer" | "LabelText"
            | "LayoutTable" | "LayoutTableRow" | "LayoutTableCell" | "LayoutTableColumn"
            | "strong" | "emphasis" | "code" | "mark" | "subscript" | "superscript" | "time"
            | "Abbr" | "deletion" | "insertion" | "Ruby" | "Section" | "MenuListPopup" => {
                Class::Generic
            }
            "button" | "link" | "textbox" | "searchbox" | "checkbox" | "radio" | "combobox"
            | "listbox" | "option" | "menuitem" | "menuitemcheckbox" | "menuitemradio"
            | "tab" | "switch" | "slider" | "spinbutton" | "treeitem" | "DisclosureTriangle"
            | "ColorWell" | "Date" | "DateTime" | "InputTime" | "PopUpButton"
            | "ToggleButton" | "MenuListOption" => Class::Interactive,
            _ => Class::Structural,
        }
    }

    fn is_leaf(role: &str) -> bool {
        matches!(
            role,
            "textbox"
                | "searchbox"
                | "spinbutton"
                | "slider"
                | "ColorWell"
                | "Date"
                | "DateTime"
                | "InputTime"
                | "img"
                | "image"
                | "meter"
                | "progressbar"
        )
    }

    pub(crate) fn normalize(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    pub(crate) fn clip(text: &str, limit: usize) -> String {
        if text.chars().count() <= limit {
            return text.to_string();
        }
        let mut clipped: String = text.chars().take(limit.saturating_sub(1)).collect();
        clipped.push('…');
        clipped
    }

    fn truthy(value: &Value) -> Option<&str> {
        match value {
            Value::Bool(true) => Some("true"),
            Value::Bool(false) => Some("false"),
            Value::String(text) => Some(text.as_str()),
            _ => None,
        }
    }

    fn states(node: &AxNode) -> Vec<String> {
        let mut states = Vec::new();
        for property in &node.properties {
            let Some(value) = truthy(&property.value.value) else {
                continue;
            };
            let state = match (property.name.as_str(), value) {
                ("focused", "true") => "focused",
                ("disabled", "true") => "disabled",
                ("checked", "true") => "checked",
                ("checked", "mixed") => "checked=mixed",
                ("pressed", "true") => "pressed",
                ("pressed", "mixed") => "pressed=mixed",
                ("expanded", "true") => "expanded",
                ("expanded", "false") => "collapsed",
                ("selected", "true") => "selected",
                ("required", "true") => "required",
                ("readonly", "true") => "readonly",
                ("invalid", "true" | "grammar" | "spelling") => "invalid",
                _ => continue,
            };
            states.push(state.to_string());
        }
        states
    }

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) struct OutNode {
        pub element: SnapshotElement,
        pub backend_id: Option<i64>,
        pub frame_path: Vec<i64>,
        pub children: Vec<OutNode>,
    }

    pub(crate) struct WalkOptions {
        pub interactive_only: bool,
        pub max_depth: Option<u32>,
        pub clickable: HashSet<i64>,
    }

    #[derive(Clone, Copy)]
    struct Position<'p> {
        depth: u32,
        raw_depth: usize,
        ancestor_name: Option<&'p str>,
        inside_interactive: bool,
    }

    pub(crate) struct Walker<'a> {
        options: &'a WalkOptions,
        frames: &'a HashMap<i64, AxTree>,
        visited: HashSet<i64>,
    }

    impl<'a> Walker<'a> {
        pub fn new(options: &'a WalkOptions, frames: &'a HashMap<i64, AxTree>) -> Self {
            Self {
                options,
                frames,
                visited: HashSet::new(),
            }
        }

        pub fn walk(&mut self, tree: &AxTree, index: usize, frame_path: &[i64]) -> Vec<OutNode> {
            let mut out = Vec::new();
            let position = Position {
                depth: 0,
                raw_depth: 0,
                ancestor_name: None,
                inside_interactive: false,
            };
            self.visit(tree, index, frame_path, position, &mut out);
            out
        }

        fn visit_children(
            &mut self,
            tree: &AxTree,
            index: usize,
            frame_path: &[i64],
            position: Position<'_>,
            out: &mut Vec<OutNode>,
        ) {
            for child in &tree.nodes[index].child_ids {
                if let Some(&child) = tree.index.get(child) {
                    self.visit(tree, child, frame_path, position, out);
                }
            }
        }

        fn visit(
            &mut self,
            tree: &AxTree,
            index: usize,
            frame_path: &[i64],
            position: Position<'_>,
            out: &mut Vec<OutNode>,
        ) {
            if position.raw_depth > MAX_RAW_DEPTH {
                return;
            }
            let node = &tree.nodes[index];
            let class = classify(node.role());
            if class == Class::Skip {
                return;
            }
            if let Some(backend) = node.backend_dom_node_id
                && !self.visited.insert(backend)
            {
                return;
            }
            let deeper = Position {
                raw_depth: position.raw_depth + 1,
                ..position
            };
            let too_deep = self
                .options
                .max_depth
                .is_some_and(|max_depth| position.depth > max_depth);
            let promotable = !position.inside_interactive
                && !(node.ignored && node.hidden())
                && node
                    .backend_dom_node_id
                    .is_some_and(|backend| self.options.clickable.contains(&backend));
            let class = if node.ignored { Class::Generic } else { class };
            match class {
                Class::Root => self.visit_children(tree, index, frame_path, deeper, out),
                Class::Generic if !promotable => {
                    self.visit_children(tree, index, frame_path, deeper, out)
                }
                Class::Text => {
                    if !self.options.interactive_only && !too_deep {
                        push_text(out, node, position);
                    }
                }
                _ if too_deep => {}
                Class::Structural if self.options.interactive_only => {
                    let nested = Position {
                        depth: position.depth + 1,
                        ..deeper
                    };
                    self.visit_children(tree, index, frame_path, nested, out)
                }
                Class::Frame => self.visit_frame(tree, index, frame_path, position, out),
                _ => self.visit_element(tree, index, frame_path, position, class, out),
            }
        }

        fn visit_frame(
            &mut self,
            tree: &AxTree,
            index: usize,
            frame_path: &[i64],
            position: Position<'_>,
            out: &mut Vec<OutNode>,
        ) {
            let node = &tree.nodes[index];
            let mut children = Vec::new();
            let frames = self.frames;
            if let Some(backend) = node.backend_dom_node_id
                && let Some(child) = frames.get(&backend)
                && let Some(root) = child.root()
            {
                let mut path = frame_path.to_vec();
                path.push(backend);
                let nested = Position {
                    depth: position.depth + 1,
                    raw_depth: position.raw_depth + 1,
                    ancestor_name: None,
                    inside_interactive: false,
                };
                self.visit(child, root, &path, nested, &mut children);
            }
            if self.options.interactive_only {
                out.extend(children);
                return;
            }
            let mut element = self.element(tree, index, "iframe", false);
            element.depth = position.depth;
            out.push(OutNode {
                element,
                backend_id: node.backend_dom_node_id,
                frame_path: frame_path.to_vec(),
                children,
            });
        }

        fn visit_element(
            &mut self,
            tree: &AxTree,
            index: usize,
            frame_path: &[i64],
            position: Position<'_>,
            class: Class,
            out: &mut Vec<OutNode>,
        ) {
            let node = &tree.nodes[index];
            let promoted = class == Class::Generic;
            let role = if promoted { "generic" } else { node.role() };
            let mut element =
                self.element(tree, index, role, promoted || class == Class::Interactive);
            element.depth = position.depth;
            if promoted {
                element.states.push("cursor=pointer".to_string());
            }
            let name = element.name.clone();
            let nested = Position {
                depth: position.depth + 1,
                raw_depth: position.raw_depth + 1,
                ancestor_name: if name.is_empty() {
                    position.ancestor_name
                } else {
                    Some(name.as_str())
                },
                inside_interactive: position.inside_interactive
                    || promoted
                    || class == Class::Interactive,
            };
            let flat = self.options.interactive_only;
            let collapsed_select = flat && matches!(role, "combobox" | "PopUpButton");
            let mut children = Vec::new();
            if !is_leaf(role) && !collapsed_select {
                self.visit_children(tree, index, frame_path, nested, &mut children);
            }
            out.push(OutNode {
                element,
                backend_id: node.backend_dom_node_id,
                frame_path: frame_path.to_vec(),
                children: if flat { Vec::new() } else { std::mem::take(&mut children) },
            });
            out.extend(children);
        }

        fn element(
            &self,
            tree: &AxTree,
            index: usize,
            role: &str,
            name_from_text: bool,
        ) -> SnapshotElement {
            let node = &tree.nodes[index];
            let mut name = AxNode::text_of(&node.name);
            if name.is_empty() && name_from_text {
                name = tree.subtree_text(index, NAME_LIMIT);
            }
            let url = (role == "link")
                .then(|| node.property("url").and_then(Value::as_str))
                .flatten()
                .filter(|url| !url.is_empty())
                .map(str::to_string);
            let value = Some(AxNode::text_of(&node.value))
                .filter(|value| !value.is_empty() && Some(value) != url.as_ref());
            let description = Some(AxNode::text_of(&node.description))
                .filter(|description| !description.is_empty() && *description != name);
            let level = (role == "heading")
                .then(|| node.property("level").and_then(Value::as_u64))
                .flatten()
                .and_then(|level| u32::try_from(level).ok());
            SnapshotElement {
                element_ref: None,
                role: role.to_string(),
                name,
                value,
                description,
                text: None,
                level,
                url,
                states: states(node),
                depth: 0,
            }
        }
    }

    fn push_text(out: &mut Vec<OutNode>, node: &AxNode, position: Position<'_>) {
        let text = AxNode::text_of(&node.name);
        let duplicate = position
            .ancestor_name
            .is_some_and(|name| name.to_lowercase().contains(&text.to_lowercase()));
        if text.is_empty() || duplicate {
            return;
        }
        if let Some(last) = out.last_mut()
            && last.element.role == "text"
            && last.children.is_empty()
        {
            last.element.name.push(' ');
            last.element.name.push_str(&text);
            return;
        }
        out.push(OutNode {
            element: SnapshotElement {
                role: "text".to_string(),
                name: text,
                depth: position.depth,
                ..Default::default()
            },
            backend_id: None,
            frame_path: Vec::new(),
            children: Vec::new(),
        });
    }

    fn quote(text: &str) -> String {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }

    pub(crate) fn element_line(element: &SnapshotElement, indent: u32, has_children: bool) -> String {
        let mut line = format!("{}- ", "  ".repeat(indent as usize));
        if element.role == "text" {
            line.push_str("text: ");
            line.push_str(&clip(&element.name, CONTENT_LIMIT));
            return line;
        }
        line.push_str(&element.role);
        if !element.name.is_empty() {
            line.push(' ');
            line.push_str(&quote(&clip(&element.name, NAME_LIMIT)));
        }
        for state in &element.states {
            line.push_str(&format!(" [{state}]"));
        }
        if let Some(level) = element.level {
            line.push_str(&format!(" [level={level}]"));
        }
        if let Some(reference) = &element.element_ref {
            line.push_str(&format!(" [ref={reference}]"));
        }
        match element.value.as_deref().or(element.text.as_deref()) {
            Some(content) if !content.is_empty() => {
                line.push_str(": ");
                line.push_str(&clip(&normalize(content), CONTENT_LIMIT));
            }
            _ if has_children => line.push(':'),
            _ => {}
        }
        line
    }

    fn url_line(url: &str, indent: u32) -> String {
        format!("{}- /url: {}", "  ".repeat(indent as usize + 1), clip(url, CONTENT_LIMIT))
    }

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) struct RefEntry {
        pub backend_node_id: i64,
        pub frame_path: Vec<i64>,
        pub role: String,
        pub name: String,
    }

    pub(crate) struct PreviousRef {
        pub reference: String,
        pub role: String,
        pub name: String,
    }

    pub(crate) struct Rendered {
        pub text: String,
        pub elements: Vec<SnapshotElement>,
        pub entries: Vec<(String, RefEntry)>,
        pub truncated: bool,
        pub next_ref: u64,
    }

    struct Renderer<'a> {
        previous: &'a HashMap<i64, PreviousRef>,
        flat: bool,
        max_chars: usize,
        used: usize,
        rendered: Rendered,
    }

    impl Renderer<'_> {
        fn push_line(&mut self, line: String) -> bool {
            let length = line.chars().count() + usize::from(!self.rendered.text.is_empty());
            if self.used + length > self.max_chars {
                self.rendered.truncated = true;
                return false;
            }
            if !self.rendered.text.is_empty() {
                self.rendered.text.push('\n');
            }
            self.rendered.text.push_str(&line);
            self.used += length;
            true
        }

        fn render(&mut self, nodes: &[OutNode], indent: u32) -> bool {
            for node in nodes {
                let mut element = node.element.clone();
                element.depth = indent;
                let inline_text = match node.children.as_slice() {
                    [only] if only.element.role == "text" && element.value.is_none() => {
                        Some(only.element.name.clone())
                    }
                    _ => None,
                };
                element.text = inline_text.clone();
                let mut fresh = false;
                if let Some(backend) = node.backend_id {
                    let reference = match self.previous.get(&backend) {
                        Some(previous)
                            if previous.role == element.role && previous.name == element.name =>
                        {
                            previous.reference.clone()
                        }
                        _ => {
                            fresh = true;
                            format!("e{}", self.rendered.next_ref)
                        }
                    };
                    element.element_ref = Some(reference);
                }
                let children: &[OutNode] = if inline_text.is_some() {
                    &[]
                } else {
                    &node.children
                };
                let has_children = element.url.is_some() || !children.is_empty();
                if !self.push_line(element_line(&element, indent, has_children)) {
                    return false;
                }
                if let Some(url) = &element.url
                    && !self.push_line(url_line(url, indent))
                {
                    return false;
                }
                if let (Some(reference), Some(backend)) = (&element.element_ref, node.backend_id) {
                    if fresh {
                        self.rendered.next_ref += 1;
                    }
                    self.rendered.entries.push((
                        reference.clone(),
                        RefEntry {
                            backend_node_id: backend,
                            frame_path: node.frame_path.clone(),
                            role: element.role.clone(),
                            name: element.name.clone(),
                        },
                    ));
                }
                self.rendered.elements.push(element);
                let child_indent = if self.flat { indent } else { indent + 1 };
                if !self.render(children, child_indent) {
                    return false;
                }
            }
            true
        }
    }

    pub(crate) fn render(
        nodes: &[OutNode],
        flat: bool,
        max_chars: usize,
        previous: &HashMap<i64, PreviousRef>,
        next_ref: u64,
    ) -> Rendered {
        let mut renderer = Renderer {
            previous,
            flat,
            max_chars,
            used: 0,
            rendered: Rendered {
                text: String::new(),
                elements: Vec::new(),
                entries: Vec::new(),
                truncated: false,
                next_ref,
            },
        };
        renderer.render(nodes, 0);
        renderer.rendered
    }

    /// Elements that react to clicks without an interactive role: own `cursor: pointer`
    /// (not inherited from the parent), an `onclick` attribute or a non-negative `tabindex`.
    pub(crate) fn clickable_nodes(snapshot: &Value) -> HashSet<i64> {
        let strings: Vec<&str> = snapshot["strings"]
            .as_array()
            .map(|strings| strings.iter().map(|s| s.as_str().unwrap_or_default()).collect())
            .unwrap_or_default();
        let string = |index: &Value| {
            index
                .as_u64()
                .and_then(|index| strings.get(index as usize).copied())
                .unwrap_or_default()
        };
        let mut clickable = HashSet::new();
        for document in snapshot["documents"].as_array().into_iter().flatten() {
            let nodes = &document["nodes"];
            let Some(backend_ids) = nodes["backendNodeId"].as_array() else {
                continue;
            };
            let node_types = nodes["nodeType"].as_array();
            let parents = nodes["parentIndex"].as_array();
            let attributes = nodes["attributes"].as_array();
            let mut cursors: HashMap<usize, &str> = HashMap::new();
            if let (Some(indices), Some(styles)) = (
                document["layout"]["nodeIndex"].as_array(),
                document["layout"]["styles"].as_array(),
            ) {
                for (node_index, style) in indices.iter().zip(styles) {
                    if let (Some(node_index), Some(cursor)) = (
                        node_index.as_u64(),
                        style.as_array().and_then(|style| style.first()),
                    ) {
                        cursors.insert(node_index as usize, string(cursor));
                    }
                }
            }
            for (position, backend) in backend_ids.iter().enumerate() {
                let is_element = node_types
                    .and_then(|types| types.get(position))
                    .and_then(Value::as_i64)
                    == Some(1);
                if !is_element {
                    continue;
                }
                let parent_cursor = parents
                    .and_then(|parents| parents.get(position))
                    .and_then(Value::as_i64)
                    .and_then(|parent| usize::try_from(parent).ok())
                    .and_then(|parent| cursors.get(&parent).copied());
                let own_pointer =
                    cursors.get(&position) == Some(&"pointer") && parent_cursor != Some("pointer");
                let handler = attributes
                    .and_then(|attributes| attributes.get(position))
                    .and_then(Value::as_array)
                    .is_some_and(|pairs| {
                        pairs.chunks(2).any(|pair| match string(&pair[0]) {
                            "onclick" => true,
                            "tabindex" => pair
                                .get(1)
                                .and_then(|value| string(value).trim().parse::<i32>().ok())
                                .is_some_and(|tabindex| tabindex >= 0),
                            _ => false,
                        })
                    });
                if (own_pointer || handler)
                    && let Some(backend) = backend.as_i64()
                {
                    clickable.insert(backend);
                }
            }
        }
        clickable
    }

    pub(crate) enum NameMatcher {
        Contains(String),
        Exact(String),
        Regex(flow_like_types::regex::Regex),
    }

    pub(crate) struct ElementQuery {
        pub role: Option<String>,
        pub name: Option<NameMatcher>,
        pub text: Option<String>,
    }

    impl ElementQuery {
        pub fn new(role: &str, name: &str, mode: &str, text: &str) -> flow_like_types::Result<Self> {
            let pattern = name.trim();
            let name = normalize(name);
            let name = if name.is_empty() {
                None
            } else {
                Some(match mode {
                    "exact" => NameMatcher::Exact(name),
                    "regex" => NameMatcher::Regex(
                        flow_like_types::regex::Regex::new(pattern).map_err(|error| {
                            flow_like_types::anyhow!("Invalid name regex '{pattern}': {error}")
                        })?,
                    ),
                    "contains" => NameMatcher::Contains(name.to_lowercase()),
                    other => {
                        return Err(flow_like_types::anyhow!(
                            "Unknown name match mode '{other}' (use contains, exact or regex)"
                        ));
                    }
                })
            };
            Ok(Self {
                role: Some(role.trim().to_lowercase()).filter(|role| !role.is_empty()),
                name,
                text: Some(normalize(text).to_lowercase()).filter(|text| !text.is_empty()),
            })
        }

        pub fn matches(&self, element: &SnapshotElement) -> bool {
            if element.element_ref.is_none() {
                return false;
            }
            if self
                .role
                .as_ref()
                .is_some_and(|role| element.role.to_lowercase() != *role)
            {
                return false;
            }
            let name_ok = match &self.name {
                None => true,
                Some(NameMatcher::Contains(needle)) => element.name.to_lowercase().contains(needle),
                Some(NameMatcher::Exact(expected)) => element.name == *expected,
                Some(NameMatcher::Regex(pattern)) => pattern.is_match(&element.name),
            };
            let text_ok = self.text.as_ref().is_none_or(|needle| {
                [
                    Some(element.name.as_str()),
                    element.value.as_deref(),
                    element.text.as_deref(),
                    element.description.as_deref(),
                ]
                .into_iter()
                .flatten()
                .any(|haystack| haystack.to_lowercase().contains(needle))
            });
            name_ok && text_ok
        }
    }
}

#[cfg(feature = "execute")]
pub(crate) use live::*;

#[cfg(feature = "execute")]
mod live {
    use super::tree::{self, AxTree, PreviousRef, RefEntry};
    use super::{SnapshotElement, stale_ref_error};
    use crate::browser::cdp::cdp;
    use crate::types::handles::AutomationSession;
    use crate::types::selectors::normalize_ref;
    use flow_like::flow::execution::{LogLevel, context::ExecutionContext};
    use flow_like_types::{Cacheable, json::json};
    use serde::Deserialize;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};

    const REF_OBJECT_GROUP: &str = "flowlike-refs";
    const MAX_CHILD_FRAMES: usize = 20;
    const MAX_REF_ENTRIES: usize = 20_000;
    const STASH_FUNCTION: &str = "function() { if (!this.isConnected || !this.ownerDocument) return false; this.ownerDocument[Symbol.for('flowlike.ref')] = this; return true; }";
    const TAKE_STASHED: &str = "const key = Symbol.for('flowlike.ref'); const element = document[key]; delete document[key]; return element || null;";

    #[derive(Debug, Default)]
    pub(crate) struct RefTable {
        pub generation: u64,
        pub loader_id: String,
        pub next_ref: u64,
        pub entries: HashMap<String, RefEntry>,
        pub elements: Vec<SnapshotElement>,
    }

    impl RefTable {
        fn previous_refs(&self) -> HashMap<i64, PreviousRef> {
            let mut latest: HashMap<i64, (u64, PreviousRef)> = HashMap::new();
            for (reference, entry) in &self.entries {
                let number = reference[1..].parse::<u64>().unwrap_or_default();
                if latest
                    .get(&entry.backend_node_id)
                    .is_none_or(|(existing, _)| *existing < number)
                {
                    latest.insert(
                        entry.backend_node_id,
                        (
                            number,
                            PreviousRef {
                                reference: reference.clone(),
                                role: entry.role.clone(),
                                name: entry.name.clone(),
                            },
                        ),
                    );
                }
            }
            latest
                .into_iter()
                .map(|(backend, (_, previous))| (backend, previous))
                .collect()
        }
    }

    type SharedTable = Arc<Mutex<RefTable>>;

    pub(crate) struct RefCache(SharedTable);

    impl Cacheable for RefCache {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    /// `find()` only receives the driver, so ref tables are also reachable by WebDriver
    /// session id. The run cache owns them; entries die with the cache entry.
    static REF_TABLES: LazyLock<Mutex<HashMap<String, Weak<Mutex<RefTable>>>>> =
        LazyLock::new(Default::default);

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn refs_key(session: &AutomationSession) -> String {
        format!("automation:refs:{}", session.session_ref)
    }

    async fn table_for(context: &ExecutionContext, session: &AutomationSession) -> Option<SharedTable> {
        context
            .cache
            .read()
            .await
            .get(&refs_key(session))
            .and_then(|entry| entry.as_any().downcast_ref::<RefCache>())
            .map(|cache| cache.0.clone())
    }

    fn registered_table(driver: &thirtyfour::WebDriver) -> Option<SharedTable> {
        lock(&REF_TABLES)
            .get(&driver.handle.session_id().to_string())
            .and_then(Weak::upgrade)
    }

    async fn install(
        context: &ExecutionContext,
        session: &AutomationSession,
        driver: &thirtyfour::WebDriver,
        table: RefTable,
    ) {
        let shared = Arc::new(Mutex::new(table));
        {
            let mut registry = lock(&REF_TABLES);
            registry.retain(|_, table| table.strong_count() > 0);
            registry.insert(
                driver.handle.session_id().to_string(),
                Arc::downgrade(&shared),
            );
        }
        context
            .cache
            .write()
            .await
            .insert(refs_key(session), Arc::new(RefCache(shared)));
    }

    #[derive(Deserialize, Debug, Clone)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct FrameTree {
        pub frame: FrameInfo,
        #[serde(default)]
        pub child_frames: Vec<FrameTree>,
    }

    #[derive(Deserialize, Debug, Clone)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct FrameInfo {
        pub id: String,
        #[serde(default)]
        pub loader_id: String,
        #[serde(default)]
        pub url: String,
        #[serde(default)]
        pub url_fragment: Option<String>,
    }

    impl FrameTree {
        fn descendants(&self) -> Vec<&FrameInfo> {
            let mut frames = Vec::new();
            let mut stack: Vec<&FrameTree> = self.child_frames.iter().rev().collect();
            while let Some(tree) = stack.pop() {
                frames.push(&tree.frame);
                stack.extend(tree.child_frames.iter().rev());
            }
            frames
        }
    }

    pub(crate) async fn frame_tree(driver: &thirtyfour::WebDriver) -> flow_like_types::Result<FrameTree> {
        let response = cdp(driver, "Page.getFrameTree", json!({})).await?;
        flow_like_types::json::from_value(response["frameTree"].clone())
            .map_err(|error| flow_like_types::anyhow!("Failed to parse Page.getFrameTree: {error}"))
    }

    async fn ax_tree(
        driver: &thirtyfour::WebDriver,
        frame_id: Option<&str>,
    ) -> flow_like_types::Result<AxTree> {
        let params = match frame_id {
            Some(frame_id) => json!({ "frameId": frame_id }),
            None => json!({}),
        };
        AxTree::from_response(&cdp(driver, "Accessibility.getFullAXTree", params).await?)
    }

    async fn element_for_backend(
        driver: &thirtyfour::WebDriver,
        backend: i64,
    ) -> flow_like_types::Result<Option<thirtyfour::WebElement>> {
        let Ok(resolved) = cdp(
            driver,
            "DOM.resolveNode",
            json!({ "backendNodeId": backend, "objectGroup": REF_OBJECT_GROUP }),
        )
        .await
        else {
            return Ok(None);
        };
        let Some(object_id) = resolved["object"]["objectId"].as_str() else {
            return Ok(None);
        };
        let stashed = cdp(
            driver,
            "Runtime.callFunctionOn",
            json!({ "objectId": object_id, "functionDeclaration": STASH_FUNCTION, "returnByValue": true }),
        )
        .await?;
        if stashed["result"]["value"].as_bool() != Some(true) {
            return Ok(None);
        }
        let element = driver.execute(TAKE_STASHED, Vec::new()).await?;
        if element.json().is_null() {
            return Ok(None);
        }
        Ok(Some(element.element()?))
    }

    /// Resolves a snapshot ref to a WebElement, entering the frame that owns it.
    pub(crate) async fn resolve_ref(
        driver: &thirtyfour::WebDriver,
        value: &str,
    ) -> flow_like_types::Result<thirtyfour::WebElement> {
        let reference = normalize_ref(value).ok_or_else(|| {
            flow_like_types::anyhow!("'{value}' is not a browser snapshot ref such as 'e12'")
        })?;
        let table = registered_table(driver).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Element ref '{reference}' has no browser snapshot in this session — take a browser snapshot first"
            )
        })?;
        let (entry, loader_id) = {
            let table = lock(&table);
            (table.entries.get(&reference).cloned(), table.loader_id.clone())
        };
        let entry = entry.ok_or_else(|| stale_ref_error(&reference))?;
        if frame_tree(driver).await?.frame.loader_id != loader_id {
            return Err(stale_ref_error(&reference));
        }
        driver.enter_default_frame().await?;
        for owner in &entry.frame_path {
            element_for_backend(driver, *owner)
                .await?
                .ok_or_else(|| stale_ref_error(&reference))?
                .enter_frame()
                .await?;
        }
        element_for_backend(driver, entry.backend_node_id)
            .await?
            .ok_or_else(|| stale_ref_error(&reference))
    }

    pub(crate) struct SnapshotRequest {
        pub interactive_only: bool,
        pub max_depth: Option<u32>,
        pub scope_ref: Option<String>,
        pub max_chars: usize,
        pub detect_clickable: bool,
    }

    pub(crate) struct SnapshotOutput {
        pub text: String,
        pub elements: Vec<SnapshotElement>,
        pub url: String,
        pub title: String,
        pub generation: u64,
        pub truncated: bool,
    }

    async fn clickable_nodes(
        context: &mut ExecutionContext,
        driver: &thirtyfour::WebDriver,
    ) -> HashSet<i64> {
        let _ = cdp(driver, "DOMSnapshot.enable", json!({})).await;
        match cdp(
            driver,
            "DOMSnapshot.captureSnapshot",
            json!({ "computedStyles": ["cursor"] }),
        )
        .await
        {
            Ok(snapshot) => tree::clickable_nodes(&snapshot),
            Err(error) => {
                context.log_message(
                    &format!("Browser snapshot skipped clickable-element detection: {error}"),
                    LogLevel::Warn,
                );
                HashSet::new()
            }
        }
    }

    async fn child_frame_trees(
        driver: &thirtyfour::WebDriver,
        frames: &FrameTree,
    ) -> HashMap<i64, AxTree> {
        let mut trees = HashMap::new();
        for frame in frames.descendants().into_iter().take(MAX_CHILD_FRAMES) {
            let Ok(tree) = ax_tree(driver, Some(&frame.id)).await else {
                continue;
            };
            let Ok(owner) = cdp(driver, "DOM.getFrameOwner", json!({ "frameId": frame.id })).await
            else {
                continue;
            };
            if let Some(backend) = owner["backendNodeId"].as_i64() {
                trees.insert(backend, tree);
            }
        }
        trees
    }

    /// Previous refs of the same document, reused so unchanged elements keep their ref.
    struct Carried {
        generation: u64,
        entries: HashMap<String, RefEntry>,
        previous: HashMap<i64, PreviousRef>,
        next_ref: u64,
    }

    async fn carried_refs(
        context: &ExecutionContext,
        session: &AutomationSession,
        loader_id: &str,
    ) -> Carried {
        let Some(table) = table_for(context, session).await else {
            return Carried {
                generation: 1,
                entries: HashMap::new(),
                previous: HashMap::new(),
                next_ref: 1,
            };
        };
        let table = lock(&table);
        if table.loader_id != loader_id {
            return Carried {
                generation: table.generation + 1,
                entries: HashMap::new(),
                previous: HashMap::new(),
                next_ref: 1,
            };
        }
        Carried {
            generation: table.generation + 1,
            entries: table.entries.clone(),
            previous: table.previous_refs(),
            next_ref: table.next_ref,
        }
    }

    fn scope_start<'t>(
        scope_ref: &str,
        entries: &HashMap<String, RefEntry>,
        main: &'t AxTree,
        child_trees: &'t HashMap<i64, AxTree>,
    ) -> flow_like_types::Result<(&'t AxTree, usize, Vec<i64>)> {
        let reference = normalize_ref(scope_ref).ok_or_else(|| {
            flow_like_types::anyhow!("Scope '{scope_ref}' is not a snapshot ref such as 'e12'")
        })?;
        let entry = entries
            .get(&reference)
            .ok_or_else(|| stale_ref_error(&reference))?;
        let tree = match entry.frame_path.last() {
            Some(owner) => child_trees.get(owner),
            None => Some(main),
        }
        .ok_or_else(|| stale_ref_error(&reference))?;
        let index = tree
            .find_backend(entry.backend_node_id)
            .ok_or_else(|| stale_ref_error(&reference))?;
        Ok((tree, index, entry.frame_path.clone()))
    }

    pub(crate) async fn take_snapshot(
        context: &mut ExecutionContext,
        session: &AutomationSession,
        driver: &thirtyfour::WebDriver,
        request: &SnapshotRequest,
    ) -> flow_like_types::Result<SnapshotOutput> {
        let frames = frame_tree(driver).await?;
        let _ = cdp(
            driver,
            "Runtime.releaseObjectGroup",
            json!({ "objectGroup": REF_OBJECT_GROUP }),
        )
        .await;
        cdp(driver, "Accessibility.enable", json!({})).await?;
        let main = ax_tree(driver, None).await?;
        let child_trees = child_frame_trees(driver, &frames).await;
        let clickable = if request.detect_clickable {
            clickable_nodes(context, driver).await
        } else {
            HashSet::new()
        };
        let Carried {
            generation,
            mut entries,
            previous,
            next_ref,
        } = carried_refs(context, session, &frames.frame.loader_id).await;

        let options = tree::WalkOptions {
            interactive_only: request.interactive_only,
            max_depth: request.max_depth,
            clickable,
        };
        let mut walker = tree::Walker::new(&options, &child_trees);
        let nodes = match request.scope_ref.as_deref() {
            Some(scope_ref) => {
                let (tree, index, path) = scope_start(scope_ref, &entries, &main, &child_trees)?;
                walker.walk(tree, index, &path)
            }
            None => match main.root() {
                Some(root) => walker.walk(&main, root, &[]),
                None => Vec::new(),
            },
        };
        let rendered = tree::render(
            &nodes,
            request.interactive_only,
            request.max_chars,
            &previous,
            next_ref,
        );

        let latest: HashSet<String> = rendered
            .entries
            .iter()
            .map(|(reference, _)| reference.clone())
            .collect();
        entries.extend(rendered.entries);
        if entries.len() > MAX_REF_ENTRIES {
            entries.retain(|reference, _| latest.contains(reference));
        }
        install(
            context,
            session,
            driver,
            RefTable {
                generation,
                loader_id: frames.frame.loader_id.clone(),
                next_ref: rendered.next_ref,
                entries,
                elements: rendered.elements.clone(),
            },
        )
        .await;
        Ok(SnapshotOutput {
            text: rendered.text,
            elements: rendered.elements,
            url: format!(
                "{}{}",
                frames.frame.url,
                frames.frame.url_fragment.as_deref().unwrap_or_default()
            ),
            title: main.title(),
            generation,
            truncated: rendered.truncated,
        })
    }

    /// Latest snapshot elements when the document is unchanged, otherwise a fresh full snapshot.
    pub(crate) async fn searchable_elements(
        context: &mut ExecutionContext,
        session: &AutomationSession,
        driver: &thirtyfour::WebDriver,
        refresh: bool,
    ) -> flow_like_types::Result<Vec<SnapshotElement>> {
        if !refresh && let Some(table) = table_for(context, session).await {
            let loader_id = frame_tree(driver).await?.frame.loader_id;
            let table = lock(&table);
            if table.loader_id == loader_id {
                return Ok(table.elements.clone());
            }
        }
        let request = SnapshotRequest {
            interactive_only: false,
            max_depth: None,
            scope_ref: None,
            max_chars: usize::MAX,
            detect_clickable: true,
        };
        Ok(take_snapshot(context, session, driver, &request).await?.elements)
    }

    /// Maps elements to snapshot refs, registering refs for main-frame elements the latest
    /// snapshot did not list. Elements get `None` when no snapshot exists for this document.
    pub(crate) async fn refs_for_elements(
        context: &ExecutionContext,
        session: &AutomationSession,
        driver: &thirtyfour::WebDriver,
        elements: &[thirtyfour::WebElement],
    ) -> flow_like_types::Result<Vec<Option<String>>> {
        let mut refs = vec![None; elements.len()];
        let Some(table) = table_for(context, session).await else {
            return Ok(refs);
        };
        if elements.is_empty() {
            return Ok(refs);
        }
        let loader_id = frame_tree(driver).await?.frame.loader_id;
        if lock(&table).loader_id != loader_id {
            return Ok(refs);
        }
        let arguments = elements
            .iter()
            .map(|element| element.to_json())
            .collect::<Result<Vec<_>, _>>()?;
        driver
            .execute(
                "document[Symbol.for('flowlike.list')] = Array.from(arguments);",
                arguments,
            )
            .await?;
        let listed = cdp(
            driver,
            "Runtime.evaluate",
            json!({
                "expression": "(() => { const key = Symbol.for('flowlike.list'); const list = document[key]; delete document[key]; return list; })()",
                "objectGroup": REF_OBJECT_GROUP,
            }),
        )
        .await?;
        let Some(array_id) = listed["result"]["objectId"].as_str() else {
            return Ok(refs);
        };
        let properties = cdp(
            driver,
            "Runtime.getProperties",
            json!({ "objectId": array_id, "ownProperties": true }),
        )
        .await?;
        let mut backends = Vec::new();
        for property in properties["result"].as_array().into_iter().flatten() {
            let (Some(index), Some(object_id)) = (
                property["name"]
                    .as_str()
                    .and_then(|name| name.parse::<usize>().ok()),
                property["value"]["objectId"].as_str(),
            ) else {
                continue;
            };
            let described = cdp(driver, "DOM.describeNode", json!({ "objectId": object_id })).await?;
            if let Some(backend) = described["node"]["backendNodeId"].as_i64()
                && index < refs.len()
            {
                backends.push((index, backend));
            }
        }
        let mut table = lock(&table);
        let mut by_backend: HashMap<i64, String> = table
            .previous_refs()
            .into_iter()
            .map(|(backend, previous)| (backend, previous.reference))
            .collect();
        for (index, backend) in backends {
            let reference = match by_backend.get(&backend) {
                Some(reference) => reference.clone(),
                None => {
                    let reference = format!("e{}", table.next_ref);
                    table.next_ref += 1;
                    table.entries.insert(
                        reference.clone(),
                        RefEntry {
                            backend_node_id: backend,
                            frame_path: Vec::new(),
                            role: String::new(),
                            name: String::new(),
                        },
                    );
                    by_backend.insert(backend, reference.clone());
                    reference
                }
            };
            refs[index] = Some(reference);
        }
        Ok(refs)
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserSnapshotNode {}

impl BrowserSnapshotNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSnapshotNode {
    fn get_node(&self) -> Node {
        let mut node = super::manage::base_node(
            "browser_snapshot",
            "Browser Snapshot",
            "Captures the page accessibility tree as compact text with element refs (e1, e2, …) that any selector pin accepts as a Ref selector. Refs stay valid until the page navigates or the element is removed; a stale ref fails with a request to take a new snapshot. Covers the current tab including same-process iframes; out-of-process (cross-site) iframes are omitted. Requires Chrome or Edge.",
        );
        node.category = "Automation/Browser/Snapshot".to_string();
        node.set_flowscript_name("browser", "snapshot");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(6)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );
        node.add_input_pin(
            "interactive_only",
            "Interactive Only",
            "List only interactive elements (buttons, links, inputs, clickable elements) as a flat list; disable for the full page structure with text",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "max_depth",
            "Max Depth",
            "Maximum nesting depth of listed elements (-1 for unlimited)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));
        node.add_input_pin(
            "scope_ref",
            "Scope Ref",
            "Optional ref from an earlier snapshot of this page; only its subtree is captured",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "max_chars",
            "Max Characters",
            "Maximum length of the snapshot text; longer snapshots are cut and flagged as truncated",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(15000)));
        node.add_input_pin(
            "detect_clickable",
            "Detect Clickable",
            "Also list elements without an interactive role that have a pointer cursor, onclick or tabindex",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_output_pin(
            "snapshot",
            "Snapshot",
            "Indented text such as `- button \"Sign in\" [ref=e12]`",
            VariableType::String,
        );
        node.add_output_pin(
            "elements",
            "Elements",
            "Listed elements in document order",
            VariableType::Struct,
        )
        .set_schema::<SnapshotElement>()
        .set_value_type(ValueType::Array);
        node.add_output_pin("url", "URL", "Page URL", VariableType::String);
        node.add_output_pin("title", "Title", "Page title", VariableType::String);
        node.add_output_pin(
            "generation",
            "Generation",
            "Snapshot counter for this session",
            VariableType::Integer,
        );
        node.add_output_pin(
            "truncated",
            "Truncated",
            "Whether the snapshot exceeded Max Characters",
            VariableType::Boolean,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let interactive_only: bool = context.evaluate_pin("interactive_only").await?;
        let max_depth: i64 = context.evaluate_pin("max_depth").await?;
        let scope_ref: String = context.evaluate_pin("scope_ref").await?;
        let max_chars: i64 = context.evaluate_pin("max_chars").await?;
        let detect_clickable: bool = context.evaluate_pin("detect_clickable").await?;
        if max_chars < 100 {
            return Err(flow_like_types::anyhow!(
                "Max Characters must be at least 100 (got {max_chars})"
            ));
        }
        let request = SnapshotRequest {
            interactive_only,
            max_depth: u32::try_from(max_depth).ok(),
            scope_ref: Some(scope_ref.trim().to_string()).filter(|scope| !scope.is_empty()),
            max_chars: usize::try_from(max_chars).unwrap_or(usize::MAX),
            detect_clickable,
        };
        let driver = session.get_browser_driver_and_switch(context).await?;
        let snapshot = take_snapshot(context, &session, &driver, &request).await?;
        drop(driver);
        context.set_pin_value("snapshot", json!(snapshot.text)).await?;
        context
            .set_pin_value("elements", json!(snapshot.elements))
            .await?;
        context.set_pin_value("url", json!(snapshot.url)).await?;
        context.set_pin_value("title", json!(snapshot.title)).await?;
        context
            .set_pin_value("generation", json!(snapshot.generation))
            .await?;
        context
            .set_pin_value("truncated", json!(snapshot.truncated))
            .await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserFindElementsNode {}

impl BrowserFindElementsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserFindElementsNode {
    fn get_node(&self) -> Node {
        let mut node = super::manage::base_node(
            "browser_find_elements",
            "Find Elements",
            "Searches the latest browser snapshot by role, accessible name and text and returns matching refs. Takes a fresh full snapshot when none exists, the page navigated, or Refresh is set. Requires Chrome or Edge.",
        );
        node.category = "Automation/Browser/Snapshot".to_string();
        node.set_flowscript_name("browser", "findElements");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(6)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );
        node.add_input_pin(
            "role",
            "Role",
            "ARIA role such as button, link or textbox (empty matches any role)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "name",
            "Name",
            "Accessible name to match (empty matches any name)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "name_match",
            "Name Match",
            "How Name is compared: case-insensitive contains, exact, or regex",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "contains".to_string(),
                    "exact".to_string(),
                    "regex".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("contains")));
        node.add_input_pin(
            "text",
            "Text",
            "Case-insensitive text contained in the name, value, text or description",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "refresh",
            "Refresh",
            "Always take a fresh full snapshot before searching",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum matches returned",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(20)));
        node.add_output_pin("refs", "Refs", "Matching refs", VariableType::String)
            .set_value_type(ValueType::Array);
        node.add_output_pin(
            "lines",
            "Lines",
            "Snapshot line of each match",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "elements",
            "Elements",
            "Matching snapshot elements",
            VariableType::Struct,
        )
        .set_schema::<SnapshotElement>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "count",
            "Count",
            "Number of matches before the limit",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let role: String = context.evaluate_pin("role").await?;
        let name: String = context.evaluate_pin("name").await?;
        let name_match: String = context.evaluate_pin("name_match").await?;
        let text: String = context.evaluate_pin("text").await?;
        let refresh: bool = context.evaluate_pin("refresh").await?;
        let limit: i64 = context.evaluate_pin("limit").await?;
        if limit < 1 {
            return Err(flow_like_types::anyhow!(
                "Find Elements limit must be at least 1 (got {limit})"
            ));
        }
        let query = tree::ElementQuery::new(&role, &name, &name_match, &text)?;
        let driver = session.get_browser_driver_and_switch(context).await?;
        let elements = searchable_elements(context, &session, &driver, refresh).await?;
        drop(driver);
        let matches: Vec<SnapshotElement> = elements
            .into_iter()
            .filter(|element| query.matches(element))
            .collect();
        let count = matches.len();
        let matches: Vec<SnapshotElement> = matches
            .into_iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .collect();
        let refs: Vec<String> = matches
            .iter()
            .filter_map(|element| element.element_ref.clone())
            .collect();
        let lines: Vec<String> = matches
            .iter()
            .map(|element| tree::element_line(element, 0, false))
            .collect();
        context.set_pin_value("refs", json!(refs)).await?;
        context.set_pin_value("lines", json!(lines)).await?;
        context.set_pin_value("elements", json!(matches)).await?;
        context.set_pin_value("count", json!(count)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::tree::*;
    use super::*;
    use std::collections::{HashMap, HashSet};

    const LOGIN_FIXTURE: &str = include_str!("../../tests/fixtures/ax_tree_login.json");

    fn fixture() -> (AxTree, HashMap<i64, AxTree>) {
        let value: flow_like_types::Value = flow_like_types::json::from_str(LOGIN_FIXTURE).unwrap();
        let main = AxTree::from_response(&value["main"]).unwrap();
        let frame = AxTree::from_response(&value["frame"]).unwrap();
        (main, HashMap::from([(90, frame)]))
    }

    fn snapshot(interactive_only: bool, max_depth: Option<u32>, max_chars: usize) -> Rendered {
        let (main, frames) = fixture();
        let options = WalkOptions {
            interactive_only,
            max_depth,
            clickable: HashSet::from([50]),
        };
        let nodes = Walker::new(&options, &frames).walk(&main, main.root().unwrap(), &[]);
        render(&nodes, interactive_only, max_chars, &HashMap::new(), 1)
    }

    #[test]
    fn interactive_snapshot_lists_refs_in_document_order() {
        let rendered = snapshot(true, None, 15_000);
        assert_eq!(
            rendered.text,
            [
                "- textbox \"Email\" [focused] [required] [ref=e1]: ada@example.com",
                "- checkbox \"Remember me\" [checked] [ref=e2]",
                "- button \"Sign in\" [disabled] [ref=e3]",
                "- link \"Forgot password?\" [ref=e4]:",
                "  - /url: https://example.com/forgot",
                "- generic \"Open menu\" [cursor=pointer] [ref=e5]",
                "- button \"Inside frame\" [ref=e6]",
            ]
            .join("\n")
        );
        assert!(!rendered.truncated);
        assert_eq!(rendered.next_ref, 7);
        let (reference, entry) = &rendered.entries[5];
        assert_eq!(reference, "e6");
        assert_eq!(entry.backend_node_id, 91);
        assert_eq!(entry.frame_path, vec![90]);
        assert_eq!(rendered.elements[3].url.as_deref(), Some("https://example.com/forgot"));
    }

    #[test]
    fn full_snapshot_keeps_structure_and_text() {
        let rendered = snapshot(false, None, 15_000);
        assert_eq!(
            rendered.text,
            [
                "- heading \"Welcome back\" [level=1] [ref=e1]",
                "- paragraph [ref=e2]: Please sign in to continue.",
                "- form \"Login\" [ref=e3]:",
                "  - text: Email",
                "  - textbox \"Email\" [focused] [required] [ref=e4]: ada@example.com",
                "  - checkbox \"Remember me\" [checked] [ref=e5]",
                "  - button \"Sign in\" [disabled] [ref=e6]",
                "- link \"Forgot password?\" [ref=e7]:",
                "  - /url: https://example.com/forgot",
                "- generic \"Open menu\" [cursor=pointer] [ref=e8]",
                "- iframe \"Widget\" [ref=e9]:",
                "  - button \"Inside frame\" [ref=e10]",
            ]
            .join("\n")
        );
        assert_eq!(rendered.elements[1].text.as_deref(), Some("Please sign in to continue."));
    }

    #[test]
    fn depth_and_character_limits() {
        let shallow = snapshot(false, Some(0), 15_000);
        assert!(!shallow.text.contains("textbox"));
        assert!(shallow.text.contains("- form \"Login\" [ref=e3]"));
        let cut = snapshot(true, None, 120);
        assert!(cut.truncated);
        assert!(cut.text.chars().count() <= 120);
        assert_eq!(cut.elements.len(), cut.text.lines().count());
        assert_eq!(cut.entries.len(), cut.elements.len());
    }

    #[test]
    fn refs_are_reused_for_unchanged_elements() {
        let (main, frames) = fixture();
        let options = WalkOptions {
            interactive_only: true,
            max_depth: None,
            clickable: HashSet::new(),
        };
        let nodes = Walker::new(&options, &frames).walk(&main, main.root().unwrap(), &[]);
        let previous = HashMap::from([(
            21,
            PreviousRef {
                reference: "e40".into(),
                role: "button".into(),
                name: "Sign in".into(),
            },
        )]);
        let rendered = render(&nodes, true, 15_000, &previous, 41);
        assert!(rendered.text.contains("- button \"Sign in\" [disabled] [ref=e40]"));
        assert!(rendered.text.contains("- textbox \"Email\" [focused] [required] [ref=e41]"));
        assert_eq!(rendered.next_ref, 45);
    }

    #[test]
    fn clickable_detection_uses_own_pointer_and_handlers() {
        let snapshot = flow_like_types::json::json!({
            "strings": ["pointer", "auto", "onclick", "", "tabindex", "0", "-1"],
            "documents": [{
                "nodes": {
                    "backendNodeId": [1, 2, 3, 4, 5, 6],
                    "nodeType": [9, 1, 1, 1, 1, 3],
                    "parentIndex": [-1, 0, 1, 2, 1, 4],
                    "attributes": [[], [], [], [2, 3], [4, 6], []]
                },
                "layout": {
                    "nodeIndex": [1, 2, 3, 4, 5],
                    "styles": [[1], [0], [0], [1], [0]]
                }
            }]
        });
        assert_eq!(clickable_nodes(&snapshot), HashSet::from([3, 4]));
    }

    #[test]
    fn element_queries_match_role_name_and_text() {
        let rendered = snapshot(false, None, 15_000);
        let find = |role: &str, name: &str, mode: &str, text: &str| -> Vec<String> {
            let query = ElementQuery::new(role, name, mode, text).unwrap();
            rendered
                .elements
                .iter()
                .filter(|element| query.matches(element))
                .filter_map(|element| element.element_ref.clone())
                .collect()
        };
        assert_eq!(find("button", "", "contains", ""), vec!["e6", "e10"]);
        assert_eq!(find("Button", "sign", "contains", ""), vec!["e6"]);
        assert_eq!(find("", "Sign In", "exact", ""), Vec::<String>::new());
        assert_eq!(find("", "^Forgot", "regex", ""), vec!["e7"]);
        assert_eq!(find("", "", "contains", "ada@"), vec!["e4"]);
        assert_eq!(find("paragraph", "", "contains", "continue"), vec!["e2"]);
        assert!(ElementQuery::new("", "(", "regex", "").is_err());
        assert!(ElementQuery::new("", "x", "fuzzy", "").is_err());
    }

    #[test]
    fn interactive_snapshots_collapse_native_select_options() {
        let tree = AxTree::from_response(&flow_like_types::json::json!({ "nodes": [
            { "nodeId": "1", "role": { "value": "RootWebArea" }, "childIds": ["2"], "backendDOMNodeId": 1 },
            { "nodeId": "2", "parentId": "1", "role": { "value": "combobox" }, "name": { "value": "Country" },
              "value": { "value": "Germany" }, "childIds": ["3"], "backendDOMNodeId": 2 },
            { "nodeId": "3", "parentId": "2", "role": { "value": "MenuListPopup" }, "childIds": ["4"], "backendDOMNodeId": 3 },
            { "nodeId": "4", "parentId": "3", "role": { "value": "option" }, "name": { "value": "Germany" },
              "properties": [{ "name": "selected", "value": { "value": true } }], "backendDOMNodeId": 4 }
        ]}))
        .unwrap();
        let frames = HashMap::new();
        let render_mode = |interactive_only: bool| {
            let options = WalkOptions {
                interactive_only,
                max_depth: None,
                clickable: HashSet::new(),
            };
            let nodes = Walker::new(&options, &frames).walk(&tree, 0, &[]);
            render(&nodes, interactive_only, 15_000, &HashMap::new(), 1).text
        };
        assert_eq!(render_mode(true), "- combobox \"Country\" [ref=e1]: Germany");
        assert_eq!(
            render_mode(false),
            "- combobox \"Country\" [ref=e1]: Germany\n  - option \"Germany\" [selected] [ref=e2]"
        );
    }

    #[test]
    fn stale_error_wording() {
        assert_eq!(
            stale_ref_error("e12").to_string(),
            "Stale element ref 'e12' — take a new browser snapshot"
        );
    }
}
