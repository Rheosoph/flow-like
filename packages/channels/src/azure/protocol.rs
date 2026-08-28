//! Frames of the `json.webpubsub.azure.v1` subprotocol and the [`ChannelPush`] decoding of
//! a received group message.

use flow_like_types::Value;
use flow_like_types::channel::ChannelPush;
use serde::{Deserialize, Serialize};

pub const SUBPROTOCOL: &str = "json.webpubsub.azure.v1";

#[derive(Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ClientFrame<'a> {
    JoinGroup { group: &'a str, ack_id: u64 },
    LeaveGroup { group: &'a str, ack_id: u64 },
    Ping,
}

impl ClientFrame<'_> {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::JoinGroup { .. } => "joinGroup",
            Self::LeaveGroup { .. } => "leaveGroup",
            Self::Ping => "ping",
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct AckError {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ServerFrame {
    Ack {
        ack_id: u64,
        success: bool,
        #[serde(default)]
        error: Option<AckError>,
    },
    Message {
        #[serde(default)]
        from: String,
        #[serde(default)]
        data_type: String,
        #[serde(default)]
        data: Value,
        #[serde(default)]
        from_user_id: Option<String>,
    },
    System {
        event: String,
        #[serde(default)]
        connection_id: Option<String>,
        #[serde(default)]
        user_id: Option<String>,
        #[serde(default)]
        message: Option<String>,
    },
    Pong,
    #[serde(other)]
    Unknown,
}

impl ServerFrame {
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}

/// `dataType: json` carries the push as an object, `text` as a JSON string; `binary` is not
/// a shape any client of ours produces.
pub(crate) fn push_from_message(data_type: &str, data: Value) -> Result<ChannelPush, String> {
    match data_type {
        "text" => match data {
            Value::String(text) => serde_json::from_str(&text)
                .map_err(|e| format!("text payload is not a ChannelPush: {e}")),
            _ => Err("text payload is not a string".to_string()),
        },
        "binary" => Err("binary payloads are not supported".to_string()),
        _ => serde_json::from_value(data)
            .map_err(|e| format!("json payload is not a ChannelPush: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_frames_serialize_to_subprotocol_shape() {
        let join = serde_json::to_value(ClientFrame::JoinGroup {
            group: "run:a",
            ack_id: 1,
        })
        .unwrap();
        assert_eq!(
            join,
            serde_json::json!({"type": "joinGroup", "group": "run:a", "ackId": 1})
        );
        assert_eq!(
            serde_json::to_value(ClientFrame::Ping).unwrap(),
            serde_json::json!({"type": "ping"})
        );
    }

    #[test]
    fn server_frames_parse() {
        assert!(matches!(
            ServerFrame::parse(r#"{"type":"ack","ackId":1,"success":true}"#).unwrap(),
            ServerFrame::Ack {
                ack_id: 1,
                success: true,
                error: None
            }
        ));
        assert!(matches!(
            ServerFrame::parse(
                r#"{"type":"system","event":"connected","userId":"u","connectionId":"c"}"#
            )
            .unwrap(),
            ServerFrame::System { .. }
        ));
        assert!(matches!(
            ServerFrame::parse(r#"{"type":"pong"}"#).unwrap(),
            ServerFrame::Pong
        ));
        assert!(matches!(
            ServerFrame::parse(r#"{"type":"somethingNew","x":1}"#).unwrap(),
            ServerFrame::Unknown
        ));
    }

    #[test]
    fn text_payload_is_parsed_as_json_string() {
        let push = push_from_message(
            "text",
            Value::from(r#"{"channel_id":"c","request_id":"r","value":5}"#),
        )
        .unwrap();
        assert_eq!(push.request_id.as_deref(), Some("r"));
        assert_eq!(push.value, Value::from(5));
        assert!(push_from_message("binary", Value::from("AAAA")).is_err());
        assert!(push_from_message("text", Value::from(1)).is_err());
    }
}
