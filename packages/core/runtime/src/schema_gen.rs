use crate::{
    app::{App, AppSearchQuery},
    bit::{
        Bit, BitModelPreference, BitPack, LLMParameters, SttModelParameters, TtsModelParameters,
        VLMParameters,
    },
    flow::{
        board::{
            Board, VersionType,
            commands::{
                GenericCommand,
                comments::{
                    remove_comment::RemoveCommentCommand, upsert_comment::UpsertCommentCommand,
                },
                layer::{
                    move_to_layer::MoveToLayerCommand, remove_layer::RemoveLayerCommand,
                    upsert_layer::UpsertLayerCommand,
                },
                nodes::{
                    add_node::AddNodeCommand, copy_paste::CopyPasteCommand,
                    move_node::MoveNodeCommand, remove_node::RemoveNodeCommand,
                    update_node::UpdateNodeCommand,
                },
                pins::{
                    connect_pins::ConnectPinsCommand, disconnect_pins::DisconnectPinsCommand,
                    upsert_pin::UpsertPinCommand,
                },
                variables::{
                    remove_variable::RemoveVariableCommand, upsert_variable::UpsertVariableCommand,
                },
            },
        },
        event::{
            ApiEventParameters, CanaryEvent, ChatEventParameters, EmailEventParameters, Event,
            EventPayload, EventVariant,
        },
        execution::{LogMeta, RunPayload, log::LogMessage},
        node::Node,
        pin::Pin,
        variable::Variable,
    },
    hub::{BitSearchQuery, Hub},
    profile::Profile,
};
use flow_like_model_provider::{
    history::History,
    provider::{EmbeddingModelProvider, ImageEmbeddingModelProvider},
    response::Response,
    response_chunk::ResponseChunk,
};
use flow_like_storage::files::store::StorageItem;
use flow_like_types::{Result, intercom::InterComEvent, json::to_string_pretty};
use schemars::{JsonSchema, schema_for};
use serde::Serialize;
use std::{
    fs::write,
    path::{Path, PathBuf},
};

fn save_schema<T: ?Sized + Serialize>(schema: &T, path: &PathBuf) -> Result<()> {
    let schema_str = to_string_pretty(schema)?;
    write(path, schema_str)?;
    Ok(())
}

fn generate_and_save_schema<T: Serialize + JsonSchema>(base_path: &Path, path: &str) -> Result<()> {
    let schema = schema_for!(T);
    let full_path = base_path.join(path);

    if !full_path.parent().unwrap().exists() {
        std::fs::create_dir_all(full_path.parent().unwrap())?;
    }

    save_schema(&schema, &full_path)
}
pub fn generate_schema(base_path: PathBuf) -> flow_like_types::Result<()> {
    generate_and_save_schema::<InterComEvent>(&base_path, "events/intercom-event.json")?;

    generate_and_save_schema::<StorageItem>(&base_path, "storage/storage-item.json")?;

    generate_and_save_schema::<History>(&base_path, "llm/history.json")?;
    generate_and_save_schema::<Response>(&base_path, "llm/response.json")?;
    generate_and_save_schema::<ResponseChunk>(&base_path, "llm/response-chunk.json")?;

    generate_and_save_schema::<EmbeddingModelProvider>(
        &base_path,
        "bit/bit/embedding-model-parameters.json",
    )?;
    generate_and_save_schema::<ImageEmbeddingModelProvider>(
        &base_path,
        "bit/bit/image-embedding-model-parameters.json",
    )?;
    generate_and_save_schema::<VLMParameters>(&base_path, "bit/bit/vlm-parameters.json")?;
    generate_and_save_schema::<LLMParameters>(&base_path, "bit/bit/llm-parameters.json")?;
    generate_and_save_schema::<TtsModelParameters>(&base_path, "bit/bit/tts-parameters.json")?;
    generate_and_save_schema::<SttModelParameters>(&base_path, "bit/bit/stt-parameters.json")?;

    generate_and_save_schema::<Bit>(&base_path, "bit/bit.json")?;
    generate_and_save_schema::<BitModelPreference>(&base_path, "bit/preferences.json")?;
    generate_and_save_schema::<BitPack>(&base_path, "bit/bit-pack.json")?;

    generate_and_save_schema::<RunPayload>(&base_path, "flow/run-payload.json")?;
    generate_and_save_schema::<Board>(&base_path, "flow/board.json")?;
    generate_and_save_schema::<Event>(&base_path, "flow/event.json")?;
    generate_and_save_schema::<EventPayload>(&base_path, "flow/event-payload.json")?;
    generate_and_save_schema::<EmailEventParameters>(&base_path, "flow/event-payload-mail.json")?;
    generate_and_save_schema::<ChatEventParameters>(&base_path, "flow/event-payload-chat.json")?;
    generate_and_save_schema::<ApiEventParameters>(&base_path, "flow/event-payload-api.json")?;
    generate_and_save_schema::<VersionType>(&base_path, "flow/version-type.json")?;
    generate_and_save_schema::<CanaryEvent>(&base_path, "flow/canary.json")?;
    generate_and_save_schema::<EventVariant>(&base_path, "flow/variant.json")?;
    generate_and_save_schema::<GenericCommand>(
        &base_path,
        "flow/board/commands/generic-command.json",
    )?;
    generate_and_save_schema::<RemoveCommentCommand>(
        &base_path,
        "flow/board/commands/remove-comment.json",
    )?;
    generate_and_save_schema::<UpsertCommentCommand>(
        &base_path,
        "flow/board/commands/upsert-comment.json",
    )?;
    generate_and_save_schema::<AddNodeCommand>(&base_path, "flow/board/commands/add-node.json")?;
    generate_and_save_schema::<CopyPasteCommand>(
        &base_path,
        "flow/board/commands/copy-paste.json",
    )?;
    generate_and_save_schema::<MoveNodeCommand>(&base_path, "flow/board/commands/move-node.json")?;
    generate_and_save_schema::<RemoveNodeCommand>(
        &base_path,
        "flow/board/commands/remove-node.json",
    )?;
    generate_and_save_schema::<UpdateNodeCommand>(
        &base_path,
        "flow/board/commands/update-node.json",
    )?;
    generate_and_save_schema::<DisconnectPinsCommand>(
        &base_path,
        "flow/board/commands/disconnect-pins.json",
    )?;
    generate_and_save_schema::<ConnectPinsCommand>(
        &base_path,
        "flow/board/commands/connect-pins.json",
    )?;
    generate_and_save_schema::<UpsertPinCommand>(
        &base_path,
        "flow/board/commands/upsert-pin.json",
    )?;
    generate_and_save_schema::<RemoveVariableCommand>(
        &base_path,
        "flow/board/commands/remove-variable.json",
    )?;
    generate_and_save_schema::<UpsertVariableCommand>(
        &base_path,
        "flow/board/commands/upsert-variable.json",
    )?;
    generate_and_save_schema::<UpsertLayerCommand>(
        &base_path,
        "flow/board/commands/upsert-layer.json",
    )?;
    generate_and_save_schema::<RemoveLayerCommand>(
        &base_path,
        "flow/board/commands/remove-layer.json",
    )?;
    generate_and_save_schema::<MoveToLayerCommand>(
        &base_path,
        "flow/board/commands/move-to-layer.json",
    )?;
    generate_and_save_schema::<Node>(&base_path, "flow/node.json")?;
    generate_and_save_schema::<Pin>(&base_path, "flow/pin.json")?;
    generate_and_save_schema::<Variable>(&base_path, "flow/variable.json")?;
    generate_and_save_schema::<LogMessage>(&base_path, "flow/log.json")?;
    generate_and_save_schema::<LogMeta>(&base_path, "flow/log-metadata.json")?;

    generate_and_save_schema::<Profile>(&base_path, "profile/profile.json")?;

    generate_and_save_schema::<Hub>(&base_path, "hub/hub.json")?;
    generate_and_save_schema::<BitSearchQuery>(&base_path, "hub/bit-search-query.json")?;

    generate_and_save_schema::<App>(&base_path, "app/app.json")?;
    generate_and_save_schema::<AppSearchQuery>(&base_path, "app/app-search-query.json")?;

    generate_and_save_schema::<StorageItem>(&base_path, "files/file-metadata.json")?;

    Ok(())
}
