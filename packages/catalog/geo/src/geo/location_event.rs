use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{
    Value, async_trait,
    geometry::{GeometryKind, marker, validate_geometry},
};

fn validate_transition(payload: &Value) -> flow_like_types::Result<()> {
    use flow_like_types::bail;
    if !matches!(payload["transition"].as_str(), Some("enter" | "exit")) {
        bail!("A location Event requires an enter or exit transition");
    }
    for field in ["id", "registrationId", "appId", "eventId"] {
        if !payload[field]
            .as_str()
            .is_some_and(|value| !value.is_empty() && value.len() <= 512)
        {
            bail!("A location Event requires a valid {field}");
        }
    }
    validate_geometry(&payload["geometry"], Some(GeometryKind::Point))?;
    for field in ["occurredAt", "radiusMeters"] {
        if !payload[field]
            .as_f64()
            .is_some_and(|value| value.is_finite() && value > 0.0)
        {
            bail!("A location Event requires a positive {field}");
        }
    }
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct LocationEventNode;

#[async_trait]
impl NodeLogic for LocationEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_location",
            "Location Event",
            "Starts when the device enters or leaves a configured circular region. Configure foreground or background monitoring in the Event settings.",
            "Events",
        );
        node.set_flowscript_name("events", "location");
        node.add_icon("/flow/icons/map.svg");
        node.set_start(true);
        node.add_output_pin(
            "exec_out",
            "Output",
            "A monitored region transition occurred",
            VariableType::Execution,
        );
        node.add_output_pin("region", "Region center", "Center of the monitored circle as a WGS 84 Point. This is not a measured device position", VariableType::Geometry)
            .schema = Some(marker(GeometryKind::Point).to_string());
        node.add_output_pin(
            "radius_meters",
            "Radius",
            "Radius of the monitored circle in meters",
            VariableType::Float,
        );
        node.add_output_pin(
            "transition",
            "Transition",
            "enter or exit",
            VariableType::String,
        );
        node.add_output_pin(
            "timestamp",
            "Timestamp",
            "Time the native system delivered the transition, in Unix milliseconds",
            VariableType::Float,
        );
        node.add_output_pin(
            "transition_id",
            "Transition ID",
            "Stable identifier for deduplicating retried transition deliveries",
            VariableType::String,
        );
        node.add_output_pin(
            "payload",
            "Payload",
            "Native region transition and its Event identifiers",
            VariableType::Struct,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let payload = context
            .get_payload()
            .await?
            .payload
            .clone()
            .ok_or_else(|| {
                flow_like_types::anyhow!("Location Events require a region transition payload")
            })?;
        validate_transition(&payload)?;
        context
            .set_pin_value("region", payload["geometry"].clone())
            .await?;
        context
            .set_pin_value("radius_meters", payload["radiusMeters"].clone())
            .await?;
        context
            .set_pin_value("transition", payload["transition"].clone())
            .await?;
        context
            .set_pin_value("timestamp", payload["occurredAt"].clone())
            .await?;
        context
            .set_pin_value("transition_id", payload["id"].clone())
            .await?;
        context.set_pin_value("payload", payload).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    fn transition() -> Value {
        json!({"id":"transition-1","registrationId":"registration-1","appId":"app-1","eventId":"event-1",
            "geometry":{"type":"Point","coordinates":[13.4,52.5]},"radiusMeters":200,
            "transition":"enter","occurredAt":1_800_000_000_000_u64})
    }

    #[test]
    fn region_event_accepts_both_transitions_with_geometry_center() {
        let mut value = transition();
        validate_transition(&value).unwrap();
        value["transition"] = json!("exit");
        validate_transition(&value).unwrap();
    }

    #[test]
    fn region_event_rejects_missing_delivery_identity_and_invalid_geometry() {
        for field in [
            "id",
            "registrationId",
            "appId",
            "eventId",
            "geometry",
            "occurredAt",
            "radiusMeters",
        ] {
            let mut value = transition();
            value.as_object_mut().unwrap().remove(field);
            assert!(validate_transition(&value).is_err(), "{field}");
        }
        for geometry in [
            json!({"type":"Point","coordinates":[181,52]}),
            json!({"type":"Point","coordinates":[13,52,100]}),
        ] {
            let mut value = transition();
            value["geometry"] = geometry;
            assert!(validate_transition(&value).is_err());
        }
    }

    #[test]
    fn region_event_does_not_accept_unknown_transitions_or_negative_radius() {
        let mut value = transition();
        value["transition"] = json!("unknown");
        assert!(validate_transition(&value).is_err());
        value = transition();
        value["radiusMeters"] = json!(-1);
        assert!(validate_transition(&value).is_err());
    }

    #[test]
    fn event_can_run_remotely_and_exposes_a_typed_region_point() {
        let node = LocationEventNode.get_node();
        assert!(!node.only_offline);
        let region = node.pins.values().find(|pin| pin.name == "region").unwrap();
        assert_eq!(region.data_type, VariableType::Geometry);
        assert_eq!(region.schema.as_deref(), Some(marker(GeometryKind::Point)));
    }
}
