//! Secret values inside the board commands a client receives and replays.
//!
//! An executed command carries the state undo and redo need: the variable a removal deleted,
//! the node an update replaced. For a secret variable or a sensitive pin that state is the
//! secret, and a board response must never hold one (see [`super::secrets`]).
//!
//! A command therefore leaves the API with those values cut out and, when it had any, with
//! them encrypted under a key only the server holds, in `__sealed`. Clients keep and replay the
//! command as they received it, and the values go back in before it runs. A command without
//! `__sealed` runs as it is.

use std::collections::{BTreeMap, HashMap};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use flow_like::flow::{
    board::{Layer, commands::GenericCommand},
    node::Node,
    pin::Pin,
    variable::Variable,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::ApiError,
    state::AppState,
    utils::crypto::{decrypt_secret, encrypt_secret},
};

const KEY_CONTEXT: &str = "flow-like 2026-11-24 board command seal";
const VERSION: u8 = 1;
const SEPARATOR: &str = "\u{1f}";

/// A board command as clients send and receive it.
#[derive(Clone, Serialize, Deserialize)]
pub struct SealedCommand {
    #[serde(flatten)]
    pub command: GenericCommand,
    #[serde(rename = "__sealed", default, skip_serializing_if = "Option::is_none")]
    pub sealed: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SealedValues {
    version: u8,
    app_id: String,
    board_id: String,
    values: BTreeMap<String, String>,
}

/// Seals and opens the commands of one board. A seal made for another board, or with a key
/// this server does not hold, does not open.
pub struct CommandSeal<'a> {
    key: [u8; 32],
    app_id: &'a str,
    board_id: &'a str,
}

impl<'a> CommandSeal<'a> {
    pub fn new(state: &AppState, app_id: &'a str, board_id: &'a str) -> Self {
        Self::with_key(&state.encryption_key, app_id, board_id)
    }

    fn with_key(key: &[u8; 32], app_id: &'a str, board_id: &'a str) -> Self {
        Self {
            key: blake3::derive_key(KEY_CONTEXT, key),
            app_id,
            board_id,
        }
    }

    /// Cuts every secret value out of `commands` and hands each command back with its values
    /// sealed beside it.
    pub fn seal(&self, commands: Vec<GenericCommand>) -> Vec<SealedCommand> {
        commands
            .into_iter()
            .map(|command| self.seal_one(command))
            .collect()
    }

    fn seal_one(&self, mut command: GenericCommand) -> SealedCommand {
        let mut values = BTreeMap::new();
        for (path, value) in secret_slots(&mut command) {
            if let Some(value) = value.take() {
                values.insert(path, BASE64.encode(value));
            }
        }
        let sealed = (!values.is_empty()).then(|| self.encrypt(values));
        SealedCommand { command, sealed }
    }

    /// Seals the commands of `stored` that were kept before sealing existed and leaves the rest
    /// as they are.
    pub fn seal_stored(&self, stored: Vec<SealedCommand>) -> Vec<SealedCommand> {
        stored
            .into_iter()
            .map(|stored| match stored.sealed {
                Some(_) => stored,
                None => self.seal_one(stored.command),
            })
            .collect()
    }

    /// Puts sealed values back into the secret slots a replayed command left empty. A slot the
    /// command no longer marks secret stays as the client sent it.
    pub fn open(&self, commands: Vec<SealedCommand>) -> Result<Vec<GenericCommand>, ApiError> {
        commands
            .into_iter()
            .map(|command| self.open_one(command))
            .collect()
    }

    fn open_one(&self, sealed: SealedCommand) -> Result<GenericCommand, ApiError> {
        let SealedCommand {
            mut command,
            sealed,
        } = sealed;
        let Some(sealed) = sealed else {
            return Ok(command);
        };
        let mut values = self.decrypt(&sealed)?;
        for (path, value) in secret_slots(&mut command) {
            if value.is_none()
                && let Some(sealed_value) = values.remove(&path)
            {
                *value = Some(BASE64.decode(sealed_value).map_err(|_| unreadable())?);
            }
        }
        Ok(command)
    }

    /// `body` as the JSON it serializes to, with `commands` sealed in place of its own.
    pub fn body_with_commands<T: Serialize>(
        &self,
        body: &T,
        commands: Vec<GenericCommand>,
    ) -> Result<serde_json::Value, ApiError> {
        let internal = |error: serde_json::Error| {
            ApiError::internal(format!("command response serialization failed: {error}"))
        };
        let mut body = serde_json::to_value(body).map_err(internal)?;
        let commands = serde_json::to_value(self.seal(commands)).map_err(internal)?;
        body.as_object_mut()
            .ok_or_else(|| ApiError::internal("command response is not a JSON object"))?
            .insert("commands".to_string(), commands);
        Ok(body)
    }

    fn encrypt(&self, values: BTreeMap<String, String>) -> String {
        let sealed = SealedValues {
            version: VERSION,
            app_id: self.app_id.to_string(),
            board_id: self.board_id.to_string(),
            values,
        };
        let plaintext = serde_json::to_string(&sealed).expect("string maps serialize");
        encrypt_secret(&plaintext, &self.key)
    }

    fn decrypt(&self, sealed: &str) -> Result<BTreeMap<String, String>, ApiError> {
        let sealed = decrypt_secret(sealed, &self.key)
            .and_then(|plaintext| serde_json::from_str::<SealedValues>(&plaintext).ok())
            .filter(|sealed| {
                sealed.version == VERSION
                    && sealed.app_id == self.app_id
                    && sealed.board_id == self.board_id
            })
            .ok_or_else(unreadable)?;
        Ok(sealed.values)
    }
}

fn unreadable() -> ApiError {
    ApiError::unprocessable(
        "This change carries protected values that were sealed for another board or can no \
         longer be read, so it cannot be replayed. Nothing was changed.",
    )
}

fn join(path: &str, part: &str) -> String {
    format!("{path}{SEPARATOR}{part}")
}

struct Slots<'a>(Vec<(String, &'a mut Option<Vec<u8>>)>);

impl<'a> Slots<'a> {
    fn variable(&mut self, at: &str, variable: &'a mut Variable) {
        if variable.secret {
            self.0
                .push((join(at, &variable.id), &mut variable.default_value));
        }
    }

    fn pin(&mut self, at: &str, pin: &'a mut Pin) {
        if pin.is_sensitive() {
            self.0.push((join(at, &pin.id), &mut pin.default_value));
        }
    }

    fn pins(&mut self, at: &str, pins: &'a mut HashMap<String, Pin>) {
        for (key, pin) in pins {
            if pin.is_sensitive() {
                self.0.push((join(at, key), &mut pin.default_value));
            }
        }
    }

    fn node(&mut self, at: &str, node: &'a mut Node) {
        let at = join(at, &node.id);
        self.pins(&at, &mut node.pins);
    }

    fn nodes(&mut self, at: &str, nodes: &'a mut [Node]) {
        for (index, node) in nodes.iter_mut().enumerate() {
            self.node(&join(at, &index.to_string()), node);
        }
    }

    fn layer(&mut self, at: &str, layer: &'a mut Layer) {
        let at = join(at, &layer.id);
        let variables = join(&at, "variables");
        for variable in layer.variables.values_mut() {
            self.variable(&variables, variable);
        }
        self.pins(&join(&at, "pins"), &mut layer.pins);
        let nodes = join(&at, "nodes");
        for node in layer.nodes.values_mut() {
            self.node(&nodes, node);
        }
    }

    fn layers(&mut self, at: &str, layers: &'a mut [Layer]) {
        for (index, layer) in layers.iter_mut().enumerate() {
            self.layer(&join(at, &index.to_string()), layer);
        }
    }
}

/// Every secret variable value and sensitive pin literal `command` carries, each under a path
/// that names its place in the command. The match is exhaustive on purpose: a new command must
/// say here which of its fields can hold a secret.
fn secret_slots(command: &mut GenericCommand) -> Vec<(String, &mut Option<Vec<u8>>)> {
    let mut slots = Slots(Vec::new());
    match command {
        GenericCommand::UpsertVariable(command) => {
            slots.variable("variable", &mut command.variable);
            if let Some(old_variable) = &mut command.old_variable {
                slots.variable("old_variable", old_variable);
            }
        }
        GenericCommand::RemoveVariable(command) => {
            slots.variable("variable", &mut command.variable);
        }
        GenericCommand::AddNode(command) => slots.node("node", &mut command.node),
        GenericCommand::UpdateNode(command) => {
            slots.node("node", &mut command.node);
            if let Some(old_node) = &mut command.old_node {
                slots.node("old_node", old_node);
            }
        }
        GenericCommand::RemoveNode(command) => {
            slots.node("node", &mut command.node);
            slots.nodes("connected_nodes", &mut command.connected_nodes);
        }
        GenericCommand::UpsertPin(command) => {
            slots.pin("pin", &mut command.pin);
            if let Some(old_pin) = &mut command.old_pin {
                slots.pin("old_pin", old_pin);
            }
        }
        GenericCommand::CopyPaste(command) => {
            slots.nodes("original_nodes", &mut command.original_nodes);
            slots.nodes("new_nodes", &mut command.new_nodes);
            slots.layers("original_layers", &mut command.original_layers);
            slots.layers("new_layers", &mut command.new_layers);
            for (index, variable) in command.original_variables.iter_mut().enumerate() {
                slots.variable(&join("original_variables", &index.to_string()), variable);
            }
        }
        GenericCommand::UpsertLayer(command) => {
            slots.layer("layer", &mut command.layer);
            if let Some(old_layer) = &mut command.old_layer {
                slots.layer("old_layer", old_layer);
            }
        }
        GenericCommand::RemoveLayer(command) => {
            slots.layer("layer", &mut command.layer);
            slots.layers("layers", &mut command.layers);
            slots.nodes("nodes", &mut command.nodes);
        }
        GenericCommand::MoveToLayer(command) => {
            for (index, before) in command.previous_pins.iter_mut().enumerate() {
                slots.pin(&join("previous_pins", &index.to_string()), &mut before.pin);
            }
        }
        GenericCommand::RemoveComment(_)
        | GenericCommand::UpsertComment(_)
        | GenericCommand::MoveNode(_)
        | GenericCommand::DisconnectPin(_)
        | GenericCommand::ConnectPin(_) => {}
    }
    slots.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use flow_like::flow::{
        board::LayerType,
        board::commands::{
            layer::{
                move_to_layer::{MoveToLayerCommand, PinBefore, PinHolder},
                remove_layer::RemoveLayerCommand,
                upsert_layer::UpsertLayerCommand,
            },
            nodes::{
                add_node::AddNodeCommand, copy_paste::CopyPasteCommand,
                remove_node::RemoveNodeCommand, update_node::UpdateNodeCommand,
            },
            pins::upsert_pin::UpsertPinCommand,
            variables::{
                remove_variable::RemoveVariableCommand, upsert_variable::UpsertVariableCommand,
            },
        },
        pin::{PinOptions, ValueType},
        variable::VariableType,
    };
    use serde_json::{Value, json};

    const KEY: [u8; 32] = [7; 32];
    const SECRET: &str = "s3cr3t-value";

    fn seal() -> CommandSeal<'static> {
        CommandSeal::with_key(&KEY, "app-1", "board-1")
    }

    fn bytes(value: &str) -> Vec<u8> {
        serde_json::to_vec(&json!(value)).unwrap()
    }

    fn secret_variable(id: &str) -> Variable {
        let mut variable = Variable::new("token", VariableType::String, ValueType::Normal);
        variable.id = id.to_string();
        variable.secret = true;
        variable.default_value = Some(bytes(SECRET));
        variable
    }

    fn node_with_key(id: &str) -> Node {
        let mut node = Node::new("demo", "Demo", "", "Test");
        node.id = id.to_string();
        node.add_input_pin("api_key", "API key", "", VariableType::String)
            .set_options(PinOptions::new().set_sensitive(true).build())
            .set_default_value(Some(json!(SECRET)));
        node.add_input_pin("plain", "Plain", "", VariableType::String)
            .set_default_value(Some(json!("visible")));
        node
    }

    fn sensitive_pin(node: &Node) -> Pin {
        node.get_pin_by_name("api_key").unwrap().clone()
    }

    fn layer_with_secrets(id: &str) -> Layer {
        let mut layer = Layer::new(id.to_string(), "Layer".to_string(), LayerType::Collapsed);
        let inner = node_with_key(&format!("{id}-node"));
        let boundary = sensitive_pin(&inner);
        layer.pins.insert(boundary.id.clone(), boundary);
        layer.nodes.insert(inner.id.clone(), inner);
        let variable = secret_variable(&format!("{id}-variable"));
        layer.variables.insert(variable.id.clone(), variable);
        layer
    }

    /// One command per variant that can hold a secret, with a secret in every such field.
    fn commands_holding_secrets() -> Vec<GenericCommand> {
        let node = node_with_key("node-1");
        let mut upsert_variable = UpsertVariableCommand::new(secret_variable("variable-1"));
        upsert_variable.old_variable = Some(secret_variable("variable-1"));
        let mut update_node = UpdateNodeCommand::new(node.clone());
        update_node.old_node = Some(node.clone());
        let mut upsert_pin = UpsertPinCommand::new(node.id.clone(), sensitive_pin(&node));
        upsert_pin.old_pin = Some(sensitive_pin(&node));
        let mut copy_paste = CopyPasteCommand::new(
            vec![node.clone()],
            vec![],
            vec![layer_with_secrets("layer-original")],
            (0.0, 0.0, 0.0),
        );
        copy_paste.new_nodes = vec![node_with_key("node-pasted")];
        copy_paste.new_layers = vec![layer_with_secrets("layer-pasted")];
        copy_paste.original_variables = vec![secret_variable("variable-copied")];
        let mut upsert_layer = UpsertLayerCommand::new(layer_with_secrets("layer-1"));
        upsert_layer.old_layer = Some(layer_with_secrets("layer-1"));
        let mut remove_layer =
            RemoveLayerCommand::new(layer_with_secrets("layer-2"), vec![node.clone()], false);
        remove_layer.layers = vec![layer_with_secrets("layer-child")];
        let mut move_to_layer = MoveToLayerCommand::new(vec![node.id.clone()], None);
        move_to_layer.previous_pins = vec![PinBefore {
            holder: PinHolder::Node(node.id.clone()),
            pin: sensitive_pin(&node),
        }];
        vec![
            GenericCommand::UpsertVariable(upsert_variable),
            GenericCommand::RemoveVariable(RemoveVariableCommand::new(secret_variable(
                "variable-2",
            ))),
            GenericCommand::AddNode(AddNodeCommand {
                node: node.clone(),
                current_layer: None,
            }),
            GenericCommand::UpdateNode(update_node),
            GenericCommand::RemoveNode(RemoveNodeCommand {
                node: node.clone(),
                connected_nodes: vec![node_with_key("node-2")],
            }),
            GenericCommand::UpsertPin(upsert_pin),
            GenericCommand::CopyPaste(copy_paste),
            GenericCommand::UpsertLayer(upsert_layer),
            GenericCommand::RemoveLayer(remove_layer),
            GenericCommand::MoveToLayer(move_to_layer),
        ]
    }

    fn wire(commands: &[SealedCommand]) -> String {
        serde_json::to_string(commands).unwrap()
    }

    /// The value as `serde_json` writes a byte vector: a JSON array of numbers.
    fn secret_on_the_wire() -> String {
        serde_json::to_string(&bytes(SECRET)).unwrap()
    }

    fn canonical(commands: &[GenericCommand]) -> Vec<Value> {
        commands
            .iter()
            .map(|command| serde_json::to_value(command).unwrap())
            .collect()
    }

    #[test]
    fn a_sealed_command_shows_no_secret_and_replays_exactly_as_it_was_executed() {
        let executed = commands_holding_secrets();
        assert!(
            serde_json::to_string(&executed)
                .unwrap()
                .contains(&secret_on_the_wire())
        );

        let sealed = seal().seal(executed.clone());
        let on_the_wire = wire(&sealed);
        assert!(!on_the_wire.contains(&secret_on_the_wire()));
        assert!(!on_the_wire.contains(SECRET));
        assert!(sealed.iter().all(|command| command.sealed.is_some()));

        let replayed: Vec<SealedCommand> = serde_json::from_str(&on_the_wire).unwrap();
        let opened = seal().open(replayed).unwrap();
        assert_eq!(canonical(&opened), canonical(&executed));
    }

    #[test]
    fn every_secret_a_command_holds_has_a_slot_of_its_own() {
        for mut command in commands_holding_secrets() {
            let before = serde_json::to_string(&command).unwrap();
            let held = before.matches(&secret_on_the_wire()).count();
            let slots = secret_slots(&mut command);
            assert_eq!(slots.len(), held, "{before}");
            let paths: std::collections::HashSet<&String> =
                slots.iter().map(|(path, _)| path).collect();
            assert_eq!(paths.len(), slots.len(), "paths must not collide");
        }
    }

    #[test]
    fn a_command_without_secrets_crosses_the_api_unchanged() {
        let mut node = node_with_key("node-1");
        node.pins.retain(|_, pin| !pin.is_sensitive());
        let command = GenericCommand::UpdateNode(UpdateNodeCommand::new(node));
        let before = serde_json::to_value(&command).unwrap();

        let sealed = seal().seal(vec![command]);
        assert!(sealed[0].sealed.is_none());
        assert_eq!(serde_json::to_value(&sealed[0]).unwrap(), before);

        let legacy: Vec<SealedCommand> = serde_json::from_value(json!([before])).unwrap();
        let opened = seal().open(legacy).unwrap();
        assert_eq!(canonical(&opened), [before]);
    }

    #[test]
    fn history_recorded_before_sealing_still_replays_with_its_own_values() {
        let executed = commands_holding_secrets();
        let recorded: Vec<SealedCommand> =
            serde_json::from_value(Value::Array(canonical(&executed))).unwrap();
        let opened = seal().open(recorded).unwrap();
        assert_eq!(canonical(&opened), canonical(&executed));
    }

    #[test]
    fn a_seal_opens_only_for_the_board_and_key_it_was_made_for() {
        let sealed = seal().seal(commands_holding_secrets());
        for other in [
            CommandSeal::with_key(&KEY, "app-1", "board-2"),
            CommandSeal::with_key(&KEY, "app-2", "board-1"),
            CommandSeal::with_key(&[8; 32], "app-1", "board-1"),
        ] {
            let refused = other.open(sealed.clone()).err().expect("refused");
            assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        }

        let mut tampered = sealed;
        tampered[0].sealed = Some("not a seal".to_string());
        assert!(seal().open(tampered).is_err());
    }

    #[test]
    fn a_seal_is_not_a_value_another_feature_encrypted_with_the_server_key() {
        let foreign = encrypt_secret(
            r#"{"version":1,"app_id":"app-1","board_id":"board-1","values":{}}"#,
            &KEY,
        );
        let mut sealed = seal().seal(commands_holding_secrets());
        sealed[0].sealed = Some(foreign);
        assert!(seal().open(sealed).is_err());
    }

    #[test]
    fn a_sealed_value_never_lands_in_a_slot_the_client_made_readable() {
        let command = GenericCommand::RemoveVariable(RemoveVariableCommand::new(secret_variable(
            "variable-1",
        )));
        let mut sealed = seal().seal(vec![command]);
        let GenericCommand::RemoveVariable(replayed) = &mut sealed[0].command else {
            unreachable!()
        };
        replayed.variable.secret = false;

        let opened = seal().open(sealed).unwrap();
        let GenericCommand::RemoveVariable(opened) = &opened[0] else {
            unreachable!()
        };
        assert_eq!(opened.variable.default_value, None);
    }

    #[test]
    fn a_value_the_client_sends_wins_over_the_sealed_one() {
        let command = GenericCommand::UpsertVariable(UpsertVariableCommand::new(secret_variable(
            "variable-1",
        )));
        let mut sealed = seal().seal(vec![command]);
        let GenericCommand::UpsertVariable(replayed) = &mut sealed[0].command else {
            unreachable!()
        };
        replayed.variable.default_value = Some(bytes("rotated"));

        let opened = seal().open(sealed).unwrap();
        let GenericCommand::UpsertVariable(opened) = &opened[0] else {
            unreachable!()
        };
        assert_eq!(opened.variable.default_value, Some(bytes("rotated")));
    }

    #[test]
    fn a_response_body_keeps_its_fields_and_gets_sealed_commands() {
        #[derive(Serialize)]
        struct Body {
            status: &'static str,
            commands: Vec<GenericCommand>,
        }
        let body = Body {
            status: "applied",
            commands: Vec::new(),
        };
        let json = seal()
            .body_with_commands(&body, commands_holding_secrets())
            .unwrap();
        assert_eq!(json["status"], "applied");
        let commands = json["commands"].as_array().unwrap();
        assert_eq!(commands.len(), commands_holding_secrets().len());
        assert!(
            commands
                .iter()
                .all(|command| command["__sealed"].is_string())
        );
        assert!(!json.to_string().contains(&secret_on_the_wire()));
    }

    #[test]
    fn every_route_that_returns_or_replays_commands_seals_and_opens_them() {
        let execute = include_str!("execute_commands.rs");
        assert!(execute.contains("seal.open(params.commands)?"));
        assert!(execute.contains("seal.seal(commands)"));
        let replay = include_str!("undo_redo_board.rs");
        assert_eq!(replay.matches(".open(params.commands)?").count(), 2);
        assert!(!replay.contains("Vec<GenericCommand>"));
        let flowscript = include_str!("apply_flowscript.rs");
        assert!(flowscript.contains(".body_with_commands(&result, commands)?"));
        assert!(!flowscript.contains("Json(result)"));
        let commit = include_str!("flow_ir_commit.rs");
        assert!(commit.contains(".seal(apply_result.commands.clone())"));
        assert!(commit.contains(".seal_stored(receipt.commands)"));
        assert!(!commit.contains("Vec<GenericCommand>"));
    }

    #[test]
    fn a_receipt_stored_before_sealing_is_sealed_when_it_is_replayed() {
        let executed = commands_holding_secrets();
        let stored: Vec<SealedCommand> =
            serde_json::from_value(Value::Array(canonical(&executed))).unwrap();
        let replayed = seal().seal_stored(stored);
        assert!(!wire(&replayed).contains(&secret_on_the_wire()));

        let again = seal().seal_stored(replayed.clone());
        assert_eq!(wire(&again), wire(&replayed));
        let opened = seal().open(again).unwrap();
        assert_eq!(canonical(&opened), canonical(&executed));
    }
}
