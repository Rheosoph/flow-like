use std::time::SystemTime;

use flow_like_types::{FromProto, Timestamp, ToProto};

use crate::flow::{
    event::{
        CanaryEvent, Event, EventExecutionMode, EventExposure, EventInput, EventVariant,
        EventVariantMode, ReleaseNotes,
    },
    variable::Variable,
};

impl ToProto<flow_like_types::proto::Event> for Event {
    fn to_proto(&self) -> flow_like_types::proto::Event {
        flow_like_types::proto::Event {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            board_id: self.board_id.clone(),
            board_version: self.board_version.map(|v| flow_like_types::proto::Version {
                major: v.0,
                minor: v.1,
                patch: v.2,
            }),
            node_id: self.node_id.clone(),
            variables: self
                .variables
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            config: self.config.clone(),
            active: self.active,
            canary: self.canary.as_ref().map(|c| c.to_proto()),
            notes: self.notes.as_ref().map(|n| match n {
                ReleaseNotes::NOTES(s) => {
                    flow_like_types::proto::event::Notes::ReleaseNotes(s.clone())
                }
                ReleaseNotes::URL(s) => {
                    flow_like_types::proto::event::Notes::ReleaseNotesUrl(s.clone())
                }
            }),
            event_version: Some(flow_like_types::proto::Version {
                major: self.event_version.0,
                minor: self.event_version.1,
                patch: self.event_version.2,
            }),
            priority: self.priority,
            created_at: Some(Timestamp::from(self.created_at)),
            updated_at: Some(Timestamp::from(self.updated_at)),
            event_type: self.event_type.clone(),
            default_page_id: self.default_page_id.clone(),
            inputs: self.inputs.iter().map(|i| i.to_proto()).collect(),
            route: self.route.clone(),
            is_default: self.is_default,
            execution_mode: Some(self.execution_mode.as_str().to_string()),
            exposure: Some(self.exposure.as_str().to_string()),
            correlation_mappings: self.correlation_mappings.clone().unwrap_or_default(),
            variants: self.variants.iter().map(|v| v.to_proto()).collect(),
        }
    }
}

impl ToProto<flow_like_types::proto::EventInput> for EventInput {
    fn to_proto(&self) -> flow_like_types::proto::EventInput {
        flow_like_types::proto::EventInput {
            id: self.id.clone(),
            name: self.name.clone(),
            friendly_name: self.friendly_name.clone(),
            description: self.description.clone(),
            data_type: self.data_type.clone(),
            value_type: self.value_type.clone(),
            schema: self.schema.clone(),
            default_value: self.default_value.clone(),
            index: self.index as u32,
            optional: self.optional,
            sensitive: self.sensitive,
            valid_values: self.valid_values.clone().unwrap_or_default(),
            range_min: self.range.map(|(min, _)| min),
            range_max: self.range.map(|(_, max)| max),
            step: self.step,
            default_omitted: self.default_omitted,
            inputs_format: self.inputs_format,
        }
    }
}

impl FromProto<flow_like_types::proto::EventInput> for EventInput {
    fn from_proto(proto: flow_like_types::proto::EventInput) -> Self {
        EventInput {
            id: proto.id,
            name: proto.name,
            friendly_name: proto.friendly_name,
            description: proto.description,
            data_type: proto.data_type,
            value_type: proto.value_type,
            schema: proto.schema,
            default_value: proto.default_value,
            optional: proto.optional,
            index: proto.index as u16,
            sensitive: proto.sensitive,
            valid_values: (!proto.valid_values.is_empty()).then_some(proto.valid_values),
            range: proto.range_min.zip(proto.range_max),
            step: proto.step,
            default_omitted: proto.default_omitted,
            inputs_format: proto.inputs_format,
        }
    }
}

impl ToProto<flow_like_types::proto::Canary> for CanaryEvent {
    fn to_proto(&self) -> flow_like_types::proto::Canary {
        flow_like_types::proto::Canary {
            board_id: self.board_id.clone(),
            board_version: self.board_version.map(|v| flow_like_types::proto::Version {
                major: v.0,
                minor: v.1,
                patch: v.2,
            }),
            node_id: self.node_id.clone(),
            weight: self.weight,
            variables: self
                .variables
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            created_at: Some(Timestamp::from(self.created_at)),
            updated_at: Some(Timestamp::from(self.updated_at)),
        }
    }
}

impl ToProto<flow_like_types::proto::EventVariant> for EventVariant {
    fn to_proto(&self) -> flow_like_types::proto::EventVariant {
        flow_like_types::proto::EventVariant {
            name: self.name.clone(),
            board_id: self.board_id.clone(),
            board_version: self.board_version.map(|v| flow_like_types::proto::Version {
                major: v.0,
                minor: v.1,
                patch: v.2,
            }),
            node_id: self.node_id.clone(),
            variables: self
                .variables
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            default_page_id: self.default_page_id.clone(),
            mode: Some(match &self.mode {
                EventVariantMode::Live { weight } => {
                    flow_like_types::proto::event_variant::Mode::Live(
                        flow_like_types::proto::LiveVariantMode { weight: *weight },
                    )
                }
                EventVariantMode::Shadow { sample_rate } => {
                    flow_like_types::proto::event_variant::Mode::Shadow(
                        flow_like_types::proto::ShadowVariantMode {
                            sample_rate: *sample_rate,
                        },
                    )
                }
            }),
            created_at: Some(Timestamp::from(self.created_at)),
            updated_at: Some(Timestamp::from(self.updated_at)),
        }
    }
}

impl FromProto<flow_like_types::proto::EventVariant> for EventVariant {
    fn from_proto(proto: flow_like_types::proto::EventVariant) -> Self {
        EventVariant {
            name: proto.name,
            board_id: proto.board_id,
            board_version: proto.board_version.map(|v| (v.major, v.minor, v.patch)),
            node_id: proto.node_id,
            variables: proto
                .variables
                .into_iter()
                .map(|(k, v)| (k, Variable::from_proto(v)))
                .collect(),
            default_page_id: proto.default_page_id,
            mode: match proto.mode {
                Some(flow_like_types::proto::event_variant::Mode::Live(live)) => {
                    EventVariantMode::Live {
                        weight: live.weight,
                    }
                }
                Some(flow_like_types::proto::event_variant::Mode::Shadow(shadow)) => {
                    EventVariantMode::Shadow {
                        sample_rate: shadow.sample_rate,
                    }
                }
                // A wire variant without a mode is malformed; a zero-weight
                // Live variant is inert.
                None => EventVariantMode::Live { weight: 0.0 },
            },
            created_at: proto
                .created_at
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
            updated_at: proto
                .updated_at
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
        }
    }
}

impl FromProto<flow_like_types::proto::Event> for Event {
    fn from_proto(proto: flow_like_types::proto::Event) -> Self {
        Event {
            id: proto.id,
            name: proto.name,
            description: proto.description,
            board_id: proto.board_id,
            board_version: proto.board_version.map(|v| (v.major, v.minor, v.patch)),
            node_id: proto.node_id,
            variables: proto
                .variables
                .into_iter()
                .map(|(k, v)| (k, Variable::from_proto(v)))
                .collect(),
            active: proto.active,
            config: proto.config,
            canary: proto.canary.map(CanaryEvent::from_proto),
            notes: proto.notes.map(|n| match n {
                flow_like_types::proto::event::Notes::ReleaseNotes(s) => ReleaseNotes::NOTES(s),
                flow_like_types::proto::event::Notes::ReleaseNotesUrl(s) => ReleaseNotes::URL(s),
            }),
            event_version: {
                let version = proto.event_version.unwrap_or_default();
                (version.major, version.minor, version.patch)
            },
            priority: proto.priority,
            created_at: proto
                .created_at
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
            updated_at: proto
                .updated_at
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
            event_type: proto.event_type,
            default_page_id: proto.default_page_id,
            inputs: proto
                .inputs
                .into_iter()
                .map(EventInput::from_proto)
                .collect(),
            route: proto.route,
            is_default: proto.is_default,
            execution_mode: proto
                .execution_mode
                .as_deref()
                .map(EventExecutionMode::parse)
                .unwrap_or_default(),
            exposure: proto
                .exposure
                .as_deref()
                .map(EventExposure::parse)
                .unwrap_or_default(),
            correlation_mappings: if proto.correlation_mappings.is_empty() {
                None
            } else {
                Some(proto.correlation_mappings)
            },
            variants: proto
                .variants
                .into_iter()
                .map(EventVariant::from_proto)
                .collect(),
        }
    }
}

impl FromProto<flow_like_types::proto::Canary> for CanaryEvent {
    fn from_proto(proto: flow_like_types::proto::Canary) -> Self {
        CanaryEvent {
            board_id: proto.board_id,
            board_version: proto.board_version.map(|v| (v.major, v.minor, v.patch)),
            node_id: proto.node_id,
            weight: proto.weight,
            variables: proto
                .variables
                .into_iter()
                .map(|(k, v)| (k, Variable::from_proto(v)))
                .collect(),
            created_at: proto
                .created_at
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
            updated_at: proto
                .updated_at
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::event::EVENT_INPUTS_FORMAT;
    use flow_like_types::Message;

    fn input() -> EventInput {
        EventInput {
            id: "pin-ratio".into(),
            name: "ratio".into(),
            friendly_name: "Ratio".into(),
            description: "How much of it".into(),
            data_type: "Float".into(),
            value_type: "Normal".into(),
            schema: None,
            default_value: Some(b"1.5".to_vec()),
            optional: true,
            index: 2,
            sensitive: false,
            valid_values: Some(vec!["0.5".into(), "1.5".into()]),
            range: Some((0.0, 2.0)),
            step: Some(0.5),
            default_omitted: false,
            inputs_format: EVENT_INPUTS_FORMAT,
        }
    }

    fn through_the_wire(input: &EventInput) -> EventInput {
        let bytes = input.to_proto().encode_to_vec();
        let proto = flow_like_types::proto::EventInput::decode(bytes.as_slice())
            .expect("decode a stored event input");
        EventInput::from_proto(proto)
    }

    #[test]
    fn event_input_round_trips_through_proto() {
        let options = input();
        assert_eq!(through_the_wire(&options), options);

        let secret = EventInput {
            default_value: None,
            sensitive: true,
            default_omitted: true,
            valid_values: None,
            range: None,
            step: None,
            ..input()
        };
        assert_eq!(through_the_wire(&secret), secret);
    }

    #[test]
    fn event_input_proto_reads_empty_lists_and_half_ranges_as_absent() {
        let empty = EventInput {
            valid_values: Some(Vec::new()),
            ..input()
        };
        assert_eq!(through_the_wire(&empty).valid_values, None);

        let mut half = input().to_proto();
        half.range_max = None;
        assert_eq!(EventInput::from_proto(half).range, None);
    }

    #[test]
    fn event_input_proto_stored_before_the_format_reads_as_format_zero() {
        let stored = flow_like_types::proto::EventInput {
            id: "pin-title".into(),
            name: "title".into(),
            friendly_name: "Title".into(),
            description: "16248035215404677707".into(),
            data_type: "String".into(),
            value_type: "Normal".into(),
            index: 1,
            ..Default::default()
        };
        let input = EventInput::from_proto(stored);
        assert_eq!(input.inputs_format, 0);
        assert!(!input.sensitive && !input.default_omitted);
        assert_eq!(
            (input.valid_values, input.range, input.step),
            (None, None, None)
        );
    }
}
