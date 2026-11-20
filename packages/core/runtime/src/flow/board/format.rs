//! Board document compatibility, independent of publication and compiled artifact versions.
//!
//! Missing document or client versions mean version 1. A request scope restricts every shared
//! load and persistence boundary to the client's supported version. The runtime also applies its
//! own ceiling when an executor or local editor operates without an HTTP request.

use std::future::Future;

use flow_like_types::{Result, proto, tokio};
use serde_json::Value;

use super::Board;
use crate::flow::{
    pin::{SchemaRefs, resolve_schema},
    variable::VariableType,
};

pub const LEGACY_BOARD_FORMAT_VERSION: u32 = 1;
pub const CURRENT_BOARD_FORMAT_VERSION: u32 = 2;

pub const fn legacy_board_format_version() -> u32 {
    LEGACY_BOARD_FORMAT_VERSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardFormatError {
    pub required: u32,
    pub supported: u32,
}

impl std::fmt::Display for BoardFormatError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Board format version {} is required; this client or runtime supports version {}",
            self.required, self.supported
        )
    }
}

impl std::error::Error for BoardFormatError {}

tokio::task_local! {
    static SUPPORTED_VERSION: u32;
}

/// The ceiling for the current task. Spawned tasks must establish their own scope.
pub fn supported_version() -> u32 {
    SUPPORTED_VERSION
        .try_with(|version| *version)
        .unwrap_or(CURRENT_BOARD_FORMAT_VERSION)
        .min(CURRENT_BOARD_FORMAT_VERSION)
}

pub async fn with_supported_version<F: Future>(supported: u32, future: F) -> F::Output {
    SUPPORTED_VERSION.scope(supported, future).await
}

pub fn ensure_supported(required: u32) -> Result<()> {
    let supported = supported_version();
    if required > supported {
        return Err(BoardFormatError {
            required,
            supported,
        }
        .into());
    }
    Ok(())
}

fn schema_requires_geometry(schema: Option<&str>, refs: &impl SchemaRefs) -> bool {
    let Some(schema) = schema.and_then(|schema| resolve_schema(schema, refs).ok()) else {
        return false;
    };
    let Ok(schema) = serde_json::from_str::<Value>(schema) else {
        return false;
    };
    let mut pending = vec![&schema];
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(object) => {
                if object.get("x-flow-like-type").and_then(Value::as_str) == Some("geometry")
                    || object.get("$id").and_then(Value::as_str) == Some("flow:geometry")
                {
                    return true;
                }
                pending.extend(object.values());
            }
            Value::Array(values) => pending.extend(values),
            _ => {}
        }
    }
    false
}

pub(crate) fn required_version<'a>(
    declared: u32,
    types: impl Iterator<Item = (bool, Option<&'a str>)>,
    refs: &impl SchemaRefs,
) -> u32 {
    let mut required = declared.max(LEGACY_BOARD_FORMAT_VERSION);
    if required >= CURRENT_BOARD_FORMAT_VERSION {
        return required;
    }
    for (geometry, schema) in types {
        if geometry || schema_requires_geometry(schema, refs) {
            required = required.max(2);
            break;
        }
    }
    required
}

impl Board {
    /// Preserve declared requirements and infer the floor for documents written before versioning.
    pub fn required_format_version(&self) -> u32 {
        let variables = self.variables.values().chain(
            self.layers
                .values()
                .flat_map(|layer| layer.variables.values()),
        );
        let pins = self
            .nodes
            .values()
            .flat_map(|node| node.pins.values())
            .chain(self.layers.values().flat_map(|layer| layer.pins.values()))
            .chain(
                self.layers
                    .values()
                    .flat_map(|layer| layer.nodes.values())
                    .flat_map(|node| node.pins.values()),
            );
        let types = variables
            .map(|value| {
                (
                    value.data_type == VariableType::Geometry,
                    value.schema.as_deref(),
                )
            })
            .chain(pins.map(|value| {
                (
                    value.data_type == VariableType::Geometry,
                    value.schema.as_deref(),
                )
            }));
        required_version(self.format_version, types, &self.refs)
    }

    pub fn ensure_supported_format(&self) -> Result<()> {
        ensure_supported(self.required_format_version())
    }

    /// Raw wire preflight must happen before unknown enum codes can be converted to defaults.
    pub(crate) fn ensure_supported_proto_format(board: &proto::Board) -> Result<()> {
        ensure_supported(board.format_version.max(LEGACY_BOARD_FORMAT_VERSION))?;
        ensure_supported(Self::required_proto_format_version(board))
    }

    /// The document requirement for raw protobuf copies that retain all wire codes.
    pub fn required_proto_format_version(board: &proto::Board) -> u32 {
        let variables = board.variables.values().chain(
            board
                .layers
                .values()
                .flat_map(|layer| layer.variables.values()),
        );
        let pins = board
            .nodes
            .values()
            .flat_map(|node| node.pins.values())
            .chain(board.layers.values().flat_map(|layer| layer.pins.values()))
            .chain(
                board
                    .layers
                    .values()
                    .flat_map(|layer| layer.nodes.values())
                    .flat_map(|node| node.pins.values()),
            );
        let types = variables
            .map(|value| {
                (
                    value.data_type == proto::VariableType::Geometry as i32,
                    value.schema.as_deref(),
                )
            })
            .chain(pins.map(|value| {
                (
                    value.data_type == proto::VariableType::Geometry as i32,
                    (!value.schema.is_empty()).then_some(value.schema.as_str()),
                )
            }));
        required_version(board.format_version, types, &board.refs)
    }

    pub(super) fn format_rollback_snapshot(&self) -> Option<Self> {
        (supported_version() < CURRENT_BOARD_FORMAT_VERSION).then(|| self.clone())
    }

    pub(super) fn finish_format_mutation(&mut self, previous: Option<Self>) -> Result<()> {
        if let Err(error) = self.ensure_supported_format() {
            if let Some(previous) = previous {
                *self = previous;
            }
            return Err(error);
        }
        self.format_version = self.required_format_version();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use flow_like_storage::{
        Path,
        object_store::{ObjectStore, memory::InMemory},
    };
    use flow_like_types::{FromProto, Message, ToProto};

    use super::*;
    use crate::{
        flow::{
            board::{
                Layer, LayerType,
                commands::{GenericCommand, variables::upsert_variable::UpsertVariableCommand},
            },
            node::Node,
            pin::ValueType,
            variable::Variable,
        },
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };

    fn board() -> Board {
        Board::new_detached(Some("format-test".into()), Path::from("boards"))
    }

    fn geometry_variable() -> Variable {
        Variable::new("location", VariableType::Geometry, ValueType::Normal)
    }

    fn geometry_board() -> Board {
        let mut board = board();
        let variable = geometry_variable();
        board.variables.insert(variable.id.clone(), variable);
        board
    }

    fn state() -> Arc<FlowLikeState> {
        Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ))
    }

    fn assert_format_error(error: flow_like_types::Error, required: u32, supported: u32) {
        assert_eq!(
            error.downcast_ref::<BoardFormatError>(),
            Some(&BoardFormatError {
                required,
                supported
            })
        );
    }

    #[test]
    fn legacy_json_and_proto_default_to_one_and_new_writes_declare_the_required_version() {
        let mut json = serde_json::to_value(board()).unwrap();
        json.as_object_mut().unwrap().remove("format_version");
        let legacy: Board = serde_json::from_value(json).unwrap();
        assert_eq!(legacy.format_version, 1);
        let mut wire = legacy.to_proto();
        wire.format_version = 0;
        let restored =
            Board::from_proto(proto::Board::decode(wire.encode_to_vec().as_slice()).unwrap());
        assert_eq!(restored.format_version, 1);
        assert_eq!(restored.to_proto().format_version, 1);

        let mut unversioned_geometry = geometry_board().to_proto();
        unversioned_geometry.format_version = 0;
        let restored = Board::from_proto(unversioned_geometry);
        assert_eq!(restored.format_version, 2);
        assert_eq!(
            serde_json::to_value(&restored).unwrap()["format_version"],
            2
        );

        let mut geometry = geometry_board();
        geometry.mark_changed();
        assert_eq!(
            serde_json::to_value(&geometry).unwrap()["format_version"],
            2
        );
        let restored = Board::from_proto(
            proto::Board::decode(geometry.to_proto().encode_to_vec().as_slice()).unwrap(),
        );
        assert_eq!(restored.format_version, 2);
        geometry.variables.clear();
        geometry.mark_changed();
        assert_eq!(geometry.required_format_version(), 2);
    }

    #[test]
    fn future_declared_versions_are_preserved_and_rejected_before_unknown_wire_types() {
        let mut future = board();
        future.format_version = 99;
        let mut wire = future.to_proto();
        let mut variable = geometry_variable().to_proto();
        variable.data_type = 999;
        wire.variables.insert("future".into(), variable);
        assert_format_error(Board::validate_proto_types(&wire).unwrap_err(), 99, 2);
        let restored: Board =
            serde_json::from_value(serde_json::to_value(future).unwrap()).unwrap();
        assert_eq!(restored.to_proto().format_version, 99);
        assert_format_error(restored.ensure_supported_format().unwrap_err(), 99, 2);
    }

    #[test]
    fn geometry_requirements_cover_layers_pins_and_resolved_nested_schemas() {
        let mut board = board();
        let mut layer = Layer::new("function".into(), "Function".into(), LayerType::Function);
        let variable = geometry_variable();
        layer.variables.insert(variable.id.clone(), variable);
        board.layers.insert(layer.id.clone(), layer);
        assert_eq!(board.required_format_version(), 2);
        board.layers.get_mut("function").unwrap().variables.clear();
        let mut node = Node::new("test", "Test", "", "test");
        node.add_output_pin("geometry", "Geometry", "", VariableType::Geometry);
        board
            .layers
            .get_mut("function")
            .unwrap()
            .nodes
            .insert(node.id.clone(), node);
        assert_eq!(board.required_format_version(), 2);

        board.layers.clear();
        let mut variable = Variable::new("wrapper", VariableType::Struct, ValueType::Normal);
        variable.schema = Some("ref-a".into());
        board.refs.insert("ref-a".into(), "ref-b".into());
        board.refs.insert(
            "ref-b".into(),
            r#"{"type":"object","properties":{"position":{"x-flow-like-type":"geometry"}}}"#.into(),
        );
        board.variables.insert(variable.id.clone(), variable);
        assert_eq!(board.required_format_version(), 2);
    }

    #[tokio::test]
    async fn scopes_are_isolated_and_runtime_ceiling_cannot_be_lifted_by_the_client() {
        let (old, new) = tokio::join!(
            with_supported_version(1, async {
                tokio::task::yield_now().await;
                ensure_supported(2)
            }),
            with_supported_version(99, async {
                tokio::task::yield_now().await;
                assert!(ensure_supported(2).is_ok());
                ensure_supported(3)
            })
        );
        assert_format_error(old.unwrap_err(), 2, 1);
        assert_format_error(new.unwrap_err(), 3, 2);
        assert!(ensure_supported(2).is_ok());
    }

    #[tokio::test]
    async fn compiled_documents_preserve_requirements_and_cannot_understate_geometry() {
        use crate::flow::compiled::{compile_board, encode_artifact};

        let mut compiled = compile_board(&geometry_board()).unwrap();
        assert_eq!(compiled.board_format_version, 2);
        compiled.board_format_version = 1;
        with_supported_version(1, async {
            assert_format_error(compiled.validate().unwrap_err(), 2, 1);
            assert_format_error(encode_artifact(&compiled, &[0; 32]).unwrap_err(), 2, 1);
        })
        .await;
        compiled.board_format_version = 3;
        assert_format_error(compiled.validate().unwrap_err(), 3, 2);
    }

    #[tokio::test]
    async fn legacy_clients_cannot_load_pre_version_geometry_or_write_any_board_artifact() {
        let geometry = geometry_board();
        let mut wire = geometry.to_proto();
        wire.format_version = 0;
        with_supported_version(1, async {
            assert_format_error(Board::validate_proto_types(&wire).unwrap_err(), 2, 1);
            let error = Board::from_loaded_proto(wire, Path::default(), state())
                .await
                .err()
                .unwrap();
            assert_format_error(error, 2, 1);
            let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
            assert_format_error(geometry.save(Some(store.clone())).await.unwrap_err(), 2, 1);
            assert_format_error(
                geometry
                    .snapshot_at_version((1, 0, 0), Some(store.clone()))
                    .await
                    .unwrap_err(),
                2,
                1,
            );
            assert_format_error(
                geometry
                    .save_as_template(None, Some(store.clone()))
                    .await
                    .unwrap_err(),
                2,
                1,
            );
            let mut geometry = geometry.clone();
            assert_format_error(
                geometry
                    .overwrite_template_version((1, 0, 0), None, Some(store.clone()))
                    .await
                    .unwrap_err(),
                2,
                1,
            );
            use futures::TryStreamExt;
            assert!(
                store
                    .list(None)
                    .try_collect::<Vec<_>>()
                    .await
                    .unwrap()
                    .is_empty()
            );
        })
        .await;
    }

    #[tokio::test]
    async fn rejected_geometry_introductions_restore_the_cached_board_for_commands_and_redo() {
        let state = state();
        let mut board = board();
        let before = serde_json::to_value(&board).unwrap();
        let command =
            GenericCommand::UpsertVariable(UpsertVariableCommand::new(geometry_variable()));
        with_supported_version(1, async {
            assert_format_error(
                board
                    .execute_command(command.clone(), state.clone())
                    .await
                    .err()
                    .unwrap(),
                2,
                1,
            );
            assert_eq!(serde_json::to_value(&board).unwrap(), before);
            assert_format_error(
                board
                    .execute_commands(vec![command.clone()], state.clone())
                    .await
                    .err()
                    .unwrap(),
                2,
                1,
            );
            assert_eq!(serde_json::to_value(&board).unwrap(), before);
            assert_format_error(
                board
                    .redo(vec![command.clone()], state.clone())
                    .await
                    .unwrap_err(),
                2,
                1,
            );
            assert_eq!(serde_json::to_value(&board).unwrap(), before);
        })
        .await;
        board.execute_commands(vec![command], state).await.unwrap();
        assert_eq!(board.format_version, 2);
    }
}
