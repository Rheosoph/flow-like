//! Database access is granted by the executing node's wired, typed input pins.

use super::{HostState, ModelCacheHandle};
use crate::abi::WasmNodeDefinition;
use crate::limits::WasmCapabilities;
use flow_like::flow::{
    execution::{context::ExecutionContext, internal_pin::InternalPin},
    pin::{PinType, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::{CachedDB, NodeDBConnection};
use flow_like_catalog_data_support::data::datafusion::session::{
    CachedDataFusionSession, DataFusionSession,
};
use flow_like_storage::{
    databases::vector::{
        VectorStore, buffered::BufferedWriteOrigin, lancedb::record_batches_to_vec,
    },
    datafusion::{
        catalog::{CatalogProvider, MemoryCatalogProvider, MemorySchemaProvider, SchemaProvider},
        execution::{
            context::SQLOptions,
            disk_manager::{DiskManagerBuilder, DiskManagerMode},
            object_store::ObjectStoreRegistry,
            runtime_env::RuntimeEnvBuilder,
            session_state::SessionStateBuilder,
        },
        prelude::{SessionConfig, SessionContext},
        sql::{
            parser::{DFParser, Statement},
            sqlparser::ast::Statement as SqlStatement,
        },
    },
};
use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

const MAX_ROWS: usize = 10_000;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SESSIONS: usize = 32;
const MAX_TABLES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HandleKind {
    Database,
    Session,
}

pub(crate) fn handle_kind(schema: Option<&str>) -> Option<HandleKind> {
    let schema: Value = serde_json::from_str(schema?).ok()?;
    for (name, kind) in [
        ("NodeDBConnection", HandleKind::Database),
        ("DataFusionSession", HandleKind::Session),
    ] {
        let expected: Value = serde_json::from_str(super::schema::get_type_schema(name)?).ok()?;
        if schema == expected {
            return Some(kind);
        }
    }
    None
}

/// This map contains resolved objects, never an API to look up guest-supplied cache keys.
/// A fresh map is installed for every invocation, including reused package instances.
#[derive(Default)]
pub struct DatabaseContext {
    databases: RwLock<HashMap<String, CachedDB>>,
    database_generations: RwLock<HashMap<String, u64>>,
    sessions: RwLock<HashMap<String, CachedDataFusionSession>>,
    cache: Option<ModelCacheHandle>,
    run_id: String,
    origin: Option<BufferedWriteOrigin>,
}

impl std::fmt::Debug for DatabaseContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseContext").finish_non_exhaustive()
    }
}

impl DatabaseContext {
    pub(crate) async fn from_inputs(
        context: &mut ExecutionContext,
        definition: &WasmNodeDefinition,
        inputs: &serde_json::Map<String, Value>,
        capabilities: WasmCapabilities,
    ) -> flow_like_types::Result<Arc<Self>> {
        let grants = Arc::new(Self {
            cache: Some(context.cache.clone()),
            run_id: context.run_id().to_string(),
            origin: Some(CachedDB::write_origin(context)),
            ..Self::default()
        });
        if !capabilities
            .intersects(WasmCapabilities::DATABASE_READ | WasmCapabilities::DATABASE_WRITE)
        {
            return Ok(grants);
        }
        for declared in &definition.pins {
            if !declared.pin_type.eq_ignore_ascii_case("input") {
                continue;
            }
            let Some(kind) = handle_kind(declared.schema.as_deref()) else {
                continue;
            };
            let Some(value) = inputs.get(&declared.name).filter(|value| !value.is_null()) else {
                continue;
            };
            let pin = context.get_pin_by_name(&declared.name).await?;
            // Literal defaults, open JSON, and nested cache-key strings are not grants.
            let board = context.get_board().await?;
            let Some(source) = wired_handle_source(&pin, kind, &board.refs) else {
                return Err(flow_like_types::anyhow!(
                    "Database handles require a wired typed input pin"
                ));
            };
            ensure_trusted_handle_source(&source, value).await?;
            let handle: Handle = serde_json::from_value(value.clone())?;
            match kind {
                HandleKind::Database => {
                    let (database, generation) = NodeDBConnection {
                        cache_key: handle.cache_key.clone(),
                    }
                    .load_with_generation(context)
                    .await?;
                    grants
                        .database_generations
                        .write()
                        .insert(handle.cache_key.clone(), generation);
                    grants.databases.write().insert(handle.cache_key, database);
                }
                HandleKind::Session => {
                    let session = DataFusionSession {
                        cache_key: handle.cache_key.clone(),
                    }
                    .load(context)
                    .await?;
                    grants.sessions.write().insert(handle.cache_key, session);
                }
            }
        }
        Ok(grants)
    }

    pub(crate) async fn validate_outputs(
        &self,
        context: &ExecutionContext,
        definition: &WasmNodeDefinition,
        outputs: &serde_json::Map<String, Value>,
    ) -> flow_like_types::Result<()> {
        let board = context.get_board().await?;
        for (name, value) in outputs {
            let actual = context.get_pin_by_name(name).await?;
            let schema = actual
                .schema
                .as_deref()
                .map(|schema| flow_like::flow::pin::resolve_schema(schema, &board.refs))
                .transpose()?;
            if let Some(kind) = handle_kind(schema) {
                self.validate_output(kind, value)?;
            }
            // Check both the host graph and the loaded definition when a saved board
            // still carries a schema from an earlier package version.
            if let Some(pin) = definition
                .pins
                .iter()
                .find(|pin| pin.name == *name && pin.pin_type.eq_ignore_ascii_case("output"))
            {
                if let Some(kind) = handle_kind(pin.schema.as_deref()) {
                    self.validate_output(kind, value)?;
                }
            }
        }
        Ok(())
    }

    fn validate_output(&self, kind: HandleKind, value: &Value) -> flow_like_types::Result<()> {
        if value.is_null() {
            return Ok(());
        }
        let handle: Handle = serde_json::from_value(value.clone())?;
        let granted = match kind {
            HandleKind::Database => self.databases.read().contains_key(&handle.cache_key),
            HandleKind::Session => self.sessions.read().contains_key(&handle.cache_key),
        };
        if !granted {
            return Err(flow_like_types::anyhow!(
                "WASM output contains an ungranted database handle"
            ));
        }
        Ok(())
    }

    async fn dispatch(
        &self,
        op: u32,
        connection: &str,
        payload: &str,
    ) -> flow_like_types::Result<Value> {
        if payload.len() > MAX_BYTES || connection.len() > 4096 {
            return Err(flow_like_types::anyhow!("Database request exceeds limit"));
        }
        if op == 20 {
            if self.sessions.read().len() >= MAX_SESSIONS {
                return Err(flow_like_types::anyhow!("Session limit reached"));
            }
            let cache = self
                .cache
                .as_ref()
                .ok_or_else(|| flow_like_types::anyhow!("No execution owner"))?;
            let mut random = [0u8; 32];
            getrandom::fill(&mut random)
                .map_err(|_| flow_like_types::anyhow!("Cannot create session handle"))?;
            let key = format!(
                "wasm_df_{}_{}",
                self.run_id,
                blake3::Hash::from_bytes(random).to_hex()
            );
            let session = CachedDataFusionSession::new(isolated_session());
            cache
                .write()
                .await
                .insert(key.clone(), Arc::new(session.clone()));
            self.sessions.write().insert(key.clone(), session);
            return Ok(json!({"cache_key": key}));
        }
        let handle: Handle = serde_json::from_str(connection)?;
        if matches!(op, 21 | 22) {
            let session = self
                .sessions
                .read()
                .get(&handle.cache_key)
                .cloned()
                .ok_or_else(|| flow_like_types::anyhow!("Session not granted"))?;
            if op == 21 {
                let request: Mount = serde_json::from_str(payload)?;
                validate_table_name(&request.table_name)?;
                if table_count(&session.ctx) >= MAX_TABLES {
                    return Err(flow_like_types::anyhow!("Session table limit reached"));
                }
                let database = self
                    .databases
                    .read()
                    .get(&request.database.cache_key)
                    .cloned()
                    .ok_or_else(|| flow_like_types::anyhow!("Database not granted"))?;
                database.ensure_flushed().await?;
                let guard = database.db.read().await;
                let provider = guard.inner().to_datafusion().await?;
                // Duplicate names fail instead of replacing a host-configured table.
                session
                    .ctx
                    .register_table(request.table_name.as_str(), provider)?;
                let generation = self
                    .database_generations
                    .read()
                    .get(&request.database.cache_key)
                    .copied()
                    .unwrap_or(0);
                session
                    .track_lance_table(
                        request.table_name,
                        NodeDBConnection {
                            cache_key: request.database.cache_key,
                        },
                        generation,
                    )
                    .await;
                return Ok(json!(true));
            }
            let request: SqlQuery = serde_json::from_str(payload)?;
            validate_sql(&request.sql)?;
            let isolated = copy_tables(&session.ctx).await?;
            let options = SQLOptions::new()
                .with_allow_ddl(false)
                .with_allow_dml(false)
                .with_allow_statements(false);
            let frame = isolated
                .sql_with_options(&request.sql, options)
                .await?
                .limit(0, Some(row_limit(request.max_rows)?))?;
            return Ok(serde_json::to_value(record_batches_to_vec(Some(
                frame.collect().await?,
            ))?)?);
        }
        let database = self
            .databases
            .read()
            .get(&handle.cache_key)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("Database not granted"))?;
        if matches!(op, 4 | 5) {
            let (items, id_field) = if op == 4 {
                (serde_json::from_str::<Vec<Value>>(payload)?, None)
            } else {
                let request: Upsert = serde_json::from_str(payload)?;
                validate_table_name(&request.id_field)?;
                (request.items, Some(request.id_field))
            };
            if items.len() > MAX_ROWS || items.iter().any(|row| !row.is_object()) {
                return Err(flow_like_types::anyhow!("Invalid database rows"));
            }
            let origin = self
                .origin
                .clone()
                .ok_or_else(|| flow_like_types::anyhow!("No execution owner"))?;
            let mut guard = database.db.write().await;
            if let Some(field) = id_field {
                guard.upsert_with_origin(items, field, origin).await?;
            } else {
                guard.insert_with_origin(items, origin).await?;
            }
            // Report durable failure while this invocation still owns the operation.
            guard.flush().await?;
            return Ok(json!(true));
        }
        database.ensure_flushed().await?;
        let request: Query = serde_json::from_str(payload)?;
        let limit = row_limit(request.limit)?;
        let offset = usize::try_from(request.offset.unwrap_or(0))?;
        let guard = database.db.read().await;
        Ok(match op {
            1 => serde_json::to_value(
                guard
                    .vector_search(
                        request.vector,
                        request.filter.as_deref(),
                        request.select,
                        limit,
                        offset,
                    )
                    .await?,
            )?,
            2 => serde_json::to_value(
                guard
                    .fts_search(
                        &request.text,
                        request.filter.as_deref(),
                        request.select,
                        request.fields,
                        limit,
                        offset,
                    )
                    .await?,
            )?,
            3 => serde_json::to_value(
                guard
                    .hybrid_search(
                        request.vector,
                        &request.text,
                        request.filter.as_deref(),
                        request.select,
                        request.fields,
                        limit,
                        offset,
                        request.rerank,
                    )
                    .await?,
            )?,
            6 => {
                let filter = request
                    .filter
                    .filter(|filter| !filter.trim().is_empty())
                    .ok_or_else(|| flow_like_types::anyhow!("Delete requires a filter"))?;
                guard.delete(&filter).await?;
                json!(true)
            }
            7 => serde_json::to_value(guard.list(request.select, limit, offset).await?)?,
            8 => json!(guard.count(request.filter).await?),
            _ => return Err(flow_like_types::anyhow!("Unknown database operation")),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Handle {
    cache_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mount {
    database: Handle,
    table_name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SqlQuery {
    sql: String,
    max_rows: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upsert {
    items: Vec<Value>,
    id_field: String,
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Query {
    vector: Vec<f64>,
    text: String,
    filter: Option<String>,
    select: Option<Vec<String>>,
    fields: Option<Vec<String>>,
    limit: Option<u64>,
    offset: Option<u64>,
    rerank: bool,
}

/// The single typed output pin a handle input is wired to, if the wiring is
/// the only way the value could have arrived.
fn wired_handle_source(
    pin: &InternalPin,
    kind: HandleKind,
    refs: &HashMap<String, String>,
) -> Option<Arc<InternalPin>> {
    let resolved_kind = |schema: Option<&str>| {
        schema
            .and_then(|schema| flow_like::flow::pin::resolve_schema(schema, refs).ok())
            .and_then(|schema| handle_kind(Some(schema)))
    };
    if pin.pin_type != PinType::Input
        || pin.data_type != VariableType::Struct
        || pin.value_type != ValueType::Normal
        || resolved_kind(pin.schema.as_deref()) != Some(kind)
        || pin.depends_on().len() != 1
    {
        return None;
    }
    pin.depends_on()[0].upgrade().filter(|source| {
        source.pin_type == PinType::Output
            && source.data_type == VariableType::Struct
            && source.value_type == ValueType::Normal
            && resolved_kind(source.schema.as_deref()) == Some(kind)
    })
}

#[cfg(test)]
fn is_wired_handle(pin: &InternalPin, kind: HandleKind, refs: &HashMap<String, String>) -> bool {
    wired_handle_source(pin, kind, refs).is_some()
}

/// A wire alone is not authority: the handle must be exactly what the source
/// pin holds at this point of the run. A pin default or a stale copy never
/// counts, so a package cannot declare a typed output with a guessed cache key
/// and let it flow into its own downstream node. Built-in nodes mint handles;
/// a WASM producer may forward or export one only after `validate_outputs`
/// confirmed it was granted or created (op 20) by that node.
async fn ensure_trusted_handle_source(
    source: &InternalPin,
    value: &Value,
) -> flow_like_types::Result<()> {
    if source.node().and_then(std::sync::Weak::upgrade).is_none() {
        return Err(flow_like_types::anyhow!(
            "Database handle source node is unavailable"
        ));
    }
    if source.value.read().as_ref() != Some(value) {
        return Err(flow_like_types::anyhow!(
            "Database handle does not match the value held by its source pin"
        ));
    }
    Ok(())
}

fn row_limit(limit: Option<u64>) -> flow_like_types::Result<usize> {
    let limit = usize::try_from(limit.unwrap_or(1000))?;
    if limit == 0 || limit > MAX_ROWS {
        return Err(flow_like_types::anyhow!(
            "Row limit must be between 1 and 10000"
        ));
    }
    Ok(limit)
}

fn validate_table_name(name: &str) -> flow_like_types::Result<()> {
    if name.is_empty()
        || name.len() > 128
        || !name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
    {
        return Err(flow_like_types::anyhow!(
            "Expected an unqualified SQL identifier"
        ));
    }
    Ok(())
}

fn validate_sql(sql: &str) -> flow_like_types::Result<()> {
    if sql.len() > 64 * 1024 {
        return Err(flow_like_types::anyhow!("SQL exceeds limit"));
    }
    let statements = DFParser::parse_sql(sql)?;
    if statements.len() != 1
        || !matches!(&statements[0], Statement::Statement(statement) if matches!(statement.as_ref(), SqlStatement::Query(_)))
    {
        return Err(flow_like_types::anyhow!("Only one SELECT query is allowed"));
    }
    Ok(())
}

fn isolated_session() -> SessionContext {
    isolated_session_with_stores(None)
}

fn isolated_session_with_stores(stores: Option<Arc<dyn ObjectStoreRegistry>>) -> SessionContext {
    let mut runtime = RuntimeEnvBuilder::new()
        .with_memory_limit(128 * 1024 * 1024, 1.0)
        .with_disk_manager_builder(
            DiskManagerBuilder::default().with_mode(DiskManagerMode::Disabled),
        );
    if let Some(stores) = stores {
        runtime = runtime.with_object_store_registry(stores);
    }
    let runtime = runtime
        .build_arc()
        .expect("bounded in-memory runtime has no filesystem setup");
    let state = SessionStateBuilder::new()
        .with_default_features()
        .with_runtime_env(runtime)
        .with_config(
            SessionConfig::new()
                .with_information_schema(false)
                .with_batch_size(1024),
        )
        .with_table_functions(HashMap::new())
        .with_table_factories(HashMap::new())
        .build();
    let session = SessionContext::new_with_state(state);
    flow_like_storage::geometry::register_geo_functions(&session);
    session
}

fn table_count(session: &SessionContext) -> usize {
    session
        .catalog_names()
        .iter()
        .filter_map(|name| session.catalog(name))
        .map(|catalog| {
            catalog
                .schema_names()
                .iter()
                .filter_map(|name| catalog.schema(name))
                .map(|schema| schema.table_names().len())
                .sum::<usize>()
        })
        .sum()
}

async fn copy_tables(source: &SessionContext) -> flow_like_types::Result<SessionContext> {
    // Registered providers may need their host-owned object stores. SQL cannot add
    // tables or invoke table functions against these stores.
    let destination =
        isolated_session_with_stores(Some(source.runtime_env().object_store_registry.clone()));
    let mut count = 0;
    for catalog_name in source.catalog_names() {
        let Some(catalog) = source.catalog(&catalog_name) else {
            continue;
        };
        let copied_catalog = Arc::new(MemoryCatalogProvider::new());
        for schema_name in catalog.schema_names() {
            if schema_name == "information_schema" {
                continue;
            }
            let Some(schema) = catalog.schema(&schema_name) else {
                continue;
            };
            let copied_schema = Arc::new(MemorySchemaProvider::new());
            for table_name in schema.table_names() {
                count += 1;
                if count > MAX_TABLES {
                    return Err(flow_like_types::anyhow!("Session table limit exceeded"));
                }
                if let Some(table) = schema.table(&table_name).await? {
                    copied_schema.register_table(table_name, table)?;
                }
            }
            copied_catalog.register_schema(&schema_name, copied_schema)?;
        }
        destination.register_catalog(catalog_name, copied_catalog);
    }
    Ok(destination)
}

pub async fn query(host: &HostState, op: u32, connection: &str, payload: &str) -> Option<String> {
    let required = match op {
        1..=3 | 7 | 8 | 20..=22 => WasmCapabilities::DATABASE_READ,
        4..=6 => WasmCapabilities::DATABASE_WRITE,
        _ => return None,
    };
    if !host.has_capability(required) {
        return None;
    }
    let context = host.database_context.as_ref()?;
    let result = tokio::time::timeout(host.node_timeout, context.dispatch(op, connection, payload))
        .await
        .ok()?
        .ok()?;
    let json = serde_json::to_string(&result).ok()?;
    (json.len() <= MAX_BYTES).then_some(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle_pin(input: bool, database: bool) -> Arc<InternalPin> {
        let mut node = flow_like::flow::node::Node::new("test", "Test", "", "Test");
        let pin = if input {
            node.add_input_pin("handle", "Handle", "", VariableType::Struct)
        } else {
            node.add_output_pin("handle", "Handle", "", VariableType::Struct)
        };
        if database {
            pin.set_schema::<NodeDBConnection>();
        } else {
            pin.set_schema::<DataFusionSession>();
        }
        Arc::new(InternalPin::new(pin, false))
    }

    #[test]
    fn only_one_wired_matching_typed_pin_grants_authority() {
        let database = handle_pin(false, true);
        let session = handle_pin(false, false);
        let literal = handle_pin(true, true);
        assert!(!is_wired_handle(
            &literal,
            HandleKind::Database,
            &HashMap::new()
        ));
        let valid = handle_pin(true, true);
        valid.init_depends_on(vec![Arc::downgrade(&database)]);
        assert!(is_wired_handle(
            &valid,
            HandleKind::Database,
            &HashMap::new()
        ));
        let wrong = handle_pin(true, true);
        wrong.init_depends_on(vec![Arc::downgrade(&session)]);
        assert!(!is_wired_handle(
            &wrong,
            HandleKind::Database,
            &HashMap::new()
        ));
        let multiple = handle_pin(true, true);
        multiple.init_depends_on(vec![Arc::downgrade(&database), Arc::downgrade(&session)]);
        assert!(!is_wired_handle(
            &multiple,
            HandleKind::Database,
            &HashMap::new()
        ));
        let stale = handle_pin(true, true);
        stale.init_depends_on(vec![std::sync::Weak::new()]);
        assert!(!is_wired_handle(
            &stale,
            HandleKind::Database,
            &HashMap::new()
        ));
        assert_eq!(
            handle_kind(Some(r#"{"title":"NodeDBConnection","type":"object"}"#)),
            None
        );
    }

    #[test]
    fn compact_schema_references_are_resolved_before_granting() {
        let mut database = handle_pin(false, true);
        let mut input = handle_pin(true, true);
        let schema = database.schema.as_deref().unwrap().to_string();
        Arc::get_mut(&mut database).unwrap().schema = Some(Arc::from("schema-ref"));
        Arc::get_mut(&mut input).unwrap().schema = Some(Arc::from("schema-ref"));
        input.init_depends_on(vec![Arc::downgrade(&database)]);
        let refs = HashMap::from([("schema-ref".to_string(), schema)]);
        assert!(is_wired_handle(&input, HandleKind::Database, &refs));
        assert!(!is_wired_handle(
            &input,
            HandleKind::Database,
            &HashMap::new()
        ));
        let cyclic = HashMap::from([("schema-ref".to_string(), "schema-ref".to_string())]);
        assert!(!is_wired_handle(&input, HandleKind::Database, &cyclic));
    }

    #[tokio::test]
    async fn handle_sources_must_hold_the_value_at_run_time() {
        use ahash::AHashMap;
        use flow_like::flow::execution::internal_node::InternalNode;
        use flow_like::flow::node::{Node, NodeLogic, NodeWasm};

        struct Noop;
        #[flow_like_types::async_trait]
        impl NodeLogic for Noop {
            fn get_node(&self) -> Node {
                Node::new("noop", "Noop", "", "Test")
            }
            async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
                Ok(())
            }
        }

        let attach = |wasm: bool| {
            let mut node = Node::new("producer", "Producer", "", "Test");
            if wasm {
                node.wasm = Some(NodeWasm {
                    package_id: "com.example.pkg".into(),
                    permissions: vec![],
                });
            }
            let pin = handle_pin(false, true);
            let internal = Arc::new(InternalNode::new(
                node,
                AHashMap::from([(pin.id.to_string(), pin.clone())]),
                Arc::new(Noop),
                AHashMap::new(),
            ));
            pin.init_node(Arc::downgrade(&internal));
            (pin, internal)
        };
        let value = json!({"cache_key":"granted"});

        let (native, _native_node) = attach(false);
        *native.value.write() = Some(value.clone());
        assert!(ensure_trusted_handle_source(&native, &value).await.is_ok());
        assert!(
            ensure_trusted_handle_source(&native, &json!({"cache_key":"other"}))
                .await
                .is_err()
        );

        // A WASM producer that created a session (op 20) or forwards a granted
        // handle is a valid source; `validate_outputs` already vets what it emits.
        let (forwarded, _wasm_node) = attach(true);
        *forwarded.value.write() = Some(value.clone());
        assert!(ensure_trusted_handle_source(&forwarded, &value).await.is_ok());

        // An output the guest never wrote leaves only a pin default downstream.
        let (unset, _unset_node) = attach(true);
        assert!(ensure_trusted_handle_source(&unset, &value).await.is_err());

        let orphan = handle_pin(false, true);
        *orphan.value.write() = Some(value.clone());
        assert!(ensure_trusted_handle_source(&orphan, &value).await.is_err());
    }

    #[test]
    fn a_guest_cannot_export_a_forged_typed_handle() {
        let value = json!({"cache_key":"private"});
        assert!(
            DatabaseContext::default()
                .validate_output(HandleKind::Database, &value)
                .is_err()
        );
        assert!(
            DatabaseContext::default()
                .validate_output(HandleKind::Session, &value)
                .is_err()
        );
    }

    #[tokio::test]
    async fn granted_lance_rows_can_be_written_mounted_and_queried() {
        use flow_like_storage::databases::vector::{
            buffered::BufferedVectorStore, lancedb::LanceDBVectorStore,
        };
        let directory = tempfile::tempdir().unwrap();
        let mut store = LanceDBVectorStore::new(directory.path().into(), "entities".into())
            .await
            .unwrap();
        use flow_like_storage::arrow_schema::{DataType, Field, Schema};
        store
            .create_empty_table(
                Schema::new(vec![
                    Field::new("id", DataType::Utf8, false),
                    Field::new("longitude", DataType::Float64, false),
                    flow_like_storage::geometry::geometry_field("geometry", true),
                ]),
                false,
            )
            .await
            .unwrap();
        let database = CachedDB {
            db: Arc::new(flow_like_types::sync::RwLock::new(
                BufferedVectorStore::new(store, 100),
            )),
        };
        let context = Arc::new(DatabaseContext {
            cache: Some(Arc::new(flow_like_types::sync::RwLock::new(
                ahash::AHashMap::new(),
            ))),
            run_id: "test-run".into(),
            origin: Some(BufferedWriteOrigin::new(Arc::from("test-node"), None)),
            ..DatabaseContext::default()
        });
        context
            .databases
            .write()
            .insert("granted".into(), database.clone());
        let mut host =
            HostState::new(WasmCapabilities::DATABASE_READ | WasmCapabilities::DATABASE_WRITE);
        host.database_context = Some(context);
        let handle = r#"{"cache_key":"granted"}"#;
        assert!(
            query(&host, 4, handle, r#"[{"id":"person-1","longitude":11.5,"geometry":{"type":"Point","coordinates":[11.5,48.1]}}]"#)
                .await
                .is_some()
        );
        assert_eq!(query(&host, 8, handle, "{}").await.as_deref(), Some("1"));
        let session = query(&host, 20, "{}", "{}").await.unwrap();
        assert!(
            query(
                &host,
                21,
                &session,
                r#"{"database":{"cache_key":"ungranted"},"table_name":"other"}"#
            )
            .await
            .is_none()
        );
        assert!(
            query(
                &host,
                21,
                &session,
                r#"{"database":{"cache_key":"granted"},"table_name":"entities"}"#
            )
            .await
            .is_some()
        );
        let rows = query(
            &host,
            22,
            &session,
            r#"{"sql":"SELECT id, geometry FROM entities","max_rows":10}"#,
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&rows).unwrap(),
            json!([{"id":"person-1","geometry":{"type":"Point","coordinates":[11.5,48.1]}}])
        );
        let session_value: Value = serde_json::from_str(&session).unwrap();
        let owner = host.database_context.as_ref().unwrap();
        owner
            .validate_output(HandleKind::Session, &session_value)
            .unwrap();
        let key = session_value["cache_key"].as_str().unwrap();
        let cached = owner
            .cache
            .as_ref()
            .unwrap()
            .read()
            .await
            .get(key)
            .cloned()
            .unwrap();
        let native_session = cached
            .as_any()
            .downcast_ref::<CachedDataFusionSession>()
            .unwrap()
            .clone();
        let downstream = Arc::new(DatabaseContext::default());
        downstream
            .sessions
            .write()
            .insert(key.to_string(), native_session);
        let mut consumer = HostState::new(WasmCapabilities::DATABASE_READ);
        consumer.database_context = Some(downstream);
        // A forwarded session grants its mounted tables without exposing a database handle.
        assert_eq!(
            query(
                &consumer,
                22,
                &session,
                r#"{"sql":"SELECT id, geometry FROM entities","max_rows":10}"#
            )
            .await
            .unwrap(),
            rows
        );
        assert!(query(&consumer, 8, handle, "{}").await.is_none());
        host.capabilities = WasmCapabilities::DATABASE_READ;
        assert!(
            query(&host, 6, handle, r#"{"filter":"id = 'person-1'"}"#)
                .await
                .is_none()
        );
        assert_eq!(database.db.read().await.count(None).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn reused_package_globals_cannot_keep_previous_pin_grants() {
        use crate::abi::WasmExecutionInput;
        use crate::package_runtime::PackageRuntime;
        use crate::{LoadedWasm, WasmAbi, WasmConfig, WasmEngine, WasmSecurityConfig};
        let engine = WasmEngine::new(WasmConfig::development()).unwrap();
        let connection = r#"{"cache_key":"granted"}"#;
        let sql = r#"{"sql":"SELECT 1","max_rows":1}"#;
        let result = r#"{"outputs":{"granted":0,"calls":0}}"#;
        let granted_digit = 512 + result.find('0').unwrap();
        let calls_digit = 512 + result.rfind('0').unwrap();
        let wat = format!(
            r#"(module
            (import "flowlike_db" "query" (func $query (param i32 i32 i32 i32 i32) (result i64)))
            (memory (export "memory") 1)
            (global $calls (mut i32) (i32.const 0))
            (global $retained_handle (mut i32) (i32.const 1024))
            (data (i32.const 512) {result:?})
            (data (i32.const 1024) {connection:?})
            (data (i32.const 2048) {sql:?})
            (func (export "get_node") (result i64) i64.const 0)
            (func (export "run") (param i32 i32) (result i64)
                i32.const {granted_digit}
                i32.const 22 global.get $retained_handle i32.const {connection_len} i32.const 2048 i32.const {sql_len}
                call $query i64.eqz i32.eqz i32.const 48 i32.add i32.store8
                global.get $calls i32.const 1 i32.add global.set $calls
                i32.const {calls_digit} global.get $calls i32.const 48 i32.add i32.store8
                i64.const {packed}))"#,
            connection_len = connection.len(),
            sql_len = sql.len(),
            packed = WasmAbi::pack_ptr_len(512, result.len() as u32)
        );
        let loaded = LoadedWasm::Module(
            engine
                .load_module(&wat::parse_str(wat).unwrap())
                .await
                .unwrap(),
        );
        let security = WasmSecurityConfig {
            capabilities: WasmCapabilities::DATABASE_READ,
            ..WasmSecurityConfig::default()
        };
        let package = PackageRuntime::default();
        let grants = Arc::new(DatabaseContext::default());
        grants.sessions.write().insert(
            "granted".into(),
            CachedDataFusionSession::new(isolated_session()),
        );
        let mut first = HostState::with_security(&security);
        first.database_context = Some(grants);
        let mut input: WasmExecutionInput = serde_json::from_value(json!({"inputs":{}, "node_id":"first", "node_name":"query", "run_id":"run", "app_id":"app", "board_id":"board", "user_id":"user", "stream_state":false, "log_level":1})).unwrap();
        let first = package
            .call(&loaded, &engine, &security, first, &input)
            .await
            .unwrap();
        assert_eq!(first.result.outputs["granted"], json!(1));
        input.node_id = "second".into();
        let second = package
            .call(
                &loaded,
                &engine,
                &security,
                HostState::with_security(&security),
                &input,
            )
            .await
            .unwrap();
        assert_eq!(second.result.outputs["granted"], json!(0));
        assert_eq!(second.result.outputs["calls"], json!(2));
    }

    #[test]
    fn rejects_sql_side_effects_and_multiple_statements() {
        for sql in [
            "DROP TABLE private",
            "SELECT 1; SELECT 2",
            "CREATE EXTERNAL TABLE stolen STORED AS CSV LOCATION '/etc/passwd'",
            "COPY (SELECT 1) TO '/tmp/leak'",
            "SET datafusion.execution.batch_size = 1",
            "INSERT INTO private VALUES (1)",
        ] {
            assert!(validate_sql(sql).is_err(), "{sql}");
        }
        assert!(validate_sql("WITH x AS (SELECT 1 AS id) SELECT id FROM x").is_ok());
    }

    #[tokio::test]
    async fn guessed_handles_and_missing_permissions_are_denied() {
        let mut host = HostState::new(WasmCapabilities::ALL);
        host.database_context = Some(Arc::new(DatabaseContext::default()));
        assert!(
            query(&host, 7, r#"{"cache_key":"private"}"#, "{}")
                .await
                .is_none()
        );
        assert!(
            query(
                &host,
                22,
                r#"{"cache_key":"df_session_default"}"#,
                r#"{"sql":"SELECT 1"}"#
            )
            .await
            .is_none()
        );
        let session = CachedDataFusionSession::new(isolated_session());
        host.database_context
            .as_ref()
            .unwrap()
            .sessions
            .write()
            .insert("granted".into(), session);
        let handle = r#"{"cache_key":"granted"}"#;
        let sql = r#"{"sql":"SELECT 42 AS answer","max_rows":1}"#;
        assert_eq!(
            query(&host, 22, handle, sql).await.as_deref(),
            Some(r#"[{"answer":42}]"#)
        );
        host.capabilities = WasmCapabilities::MODELS;
        assert!(query(&host, 22, handle, sql).await.is_none());
        let mut next = HostState::new(WasmCapabilities::ALL);
        next.database_context = Some(Arc::new(DatabaseContext::default()));
        assert!(query(&next, 22, handle, sql).await.is_none());
    }

    #[tokio::test]
    async fn queries_cannot_open_external_tables_or_inherit_functions() {
        let session = isolated_session();
        for sql in [
            "SELECT * FROM read_csv('/etc/passwd')",
            "SELECT * FROM read_parquet('https://example.com/private')",
            "SELECT * FROM '/etc/passwd'",
            "SELECT read_file('/etc/passwd')",
            "SELECT * FROM information_schema.tables",
            "SELECT * FROM private",
        ] {
            assert!(session.sql(sql).await.is_err(), "{sql}");
        }
    }
}
