//! Graph-derived facts a board *listing* needs, computed once server-side so listing UIs never
//! have to transfer the graph. Shared by the API's `/board/summaries` and the desktop's local
//! `get_app_board_summaries`; both must stay shape-compatible with `IBoardSummary` on the client.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Board, LayerType};

/// A node flagged as an entry point (`start == true`). `friendly_name` is what event bindings
/// and run labels display, so it travels with the id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoardEntryNode {
    pub node_id: String,
    pub node_type: String,
    #[serde(default)]
    pub friendly_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct BoardVariableCounts {
    pub total: u32,
    pub secret: u32,
    /// Secret or explicitly runtime-configured — what a run will prompt for.
    pub prompted_at_runtime: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct BoardLayerCounts {
    pub total: u32,
    pub collapsed: u32,
    pub function: u32,
    pub r#macro: u32,
    pub module: u32,
}

/// The metrics the flows overview renders per board. Mirrors `packages/ui/lib/board-metrics.ts`,
/// which computed the same from a full board client-side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct BoardSummaryMetrics {
    /// Every node including reroutes and legacy layer nodes.
    pub total_node_count: u32,
    /// Deduped, sorted WASM package ids present anywhere on the board.
    pub wasm_packages: Vec<String>,
    /// Deduped, sorted union of the permissions those packages declare.
    pub wasm_permissions: Vec<String>,
    pub variable_counts: BoardVariableCounts,
    pub layer_counts: BoardLayerCounts,
    /// Table names opened by a literal on an unconnected "Open Database" pin, sorted. Tables
    /// opened through a variable or wire are not known without running the board.
    #[serde(default)]
    pub table_refs: Vec<String>,
    /// OAuth provider ids the nodes require, each with the sorted union of their scopes.
    #[serde(default)]
    pub oauth_providers: BTreeMap<String, Vec<String>>,
    /// Some node only runs on the member's own device.
    #[serde(default)]
    pub requires_local: bool,
}

const OPEN_DATABASE_NODE: &str = "open_local_db";
const OPEN_DATABASE_TABLE_PIN: &str = "name";

fn literal_table_ref(node: &super::Node) -> Option<String> {
    if node.name != OPEN_DATABASE_NODE {
        return None;
    }
    let pin = node.pins.values().find(|pin| {
        pin.name == OPEN_DATABASE_TABLE_PIN && pin.pin_type == crate::flow::pin::PinType::Input
    })?;
    if !pin.depends_on.is_empty() {
        return None;
    }
    let name: String = flow_like_types::json::from_slice(pin.default_value.as_deref()?).ok()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

impl Board {
    /// Distinct node type names, sorted.
    pub fn summary_node_types(&self) -> Vec<String> {
        self.nodes
            .values()
            .map(|node| node.name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Entry nodes sorted by id so the listing is stable across loads.
    pub fn summary_entry_nodes(&self) -> Vec<BoardEntryNode> {
        let mut entries: Vec<BoardEntryNode> = self
            .nodes
            .values()
            .filter(|node| node.start == Some(true))
            .map(|node| BoardEntryNode {
                node_id: node.id.clone(),
                node_type: node.name.clone(),
                friendly_name: node.friendly_name.clone(),
            })
            .collect();
        entries.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        entries
    }

    pub fn summary_metrics(&self) -> BoardSummaryMetrics {
        let mut wasm_packages = BTreeSet::new();
        let mut wasm_permissions = BTreeSet::new();
        let mut total_node_count = 0u32;
        let mut table_refs = BTreeSet::new();
        let mut oauth: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut requires_local = false;
        let mut collect = |node: &super::Node| {
            total_node_count += 1;
            requires_local |= node.only_offline;
            if let Some(table) = literal_table_ref(node) {
                table_refs.insert(table);
            }
            for provider in node.oauth_providers.iter().flatten() {
                oauth.entry(provider.clone()).or_default();
            }
            for (provider, scopes) in node.required_oauth_scopes.iter().flatten() {
                oauth
                    .entry(provider.clone())
                    .or_default()
                    .extend(scopes.iter().cloned());
            }
            if let Some(wasm) = &node.wasm {
                wasm_packages.insert(wasm.package_id.clone());
                for permission in &wasm.permissions {
                    wasm_permissions.insert(
                        flow_like_types::json::to_value(permission)
                            .ok()
                            .and_then(|value| value.as_str().map(str::to_string))
                            .unwrap_or_else(|| format!("{permission:?}")),
                    );
                }
            }
        };
        for node in self.nodes.values() {
            collect(node);
        }
        for layer in self.layers.values() {
            for node in layer.nodes.values() {
                collect(node);
            }
        }

        let mut variable_counts = BoardVariableCounts::default();
        for variable in self.variables.values() {
            variable_counts.total += 1;
            if variable.secret {
                variable_counts.secret += 1;
            }
            if variable.secret || variable.runtime_configured {
                variable_counts.prompted_at_runtime += 1;
            }
        }

        let mut layer_counts = BoardLayerCounts::default();
        for layer in self.layers.values() {
            layer_counts.total += 1;
            match layer.r#type {
                LayerType::Collapsed => layer_counts.collapsed += 1,
                LayerType::Function => layer_counts.function += 1,
                LayerType::Macro => layer_counts.r#macro += 1,
                LayerType::Module => layer_counts.module += 1,
            }
        }

        BoardSummaryMetrics {
            total_node_count,
            wasm_packages: wasm_packages.into_iter().collect(),
            wasm_permissions: wasm_permissions.into_iter().collect(),
            variable_counts,
            layer_counts,
            table_refs: table_refs.into_iter().collect(),
            oauth_providers: oauth
                .into_iter()
                .map(|(provider, scopes)| (provider, scopes.into_iter().collect()))
                .collect(),
            requires_local,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::node::{Node, NodePermission, NodeWasm};
    use crate::flow::pin::ValueType;
    use crate::flow::variable::{Variable, VariableType};
    use flow_like_storage::Path;

    #[test]
    fn metrics_record_literal_tables_oauth_and_local_nodes() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());

        let mut literal = Node::new(OPEN_DATABASE_NODE, "Open Database", "", "Data");
        literal
            .add_input_pin(
                OPEN_DATABASE_TABLE_PIN,
                "Table Name",
                "",
                VariableType::String,
            )
            .set_default_value(Some(flow_like_types::json::json!("orders")));
        literal.add_oauth_provider("google");
        literal.add_required_oauth_scopes("google", vec!["mail.read", "drive"]);
        board.nodes.insert(literal.id.clone(), literal);

        let mut wired = Node::new(OPEN_DATABASE_NODE, "Open Database", "", "Data");
        let pin = wired.add_input_pin(
            OPEN_DATABASE_TABLE_PIN,
            "Table Name",
            "",
            VariableType::String,
        );
        pin.set_default_value(Some(flow_like_types::json::json!("secret_table")));
        pin.depends_on.insert("upstream".into());
        wired.add_required_oauth_scopes("google", vec!["mail.read"]);
        wired.only_offline = true;
        board.nodes.insert(wired.id.clone(), wired);

        let metrics = board.summary_metrics();
        assert_eq!(metrics.table_refs, vec!["orders".to_string()]);
        assert_eq!(
            metrics.oauth_providers.get("google"),
            Some(&vec!["drive".to_string(), "mail.read".to_string()])
        );
        assert!(metrics.requires_local);
    }

    #[test]
    fn metrics_cover_layers_wasm_and_variables() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        let mut start = Node::new("event_start", "Start", "", "Events");
        start.set_start(true);
        board.nodes.insert(start.id.clone(), start.clone());
        let mut wasm = Node::new("pkg_node", "Pkg", "", "Pkg");
        wasm.wasm = Some(NodeWasm {
            package_id: "pkg-1".into(),
            permissions: vec![NodePermission::NetworkHttp],
        });
        board.nodes.insert(wasm.id.clone(), wasm);
        let mut secret = Variable::new("token", VariableType::String, ValueType::Normal);
        secret.secret = true;
        board.variables.insert(secret.id.clone(), secret);
        let mut runtime = Variable::new("cfg", VariableType::String, ValueType::Normal);
        runtime.runtime_configured = true;
        board.variables.insert(runtime.id.clone(), runtime);
        board.layers.insert(
            "f".into(),
            super::super::Layer::new("f".into(), "Fn".into(), LayerType::Function),
        );
        board.layers.insert(
            "m".into(),
            super::super::Layer::new("m".into(), "Mod".into(), LayerType::Module),
        );

        let metrics = board.summary_metrics();
        assert_eq!(metrics.total_node_count, 2);
        assert_eq!(metrics.wasm_packages, vec!["pkg-1"]);
        assert_eq!(metrics.wasm_permissions, vec!["network:http"]);
        assert_eq!(metrics.variable_counts.total, 2);
        assert_eq!(metrics.variable_counts.secret, 1);
        assert_eq!(metrics.variable_counts.prompted_at_runtime, 2);
        assert_eq!(metrics.layer_counts.function, 1);
        assert_eq!(metrics.layer_counts.module, 1);
        assert_eq!(metrics.layer_counts.total, 2);

        let entries = board.summary_entry_nodes();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].node_id, start.id);
        assert_eq!(entries[0].friendly_name, "Start");
        assert_eq!(board.summary_node_types(), vec!["event_start", "pkg_node"]);
    }
}
