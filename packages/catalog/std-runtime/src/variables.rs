pub mod get;
pub mod set;

use flow_like::flow::{
    board::Board,
    pin::{Pin, geometry_pins_are_compatible, schemas_are_compatible},
    variable::VariableType,
};

fn enforces_generic_value_type(pin: &Pin) -> bool {
    pin.options
        .as_ref()
        .and_then(|options| options.enforce_generic_value_type)
        .unwrap_or(false)
}

/// Whether the wire `source -> target` survives a variable node re-typing its end of it.
///
/// A `Generic` peer is kept, as the editor's connection rule keeps it: it takes its type from this
/// wire through its own `match_type`, which may run later in the same `node_updates` pass. Judging
/// it by its not-yet-derived type dropped every wire that one command batch made into a variable
/// node typed in that same batch (a replayed FlowScript apply lost `item.field -> Set x`).
pub(crate) fn wire_survives_retype(source: &Pin, target: &Pin, board: &Board) -> bool {
    let generic_peer =
        source.data_type == VariableType::Generic || target.data_type == VariableType::Generic;
    if generic_peer {
        let enforced = enforces_generic_value_type(source) || enforces_generic_value_type(target);
        if enforced && source.value_type != target.value_type {
            return false;
        }
    } else if source.data_type != target.data_type || source.value_type != target.value_type {
        return false;
    }
    if source.data_type == VariableType::Geometry || target.data_type == VariableType::Geometry {
        return geometry_pins_are_compatible(source, target, &board.refs).unwrap_or(false);
    }
    generic_peer || schemas_are_compatible(source.schema.as_deref(), target.schema.as_deref())
}

#[cfg(test)]
mod tests {
    use super::{get::GetVariable, set::SetVariable};
    use flow_like::flow::{
        board::Board,
        node::{Node, NodeLogic},
        pin::{PinOptions, ValueType},
        variable::{Variable, VariableType},
    };
    use flow_like_types::json::json;

    fn board_with(variable: &Variable) -> Board {
        let mut board = Board::new_detached(Some("variable-retype".into()), Default::default());
        board
            .variables
            .insert(variable.id.clone(), variable.clone());
        board
    }

    fn peer(name: &str, output: bool, data_type: VariableType, value_type: ValueType) -> Node {
        let mut node = Node::new(name, name, "", "Test");
        let pin = if output {
            node.add_output_pin("value", "Value", "", data_type)
        } else {
            node.add_input_pin("value", "Value", "", data_type)
        };
        pin.value_type = value_type;
        node
    }

    fn pin_id(node: &Node, name: &str) -> String {
        node.get_pin_by_name(name).unwrap().id.clone()
    }

    fn bound(logic: &dyn NodeLogic, variable: &Variable) -> Node {
        let mut node = logic.get_node();
        node.get_pin_mut_by_name("var_ref")
            .unwrap()
            .set_default_value(Some(json!(variable.id)));
        node
    }

    async fn setter_sources_after_update(source: Node) -> Vec<String> {
        let variable = Variable::new("quizLanguage", VariableType::String, ValueType::Normal);
        let mut board = board_with(&variable);
        let mut setter = bound(&SetVariable::new(), &variable);
        let source_pin = pin_id(&source, "value");
        let target_pin = pin_id(&setter, "value_in");
        setter
            .get_pin_mut_by_name("value_in")
            .unwrap()
            .depends_on
            .insert(source_pin.clone());
        let mut source = source;
        source
            .get_pin_mut_by_name("value")
            .unwrap()
            .connected_to
            .insert(target_pin);
        board.nodes.insert(source.id.clone(), source);

        SetVariable::new().on_update(&mut setter, &board).await;

        let value_in = setter.get_pin_by_name("value_in").unwrap();
        assert_eq!(value_in.data_type, VariableType::String);
        value_in.depends_on.iter().cloned().collect()
    }

    async fn getter_targets_after_update(target: Node) -> Vec<String> {
        let variable = Variable::new("quizLanguage", VariableType::String, ValueType::Normal);
        let mut board = board_with(&variable);
        let mut getter = bound(&GetVariable::new(), &variable);
        let target_pin = pin_id(&target, "value");
        let source_pin = pin_id(&getter, "value_ref");
        getter
            .get_pin_mut_by_name("value_ref")
            .unwrap()
            .connected_to
            .insert(target_pin.clone());
        let mut target = target;
        target
            .get_pin_mut_by_name("value")
            .unwrap()
            .depends_on
            .insert(source_pin);
        board.nodes.insert(target.id.clone(), target);

        GetVariable::new().on_update(&mut getter, &board).await;

        let value_ref = getter.get_pin_by_name("value_ref").unwrap();
        assert_eq!(value_ref.data_type, VariableType::String);
        value_ref.connected_to.iter().cloned().collect()
    }

    #[tokio::test]
    async fn setter_typed_in_the_same_batch_keeps_a_generic_source() {
        let source = peer("struct_get", true, VariableType::Generic, ValueType::Normal);
        let source_pin = pin_id(&source, "value");
        assert_eq!(setter_sources_after_update(source).await, vec![source_pin]);
    }

    #[tokio::test]
    async fn setter_still_drops_a_concrete_source_of_another_type() {
        let source = peer("int_source", true, VariableType::Integer, ValueType::Normal);
        assert!(setter_sources_after_update(source).await.is_empty());
    }

    #[tokio::test]
    async fn setter_drops_a_generic_source_that_enforces_another_container() {
        let mut source = peer(
            "array_source",
            true,
            VariableType::Generic,
            ValueType::Array,
        );
        source.get_pin_mut_by_name("value").unwrap().options = Some(PinOptions {
            enforce_generic_value_type: Some(true),
            ..PinOptions::default()
        });
        assert!(setter_sources_after_update(source).await.is_empty());
    }

    #[tokio::test]
    async fn getter_typed_in_the_same_batch_keeps_a_generic_consumer() {
        let target = peer(
            "generic_consumer",
            false,
            VariableType::Generic,
            ValueType::Normal,
        );
        let target_pin = pin_id(&target, "value");
        assert_eq!(getter_targets_after_update(target).await, vec![target_pin]);
    }

    #[tokio::test]
    async fn getter_still_drops_a_concrete_consumer_of_another_type() {
        let target = peer(
            "int_consumer",
            false,
            VariableType::Integer,
            ValueType::Normal,
        );
        assert!(getter_targets_after_update(target).await.is_empty());
    }
}
