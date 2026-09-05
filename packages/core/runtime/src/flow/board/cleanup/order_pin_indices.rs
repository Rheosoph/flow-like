use crate::flow::{board::cleanup::BoardCleanupLogic, pin::PinType};

#[derive(Default)]
pub struct PinIndicesCleanup {}

impl BoardCleanupLogic for PinIndicesCleanup {
    fn init(_board: &mut crate::flow::board::Board) -> Self
    where
        Self: Sized,
    {
        PinIndicesCleanup {}
    }

    fn main_node_iteration(
        &mut self,
        node: &mut crate::flow::node::Node,
        _pin_lookup: &super::PinLookup,
    ) {
        let mut input_pins = node
            .pins
            .iter_mut()
            .filter(|pin| pin.1.pin_type == PinType::Input)
            .map(|pin| pin.1)
            .collect::<Vec<_>>();
        input_pins.sort_by_key(|pin| pin.index);

        for (index, pin) in input_pins.into_iter().enumerate() {
            pin.index = index as u16 + 1;
        }

        let mut output_pins = node
            .pins
            .iter_mut()
            .filter(|pin| pin.1.pin_type == PinType::Output)
            .map(|pin| pin.1)
            .collect::<Vec<_>>();
        output_pins.sort_by_key(|pin| pin.index);

        for (index, pin) in output_pins.into_iter().enumerate() {
            pin.index = index as u16 + 1;
        }
    }

    fn main_layer_iteration(
        &mut self,
        layer: &mut crate::flow::board::Layer,
        _pin_lookup: &super::PinLookup,
    ) {
        let mut input_pins = layer
            .pins
            .iter_mut()
            .filter(|pin| pin.1.pin_type == PinType::Input)
            .map(|pin| pin.1)
            .collect::<Vec<_>>();
        input_pins.sort_by_key(|pin| pin.index);

        for (index, pin) in input_pins.into_iter().enumerate() {
            pin.index = index as u16 + 1;
        }

        let mut output_pins = layer
            .pins
            .iter_mut()
            .filter(|pin| pin.1.pin_type == PinType::Output)
            .map(|pin| pin.1)
            .collect::<Vec<_>>();
        output_pins.sort_by_key(|pin| pin.index);

        for (index, pin) in output_pins.into_iter().enumerate() {
            pin.index = index as u16 + 1;
        }
    }
}
