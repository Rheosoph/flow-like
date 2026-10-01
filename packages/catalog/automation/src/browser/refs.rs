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
    flow_like_browser::BrowserError::StaleRef {
        reference: reference.to_string(),
    }
    .into()
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
    use super::{SnapshotElement, stale_ref_error};
    use crate::types::selectors::normalize_ref;
    use flow_like_browser::snapshot::{AxForest, AxFrame};
    use flow_like_browser::types::{LoaderId, TargetId};
    use flow_like_browser::{NodeRef, RefAllocator, RefTable};
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

        pub fn from_nodes(nodes: &Value) -> flow_like_types::Result<Self> {
            let nodes = Vec::<AxNode>::deserialize(nodes).map_err(|error| {
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
            self.nodes.iter().position(|node| {
                node.role() != "InlineTextBox" && node.backend_dom_node_id == Some(backend)
            })
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

    pub(crate) fn main_frame(forest: &AxForest) -> flow_like_types::Result<&AxFrame> {
        forest.frames.first().ok_or_else(|| {
            flow_like_types::anyhow!(
                "The browser returned no accessibility tree for page {}",
                forest.page
            )
        })
    }

    /// Parsed accessibility trees of a snapshot, indexed like `AxForest.frames`.
    pub(crate) struct AxForestView {
        trees: Vec<AxTree>,
        children: HashMap<(usize, i64), usize>,
        pub warnings: Vec<String>,
    }

    impl AxForestView {
        pub fn new(forest: &AxForest) -> flow_like_types::Result<Self> {
            let main = main_frame(forest)?;
            let mut view = Self {
                trees: vec![AxTree::from_nodes(&main.nodes)?],
                children: HashMap::new(),
                warnings: Vec::new(),
            };
            for (index, frame) in forest.frames.iter().enumerate().skip(1) {
                match AxTree::from_nodes(&frame.nodes) {
                    Ok(tree) => {
                        if let Some(owner) = frame.owner {
                            view.children
                                .insert((owner.parent, owner.backend_node_id), index);
                        }
                        view.trees.push(tree);
                    }
                    Err(error) => {
                        view.warnings.push(format!(
                            "Frame {} was left out of the snapshot: {error}",
                            frame.frame_id
                        ));
                        view.trees.push(AxTree::new(Vec::new()));
                    }
                }
            }
            Ok(view)
        }

        pub fn tree(&self, frame: usize) -> &AxTree {
            &self.trees[frame]
        }

        pub fn child(&self, frame: usize, owner: i64) -> Option<usize> {
            self.children.get(&(frame, owner)).copied()
        }

        pub fn title(&self) -> String {
            self.trees[0].title()
        }
    }

    /// One `NodeRef` per forest frame; the renderer fills in the backend node id.
    pub(crate) fn frame_refs(forest: &AxForest) -> Vec<NodeRef> {
        forest
            .frames
            .iter()
            .map(|frame| NodeRef {
                page: forest.page.clone(),
                local_root: frame.local_root.clone(),
                frame_id: frame.frame_id.clone(),
                loader_id: frame.loader_id.clone(),
                backend_node_id: 0,
            })
            .collect()
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
            | "listbox" | "option" | "menuitem" | "menuitemcheckbox" | "menuitemradio" | "tab"
            | "switch" | "slider" | "spinbutton" | "treeitem" | "DisclosureTriangle"
            | "ColorWell" | "Date" | "DateTime" | "InputTime" | "PopUpButton" | "ToggleButton"
            | "MenuListOption" => Class::Interactive,
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
        pub frame: usize,
        pub children: Vec<OutNode>,
    }

    pub(crate) struct WalkOptions {
        pub interactive_only: bool,
        pub max_depth: Option<u32>,
        pub clickable: HashSet<(usize, i64)>,
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
        forest: &'a AxForestView,
        visited: HashSet<(usize, i64)>,
    }

    impl<'a> Walker<'a> {
        pub fn new(options: &'a WalkOptions, forest: &'a AxForestView) -> Self {
            Self {
                options,
                forest,
                visited: HashSet::new(),
            }
        }

        pub fn walk(&mut self, frame: usize, index: usize) -> Vec<OutNode> {
            let mut out = Vec::new();
            let position = Position {
                depth: 0,
                raw_depth: 0,
                ancestor_name: None,
                inside_interactive: false,
            };
            self.visit(frame, index, position, &mut out);
            out
        }

        fn tree(&self, frame: usize) -> &'a AxTree {
            let forest = self.forest;
            forest.tree(frame)
        }

        fn visit_children(
            &mut self,
            frame: usize,
            index: usize,
            position: Position<'_>,
            out: &mut Vec<OutNode>,
        ) {
            let tree = self.tree(frame);
            for child in &tree.nodes[index].child_ids {
                if let Some(&child) = tree.index.get(child) {
                    self.visit(frame, child, position, out);
                }
            }
        }

        fn visit(
            &mut self,
            frame: usize,
            index: usize,
            position: Position<'_>,
            out: &mut Vec<OutNode>,
        ) {
            if position.raw_depth > MAX_RAW_DEPTH {
                return;
            }
            let node = &self.tree(frame).nodes[index];
            let class = classify(node.role());
            if class == Class::Skip {
                return;
            }
            if let Some(backend) = node.backend_dom_node_id
                && !self.visited.insert((frame, backend))
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
                    .is_some_and(|backend| self.options.clickable.contains(&(frame, backend)));
            let class = if node.ignored { Class::Generic } else { class };
            match class {
                Class::Root => self.visit_children(frame, index, deeper, out),
                Class::Generic if !promotable => self.visit_children(frame, index, deeper, out),
                Class::Text => {
                    if !self.options.interactive_only && !too_deep {
                        push_text(out, node, frame, position);
                    }
                }
                _ if too_deep => {}
                Class::Structural if self.options.interactive_only => {
                    let nested = Position {
                        depth: position.depth + 1,
                        ..deeper
                    };
                    self.visit_children(frame, index, nested, out)
                }
                Class::Frame => self.visit_frame(frame, index, position, out),
                _ => self.visit_element(frame, index, position, class, out),
            }
        }

        fn visit_frame(
            &mut self,
            frame: usize,
            index: usize,
            position: Position<'_>,
            out: &mut Vec<OutNode>,
        ) {
            let tree = self.tree(frame);
            let node = &tree.nodes[index];
            let mut children = Vec::new();
            if let Some(backend) = node.backend_dom_node_id
                && let Some(child) = self.forest.child(frame, backend)
                && let Some(root) = self.tree(child).root()
            {
                let nested = Position {
                    depth: position.depth + 1,
                    raw_depth: position.raw_depth + 1,
                    ancestor_name: None,
                    inside_interactive: false,
                };
                self.visit(child, root, nested, &mut children);
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
                frame,
                children,
            });
        }

        fn visit_element(
            &mut self,
            frame: usize,
            index: usize,
            position: Position<'_>,
            class: Class,
            out: &mut Vec<OutNode>,
        ) {
            let tree = self.tree(frame);
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
                self.visit_children(frame, index, nested, &mut children);
            }
            out.push(OutNode {
                element,
                backend_id: node.backend_dom_node_id,
                frame,
                children: if flat {
                    Vec::new()
                } else {
                    std::mem::take(&mut children)
                },
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

    fn push_text(out: &mut Vec<OutNode>, node: &AxNode, frame: usize, position: Position<'_>) {
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
            frame,
            children: Vec::new(),
        });
    }

    fn quote(text: &str) -> String {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }

    pub(crate) fn element_line(
        element: &SnapshotElement,
        indent: u32,
        has_children: bool,
    ) -> String {
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
        format!(
            "{}- /url: {}",
            "  ".repeat(indent as usize + 1),
            clip(url, CONTENT_LIMIT)
        )
    }

    pub(crate) struct Rendered {
        pub text: String,
        pub elements: Vec<SnapshotElement>,
        pub truncated: bool,
    }

    struct Renderer<'a> {
        allocator: &'a mut RefAllocator,
        frames: &'a [NodeRef],
        flat: bool,
        max_chars: usize,
        used: usize,
        rendered: Rendered,
    }

    impl Renderer<'_> {
        /// Appends `lines` whole or not at all.
        fn push_lines(&mut self, lines: String) -> bool {
            let length = lines.chars().count() + usize::from(!self.rendered.text.is_empty());
            if self.used + length > self.max_chars {
                self.rendered.truncated = true;
                return false;
            }
            if !self.rendered.text.is_empty() {
                self.rendered.text.push('\n');
            }
            self.rendered.text.push_str(&lines);
            self.used += length;
            true
        }

        fn node_ref(&self, node: &OutNode) -> Option<NodeRef> {
            let backend_node_id = node.backend_id?;
            let frame = self.frames.get(node.frame)?;
            Some(NodeRef {
                backend_node_id,
                ..frame.clone()
            })
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
                let proposed = self.node_ref(node).map(|target| {
                    let proposed = self
                        .allocator
                        .propose(&target, &element.role, &element.name);
                    (target, proposed)
                });
                element.element_ref = proposed
                    .as_ref()
                    .map(|(_, proposed)| proposed.reference.clone());
                let children: &[OutNode] = if inline_text.is_some() {
                    &[]
                } else {
                    &node.children
                };
                let has_children = element.url.is_some() || !children.is_empty();
                let mut lines = element_line(&element, indent, has_children);
                if let Some(url) = &element.url {
                    lines.push('\n');
                    lines.push_str(&url_line(url, indent));
                }
                if !self.push_lines(lines) {
                    return false;
                }
                if let Some((target, proposed)) = proposed {
                    self.allocator.commit(
                        proposed,
                        target,
                        element.role.clone(),
                        element.name.clone(),
                    );
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

    /// Renders the walked nodes. An element line and its /url line fit together or are both
    /// cut, and a ref is committed only once they fit, so every ref in the text resolves.
    pub(crate) fn render(
        nodes: &[OutNode],
        flat: bool,
        max_chars: usize,
        allocator: &mut RefAllocator,
        frames: &[NodeRef],
    ) -> Rendered {
        let mut renderer = Renderer {
            allocator,
            frames,
            flat,
            max_chars,
            used: 0,
            rendered: Rendered {
                text: String::new(),
                elements: Vec::new(),
                truncated: false,
            },
        };
        renderer.render(nodes, 0);
        renderer.rendered
    }

    /// Whether `table` holds refs of the current document of `page`.
    pub(crate) fn describes_document(
        table: &RefTable,
        page: &TargetId,
        main_loader: Option<&LoaderId>,
    ) -> bool {
        main_loader.is_some() && table.page() == Some(page) && table.main_loader() == main_loader
    }

    /// Frame index and tree position of the subtree a Scope Ref names.
    pub(crate) fn scope_start(
        scope_ref: &str,
        table: &RefTable,
        forest: &AxForest,
        view: &AxForestView,
    ) -> flow_like_types::Result<(usize, usize)> {
        let reference = normalize_ref(scope_ref).ok_or_else(|| {
            flow_like_types::anyhow!("Scope '{scope_ref}' is not a snapshot ref such as 'e12'")
        })?;
        let main = main_frame(forest)?;
        if !describes_document(table, &forest.page, Some(&main.loader_id)) {
            return Err(stale_ref_error(&reference));
        }
        let node = &table.lookup(&reference)?.node;
        document_of(forest, node)
            .and_then(|frame| {
                let index = view.tree(frame).find_backend(node.backend_node_id)?;
                Some((frame, index))
            })
            .ok_or_else(|| stale_ref_error(&reference))
    }

    fn document_of(forest: &AxForest, node: &NodeRef) -> Option<usize> {
        forest.frames.iter().position(|frame| {
            frame.frame_id == node.frame_id
                && frame.loader_id == node.loader_id
                && frame.local_root == node.local_root
        })
    }

    /// Elements that react to clicks without an interactive role: own `cursor: pointer`
    /// (not inherited from the parent), an `onclick` attribute or a non-negative `tabindex`.
    pub(crate) fn clickable_nodes(snapshot: &Value) -> HashSet<i64> {
        let strings: Vec<&str> = snapshot["strings"]
            .as_array()
            .map(|strings| {
                strings
                    .iter()
                    .map(|s| s.as_str().unwrap_or_default())
                    .collect()
            })
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

    /// Clickable backend ids per frame, read from the DOM snapshot of the local root of each frame.
    pub(crate) fn clickable_by_frame(forest: &AxForest) -> HashSet<(usize, i64)> {
        let mut by_root: HashMap<&TargetId, HashSet<i64>> = HashMap::new();
        let mut clickable = HashSet::new();
        for (index, frame) in forest.frames.iter().enumerate() {
            let Some(snapshot) = forest.dom_snapshots.get(&frame.local_root) else {
                continue;
            };
            let backends = by_root
                .entry(&frame.local_root)
                .or_insert_with(|| clickable_nodes(snapshot));
            clickable.extend(backends.iter().map(|backend| (index, *backend)));
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
        pub fn new(
            role: &str,
            name: &str,
            mode: &str,
            text: &str,
        ) -> flow_like_types::Result<Self> {
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
    use super::SnapshotElement;
    use super::tree::{self, AxForestView};
    use crate::browser::driver::PageContext;
    use crate::types::handles::locked;
    use flow_like::flow::execution::{LogLevel, context::ExecutionContext};
    use flow_like_browser::snapshot::{AxForest, SnapshotOptions};
    use flow_like_browser::{RefAllocator, RefTable};

    const MAX_CHILD_FRAMES: usize = 20;

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

    fn begin_snapshot(
        ctx: &PageContext,
        forest: &AxForest,
        view: &AxForestView,
        scope_ref: Option<&str>,
    ) -> flow_like_types::Result<(RefAllocator, Option<(usize, usize)>)> {
        let main = tree::main_frame(forest)?;
        locked(&ctx.slot.refs, |refs| {
            let scope = scope_ref
                .map(|scope_ref| tree::scope_start(scope_ref, &refs.table, forest, view))
                .transpose()?;
            Ok((refs.table.begin(&forest.page, &main.loader_id), scope))
        })
    }

    fn walk_and_render(
        forest: &AxForest,
        view: &AxForestView,
        request: &SnapshotRequest,
        scope: Option<(usize, usize)>,
        allocator: &mut RefAllocator,
    ) -> tree::Rendered {
        let options = tree::WalkOptions {
            interactive_only: request.interactive_only,
            max_depth: request.max_depth,
            clickable: tree::clickable_by_frame(forest),
        };
        let mut walker = tree::Walker::new(&options, view);
        let nodes = match scope {
            Some((frame, index)) => walker.walk(frame, index),
            None => view
                .tree(0)
                .root()
                .map(|root| walker.walk(0, root))
                .unwrap_or_default(),
        };
        tree::render(
            &nodes,
            request.interactive_only,
            request.max_chars,
            allocator,
            &tree::frame_refs(forest),
        )
    }

    fn store_snapshot(ctx: &PageContext, table: RefTable, elements: Vec<SnapshotElement>) {
        locked(&ctx.slot.refs, |refs| {
            refs.table = table;
            refs.elements = elements;
        });
    }

    fn current_elements(ctx: &PageContext) -> Option<Vec<SnapshotElement>> {
        let main_loader = ctx.page.main_loader();
        locked(&ctx.slot.refs, |refs| {
            tree::describes_document(&refs.table, ctx.page.target_id(), main_loader.as_ref())
                .then(|| refs.elements.clone())
        })
    }

    pub(crate) async fn take_snapshot(
        context: &mut ExecutionContext,
        ctx: &PageContext,
        request: &SnapshotRequest,
    ) -> flow_like_types::Result<SnapshotOutput> {
        let forest = ctx
            .page
            .accessibility_forest(SnapshotOptions {
                max_child_frames: MAX_CHILD_FRAMES,
                capture_dom_snapshots: request.detect_clickable,
            })
            .await?;
        let view = AxForestView::new(&forest)?;
        for warning in forest.warnings.iter().chain(&view.warnings) {
            context.log_message(warning, LogLevel::Warn);
        }
        let main = tree::main_frame(&forest)?;
        let (mut allocator, scope) =
            begin_snapshot(ctx, &forest, &view, request.scope_ref.as_deref())?;
        let rendered = walk_and_render(&forest, &view, request, scope, &mut allocator);
        let generation = allocator.generation();
        store_snapshot(ctx, allocator.finish(), rendered.elements.clone());
        Ok(SnapshotOutput {
            text: rendered.text,
            elements: rendered.elements,
            url: format!(
                "{}{}",
                main.url,
                main.url_fragment.as_deref().unwrap_or_default()
            ),
            title: view.title(),
            generation,
            truncated: rendered.truncated,
        })
    }

    /// Latest snapshot elements when the document is unchanged, otherwise a fresh full snapshot.
    pub(crate) async fn searchable_elements(
        context: &mut ExecutionContext,
        ctx: &PageContext,
        refresh: bool,
    ) -> flow_like_types::Result<Vec<SnapshotElement>> {
        if !refresh {
            // Page::url runs inside run_op, so a navigation the previous node started commits
            // before the main loader is compared.
            ctx.page.url().await?;
            if let Some(elements) = current_elements(ctx) {
                return Ok(elements);
            }
        }
        let request = SnapshotRequest {
            interactive_only: false,
            max_depth: None,
            scope_ref: None,
            max_chars: usize::MAX,
            detect_clickable: true,
        };
        Ok(take_snapshot(context, ctx, &request).await?.elements)
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
            "Captures the page accessibility tree as compact text with element refs (e1, e2, …) that any selector pin accepts as a Ref selector. Refs stay valid until the page navigates or the element is removed; a stale ref fails with a request to take a new snapshot. Covers the current tab including same-site and cross-site iframes (up to 20 frames). Requires Chrome or Edge.",
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
        let page = session.browser_page(context).await?;
        let snapshot = take_snapshot(context, &page, &request).await?;
        drop(page);
        context
            .set_pin_value("snapshot", json!(snapshot.text))
            .await?;
        context
            .set_pin_value("elements", json!(snapshot.elements))
            .await?;
        context.set_pin_value("url", json!(snapshot.url)).await?;
        context
            .set_pin_value("title", json!(snapshot.title))
            .await?;
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
        let page = session.browser_page(context).await?;
        let elements = searchable_elements(context, &page, refresh).await?;
        drop(page);
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
    use flow_like_browser::snapshot::{AxForest, AxFrame, FrameOwner};
    use flow_like_browser::types::{FrameId, LoaderId, TargetId};
    use flow_like_browser::{BrowserError, NodeRef, RefAllocator, RefTable};
    use flow_like_types::{Value, json::json};
    use std::collections::{HashMap, HashSet};

    const LOGIN_FIXTURE: &str = include_str!("../../tests/fixtures/ax_tree_login.json");
    const PAGE: &str = "T1";
    const MAIN_LOADER: &str = "L1";

    fn ax_frame(
        frame_id: &str,
        loader: &str,
        local_root: &str,
        tree: &Value,
        owner: Option<FrameOwner>,
    ) -> AxFrame {
        AxFrame {
            frame_id: FrameId::from(frame_id),
            loader_id: LoaderId::from(loader),
            local_root: TargetId::from(local_root),
            url: format!("https://example.com/{frame_id}"),
            url_fragment: None,
            owner,
            nodes: tree["nodes"].clone(),
        }
    }

    fn forest(frames: Vec<AxFrame>) -> AxForest {
        AxForest {
            page: TargetId::from(PAGE),
            frames,
            dom_snapshots: HashMap::new(),
            warnings: Vec::new(),
        }
    }

    fn owner(parent: usize, backend_node_id: i64) -> Option<FrameOwner> {
        Some(FrameOwner {
            parent,
            backend_node_id,
        })
    }

    fn login_forest() -> AxForest {
        let fixture: Value = flow_like_types::json::from_str(LOGIN_FIXTURE).unwrap();
        forest(vec![
            ax_frame(PAGE, MAIN_LOADER, PAGE, &fixture["main"], None),
            ax_frame("F1", "LF1", PAGE, &fixture["frame"], owner(0, 90)),
        ])
    }

    fn begin(table: &RefTable) -> RefAllocator {
        table.begin(&TargetId::from(PAGE), &LoaderId::from(MAIN_LOADER))
    }

    fn node_ref(forest: &AxForest, frame: usize, backend_node_id: i64) -> NodeRef {
        NodeRef {
            backend_node_id,
            ..frame_refs(forest)[frame].clone()
        }
    }

    fn walk_options(interactive_only: bool, max_depth: Option<u32>) -> WalkOptions {
        WalkOptions {
            interactive_only,
            max_depth,
            clickable: HashSet::new(),
        }
    }

    fn render_forest(
        forest: &AxForest,
        options: &WalkOptions,
        max_chars: usize,
        previous: &RefTable,
    ) -> (Rendered, RefTable) {
        let view = AxForestView::new(forest).unwrap();
        let root = view.tree(0).root().unwrap();
        let nodes = Walker::new(options, &view).walk(0, root);
        let mut allocator = begin(previous);
        let rendered = render(
            &nodes,
            options.interactive_only,
            max_chars,
            &mut allocator,
            &frame_refs(forest),
        );
        (rendered, allocator.finish())
    }

    fn snapshot(
        interactive_only: bool,
        max_depth: Option<u32>,
        max_chars: usize,
    ) -> (Rendered, RefTable) {
        let options = WalkOptions {
            clickable: HashSet::from([(0, 50)]),
            ..walk_options(interactive_only, max_depth)
        };
        render_forest(&login_forest(), &options, max_chars, &RefTable::default())
    }

    fn next_fresh_ref(table: &RefTable) -> String {
        begin(table)
            .propose(&node_ref(&login_forest(), 0, 9_999), "button", "Unseen")
            .reference
    }

    fn stale_reference(error: &flow_like_types::Error) -> Option<&str> {
        match error.downcast_ref::<BrowserError>() {
            Some(BrowserError::StaleRef { reference }) => Some(reference.as_str()),
            _ => None,
        }
    }

    #[test]
    fn interactive_snapshot_lists_refs_in_document_order() {
        let (rendered, table) = snapshot(true, None, 15_000);
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
        assert_eq!(next_fresh_ref(&table), "e7");
        let entry = table.lookup("e6").unwrap();
        assert_eq!(entry.node, node_ref(&login_forest(), 1, 91));
        assert_eq!(
            (entry.role.as_str(), entry.name.as_str()),
            ("button", "Inside frame")
        );
        assert_eq!(
            rendered.elements[3].url.as_deref(),
            Some("https://example.com/forgot")
        );
    }

    #[test]
    fn full_snapshot_keeps_structure_and_text() {
        let (rendered, _) = snapshot(false, None, 15_000);
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
        assert_eq!(
            rendered.elements[1].text.as_deref(),
            Some("Please sign in to continue.")
        );
        let view = AxForestView::new(&login_forest()).unwrap();
        assert_eq!(view.title(), "Login – Example");
        assert!(view.warnings.is_empty());
    }

    #[test]
    fn depth_and_character_limits() {
        let (shallow, _) = snapshot(false, Some(0), 15_000);
        assert!(!shallow.text.contains("textbox"));
        assert!(shallow.text.contains("- form \"Login\" [ref=e3]"));
        let (cut, table) = snapshot(true, None, 120);
        assert!(cut.truncated);
        assert!(cut.text.chars().count() <= 120);
        assert_eq!(cut.elements.len(), cut.text.lines().count());
        for element in &cut.elements {
            assert!(
                table
                    .lookup(element.element_ref.as_deref().unwrap())
                    .is_ok()
            );
        }
        assert_eq!(
            next_fresh_ref(&table),
            format!("e{}", cut.elements.len() + 1),
            "a line cut by Max Characters consumes no ref"
        );
    }

    #[test]
    fn a_link_line_is_cut_together_with_its_url_line() {
        let (full, _) = snapshot(true, None, 15_000);
        let lines: Vec<&str> = full.text.lines().collect();
        assert!(lines[3].starts_with("- link \"Forgot password?\" [ref=e4]"));
        assert!(lines[4].starts_with("  - /url:"));
        let (cut, table) = snapshot(true, None, lines[..4].join("\n").chars().count());
        assert!(cut.truncated);
        assert_eq!(cut.text, lines[..3].join("\n"));
        assert_eq!(cut.elements.len(), 3);
        for reference in cut
            .text
            .split("[ref=")
            .skip(1)
            .filter_map(|rest| rest.split(']').next())
        {
            assert!(
                table.lookup(reference).is_ok(),
                "{reference} is shown but was not stored"
            );
        }
        assert_eq!(next_fresh_ref(&table), "e4");
    }

    #[test]
    fn refs_are_reused_for_unchanged_elements() {
        let forest = login_forest();
        let mut earlier = begin(&RefTable::default());
        for filler in 1..40 {
            let node = node_ref(&forest, 0, 1_000 + filler);
            let proposed = earlier.propose(&node, "text", "");
            earlier.commit(proposed, node, "text".into(), String::new());
        }
        let button = node_ref(&forest, 0, 21);
        let proposed = earlier.propose(&button, "button", "Sign in");
        assert_eq!(proposed.reference, "e40");
        earlier.commit(proposed, button, "button".into(), "Sign in".into());
        let (rendered, table) = render_forest(
            &forest,
            &walk_options(true, None),
            15_000,
            &earlier.finish(),
        );
        assert!(
            rendered
                .text
                .contains("- button \"Sign in\" [disabled] [ref=e40]")
        );
        assert!(
            rendered
                .text
                .contains("- textbox \"Email\" [focused] [required] [ref=e41]")
        );
        assert_eq!(next_fresh_ref(&table), "e45");
    }

    #[test]
    fn frames_with_colliding_backend_ids_get_their_own_refs() {
        let tree = |button: &str, iframe: Option<&str>| {
            let mut nodes = vec![
                json!({ "nodeId": "1", "role": { "value": "RootWebArea" }, "childIds": ["2", "3"], "backendDOMNodeId": 1 }),
                json!({ "nodeId": "2", "parentId": "1", "role": { "value": "button" }, "name": { "value": button }, "backendDOMNodeId": 5 }),
            ];
            if let Some(iframe) = iframe {
                nodes.push(json!({ "nodeId": "3", "parentId": "1", "role": { "value": "Iframe" }, "name": { "value": iframe }, "backendDOMNodeId": 7 }));
            }
            json!({ "nodes": nodes })
        };
        let forest = forest(vec![
            ax_frame(PAGE, MAIN_LOADER, PAGE, &tree("Main", Some("Ads")), None),
            ax_frame(
                "F2",
                "LF2",
                "F2",
                &tree("Inside", Some("Nested")),
                owner(0, 7),
            ),
            ax_frame("F3", "LF3", "F3", &tree("Deep", None), owner(1, 7)),
        ]);
        let (rendered, table) = render_forest(
            &forest,
            &walk_options(false, None),
            15_000,
            &RefTable::default(),
        );
        assert_eq!(
            rendered.text,
            [
                "- button \"Main\" [ref=e1]",
                "- iframe \"Ads\" [ref=e2]:",
                "  - button \"Inside\" [ref=e3]",
                "  - iframe \"Nested\" [ref=e4]:",
                "    - button \"Deep\" [ref=e5]",
            ]
            .join("\n")
        );
        assert_eq!(table.lookup("e1").unwrap().node, node_ref(&forest, 0, 5));
        assert_eq!(table.lookup("e3").unwrap().node, node_ref(&forest, 1, 5));
        assert_eq!(table.lookup("e4").unwrap().node, node_ref(&forest, 1, 7));
        assert_eq!(table.lookup("e5").unwrap().node, node_ref(&forest, 2, 5));
    }

    #[test]
    fn unparsable_child_frames_are_left_out_with_a_warning() {
        let mut forest = login_forest();
        forest.frames[1].nodes = json!({ "not": "a list" });
        let view = AxForestView::new(&forest).unwrap();
        assert_eq!(view.warnings.len(), 1);
        assert!(view.warnings[0].starts_with("Frame F1 was left out of the snapshot"));
        let nodes = Walker::new(&walk_options(true, None), &view).walk(0, 0);
        assert!(nodes.iter().all(|node| node.element.name != "Inside frame"));
        forest.frames.clear();
        assert!(AxForestView::new(&forest).is_err());
    }

    #[test]
    fn clickable_detection_uses_own_pointer_and_handlers() {
        let snapshot = json!({
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
    fn clickable_elements_are_tracked_per_frame_through_their_local_root() {
        let dom = |backend: i64| {
            json!({
                "strings": ["pointer", "auto"],
                "documents": [{
                    "nodes": {
                        "backendNodeId": [1, backend],
                        "nodeType": [9, 1],
                        "parentIndex": [-1, 0],
                        "attributes": [[], []]
                    },
                    "layout": { "nodeIndex": [0, 1], "styles": [[1], [0]] }
                }]
            })
        };
        let empty = json!({ "nodes": [] });
        let mut forest = forest(vec![
            ax_frame(PAGE, MAIN_LOADER, PAGE, &empty, None),
            ax_frame("F1", "LF1", PAGE, &empty, owner(0, 3)),
            ax_frame("F2", "LF2", "F2", &empty, owner(0, 4)),
        ]);
        forest.dom_snapshots.insert(TargetId::from(PAGE), dom(10));
        forest.dom_snapshots.insert(TargetId::from("F2"), dom(20));
        assert_eq!(
            clickable_by_frame(&forest),
            HashSet::from([(0, 10), (1, 10), (2, 20)])
        );
    }

    #[test]
    fn element_queries_match_role_name_and_text() {
        let (rendered, _) = snapshot(false, None, 15_000);
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
        let tree = json!({ "nodes": [
            { "nodeId": "1", "role": { "value": "RootWebArea" }, "childIds": ["2"], "backendDOMNodeId": 1 },
            { "nodeId": "2", "parentId": "1", "role": { "value": "combobox" }, "name": { "value": "Country" },
              "value": { "value": "Germany" }, "childIds": ["3"], "backendDOMNodeId": 2 },
            { "nodeId": "3", "parentId": "2", "role": { "value": "MenuListPopup" }, "childIds": ["4"], "backendDOMNodeId": 3 },
            { "nodeId": "4", "parentId": "3", "role": { "value": "option" }, "name": { "value": "Germany" },
              "properties": [{ "name": "selected", "value": { "value": true } }], "backendDOMNodeId": 4 }
        ]});
        let forest = forest(vec![ax_frame(PAGE, MAIN_LOADER, PAGE, &tree, None)]);
        let render_mode = |interactive_only: bool| {
            render_forest(
                &forest,
                &walk_options(interactive_only, None),
                15_000,
                &RefTable::default(),
            )
            .0
            .text
        };
        assert_eq!(
            render_mode(true),
            "- combobox \"Country\" [ref=e1]: Germany"
        );
        assert_eq!(
            render_mode(false),
            "- combobox \"Country\" [ref=e1]: Germany\n  - option \"Germany\" [selected] [ref=e2]"
        );
    }

    #[test]
    fn scope_refs_resolve_through_the_table_of_the_same_document() {
        let forest = login_forest();
        let view = AxForestView::new(&forest).unwrap();
        let (_, table) = snapshot(true, None, 15_000);
        let (frame, index) = scope_start("[ref=e6]", &table, &forest, &view).unwrap();
        assert_eq!(frame, 1);
        let nodes = Walker::new(&walk_options(true, None), &view).walk(frame, index);
        let mut allocator = begin(&table);
        let rendered = render(&nodes, true, 15_000, &mut allocator, &frame_refs(&forest));
        assert_eq!(rendered.text, "- button \"Inside frame\" [ref=e6]");

        let scope_error =
            |scope: &str, table: &RefTable| scope_start(scope, table, &forest, &view).unwrap_err();
        assert_eq!(stale_reference(&scope_error("e99", &table)), Some("e99"));
        let reloaded = RefTable::default()
            .begin(&TargetId::from(PAGE), &LoaderId::from("L2"))
            .finish();
        assert_eq!(stale_reference(&scope_error("e6", &reloaded)), Some("e6"));
        assert_eq!(
            stale_reference(&scope_error("e6", &RefTable::default())),
            Some("e6")
        );
        assert_eq!(
            scope_error("sign-in", &table).to_string(),
            "Scope 'sign-in' is not a snapshot ref such as 'e12'"
        );
    }

    #[test]
    fn a_snapshot_describes_only_its_own_page_and_main_document() {
        let (_, table) = snapshot(true, None, 15_000);
        let page = TargetId::from(PAGE);
        let main_loader = LoaderId::from(MAIN_LOADER);
        assert!(describes_document(&table, &page, Some(&main_loader)));
        assert!(!describes_document(
            &table,
            &page,
            Some(&LoaderId::from("L2"))
        ));
        assert!(!describes_document(
            &table,
            &TargetId::from("T9"),
            Some(&main_loader)
        ));
        assert!(!describes_document(&table, &page, None));
        assert!(!describes_document(&RefTable::default(), &page, None));
    }

    #[test]
    fn stale_error_wording() {
        assert_eq!(
            stale_ref_error("e12").to_string(),
            "Stale element ref 'e12' — take a new browser snapshot"
        );
        assert_eq!(
            BrowserError::StaleRef {
                reference: "e12".into()
            }
            .to_string(),
            STALE_REF_ERROR.replace("{ref}", "e12")
        );
        assert_eq!(stale_reference(&stale_ref_error("e12")), Some("e12"));
    }
}
