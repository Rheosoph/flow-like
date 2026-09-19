use super::{
    association::{AssociationConfig, Associator},
    registry::{Eviction, RegistryLimits, StateRegistry, app_key, now_ms},
    types::{AppearanceObservation, EntityAssociation, UnresolvedObservation},
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    Result, anyhow, async_trait,
    json::{from_value, json},
};
use std::{sync::LazyLock, time::Duration};

static ASSOCIATORS: LazyLock<StateRegistry<Associator>> = LazyLock::new(|| {
    StateRegistry::new(
        "Entity association",
        RegistryLimits {
            idle_ttl: Duration::from_secs(60 * 60),
            capacity: 64,
            app_capacity: 16,
            eviction: Eviction::Never,
        },
    )
});

#[crate::register_node]
#[derive(Default)]
pub struct AssociateEntitiesNode {}

impl AssociateEntitiesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AssociateEntitiesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "tracking_associate_entities",
            "Associate Entities",
            "Gives local tracks from one or more cameras a global identity by comparing appearance embeddings. A track is collected for a few observations before it creates a new entity; later tracks that look like a known entity are matched to it, even on another camera. Every board and user of this app using the same task id shares the identities. They live in this process's memory only: after an hour without calls or a restart the task starts over and entity ids begin again at 1. An app can have at most 16 active tasks (64 per process); an idle task is released after an hour.",
            "AI/ML/Tracking",
        );
        node.set_flowscript_name("tracking", "associateEntities");
        node.set_receiver("observations");
        node.add_icon("/flow/icons/chart-network.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "observations",
            "Observations",
            "Observations with appearance embeddings, e.g. from Extract Appearance. Observations with a track id accumulate evidence per track; ones without are only matched to known entities, and lost tracks only report the entity they already have. Timestamps are Unix milliseconds from one clock shared by all cameras of the task; a missing timestamp means now, and one further ahead of this machine's clock than a minute (or half the entity lifetime, if shorter) is clamped.",
            VariableType::Struct,
        )
        .set_schema::<AppearanceObservation>()
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "task_id",
            "Task Id",
            "Identities are shared by every board, user and camera of this app using the same task id",
            VariableType::String,
        )
        .set_default_value(Some(json!("default")));

        node.add_input_pin(
            "min_similarity",
            "Min Similarity",
            "Lowest cosine similarity for matching an observation to an entity",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0., 1.)).build())
        .set_default_value(Some(json!(0.4)));

        node.add_input_pin(
            "ambiguity_margin",
            "Ambiguity Margin",
            "A match is left unresolved when the next best entity is at most this much less similar",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0., 1.)).build())
        .set_default_value(Some(json!(0.05)));

        node.add_input_pin(
            "min_observations",
            "Min Observations",
            "Observations a new track needs before it is matched or creates an entity (at least 1)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3)));

        node.add_input_pin(
            "entity_ttl_ms",
            "Entity Lifetime (ms)",
            "Entities, track bindings and pending tracks unseen for this long, measured by observation timestamps, are forgotten",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(600_000)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "associations",
            "Associations",
            "Observations with their global entity id, in input order",
            VariableType::Struct,
        )
        .set_schema::<EntityAssociation>()
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_output_pin(
            "unresolved",
            "Unresolved",
            "Observations without an entity yet, with the reason and the closest entities",
            VariableType::Struct,
        )
        .set_schema::<UnresolvedObservation>()
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_output_pin(
            "entity_count",
            "Entity Count",
            "Number of entities currently known to this task",
            VariableType::Integer,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(7)
                .set_performance(8)
                .set_governance(4)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let observations: Vec<AppearanceObservation> =
            from_value(context.evaluate_pin_to_ref("observations").await?).map_err(|error| {
                anyhow!("Pin 'observations' must be a list of observations: {error}")
            })?;
        let task_id: String = context.evaluate_pin("task_id").await?;
        let config = AssociationConfig::new(
            context.evaluate_pin("min_similarity").await?,
            context.evaluate_pin("ambiguity_margin").await?,
            context.evaluate_pin("min_observations").await?,
            context.evaluate_pin("entity_ttl_ms").await?,
        )?;

        let key = app_key(context, &["associate", &task_id]);
        let output = ASSOCIATORS.with_state(&key, Associator::new, |associator| {
            associator.associate(&observations, &config, now_ms())
        })??;
        if output.clamped_timestamps > 0 {
            context.log_message(
                &format!(
                    "Clamped {} observation timestamps that were more than {} ms ahead of this machine's clock. Timestamps must be Unix milliseconds, and all cameras of task '{task_id}' must use one clock",
                    output.clamped_timestamps,
                    config.max_clock_lead_ms()
                ),
                LogLevel::Warn,
            );
        }
        if output.late_timestamps > 0 {
            context.log_message(
                &format!(
                    "{} observations were already older than the entity lifetime ({} ms) relative to the newest timestamp of task '{task_id}', so they cannot keep an identity. All cameras of a task must use one clock",
                    output.late_timestamps, config.entity_ttl_ms
                ),
                LogLevel::Warn,
            );
        }

        context
            .set_pin_value("associations", json!(output.associations))
            .await?;
        context
            .set_pin_value("unresolved", json!(output.unresolved))
            .await?;
        context
            .set_pin_value("entity_count", json!(output.entity_count))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;

    #[test]
    fn pins_match_the_contract() {
        let node = AssociateEntitiesNode::new().get_node();
        let names = |pin_type: PinType| {
            let mut names: Vec<&str> = node
                .pins
                .values()
                .filter(|pin| pin.pin_type == pin_type)
                .map(|pin| pin.name.as_str())
                .collect();
            names.sort_unstable();
            names
        };
        assert_eq!(
            names(PinType::Input),
            [
                "ambiguity_margin",
                "entity_ttl_ms",
                "exec_in",
                "min_observations",
                "min_similarity",
                "observations",
                "task_id",
            ]
        );
        assert_eq!(
            names(PinType::Output),
            ["associations", "entity_count", "exec_out", "unresolved"]
        );
        assert_eq!(node.name, "tracking_associate_entities");
    }
}
