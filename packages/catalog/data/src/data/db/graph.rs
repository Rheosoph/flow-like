use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{Cacheable, async_trait, json::json};
use std::sync::Arc;

pub mod analytics;
pub mod cypher;
pub mod drop_overlay;
pub mod list_overlays;
pub mod neighbors;
pub mod ontology_action;
pub mod ontology_action_input;
pub mod ontology_action_remote;
pub mod ontology_query;
pub mod ontology_remote_children;
pub mod ontology_remote_query;
pub mod paths;
pub mod sample;
pub mod schema;
pub mod search;
pub mod sql;
pub mod subgraph;
pub mod upsert_edge;
pub mod upsert_node;

#[cfg(feature = "execute")]
use flow_like_storage::databases::graph::lancegraph::LanceGraphStore;

/// Merges any per-property `param_*` input pins over a base parameters object.
///
/// Generated action bindings expand a flat scalar parameter schema into one
/// typed pin per property (see `flow_like_catalog_core::ontology_binding_nodes`).
/// When no such pins are present the node stays in single-struct mode and the
/// base object is returned unchanged.
#[cfg(feature = "execute")]
pub(crate) async fn merge_parameter_pins(
    context: &ExecutionContext,
    base: flow_like_types::Value,
) -> flow_like_types::Value {
    use flow_like::flow::pin::PinType;

    let param_pins: Vec<String> = {
        let node = context.node.node.lock().await;
        node.pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Input && pin.name.starts_with("param_"))
            .map(|pin| pin.name.clone())
            .collect()
    };
    if param_pins.is_empty() {
        return base;
    }
    let mut object = match base {
        flow_like_types::Value::Object(map) => map,
        _ => flow_like_types::json::Map::new(),
    };
    for pin_name in param_pins {
        let Some(key) = pin_name.strip_prefix("param_") else {
            continue;
        };
        if let Ok(value) = context
            .evaluate_pin::<flow_like_types::Value>(&pin_name)
            .await
        {
            object.insert(key.to_string(), value);
        }
    }
    flow_like_types::Value::Object(object)
}

/// Cached graph store instance, stored in the execution context cache.
#[cfg(feature = "execute")]
#[derive(Clone)]
pub struct CachedGraphStore {
    pub store: Arc<LanceGraphStore>,
}

#[cfg(feature = "execute")]
impl Cacheable for CachedGraphStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(feature = "execute")]
pub async fn load_graph_store(
    context: &ExecutionContext,
    cache_key: &str,
) -> flow_like_types::Result<Arc<LanceGraphStore>> {
    let cached =
        context
            .cache
            .read()
            .await
            .get(cache_key)
            .cloned()
            .ok_or(flow_like_types::anyhow!(
                "Graph store not found in cache (key: {})",
                cache_key
            ))?;
    let store =
        cached
            .as_any()
            .downcast_ref::<CachedGraphStore>()
            .ok_or(flow_like_types::anyhow!(
                "Could not downcast cached value to CachedGraphStore"
            ))?;
    Ok(store.store.clone())
}

/// # Open Graph Overlay
/// Opens an existing graph overlay and returns a graph connection reference.
#[crate::register_node]
#[derive(Default)]
pub struct OpenGraphOverlayNode {}

impl OpenGraphOverlayNode {
    pub fn new() -> Self {
        OpenGraphOverlayNode {}
    }
}

#[async_trait]
impl NodeLogic for OpenGraphOverlayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "open_graph_overlay",
            "Open Graph Overlay",
            "Opens an existing graph overlay and returns a connection for querying",
            "Data/Database/Graph",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "overlay_id",
            "Overlay ID",
            "ID of the graph overlay to open",
            VariableType::String,
        );
        node.add_input_pin(
            "user_scoped",
            "User Scoped",
            "Use user-scoped database instead of project-scoped",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Opened",
            "Graph overlay opened successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Failed to open graph overlay",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );
        node.add_output_pin(
            "graph",
            "Graph Connection",
            "Graph connection reference for query nodes",
            VariableType::Struct,
        )
        .set_schema::<NodeGraphConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::lancegraph::{self, LanceGraphStore};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let overlay_id: String = context.evaluate_pin("overlay_id").await?;
        let user_scoped: bool = context.evaluate_pin("user_scoped").await.unwrap_or(false);

        let cache_key = if user_scoped {
            format!("graph_user_{}", overlay_id)
        } else {
            format!("graph_{}", overlay_id)
        };

        let cache_set = context.cache.read().await.contains_key(&cache_key);
        if !cache_set {
            let context_cache = context
                .execution_cache
                .clone()
                .ok_or(flow_like_types::anyhow!("No execution cache found"))?;
            let app_id = context_cache.app_id.clone();

            let db = if let Some(credentials) = &context.credentials {
                if user_scoped {
                    credentials
                        .to_db_scoped(&context_cache.sub, &app_id)
                        .await?
                } else {
                    credentials.to_db(&app_id).await?
                }
            } else if user_scoped {
                let user_dir = context_cache.get_user_dir(false)?;
                let user_dir = user_dir.child("db");
                context
                    .app_state
                    .config
                    .read()
                    .await
                    .callbacks
                    .build_user_database
                    .clone()
                    .ok_or(flow_like_types::anyhow!("No user database builder found"))?(
                    user_dir
                )
            } else {
                let board_dir = context_cache.get_storage(false)?;
                let board_dir = board_dir.child("db");
                context
                    .app_state
                    .config
                    .read()
                    .await
                    .callbacks
                    .build_project_database
                    .clone()
                    .ok_or(flow_like_types::anyhow!("No database builder found"))?(
                    board_dir
                )
            };

            let connection = context.app_state.with_lance_session(db).execute().await?;

            let overlay = match lancegraph::load_overlay(&connection, &overlay_id).await {
                Ok(o) => o,
                Err(e) => {
                    context
                        .set_pin_value("error_message", json!(e.to_string()))
                        .await?;
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
            };

            let store = match LanceGraphStore::new(connection, overlay, None).await {
                Ok(s) => s,
                Err(e) => {
                    context
                        .set_pin_value("error_message", json!(e.to_string()))
                        .await?;
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
            };

            let cached = CachedGraphStore {
                store: Arc::new(store),
            };
            let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
            context
                .cache
                .write()
                .await
                .insert(cache_key.clone(), cacheable);
        }

        let conn = NodeGraphConnection { cache_key };
        context
            .set_pin_value("graph", flow_like_types::json::to_value(&conn)?)
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the 'execute' feature flag."
        ))
    }
}

/// # Create Graph Overlay
/// Creates a new graph overlay definition over existing database tables.
#[crate::register_node]
#[derive(Default)]
pub struct CreateGraphOverlayNode {}

impl CreateGraphOverlayNode {
    pub fn new() -> Self {
        CreateGraphOverlayNode {}
    }
}

#[async_trait]
impl NodeLogic for CreateGraphOverlayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "create_graph_overlay",
            "Create Graph Overlay",
            "Creates a new graph overlay definition over existing database tables",
            "Data/Database/Graph",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "overlay",
            "Overlay Definition",
            "The graph overlay definition (JSON)",
            VariableType::Struct,
        )
        .set_schema::<flow_like_catalog_core::GraphOverlay>();
        node.add_input_pin(
            "user_scoped",
            "User Scoped",
            "Store in user-scoped database",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Created",
            "Overlay created successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Failed to create overlay",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );
        node.add_output_pin(
            "overlay_id",
            "Overlay ID",
            "ID of the created overlay",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::lancegraph::{self, GraphOverlayDef};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let overlay: flow_like_catalog_core::GraphOverlay = context.evaluate_pin("overlay").await?;
        let user_scoped: bool = context.evaluate_pin("user_scoped").await.unwrap_or(false);

        let context_cache = context
            .execution_cache
            .clone()
            .ok_or(flow_like_types::anyhow!("No execution cache found"))?;
        let app_id = context_cache.app_id.clone();

        let db = if let Some(credentials) = &context.credentials {
            if user_scoped {
                credentials
                    .to_db_scoped(&context_cache.sub, &app_id)
                    .await?
            } else {
                credentials.to_db(&app_id).await?
            }
        } else if user_scoped {
            let user_dir = context_cache.get_user_dir(false)?;
            let user_dir = user_dir.child("db");
            context
                .app_state
                .config
                .read()
                .await
                .callbacks
                .build_user_database
                .clone()
                .ok_or(flow_like_types::anyhow!("No user database builder found"))?(
                user_dir
            )
        } else {
            let board_dir = context_cache.get_storage(false)?;
            let board_dir = board_dir.child("db");
            context
                .app_state
                .config
                .read()
                .await
                .callbacks
                .build_project_database
                .clone()
                .ok_or(flow_like_types::anyhow!("No database builder found"))?(board_dir)
        };

        let connection = context.app_state.with_lance_session(db).execute().await?;

        let overlay_id = if overlay.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            overlay.id.clone()
        };

        let now = chrono::Utc::now().to_rfc3339();
        let def = GraphOverlayDef {
            id: overlay_id.clone(),
            name: overlay.name,
            description: overlay.description,
            nodes: overlay
                .nodes
                .into_iter()
                .map(|n| lancegraph::NodeMappingDef {
                    id: n.id,
                    api_name: n.api_name,
                    label: n.label,
                    table: n.table,
                    id_column: n.id_column,
                    display_column: n.display_column,
                    property_columns: n
                        .property_columns
                        .into_iter()
                        .map(|p| lancegraph::PropertyColumnDef {
                            name: p.name,
                            data_type: p.data_type,
                            nullable: p.nullable,
                        })
                        .collect(),
                    style: flow_like_types::json::to_value(&n.style).unwrap_or_default(),
                })
                .collect(),
            edges: overlay
                .edges
                .into_iter()
                .map(|e| lancegraph::EdgeMappingDef {
                    id: e.id,
                    api_name: e.api_name,
                    label: e.label,
                    table: e.table,
                    src_column: e.src_column,
                    dst_column: e.dst_column,
                    src_label: e.src_label,
                    dst_label: e.dst_label,
                    src_node_column: e.src_node_column,
                    dst_node_column: e.dst_node_column,
                    containment: e.containment,
                    dst_ontology: e.dst_ontology,
                    dst_binding_id: e.dst_binding_id,
                    property_columns: e
                        .property_columns
                        .into_iter()
                        .map(|p| lancegraph::PropertyColumnDef {
                            name: p.name,
                            data_type: p.data_type,
                            nullable: p.nullable,
                        })
                        .collect(),
                    style: flow_like_types::json::to_value(&e.style).unwrap_or_default(),
                })
                .collect(),
            object_views: overlay
                .object_views
                .into_iter()
                .map(|view| lancegraph::ObjectViewDef {
                    object_type: view.object_type,
                    title_property: view.title_property,
                    prominent_properties: view.prominent_properties,
                })
                .collect(),
            // Governed capabilities (executable actions, cross-project
            // exposure) can only be granted through Data Studio, where event
            // materialization, contract hashing, and permissions are enforced.
            // A board write path must never mint them.
            actions: Vec::new(),
            exposed: false,
            bindings_enabled: overlay.bindings_enabled,
            default_limit: overlay.default_limit,
            created_at: now.clone(),
            updated_at: now,
        };

        let validation = lancegraph::validate_overlay_definition(&connection, &def).await?;
        if !validation.ok {
            let mut issues = validation.issues;
            for mapping in &validation.mappings {
                for issue in &mapping.issues {
                    issues.push(format!("{} '{}': {}", mapping.kind, mapping.label, issue));
                }
            }
            context
                .set_pin_value("error_message", json!(issues.join("; ")))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        match lancegraph::save_overlay(&connection, &def).await {
            Ok(()) => {
                context
                    .set_pin_value("overlay_id", json!(overlay_id))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the 'execute' feature flag."
        ))
    }
}
