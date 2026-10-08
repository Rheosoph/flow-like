#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_industrial::sparkplug::{
    EdgeAction, EdgeState, ObservedNode, Payload, PrepareSession, PreparedSession, Transition,
    WireMessage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[crate::register_node]
#[derive(Default)]
pub struct SparkplugEncodeNode;
crate::operation!(
    SparkplugEncodeNode,
    "sparkplug_encode",
    "Encode Sparkplug B",
    "Encode a Sparkplug protobuf payload, or a host STATE JSON payload, for MQTT",
    "sparkplug",
    "encode",
    "Industrial/Sparkplug",
    Payload,
    WireMessage,
    encode
);

#[crate::register_node]
#[derive(Default)]
pub struct SparkplugDecodeNode;
crate::operation!(
    SparkplugDecodeNode,
    "sparkplug_decode",
    "Decode Sparkplug B",
    "Decode MQTT bytes into the Sparkplug protobuf JSON representation",
    "sparkplug",
    "decode",
    "Industrial/Sparkplug",
    WireMessage,
    Payload,
    decode
);

#[crate::register_node]
#[derive(Default)]
pub struct SparkplugPrepareSessionNode;
crate::operation!(
    SparkplugPrepareSessionNode,
    "sparkplug_prepare_session",
    "Prepare Sparkplug Session",
    "Advance persisted bdSeq and prepare the MQTT Last Will; install it before connecting with a clean session",
    "sparkplug",
    "prepareSession",
    "Industrial/Sparkplug",
    PrepareSession,
    PreparedSession,
    prepare
);

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct TransitionRequest {
    pub state: EdgeState,
    pub action: EdgeAction,
    pub timestamp_ms: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct SparkplugTransitionNode;
crate::operation!(
    SparkplugTransitionNode,
    "sparkplug_transition",
    "Sparkplug State Transition",
    "Build birth, data, death, or rebirth messages with one shared sequence; publish the returned messages in order",
    "sparkplug",
    "transition",
    "Industrial/Sparkplug",
    TransitionRequest,
    Transition,
    transition
);

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ObserveRequest {
    pub state: ObservedNode,
    pub message: Payload,
}

#[crate::register_node]
#[derive(Default)]
pub struct SparkplugObserveNode;
crate::operation!(
    SparkplugObserveNode,
    "sparkplug_observe",
    "Observe Sparkplug State",
    "Track one edge node's online state and detect sequence gaps that require rebirth",
    "sparkplug",
    "observe",
    "Industrial/Sparkplug",
    ObserveRequest,
    ObservedNode,
    observe
);

#[cfg(feature = "execute")]
async fn encode(_: &mut ExecutionContext, input: Payload) -> flow_like_types::Result<WireMessage> {
    Ok(flow_like_industrial::sparkplug::encode(&input)?)
}
#[cfg(feature = "execute")]
async fn decode(_: &mut ExecutionContext, input: WireMessage) -> flow_like_types::Result<Payload> {
    Ok(flow_like_industrial::sparkplug::decode(&input)?)
}
#[cfg(feature = "execute")]
async fn prepare(
    _: &mut ExecutionContext,
    input: PrepareSession,
) -> flow_like_types::Result<PreparedSession> {
    Ok(EdgeState::prepare(input)?)
}
#[cfg(feature = "execute")]
async fn transition(
    _: &mut ExecutionContext,
    input: TransitionRequest,
) -> flow_like_types::Result<Transition> {
    Ok(input.state.transition(input.action, input.timestamp_ms)?)
}
#[cfg(feature = "execute")]
async fn observe(
    _: &mut ExecutionContext,
    input: ObserveRequest,
) -> flow_like_types::Result<ObservedNode> {
    Ok(input.state.apply(&input.message)?)
}
