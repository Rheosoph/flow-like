//! `@collapse` groups: the Collapsed layer the nodes of each statement are placed in, and the
//! layer commands that make the board's groups match what the text writes.
//!
//! A group is presentation only. `plan_block` opens every group into its enclosing block, so
//! execution order and bindings never see one; this module decides where the nodes go. New nodes
//! are queued into the innermost group's layer. Existing anchored statements record where the text
//! writes them, and [`StructuralPlanner::finish_groups`] turns the differences into renames,
//! re-parents, moves and the removal of groups the edit leaves empty. Those commands run after
//! every wire, so `BridgeLayersCleanup` re-bridges a finished graph.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use flow_like_ast::model::{Block, BoardAst, CollapseBlock, ModuleDecl, Stmt};

use super::{
    NodeEntity, PlannedStmt, StructuralPlanner, all_board_nodes, board_entity_location_labels,
    find_board_node, is_exec_pin,
};
use crate::flow::board::LayerType;
use crate::flow::copilot::{BoardCommand, NodePosition};
use crate::flow::node::Node;
use crate::flow::pin::PinType;

/// Where the statement being planned is written.
#[derive(Debug, Clone, Default)]
pub(super) struct GroupContext {
    /// The innermost group: a live layer id or a same-batch `$ref`. `None` is the section root.
    pub(super) layer: Option<String>,
    /// Whether a `@collapse` block encloses the statement, so its placement is written.
    pub(super) explicit: bool,
}

/// The prefix `normalize_unavailable_anchors` gives a group anchor this board does not know: text
/// copied from another board. It names no layer here, but every block carrying it is one group.
pub(super) const FOREIGN_GROUP_ANCHOR: &str = "\u{0}foreign-group:";

/// What makes two blocks one new group: the same foreign anchor anywhere in the document, else the
/// same name under the same parent within one section.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum NewGroupKey {
    Foreign(String),
    Named {
        section: usize,
        parent: Option<String>,
        name: String,
    },
}

/// A group this document creates.
#[derive(Debug, Clone)]
struct CreatedGroup {
    /// The enclosing group, `None` directly under the section root.
    parent: Option<String>,
    /// The section root the group lives in.
    root: Option<String>,
}

/// Where the text writes a live group.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WrittenGroup {
    name: String,
    /// The enclosing group, or the section root when the block is not nested in one.
    parent: Option<String>,
}

/// Where the text writes an existing node.
#[derive(Debug, Clone)]
struct Placement {
    /// The group, `None` for the section root.
    layer: Option<String>,
    explicit: bool,
    root: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct GroupPlan {
    /// Written groups are authoritative: a Replace-mode document that writes at least one
    /// `@collapse` block. Otherwise only statements written inside a block are placed.
    pub(super) authoritative: bool,
    /// The group of the statement being planned.
    pub(super) current: GroupContext,
    /// The section root of the section being planned: the `target_layer` its blocks plan with.
    root: Option<String>,
    /// Counts sections; an unanchored group is shared only within one section.
    section: usize,
    /// Every group this document creates -> its `$ref`.
    created: HashMap<NewGroupKey, String>,
    created_groups: BTreeMap<String, CreatedGroup>,
    /// Live groups the text writes, by layer id.
    written: BTreeMap<String, WrittenGroup>,
    /// Existing statement nodes -> where the text writes them.
    placements: BTreeMap<String, Placement>,
    /// Layers an event move re-parents into a module, with that module (`None` = board root).
    relocated_layers: HashMap<String, Option<String>>,
    /// Nodes an event move re-homes directly, with their new layer.
    relocated_nodes: HashMap<String, Option<String>>,
    /// Renames, re-parents, moves and removals, emitted after every other command.
    pub(super) commands: Vec<BoardCommand>,
}

/// Whether the document writes any `@collapse` block.
pub(super) fn document_has_groups(ast: &BoardAst) -> bool {
    fn block_has_groups(block: &Block) -> bool {
        block.stmts.iter().any(|stmt| match stmt {
            Stmt::Collapse(_) => true,
            Stmt::Branch { arms, .. } => arms.iter().any(|arm| block_has_groups(&arm.body)),
            Stmt::Loop { body, .. } => block_has_groups(body),
            Stmt::Handler(event) => block_has_groups(&event.body),
            _ => false,
        })
    }
    fn module_has_groups(module: &ModuleDecl) -> bool {
        module
            .functions
            .iter()
            .any(|function| block_has_groups(&function.body))
            || module
                .events
                .iter()
                .any(|event| block_has_groups(&event.body))
            || module.detached.iter().any(block_has_groups)
            || module.modules.iter().any(module_has_groups)
    }
    ast.functions
        .iter()
        .any(|function| block_has_groups(&function.body))
        || ast.events.iter().any(|event| block_has_groups(&event.body))
        || ast.detached.iter().any(block_has_groups)
        || ast.modules.iter().any(module_has_groups)
}

fn is_group_layer(kind: &LayerType) -> bool {
    matches!(kind, LayerType::Collapsed | LayerType::Macro)
}

impl StructuralPlanner<'_> {
    /// Start a top-level section whose blocks plan into `root`.
    pub(super) fn begin_group_section(&mut self, root: Option<String>) {
        self.groups.section += 1;
        self.groups.root = root;
        self.groups.current = GroupContext::default();
    }

    /// The layer a new node goes to: the current group when the caller places it in the section
    /// root, otherwise wherever the caller asked.
    pub(super) fn group_target_layer(&self, target_layer: Option<String>) -> Option<String> {
        match &self.groups.current.layer {
            Some(group) if target_layer == self.groups.root => Some(group.clone()),
            _ => target_layer,
        }
    }

    /// The section root a layer belongs to: a group resolves to the root it lives in, every other
    /// layer is its own.
    pub(super) fn group_section_root(&self, layer: &str) -> Option<String> {
        if let Some(created) = self.groups.created_groups.get(layer) {
            return created.root.clone();
        }
        match self.existing.layers.get(layer) {
            Some(live) if is_group_layer(&live.r#type) => self.live_group_root(layer),
            _ => Some(layer.to_string()),
        }
    }

    /// Record the layers and nodes an event move re-homes, so their groups are judged where they
    /// end up.
    pub(super) fn note_group_relocations(&mut self, ids: &[String], target: Option<&str>) {
        for id in ids {
            if self.existing.layers.contains_key(id) {
                self.groups
                    .relocated_layers
                    .insert(id.clone(), target.map(str::to_string));
            } else {
                self.groups
                    .relocated_nodes
                    .insert(id.clone(), target.map(str::to_string));
            }
        }
    }

    /// The statements of `block` with every group opened, each paired with the group it is
    /// written in. Groups resolve here, so a new group has its layer before its first statement.
    pub(super) fn grouped_statements<'s>(
        &mut self,
        block: &'s Block,
    ) -> Vec<(&'s Stmt, GroupContext)> {
        let mut out = Vec::with_capacity(block.stmts.len());
        let context = self.groups.current.clone();
        self.collect_grouped(&block.stmts, &context, &mut out);
        out
    }

    fn collect_grouped<'s>(
        &mut self,
        stmts: &'s [Stmt],
        context: &GroupContext,
        out: &mut Vec<(&'s Stmt, GroupContext)>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Collapse(group) => {
                    let inner = self.enter_group(group, context);
                    self.collect_grouped(&group.body.stmts, &inner, out);
                }
                other => out.push((other, context.clone())),
            }
        }
    }

    fn enter_group(&mut self, group: &CollapseBlock, context: &GroupContext) -> GroupContext {
        let mut layer = if group.absolute {
            None
        } else {
            context.layer.clone()
        };
        let last = group.path.len().saturating_sub(1);
        for (index, name) in group.path.iter().enumerate() {
            let anchor = group
                .anchor
                .as_deref()
                .map(str::trim)
                .filter(|anchor| index == last && !anchor.is_empty());
            layer = match anchor.map(|anchor| anchor.strip_prefix(FOREIGN_GROUP_ANCHOR)) {
                Some(Some(foreign)) => {
                    let key = NewGroupKey::Foreign(foreign.to_string());
                    Some(self.new_group(key, layer, name))
                }
                Some(None) => self.resolve_anchored_group(layer, name, anchor.unwrap_or_default()),
                None => {
                    let key = NewGroupKey::Named {
                        section: self.groups.section,
                        parent: layer.clone(),
                        name: name.to_string(),
                    };
                    Some(self.new_group(key, layer, name))
                }
            };
        }
        GroupContext {
            layer,
            explicit: true,
        }
    }

    /// The new group a block without a live anchor names, created on its first block.
    fn new_group(&mut self, key: NewGroupKey, parent: Option<String>, name: &str) -> String {
        if let Some(existing) = self.groups.created.get(&key) {
            return existing.clone();
        }
        let ref_id = format!("${}", self.next_ref);
        self.next_ref += 1;
        let container = parent.clone().or_else(|| self.groups.root.clone());
        let position = self.next_position(container.as_deref());
        self.add_commands.push(BoardCommand::CreateLayer {
            name: name.to_string(),
            ref_id: Some(ref_id.clone()),
            layer_type: Some("Collapsed".to_string()),
            node_ids: Vec::new(),
            pins: None,
            position: Some(position),
            color: None,
            target_layer: container,
            cache: None,
            summary: Some(format!("Create group {name}")),
        });
        self.groups.created.insert(key, ref_id.clone());
        self.groups.created_groups.insert(
            ref_id.clone(),
            CreatedGroup {
                parent,
                root: self.groups.root.clone(),
            },
        );
        ref_id
    }

    /// The live group an anchored block names. A wrong-kind or foreign-scope anchor is a
    /// diagnostic, and the block's statements stay in the enclosing group.
    fn resolve_anchored_group(
        &mut self,
        parent: Option<String>,
        name: &str,
        anchor: &str,
    ) -> Option<String> {
        let Some(layer) = self.existing.layers.get(anchor) else {
            self.result.diagnostics.push(format!(
                "group `{name}` keeps anchor `{anchor}`, but that ID identifies {}, not a group layer; remove the anchor to create a new group",
                board_entity_location_labels(self.existing, anchor).join(" and ")
            ));
            return parent;
        };
        if !is_group_layer(&layer.r#type) {
            self.result.diagnostics.push(format!(
                "group `{name}` keeps anchor `{anchor}`, which is a {:?} layer, not a group; remove the anchor to create a new group",
                layer.r#type
            ));
            return parent;
        }
        let live_root = self.live_group_root(anchor);
        if live_root != self.groups.root {
            self.result.diagnostics.push(format!(
                "group `{name}` keeps anchor `{anchor}`, but that group belongs to {} while this block is in {}; remove the anchor to create a new group here",
                self.semantic_scope_label(live_root.as_deref()),
                self.semantic_scope_label(self.groups.root.as_deref())
            ));
            return parent;
        }
        let written = WrittenGroup {
            name: name.to_string(),
            parent: parent.or_else(|| self.groups.root.clone()),
        };
        match self.groups.written.get(anchor) {
            Some(earlier) if *earlier != written => {
                self.result.diagnostics.push(format!(
                    "group `{name}` (anchor `{anchor}`) is written twice with a different name or enclosing group; every block of one group must agree"
                ));
            }
            Some(_) => {}
            None => {
                self.groups.written.insert(anchor.to_string(), written);
            }
        }
        Some(anchor.to_string())
    }

    /// The section root of a live group: the Function or module layer above it, `None` for the
    /// board root, after any event move re-homes it or an ancestor.
    fn live_group_root(&self, layer: &str) -> Option<String> {
        let mut current = Some(layer);
        let mut seen = HashSet::new();
        while let Some(id) = current {
            if !seen.insert(id) {
                return None;
            }
            if let Some(target) = self.groups.relocated_layers.get(id) {
                return target.clone();
            }
            let live = self.existing.layers.get(id)?;
            if !is_group_layer(&live.r#type) {
                return Some(id.to_string());
            }
            current = live
                .parent_id
                .as_deref()
                .filter(|parent| !parent.is_empty());
        }
        None
    }

    /// The enclosing layer of a live group once this plan's event moves applied.
    fn live_group_parent(&self, layer: &str) -> Option<String> {
        if let Some(target) = self.groups.relocated_layers.get(layer) {
            return target.clone();
        }
        self.existing
            .layers
            .get(layer)
            .and_then(|live| live.parent_id.clone())
            .filter(|parent| !parent.is_empty())
    }

    /// Remember where the text writes the node of an anchored statement. The event's own entry
    /// is a header, never a placed statement.
    pub(super) fn record_group_placement(&mut self, stmt: &Stmt, planned: Option<&PlannedStmt>) {
        let anchor = match stmt {
            Stmt::Handler(event) => event.anchor.as_deref(),
            other => other.anchor(),
        };
        let Some(anchor) = anchor.map(str::trim).filter(|anchor| !anchor.is_empty()) else {
            return;
        };
        if self.current_event_entry.as_deref() == Some(anchor) {
            return;
        }
        let resolved = match stmt {
            Stmt::Handler(_) => find_board_node(self.existing, anchor).is_some(),
            _ => matches!(
                planned.map(|planned| &planned.entity),
                Some(NodeEntity::Existing(id)) if id == anchor
            ),
        };
        if !resolved {
            return;
        }
        self.groups
            .placements
            .entry(anchor.to_string())
            .or_insert_with(|| Placement {
                layer: self.groups.current.layer.clone(),
                explicit: self.groups.current.explicit,
                root: self.groups.root.clone(),
            });
    }

    /// The layer an existing node sits in once this plan's function and event moves applied.
    fn planned_node_layer(&self, node_id: &str) -> Option<String> {
        if let Some(layer) = self.planned_node_moves.get(node_id) {
            return Some(layer.clone());
        }
        if let Some(layer) = self.groups.relocated_nodes.get(node_id) {
            return layer.clone();
        }
        find_board_node(self.existing, node_id)
            .and_then(|node| node.layer.clone())
            .filter(|layer| !layer.is_empty())
    }

    /// Turn the recorded placements into commands: renames and re-parents of written groups,
    /// node moves (pure helpers travel along), and the removal of groups this edit empties.
    pub(super) fn finish_groups(&mut self) {
        let mut destinations: BTreeMap<String, Option<String>> = BTreeMap::new();
        for (node_id, placement) in &self.groups.placements {
            if !self.groups.authoritative && !placement.explicit {
                continue;
            }
            let desired = placement.layer.clone().or_else(|| placement.root.clone());
            if self.planned_node_layer(node_id) != desired {
                destinations.insert(node_id.clone(), desired);
            }
        }
        self.travel_pure_helpers(&mut destinations);

        let mut commands = Vec::new();
        let mut reparented: BTreeMap<String, Option<String>> = BTreeMap::new();
        for (layer_id, written) in &self.groups.written {
            let Some(layer) = self.existing.layers.get(layer_id) else {
                continue;
            };
            if layer.name != written.name {
                commands.push(BoardCommand::RenameLayer {
                    layer_id: layer_id.clone(),
                    name: written.name.clone(),
                    summary: Some(format!("Rename group {} to {}", layer.name, written.name)),
                });
            }
            if self.groups.authoritative && self.live_group_parent(layer_id) != written.parent {
                reparented.insert(layer_id.clone(), written.parent.clone());
            }
        }
        for (layer_id, parent) in &reparented {
            let name = self
                .existing
                .layers
                .get(layer_id)
                .map(|layer| layer.name.as_str())
                .unwrap_or(layer_id);
            commands.push(BoardCommand::MoveToLayer {
                ids: vec![layer_id.clone()],
                target_layer: parent.clone(),
                summary: Some(format!("Move group {name} to its written position")),
            });
        }

        let mut by_destination: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
        for (node_id, destination) in &destinations {
            by_destination
                .entry(destination.clone())
                .or_default()
                .push(node_id.clone());
        }
        for (destination, ids) in &by_destination {
            let label = match destination {
                Some(layer) => self.group_label(layer),
                None => "the board root".to_string(),
            };
            self.result.corrections.push(format!(
                "Moved {} existing node(s) to {label} to match their written group.",
                ids.len()
            ));
            commands.push(BoardCommand::MoveToLayer {
                ids: ids.clone(),
                target_layer: destination.clone(),
                summary: Some(format!("Move {} node(s) to {label}", ids.len())),
            });
        }

        if self.groups.authoritative {
            for layer_id in self.emptied_groups(&destinations, &reparented) {
                let name = self
                    .existing
                    .layers
                    .get(&layer_id)
                    .map(|layer| layer.name.clone())
                    .unwrap_or_else(|| layer_id.clone());
                commands.push(BoardCommand::RemoveLayer {
                    layer_id,
                    summary: Some(format!("Ungroup {name}, which this edit left empty")),
                });
            }
        }

        self.drop_empty_created_groups(&destinations, &reparented);
        self.position_created_groups(&destinations);
        self.groups.commands = commands;
    }

    fn group_label(&self, layer: &str) -> String {
        if let Some(live) = self.existing.layers.get(layer) {
            return match live.r#type {
                LayerType::Collapsed | LayerType::Macro => format!("group `{}`", live.name),
                _ => self.semantic_scope_label(Some(layer)),
            };
        }
        self.add_commands
            .iter()
            .find_map(|command| match command {
                BoardCommand::CreateLayer {
                    ref_id: Some(ref_id),
                    name,
                    ..
                } if ref_id == layer => Some(format!("new group `{name}`")),
                _ => None,
            })
            .unwrap_or_else(|| self.semantic_scope_label(Some(layer)))
    }

    /// Pure helpers feeding moved nodes travel with them when every consumer goes to the same
    /// place and the helper sat in the layer its consumer left. A helper the text anchors is
    /// placed by its own statement.
    fn travel_pure_helpers(&self, destinations: &mut BTreeMap<String, Option<String>>) {
        let anchored: HashSet<&str> = self.groups.placements.keys().map(String::as_str).collect();
        loop {
            let mut travelling: Vec<(String, Option<String>)> = Vec::new();
            for (node_id, destination) in destinations.iter() {
                let Some(node) = find_board_node(self.existing, node_id) else {
                    continue;
                };
                let from = self.planned_node_layer(node_id);
                for producer in self.data_neighbours(node, PinType::Input) {
                    if destinations.contains_key(&producer.id)
                        || anchored.contains(producer.id.as_str())
                        || producer.pins.values().any(is_exec_pin)
                        || self.planned_node_layer(&producer.id) != from
                    {
                        continue;
                    }
                    let consumers = self.data_neighbours(producer, PinType::Output);
                    let all_follow = !consumers.is_empty()
                        && consumers.iter().all(|consumer| {
                            destinations.get(&consumer.id) == Some(destination)
                                || travelling
                                    .iter()
                                    .any(|(id, target)| *id == consumer.id && target == destination)
                        });
                    if all_follow && !travelling.iter().any(|(id, _)| *id == producer.id) {
                        travelling.push((producer.id.clone(), destination.clone()));
                    }
                }
            }
            if travelling.is_empty() {
                return;
            }
            destinations.extend(travelling);
        }
    }

    /// The nodes wired to `node`'s data pins of `direction`, followed through group bridge pins.
    fn data_neighbours(&self, node: &Node, direction: PinType) -> Vec<&Node> {
        let mut out: Vec<&Node> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        let mut pending: Vec<&str> = Vec::new();
        for pin in node.pins.values() {
            if pin.pin_type != direction || is_exec_pin(pin) {
                continue;
            }
            let edges = match direction {
                PinType::Input => &pin.depends_on,
                PinType::Output => &pin.connected_to,
            };
            pending.extend(edges.iter().map(String::as_str));
        }
        while let Some(pin_id) = pending.pop() {
            if !seen.insert(pin_id) {
                continue;
            }
            if let Some((owner, _)) = self.board_index.pin_owner.get(pin_id) {
                if owner.id != node.id && !out.iter().any(|known| known.id == owner.id) {
                    out.push(owner);
                }
                continue;
            }
            if let Some(bridge) = self.board_index.boundary_bridges.get(pin_id) {
                let edges = match direction {
                    PinType::Input => &bridge.depends_on,
                    PinType::Output => &bridge.connected_to,
                };
                pending.extend(edges.iter().map(String::as_str));
            }
        }
        out
    }

    /// Live groups this edit leaves without nodes, comments or child layers although they held
    /// nodes before, innermost first.
    fn emptied_groups(
        &self,
        destinations: &BTreeMap<String, Option<String>>,
        reparented: &BTreeMap<String, Option<String>>,
    ) -> Vec<String> {
        let mut filled: HashSet<String> = destinations
            .values()
            .flatten()
            .cloned()
            .chain(reparented.values().flatten().cloned())
            .collect();
        filled.extend(self.groups.written.keys().cloned());
        filled.extend(
            self.add_commands
                .iter()
                .filter_map(|command| match command {
                    BoardCommand::AddNode {
                        target_layer: Some(layer),
                        ..
                    } => Some(layer.clone()),
                    _ => None,
                }),
        );

        let mut members: HashMap<&str, Vec<&str>> = HashMap::new();
        for node in all_board_nodes(self.existing) {
            if let Some(layer) = node.layer.as_deref().filter(|layer| !layer.is_empty()) {
                members.entry(layer).or_default().push(node.id.as_str());
            }
        }
        let stays = |node_id: &str, layer: &str| match destinations.get(node_id) {
            Some(destination) => destination.as_deref() == Some(layer),
            None => !self.removed_node_ids.contains(node_id),
        };

        let mut removed: Vec<String> = Vec::new();
        loop {
            let mut changed = false;
            let mut candidates: Vec<&str> = self
                .existing
                .layers
                .values()
                .filter(|layer| is_group_layer(&layer.r#type))
                .map(|layer| layer.id.as_str())
                .collect();
            candidates.sort_unstable();
            for layer_id in candidates {
                if removed.iter().any(|known| known == layer_id) || filled.contains(layer_id) {
                    continue;
                }
                let own = members.get(layer_id).map(Vec::as_slice).unwrap_or_default();
                if own.is_empty() || own.iter().any(|node_id| stays(node_id, layer_id)) {
                    continue;
                }
                let has_comments = self
                    .existing
                    .comments
                    .values()
                    .any(|comment| comment.layer.as_deref() == Some(layer_id));
                let keeps_children = self.existing.layers.values().any(|child| {
                    child.parent_id.as_deref() == Some(layer_id)
                        && !removed.contains(&child.id)
                        && reparented
                            .get(&child.id)
                            .is_none_or(|parent| parent.as_deref() == Some(layer_id))
                });
                if has_comments || keeps_children {
                    continue;
                }
                removed.push(layer_id.to_string());
                changed = true;
            }
            if !changed {
                return removed;
            }
        }
    }

    /// A new group that ends up holding nothing is not created: an empty block, or one whose
    /// statements all failed to plan, would otherwise leave an empty box on the canvas.
    fn drop_empty_created_groups(
        &mut self,
        destinations: &BTreeMap<String, Option<String>>,
        reparented: &BTreeMap<String, Option<String>>,
    ) {
        let mut filled: HashSet<String> = destinations
            .values()
            .flatten()
            .chain(reparented.values().flatten())
            .cloned()
            .collect();
        filled.extend(
            self.add_commands
                .iter()
                .filter_map(|command| match command {
                    BoardCommand::AddNode {
                        target_layer: Some(layer),
                        ..
                    } => Some(layer.clone()),
                    _ => None,
                }),
        );
        loop {
            let parents: Vec<String> = self
                .groups
                .created_groups
                .iter()
                .filter(|(ref_id, _)| filled.contains(*ref_id))
                .filter_map(|(_, group)| group.parent.clone())
                .filter(|parent| !filled.contains(parent))
                .collect();
            if parents.is_empty() {
                break;
            }
            filled.extend(parents);
        }
        let empty: BTreeSet<String> = self
            .groups
            .created_groups
            .keys()
            .filter(|ref_id| !filled.contains(*ref_id))
            .cloned()
            .collect();
        if empty.is_empty() {
            return;
        }
        self.add_commands.retain(|command| {
            !matches!(
                command,
                BoardCommand::CreateLayer { ref_id: Some(ref_id), .. } if empty.contains(ref_id)
            )
        });
        for ref_id in &empty {
            self.groups.created_groups.remove(ref_id);
        }
    }

    /// A new group that collects existing nodes sits where they are, like a group made on the
    /// canvas, instead of in the next free slot of its parent.
    fn position_created_groups(&mut self, destinations: &BTreeMap<String, Option<String>>) {
        let mut sums: HashMap<&str, (f64, f64, usize)> = HashMap::new();
        for (node_id, destination) in destinations {
            let Some(destination) = destination.as_deref() else {
                continue;
            };
            if !self.groups.created_groups.contains_key(destination) {
                continue;
            }
            let Some((x, y, _)) =
                find_board_node(self.existing, node_id).and_then(|node| node.coordinates)
            else {
                continue;
            };
            let entry = sums.entry(destination).or_insert((0.0, 0.0, 0));
            entry.0 += f64::from(x);
            entry.1 += f64::from(y);
            entry.2 += 1;
        }
        let centers: HashMap<String, NodePosition> = sums
            .into_iter()
            .map(|(layer, (x, y, count))| {
                (
                    layer.to_string(),
                    NodePosition {
                        x: x / count as f64,
                        y: y / count as f64,
                    },
                )
            })
            .collect();
        for command in &mut self.add_commands {
            if let BoardCommand::CreateLayer {
                ref_id: Some(ref_id),
                position,
                ..
            } = command
                && let Some(center) = centers.get(ref_id)
            {
                *position = Some(center.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests;
