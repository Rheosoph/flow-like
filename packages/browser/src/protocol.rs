// Derived from agent-browser cli/src/native/cdp/client.rs @d01253d, Copyright 2025 Vercel Inc., Apache-2.0; modified by Rheosoph GmbH. See NOTICE.
use std::borrow::Cow;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

use crate::types::SessionId;

const PARSE_ERROR: i64 = -32700;

pub fn decode<T: serde::de::DeserializeOwned>(
    method: &str,
    value: serde_json::Value,
) -> crate::Result<T> {
    serde_json::from_value(value).map_err(|error| {
        tracing::debug!(method, %error, "typed decode failed");
        crate::BrowserError::protocol(
            method,
            PARSE_ERROR,
            format!("unexpected {method} payload: {error}"),
        )
    })
}

pub fn sanitize_lone_surrogates(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut repaired: Option<String> = None;
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }
        let Some(code) = unicode_escape(bytes, index) else {
            index += 2;
            continue;
        };
        let is_pair = (0xD800..=0xDBFF).contains(&code)
            && unicode_escape(bytes, index + 6).is_some_and(|low| (0xDC00..=0xDFFF).contains(&low));
        if is_pair {
            index += 12;
        } else if (0xD800..=0xDFFF).contains(&code) {
            let out = repaired.get_or_insert_with(|| String::with_capacity(text.len()));
            out.push_str(&text[copied..index]);
            out.push_str("\\ufffd");
            index += 6;
            copied = index;
        } else {
            index += 6;
        }
    }
    repaired.map(|mut out| {
        out.push_str(&text[copied..]);
        out
    })
}

fn unicode_escape(bytes: &[u8], at: usize) -> Option<u32> {
    if bytes.get(at) != Some(&b'\\') || bytes.get(at + 1) != Some(&b'u') {
        return None;
    }
    let digits = bytes.get(at + 2..at + 6)?;
    if !digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
}

pub(crate) enum InboundMessage {
    Response {
        id: u64,
        #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
        session: Option<SessionId>,
        result: Result<serde_json::Value, ProtocolErrorBody>,
    },
    Event {
        method: std::sync::Arc<str>,
        session: Option<SessionId>,
        params: serde_json::Value,
    },
    Malformed {
        id: Option<u64>,
        detail: String,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProtocolErrorBody {
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default, deserialize_with = "lenient_data")]
    pub data: Option<String>,
}

fn lenient_data<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text),
        Some(other) => Some(other.to_string()),
    })
}

#[derive(Deserialize)]
struct RawEnvelope<'a> {
    id: Option<u64>,
    #[serde(borrow)]
    method: Option<Cow<'a, str>>,
    #[serde(rename = "sessionId", borrow)]
    session_id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    result: Option<&'a RawValue>,
    #[serde(borrow)]
    params: Option<&'a RawValue>,
    error: Option<ProtocolErrorBody>,
}

pub(crate) fn parse_inbound(text: &str) -> InboundMessage {
    match serde_json::from_str::<RawEnvelope<'_>>(text) {
        Ok(envelope) => from_envelope(envelope),
        Err(error) => match sanitize_lone_surrogates(text) {
            Some(repaired) => match serde_json::from_str::<RawEnvelope<'_>>(&repaired) {
                Ok(envelope) => from_envelope(envelope),
                Err(error) => malformed(text.as_bytes(), error.to_string()),
            },
            None => malformed(text.as_bytes(), error.to_string()),
        },
    }
}

fn malformed(bytes: &[u8], detail: String) -> InboundMessage {
    InboundMessage::Malformed {
        id: extract_command_id(bytes),
        detail,
    }
}

fn from_envelope(envelope: RawEnvelope<'_>) -> InboundMessage {
    let session = envelope.session_id.map(|id| SessionId::new(id.as_ref()));
    if let Some(id) = envelope.id {
        let result = match (envelope.error, envelope.result) {
            (Some(error), _) => Err(error),
            (None, Some(raw)) => match decode_payload(raw) {
                Ok(value) => Ok(value),
                Err(detail) => {
                    return InboundMessage::Malformed {
                        id: Some(id),
                        detail: format!("reply {id}: {detail}"),
                    };
                }
            },
            (None, None) => Ok(Value::Null),
        };
        return InboundMessage::Response {
            id,
            session,
            result,
        };
    }
    let Some(method) = envelope.method else {
        return InboundMessage::Malformed {
            id: None,
            detail: "message has neither an id nor a method".to_owned(),
        };
    };
    let params = match envelope.params.map(decode_payload) {
        None => Value::Object(Default::default()),
        Some(Ok(value)) => value,
        Some(Err(detail)) => {
            return InboundMessage::Malformed {
                id: None,
                detail: format!("{method}: {detail}"),
            };
        }
    };
    InboundMessage::Event {
        method: Arc::from(method.as_ref()),
        session,
        params,
    }
}

fn decode_payload(raw: &RawValue) -> Result<Value, String> {
    match serde_json::from_str::<Value>(raw.get()) {
        Ok(value) => Ok(value),
        Err(error) => match sanitize_lone_surrogates(raw.get()) {
            Some(repaired) => {
                serde_json::from_str::<Value>(&repaired).map_err(|error| error.to_string())
            }
            None => Err(error.to_string()),
        },
    }
}

pub(crate) fn parse_invalid_utf8(bytes: &[u8]) -> InboundMessage {
    InboundMessage::Malformed {
        id: extract_command_id(bytes),
        detail: format!("a {}-byte binary frame is not valid UTF-8", bytes.len()),
    }
}

pub(crate) fn extract_command_id(bytes: &[u8]) -> Option<u64> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            match byte {
                b'\\' => index += 1,
                b'"' => in_string = false,
                _ => {}
            }
            index += 1;
            continue;
        }
        match byte {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            b'"' => {
                if depth == 1
                    && let Some(id) = id_after_key(bytes, index)
                {
                    return Some(id);
                }
                in_string = true;
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn id_after_key(bytes: &[u8], quote: usize) -> Option<u64> {
    let rest = bytes.get(quote..)?.strip_prefix(b"\"id\"")?;
    let rest = skip_whitespace(rest).strip_prefix(b":")?;
    let rest = skip_whitespace(rest);
    let digits = rest.iter().take_while(|byte| byte.is_ascii_digit()).count();
    let id: u64 = std::str::from_utf8(&rest[..digits]).ok()?.parse().ok()?;
    (id > 0).then_some(id)
}

fn skip_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    &bytes[start..]
}

pub(crate) fn encode_command(
    id: u64,
    method: &str,
    params: &serde_json::Value,
    session: Option<&SessionId>,
) -> String {
    let params = match params {
        Value::Null => Cow::Owned(Value::Object(Default::default())),
        other => Cow::Borrowed(other),
    };
    let method = Value::from(method);
    match session {
        Some(session) => {
            let session = Value::from(session.as_str());
            format!(r#"{{"id":{id},"method":{method},"params":{params},"sessionId":{session}}}"#)
        }
        None => format!(r#"{{"id":{id},"method":{method},"params":{params}}}"#),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn responses_and_events_are_told_apart() {
        match parse_inbound(r#"{"id":7,"sessionId":"S1","result":{"ok":true}}"#) {
            InboundMessage::Response {
                id: 7,
                session: Some(session),
                result: Ok(result),
            } => {
                assert_eq!(session.as_str(), "S1");
                assert_eq!(result, json!({"ok": true}));
            }
            _ => panic!("expected a response"),
        }
        match parse_inbound(r#"{"method":"Page.loadEventFired","params":{"timestamp":1}}"#) {
            InboundMessage::Event {
                method,
                session: None,
                params,
            } => {
                assert_eq!(&*method, "Page.loadEventFired");
                assert_eq!(params["timestamp"], 1);
            }
            _ => panic!("expected an event"),
        }
        assert!(matches!(
            parse_inbound(r#"{"id":3,"error":{"code":-32601,"message":"not found"}}"#),
            InboundMessage::Response {
                id: 3,
                result: Err(ProtocolErrorBody { code: -32601, .. }),
                ..
            }
        ));
    }

    #[test]
    fn lone_surrogates_are_repaired() {
        assert_eq!(sanitize_lone_surrogates(r#"{"a":"x"}"#), None);
        assert_eq!(
            sanitize_lone_surrogates(r#"{"a":"\ud83d"}"#).as_deref(),
            Some("{\"a\":\"\\ufffd\"}")
        );
        assert_eq!(sanitize_lone_surrogates("{\"a\":\"\\ud83d\\ude00\"}"), None);
        assert_eq!(sanitize_lone_surrogates(r#"{"a":"\\ud83d"}"#), None);
        match parse_inbound(r#"{"id":2,"result":{"value":"\udc00!"}}"#) {
            InboundMessage::Response {
                result: Ok(result), ..
            } => {
                assert_eq!(result["value"], "\u{fffd}!");
            }
            _ => panic!("expected a repaired response"),
        }
    }

    #[test]
    fn command_ids_are_found_at_the_top_level_only() {
        assert_eq!(
            extract_command_id(br#"{"result":{"id":5},"id":12}"#),
            Some(12)
        );
        assert_eq!(extract_command_id(br#"{"method":"id","params":{}}"#), None);
        assert_eq!(
            extract_command_id(b"{\"x\":\"\\\"id\\\":4\",\"id\" : 9, \xff}"),
            Some(9)
        );
        assert_eq!(extract_command_id(br#"{"id":0}"#), None);
        assert!(matches!(
            parse_invalid_utf8(b"{\"id\":4,\"result\":\"\xff\"}"),
            InboundMessage::Malformed { id: Some(4), .. }
        ));
    }

    #[test]
    fn repairs_only_unpaired_surrogate_escapes() {
        let msg = r#"{"id":1,"result":{"high":"\ud800","low":"\udfff","pair":"😀","escaped":"\\ud800","text":"café 中文"}}"#;
        let repaired = sanitize_lone_surrogates(msg).expect("lone surrogates should be repaired");
        let parsed: Value = serde_json::from_str(&repaired).expect("repaired message");
        let result = &parsed["result"];
        assert_eq!(result["high"], "\u{fffd}");
        assert_eq!(result["low"], "\u{fffd}");
        assert_eq!(result["pair"], "😀");
        assert_eq!(result["escaped"], r"\ud800");
        assert_eq!(result["text"], "café 中文");
        match parse_inbound(msg) {
            InboundMessage::Response {
                id: 1,
                result: Ok(parsed),
                ..
            } => assert_eq!(&parsed, result),
            _ => panic!("expected the repaired response"),
        }
        assert_eq!(
            sanitize_lone_surrogates(r#"{"a":"\uZZZZ\ud800"}"#).as_deref(),
            Some("{\"a\":\"\\uZZZZ\\ufffd\"}")
        );
    }

    #[test]
    fn extracts_only_positive_top_level_command_ids() {
        let id = |text: &str| extract_command_id(text.as_bytes());
        assert_eq!(id(r#"{"id":7,"result":{"value":"\ud800"}}"#), Some(7));
        assert_eq!(id(r#"{"result":{"id":42,"value":"\ud800"}}"#), None);
        assert_eq!(id(r#"{"id":-1,"result":{"value":"\ud800"}}"#), None);
        assert_eq!(
            id(r#"{"method":"Fetch.requestPaused","params":{"value":"\ud800"}}"#),
            None
        );
        assert_eq!(id(r#"{"id":0,"result":{}}"#), None);
        assert_eq!(id(r#"{"params":[{"id":3}],"x":"}{","id":8}"#), Some(8));
    }

    #[test]
    fn surrogates_are_repaired_in_events_errors_and_session_ids() {
        let event = r#"{"method":"Fetch.requestPaused","params":{"requestId":"r1","request":{"url":"https://example.test/\udfff"}},"sessionId":"s1"}"#;
        match parse_inbound(event) {
            InboundMessage::Event {
                method,
                session: Some(session),
                params,
            } => {
                assert_eq!(&*method, "Fetch.requestPaused");
                assert_eq!(session.as_str(), "s1");
                assert_eq!(params["request"]["url"], "https://example.test/\u{fffd}");
            }
            _ => panic!("expected the repaired event"),
        }
        match parse_inbound(r#"{"id":5,"error":{"code":-32000,"message":"bad \ud800 text"}}"#) {
            InboundMessage::Response {
                id: 5,
                result: Err(body),
                ..
            } => assert_eq!(body.message, "bad \u{fffd} text"),
            _ => panic!("expected the repaired error reply"),
        }
        match parse_inbound(
            r#"{"method":"Runtime.consoleAPICalled","sessionId":"S\ud83d","params":{"args":[{"value":"C\ud83d"}]}}"#,
        ) {
            InboundMessage::Event {
                session: Some(session),
                params,
                ..
            } => {
                assert_eq!(session.as_str(), "S\u{fffd}");
                assert_eq!(params["args"][0]["value"], "C\u{fffd}");
            }
            _ => panic!("expected the repaired console event"),
        }
    }

    #[test]
    fn broken_messages_are_malformed_with_the_top_level_id() {
        let malformed_id = |text: &str| match parse_inbound(text) {
            InboundMessage::Malformed { id, detail } => {
                assert!(!detail.is_empty(), "{text}");
                id
            }
            _ => panic!("expected {text} to be malformed"),
        };
        assert_eq!(
            malformed_id(r#"{"id":9,"result":{"value":"\ud800"},"method":42}"#),
            Some(9)
        );
        assert_eq!(
            malformed_id(r#"{"result":{"id":9,"value":"\ud800"},"method":42}"#),
            None
        );
        assert_eq!(malformed_id(r#"{"id":11,"result":{"a":"#), Some(11));
        assert_eq!(malformed_id(r#"{"sessionId":"S1"}"#), None);
        assert_eq!(malformed_id("[1,2]"), None);
        assert_eq!(malformed_id(""), None);
    }

    #[test]
    fn error_bodies_are_lenient() {
        let body = |text: &str| match parse_inbound(text) {
            InboundMessage::Response {
                result: Err(body), ..
            } => body,
            _ => panic!("expected an error reply in {text}"),
        };
        let invalid = body(
            r#"{"id":1,"error":{"code":-32602,"message":"Invalid parameters","data":"Failed to deserialize params.targetId"}}"#,
        );
        assert_eq!(invalid.code, -32602);
        assert_eq!(
            invalid.data.as_deref(),
            Some("Failed to deserialize params.targetId")
        );
        let structured = body(r#"{"id":2,"error":{"code":-32000,"message":"x","data":{"k":1}}}"#);
        assert_eq!(structured.data.as_deref(), Some(r#"{"k":1}"#));
        let bare = body(r#"{"id":3,"error":{"data":null}}"#);
        assert_eq!((bare.code, bare.message.as_str(), bare.data), (0, "", None));
    }

    #[test]
    fn typed_decode_failures_name_the_method() {
        let error = decode::<crate::types::TargetInfo>("Target.getTargetInfo", json!("page"))
            .expect_err("a string is not a TargetInfo");
        assert!(matches!(
            &error,
            crate::BrowserError::Protocol { method, code: -32700, message }
                if method == "Target.getTargetInfo" && message.contains("unexpected Target.getTargetInfo payload")
        ));
        let info: crate::types::TargetInfo = decode(
            "Target.getTargetInfo",
            json!({"targetId": "T1", "type": "future_kind", "extra": true}),
        )
        .unwrap();
        assert_eq!(info.target_id.as_str(), "T1");
        assert_eq!(info.type_.as_str(), "future_kind");
    }

    #[test]
    fn commands_encode_with_optional_sessions() {
        let root = encode_command(1, "Browser.getVersion", &Value::Null, None);
        assert_eq!(
            serde_json::from_str::<Value>(&root).unwrap(),
            json!({"id": 1, "method": "Browser.getVersion", "params": {}})
        );
        let session = SessionId::from("S\"1");
        let scoped = encode_command(2, "Page.enable", &json!({"a": 1}), Some(&session));
        assert_eq!(
            serde_json::from_str::<Value>(&scoped).unwrap(),
            json!({"id": 2, "method": "Page.enable", "params": {"a": 1}, "sessionId": "S\"1"})
        );
    }
}
