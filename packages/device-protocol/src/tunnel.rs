//! Ordered, bounded streams carried inside a separately authenticated Noise session.
//! A sequence gap closes the connection; streams are never replayed after reconnect.

use crate::{ManagementRequest, ProtocolError, Result, validate_management_id};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

pub const TUNNEL_VERSION: u8 = 1;
pub const TUNNEL_HEADER_LEN: usize = 20;
pub const TUNNEL_MAX_FRAME: usize = 16 * 1024;
pub const TUNNEL_MAX_DATA: usize = TUNNEL_MAX_FRAME - TUNNEL_HEADER_LEN;
pub const TUNNEL_MAX_ENVELOPE: usize = 32 * 1024;
pub const TUNNEL_INITIAL_WINDOW: u32 = 256 * 1024;
pub const TUNNEL_MAX_STREAMS: usize = 16;
pub const TUNNEL_MAX_HANDSHAKE: usize = 128;
pub const TUNNEL_RENEW_BEFORE_SECONDS: i64 = 60;
pub const TUNNEL_RENEW_TIMEOUT_SECONDS: u64 = 20;
pub const TUNNEL_HEARTBEAT_SECONDS: u64 = 20;
pub const TUNNEL_IDLE_TIMEOUT_SECONDS: u64 = 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelOpen {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub placement_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub service_id: String,
    #[serde(default, skip_serializing_if = "TunnelMode::is_tcp")]
    pub mode: TunnelMode,
    /// Absent on the wire for `Service`, so service opens keep their v1 bytes.
    #[serde(default, skip_serializing_if = "TunnelTarget::is_service")]
    pub target: TunnelTarget,
}

impl TunnelOpen {
    pub fn validate(&self) -> Result<()> {
        match self.target {
            TunnelTarget::Service => {
                validate_management_id(&self.placement_id)?;
                validate_management_id(&self.service_id)
            }
            TunnelTarget::ModelGateway
                if self.placement_id.is_empty() && self.service_id.is_empty() =>
            {
                Ok(())
            }
            TunnelTarget::ModelGateway => Err(ProtocolError::Invalid(
                "model gateway tunnel names a placement or service",
            )),
        }
    }
}

/// `ModelGateway` reaches the device's model gateway, never a placement listener. Agents
/// without the `model_host` flag cannot decode it and close the tunnel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelTarget {
    #[default]
    Service,
    ModelGateway,
}

impl TunnelTarget {
    fn is_service(&self) -> bool {
        *self == Self::Service
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelMode {
    #[default]
    Tcp,
    Http,
}

impl TunnelMode {
    fn is_tcp(&self) -> bool {
        *self == Self::Tcp
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TunnelDataOpen {
    Request {
        request: ManagementRequest,
    },
    Artifact {
        project_id: String,
        transfer_id: String,
        file_index: Option<u32>,
        offset: u64,
    },
    /// Pushes the bytes of one model asset job from `offset` on.
    ModelAsset {
        job_id: String,
        offset: u64,
    },
}

impl TunnelDataOpen {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Request { request } => {
                validate_management_id(&request.operation_id)?;
                validate_management_id(&request.device_id)?;
                if request.issued_at <= 0
                    || request.expires_at <= request.issued_at
                    || request.expires_at.saturating_sub(request.issued_at) > 300
                {
                    return Err(ProtocolError::Invalid("tunnel data request lifetime"));
                }
            }
            Self::Artifact {
                project_id,
                transfer_id,
                ..
            } => {
                crate::validate_artifact_project_id(project_id)?;
                if transfer_id.len() != 36
                    || !transfer_id.bytes().enumerate().all(|(index, byte)| {
                        if matches!(index, 8 | 13 | 18 | 23) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        }
                    })
                {
                    return Err(ProtocolError::Invalid("tunnel artifact identity"));
                }
            }
            Self::ModelAsset { job_id, .. } => {
                crate::validate_model_job_id(job_id)?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for TunnelDataOpen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Management payloads can contain secret values.
        f.debug_struct("TunnelDataOpen")
            .field(
                "kind",
                &match self {
                    Self::Request { .. } => "request",
                    Self::Artifact { .. } => "artifact",
                    Self::ModelAsset { .. } => "model_asset",
                },
            )
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelReset {
    pub code: String,
    pub message: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelRenewStart {
    pub certificate_jws: String,
    /// URL-safe base64 without padding of the first fresh Noise handshake message.
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelRenewed {
    pub expires_at: i64,
}

#[derive(Clone)]
pub enum TunnelFrameBody {
    Open(TunnelOpen),
    Opened,
    Data(Vec<u8>),
    Window(u32),
    Fin,
    Reset(TunnelReset),
    Ping([u8; 8]),
    Pong([u8; 8]),
    RenewStart(TunnelRenewStart),
    RenewReply(Vec<u8>),
    RenewFinish(Vec<u8>),
    Renewed(TunnelRenewed),
    OpenData(TunnelDataOpen),
}

impl TunnelFrameBody {
    pub fn kind(&self) -> u8 {
        match self {
            Self::Open(_) => 1,
            Self::Opened => 2,
            Self::Data(_) => 3,
            Self::Window(_) => 4,
            Self::Fin => 5,
            Self::Reset(_) => 6,
            Self::Ping(_) => 7,
            Self::Pong(_) => 8,
            Self::RenewStart(_) => 9,
            Self::RenewReply(_) => 10,
            Self::RenewFinish(_) => 11,
            Self::Renewed(_) => 12,
            Self::OpenData(_) => 13,
        }
    }
}

impl std::fmt::Debug for TunnelFrameBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Stream payloads can contain application credentials and response bodies.
        f.debug_struct("TunnelFrameBody")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct TunnelFrame {
    pub sequence: u64,
    pub stream_id: u32,
    pub body: TunnelFrameBody,
}

impl TunnelFrame {
    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_stream_id(self.body.kind(), self.stream_id)?;
        let payload: Cow<'_, [u8]> = match &self.body {
            TunnelFrameBody::OpenData(value) => {
                value.validate()?;
                json_encode(value)?.into()
            }
            TunnelFrameBody::Open(value) => {
                value.validate()?;
                json_encode(value)?.into()
            }
            TunnelFrameBody::Opened | TunnelFrameBody::Fin => (&[][..]).into(),
            TunnelFrameBody::Data(value)
            | TunnelFrameBody::RenewReply(value)
            | TunnelFrameBody::RenewFinish(value) => {
                let limit = if matches!(self.body, TunnelFrameBody::Data(_)) {
                    TUNNEL_MAX_DATA
                } else {
                    TUNNEL_MAX_HANDSHAKE
                };
                if value.is_empty() || value.len() > limit {
                    return Err(ProtocolError::Invalid("tunnel frame payload size"));
                }
                value.as_slice().into()
            }
            TunnelFrameBody::Window(value) => {
                validate_credit(*value)?;
                value.to_be_bytes().to_vec().into()
            }
            TunnelFrameBody::Reset(value) => {
                validate_reset(value)?;
                json_encode(value)?.into()
            }
            TunnelFrameBody::Ping(value) | TunnelFrameBody::Pong(value) => value.as_slice().into(),
            TunnelFrameBody::RenewStart(value) => {
                validate_handshake_start(&value.certificate_jws, &value.data)?;
                json_encode(value)?.into()
            }
            TunnelFrameBody::Renewed(value) => {
                validate_expiry(value.expires_at)?;
                json_encode(value)?.into()
            }
        };
        if payload.len() > TUNNEL_MAX_DATA {
            return Err(ProtocolError::Invalid("tunnel frame size"));
        }
        let mut output = Vec::with_capacity(TUNNEL_HEADER_LEN + payload.len());
        output.extend_from_slice(b"FLTN");
        output.extend_from_slice(&[TUNNEL_VERSION, self.body.kind(), 0, 0]);
        output.extend_from_slice(&self.stream_id.to_be_bytes());
        output.extend_from_slice(&self.sequence.to_be_bytes());
        output.extend_from_slice(&payload);
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if !(TUNNEL_HEADER_LEN..=TUNNEL_MAX_FRAME).contains(&bytes.len())
            || &bytes[..4] != b"FLTN"
            || bytes[4] != TUNNEL_VERSION
            || bytes[6..8] != [0, 0]
        {
            return Err(ProtocolError::Invalid("tunnel frame header"));
        }
        let kind = bytes[5];
        let stream_id = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
        let sequence = u64::from_be_bytes(bytes[12..20].try_into().unwrap());
        let payload = &bytes[TUNNEL_HEADER_LEN..];
        validate_stream_id(kind, stream_id)?;
        let body = match kind {
            1 => {
                let value: TunnelOpen = json_decode(payload)?;
                value.validate()?;
                TunnelFrameBody::Open(value)
            }
            2 if payload.is_empty() => TunnelFrameBody::Opened,
            3 if !payload.is_empty() => TunnelFrameBody::Data(payload.to_vec()),
            4 if payload.len() == 4 => {
                let value = u32::from_be_bytes(payload.try_into().unwrap());
                validate_credit(value)?;
                TunnelFrameBody::Window(value)
            }
            5 if payload.is_empty() => TunnelFrameBody::Fin,
            6 => {
                let value: TunnelReset = json_decode(payload)?;
                validate_reset(&value)?;
                TunnelFrameBody::Reset(value)
            }
            7 if payload.len() == 8 => TunnelFrameBody::Ping(payload.try_into().unwrap()),
            8 if payload.len() == 8 => TunnelFrameBody::Pong(payload.try_into().unwrap()),
            9 => {
                let value: TunnelRenewStart = json_decode(payload)?;
                validate_handshake_start(&value.certificate_jws, &value.data)?;
                TunnelFrameBody::RenewStart(value)
            }
            10 if (1..=TUNNEL_MAX_HANDSHAKE).contains(&payload.len()) => {
                TunnelFrameBody::RenewReply(payload.to_vec())
            }
            11 if (1..=TUNNEL_MAX_HANDSHAKE).contains(&payload.len()) => {
                TunnelFrameBody::RenewFinish(payload.to_vec())
            }
            12 => {
                let value: TunnelRenewed = json_decode(payload)?;
                validate_expiry(value.expires_at)?;
                TunnelFrameBody::Renewed(value)
            }
            13 => {
                let value: TunnelDataOpen = json_decode(payload)?;
                value.validate()?;
                TunnelFrameBody::OpenData(value)
            }
            _ => return Err(ProtocolError::Invalid("tunnel frame kind or payload")),
        };
        Ok(Self {
            sequence,
            stream_id,
            body,
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelHello {
    pub grant_id: String,
    pub certificate_jws: String,
    pub data: String,
}

#[derive(Clone, PartialEq, Eq)]
pub enum TunnelEnvelopeBody {
    Hello(TunnelHello),
    Handshake(Vec<u8>),
    Message(Vec<u8>),
    Close,
}

impl TunnelEnvelopeBody {
    pub fn kind(&self) -> u8 {
        match self {
            Self::Hello(_) => 1,
            Self::Handshake(_) => 2,
            Self::Message(_) => 3,
            Self::Close => 4,
        }
    }
}

impl std::fmt::Debug for TunnelEnvelopeBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelEnvelopeBody")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelEnvelope {
    /// Remains stable during renewal; each renewed certificate has a fresh session ID.
    pub session_id: String,
    pub body: TunnelEnvelopeBody,
}

impl TunnelEnvelope {
    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_management_id(&self.session_id)?;
        let body: Cow<'_, [u8]> = match &self.body {
            TunnelEnvelopeBody::Hello(value) => {
                validate_management_id(&value.grant_id)?;
                validate_handshake_start(&value.certificate_jws, &value.data)?;
                json_encode(value)?.into()
            }
            TunnelEnvelopeBody::Handshake(value) => {
                if !(1..=TUNNEL_MAX_HANDSHAKE).contains(&value.len()) {
                    return Err(ProtocolError::Invalid("tunnel envelope handshake size"));
                }
                value.as_slice().into()
            }
            TunnelEnvelopeBody::Message(value) => {
                if !(16..=TUNNEL_MAX_FRAME + 16).contains(&value.len()) {
                    return Err(ProtocolError::Invalid("tunnel envelope message size"));
                }
                value.as_slice().into()
            }
            TunnelEnvelopeBody::Close => (&[][..]).into(),
        };
        let mut output = Vec::with_capacity(6 + self.session_id.len() + body.len());
        output.extend_from_slice(b"FLTE");
        output.push(self.body.kind());
        output.push(self.session_id.len() as u8);
        output.extend_from_slice(self.session_id.as_bytes());
        output.extend_from_slice(&body);
        if output.len() > TUNNEL_MAX_ENVELOPE {
            return Err(ProtocolError::Invalid("tunnel envelope size"));
        }
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if !(7..=TUNNEL_MAX_ENVELOPE).contains(&bytes.len()) || &bytes[..4] != b"FLTE" {
            return Err(ProtocolError::Invalid("tunnel envelope header"));
        }
        let id_len = usize::from(bytes[5]);
        if id_len == 0 || id_len > 128 || bytes.len() < 6 + id_len {
            return Err(ProtocolError::Invalid("tunnel envelope session"));
        }
        let session_id = std::str::from_utf8(&bytes[6..6 + id_len])
            .map_err(|_| ProtocolError::Invalid("tunnel envelope session"))?;
        validate_management_id(session_id)?;
        let payload = &bytes[6 + id_len..];
        let body = match bytes[4] {
            1 => {
                let value: TunnelHello = json_decode(payload)?;
                validate_management_id(&value.grant_id)?;
                validate_handshake_start(&value.certificate_jws, &value.data)?;
                TunnelEnvelopeBody::Hello(value)
            }
            2 if (1..=TUNNEL_MAX_HANDSHAKE).contains(&payload.len()) => {
                TunnelEnvelopeBody::Handshake(payload.to_vec())
            }
            3 if (16..=TUNNEL_MAX_FRAME + 16).contains(&payload.len()) => {
                TunnelEnvelopeBody::Message(payload.to_vec())
            }
            4 if payload.is_empty() => TunnelEnvelopeBody::Close,
            _ => return Err(ProtocolError::Invalid("tunnel envelope kind or payload")),
        };
        Ok(Self {
            session_id: session_id.into(),
            body,
        })
    }
}

fn json_encode(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| ProtocolError::Invalid("tunnel JSON"))
}

fn validate_stream_id(kind: u8, stream_id: u32) -> Result<()> {
    if (1..=6).contains(&kind) || kind == 13 {
        if stream_id == 0 || stream_id.is_multiple_of(2) {
            return Err(ProtocolError::Invalid("tunnel stream identifier"));
        }
    } else if stream_id != 0 {
        return Err(ProtocolError::Invalid("tunnel control stream identifier"));
    }
    Ok(())
}

fn validate_reset(value: &TunnelReset) -> Result<()> {
    if value.code.is_empty()
        || value.code.len() > 64
        || !value
            .code
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c == b'_')
        || value.message.len() > 256
    {
        return Err(ProtocolError::Invalid("tunnel reset"));
    }
    Ok(())
}

fn validate_expiry(expires_at: i64) -> Result<()> {
    if expires_at <= 0 {
        return Err(ProtocolError::Invalid("tunnel authorization expiry"));
    }
    Ok(())
}

fn json_decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| ProtocolError::Invalid("tunnel JSON"))
}

fn validate_handshake_start(certificate_jws: &str, data: &str) -> Result<()> {
    if certificate_jws.is_empty() || certificate_jws.len() > 8192 || data.len() > 172 {
        return Err(ProtocolError::Invalid("tunnel handshake start size"));
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(data)
        .map_err(|_| ProtocolError::Invalid("tunnel handshake encoding"))?;
    if !(1..=TUNNEL_MAX_HANDSHAKE).contains(&decoded.len()) {
        return Err(ProtocolError::Invalid("tunnel handshake size"));
    }
    Ok(())
}

fn validate_credit(value: u32) -> Result<()> {
    if value == 0 || value > TUNNEL_INITIAL_WINDOW {
        return Err(ProtocolError::Invalid("tunnel credit"));
    }
    Ok(())
}

/// Credits count application bytes, excluding headers and encryption overhead.
#[derive(Debug)]
pub struct TunnelFlowControl {
    send_credit: u32,
    receive_credit: u32,
    unconsumed: u32,
}

impl Default for TunnelFlowControl {
    fn default() -> Self {
        Self {
            send_credit: TUNNEL_INITIAL_WINDOW,
            receive_credit: TUNNEL_INITIAL_WINDOW,
            unconsumed: 0,
        }
    }
}

impl TunnelFlowControl {
    pub fn send_credit(&self) -> u32 {
        self.send_credit
    }
    pub fn receive_credit(&self) -> u32 {
        self.receive_credit
    }

    pub fn send(&mut self, bytes: usize) -> Result<()> {
        let bytes = data_len(bytes)?;
        self.send_credit = self
            .send_credit
            .checked_sub(bytes)
            .ok_or(ProtocolError::Invalid("tunnel send window exhausted"))?;
        Ok(())
    }

    pub fn receive(&mut self, bytes: usize) -> Result<()> {
        let bytes = data_len(bytes)?;
        let credit = self
            .receive_credit
            .checked_sub(bytes)
            .ok_or(ProtocolError::Invalid("tunnel receive window exhausted"))?;
        self.receive_credit = credit;
        self.unconsumed += bytes;
        Ok(())
    }

    pub fn grant_send_credit(&mut self, bytes: u32) -> Result<()> {
        validate_credit(bytes)?;
        let credit = self
            .send_credit
            .checked_add(bytes)
            .filter(|credit| *credit <= TUNNEL_INITIAL_WINDOW)
            .ok_or(ProtocolError::Invalid("tunnel send credit overflow"))?;
        self.send_credit = credit;
        Ok(())
    }

    /// Emit a Window frame for this amount only after the application consumes it.
    pub fn consume(&mut self, bytes: u32) -> Result<()> {
        validate_credit(bytes)?;
        let unconsumed = self
            .unconsumed
            .checked_sub(bytes)
            .ok_or(ProtocolError::Invalid("tunnel credit for unconsumed data"))?;
        self.unconsumed = unconsumed;
        self.receive_credit += bytes;
        Ok(())
    }
}

fn data_len(bytes: usize) -> Result<u32> {
    if bytes == 0 || bytes > TUNNEL_MAX_DATA {
        return Err(ProtocolError::Invalid("tunnel data size"));
    }
    Ok(bytes as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: TunnelFrameBody) -> TunnelFrame {
        TunnelFrame {
            sequence: 0x0102_0304_0506_0708,
            stream_id: if body.kind() <= 6 || body.kind() == 13 {
                0x0102_0305
            } else {
                0
            },
            body,
        }
    }

    #[test]
    fn fixed_big_endian_header_matches_browser_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/tunnel-v1.json")).unwrap();
        let encoded = frame(TunnelFrameBody::Data(vec![0, 1, 254, 255]))
            .encode()
            .unwrap();
        let hex: String = encoded.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hex, fixture["data_frame_hex"].as_str().unwrap());
        assert_eq!(
            TunnelFrame::decode(&encoded).unwrap().sequence,
            0x0102_0304_0506_0708
        );
        for name in ["open_data_request", "open_data_artifact"] {
            let body: TunnelDataOpen =
                serde_json::from_value(fixture[name]["body"].clone()).unwrap();
            let encoded = frame(TunnelFrameBody::OpenData(body)).encode().unwrap();
            let hex: String = encoded.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(hex, fixture[name]["frame_hex"].as_str().unwrap());
            assert_eq!(
                TunnelFrame::decode(&encoded).unwrap().encode().unwrap(),
                encoded
            );
        }
    }

    #[test]
    fn every_frame_kind_round_trips() {
        for body in [
            TunnelFrameBody::Open(TunnelOpen {
                placement_id: "placement".into(),
                service_id: "hosting".into(),
                mode: TunnelMode::Http,
                target: TunnelTarget::Service,
            }),
            TunnelFrameBody::Open(TunnelOpen {
                placement_id: "placement".into(),
                service_id: "hosting".into(),
                mode: TunnelMode::Tcp,
                target: TunnelTarget::Service,
            }),
            TunnelFrameBody::Open(TunnelOpen {
                placement_id: String::new(),
                service_id: String::new(),
                mode: TunnelMode::Http,
                target: TunnelTarget::ModelGateway,
            }),
            TunnelFrameBody::Opened,
            TunnelFrameBody::Data(vec![42; TUNNEL_MAX_DATA]),
            TunnelFrameBody::Window(TUNNEL_INITIAL_WINDOW),
            TunnelFrameBody::Fin,
            TunnelFrameBody::Reset(TunnelReset {
                code: "service_unavailable".into(),
                message: "Service stopped".into(),
            }),
            TunnelFrameBody::Ping([42; 8]),
            TunnelFrameBody::Pong([42; 8]),
            TunnelFrameBody::RenewStart(TunnelRenewStart {
                certificate_jws: "signed-certificate".into(),
                data: URL_SAFE_NO_PAD.encode([42; 32]),
            }),
            TunnelFrameBody::RenewReply(vec![42; 96]),
            TunnelFrameBody::RenewFinish(vec![42; 64]),
            TunnelFrameBody::Renewed(TunnelRenewed { expires_at: 300 }),
            TunnelFrameBody::OpenData(TunnelDataOpen::Artifact {
                project_id: "project".into(),
                transfer_id: "12345678-1234-1234-1234-123456789abc".into(),
                file_index: Some(0),
                offset: 8192,
            }),
            TunnelFrameBody::OpenData(TunnelDataOpen::ModelAsset {
                job_id: "12345678-1234-1234-1234-123456789abc".into(),
                offset: u64::MAX,
            }),
        ] {
            let frame = frame(body);
            let encoded = frame.encode().unwrap();
            assert_eq!(
                TunnelFrame::decode(&encoded).unwrap().encode().unwrap(),
                encoded
            );
        }
    }

    #[test]
    fn service_opens_keep_their_v1_bytes() {
        let hex = |body: TunnelFrameBody| -> String {
            frame(body)
                .encode()
                .unwrap()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        };
        let open = |mode| {
            TunnelFrameBody::Open(TunnelOpen {
                placement_id: "placement".into(),
                service_id: "hosting".into(),
                mode,
                target: TunnelTarget::default(),
            })
        };
        let header = "464c544e010100000102030501020304050607087b22706c6163656d656e745f6964223a22706c6163656d656e74222c22736572766963655f6964223a22686f7374696e6722";
        assert_eq!(hex(open(TunnelMode::Tcp)), format!("{header}7d"));
        assert_eq!(
            hex(open(TunnelMode::Http)),
            format!("{header}2c226d6f6465223a2268747470227d")
        );
        let legacy: TunnelOpen =
            serde_json::from_str(r#"{"placement_id":"placement","service_id":"hosting"}"#).unwrap();
        assert_eq!(legacy.target, TunnelTarget::Service);
    }

    #[test]
    fn model_gateway_opens_name_no_service() {
        let gateway = TunnelOpen {
            placement_id: String::new(),
            service_id: String::new(),
            mode: TunnelMode::Http,
            target: TunnelTarget::ModelGateway,
        };
        assert_eq!(
            serde_json::to_value(&gateway).unwrap(),
            serde_json::json!({"mode":"http","target":"model_gateway"})
        );
        let encoded = frame(TunnelFrameBody::Open(gateway.clone()))
            .encode()
            .unwrap();
        let TunnelFrameBody::Open(decoded) = TunnelFrame::decode(&encoded).unwrap().body else {
            panic!("open frame did not decode as open")
        };
        assert_eq!(decoded, gateway);
        let raw = |json: &str| {
            let mut bytes = frame(TunnelFrameBody::Opened).encode().unwrap();
            bytes[5] = 1;
            bytes.extend_from_slice(json.as_bytes());
            TunnelFrame::decode(&bytes)
        };
        assert!(raw(r#"{"placement_id":"","service_id":"","target":"model_gateway"}"#).is_ok());
        for refused in [
            r#"{"placement_id":"placement","target":"model_gateway"}"#,
            r#"{"service_id":"hosting","target":"model_gateway"}"#,
            r#"{"placement_id":"placement","service_id":"hosting","target":"model_gateway"}"#,
            r#"{"target":"service"}"#,
            r#"{"target":"inference"}"#,
        ] {
            assert!(raw(refused).is_err(), "{refused}");
        }
        assert!(
            frame(TunnelFrameBody::Open(TunnelOpen {
                placement_id: "placement".into(),
                ..gateway
            }))
            .encode()
            .is_err()
        );
    }

    #[test]
    fn model_asset_streams_name_a_job_and_accept_large_offsets() {
        let open = |value: serde_json::Value| {
            let mut bytes = frame(TunnelFrameBody::Opened).encode().unwrap();
            bytes[5] = 13;
            bytes.extend_from_slice(&serde_json::to_vec(&value).unwrap());
            TunnelFrame::decode(&bytes)
        };
        let valid = serde_json::json!({"kind":"model_asset","job_id":"12345678-1234-1234-1234-123456789abc","offset":0});
        let mut large_offset = valid.clone();
        large_offset["offset"] = serde_json::json!(u64::MAX);
        assert!(open(large_offset).is_ok());
        let TunnelFrameBody::OpenData(decoded) = open(valid.clone()).unwrap().body else {
            panic!("data open did not decode")
        };
        assert_eq!(
            format!("{decoded:?}"),
            r#"TunnelDataOpen { kind: "model_asset", .. }"#
        );
        for (field, value) in [
            (
                "job_id",
                serde_json::json!("12345678-1234-1234-1234-123456789ABC"),
            ),
            ("job_id", serde_json::json!("job")),
            ("offset", serde_json::json!(-1)),
            ("project_id", serde_json::json!("project")),
        ] {
            let mut changed = valid.clone();
            changed[field] = value;
            assert!(open(changed).is_err(), "{field}");
        }
    }

    #[test]
    fn malformed_headers_lengths_streams_and_metadata_are_rejected() {
        let valid = frame(TunnelFrameBody::Data(vec![1])).encode().unwrap();
        for length in 0..=TUNNEL_HEADER_LEN {
            assert!(TunnelFrame::decode(&valid[..length]).is_err());
        }
        for (offset, value) in [(0, 0), (4, 2), (5, 0), (5, 13), (6, 1), (7, 1), (11, 4)] {
            let mut malformed = valid.clone();
            malformed[offset] = value;
            assert!(TunnelFrame::decode(&malformed).is_err(), "offset {offset}");
        }
        let mut too_large = valid.clone();
        too_large.resize(TUNNEL_MAX_FRAME + 1, 0);
        assert!(TunnelFrame::decode(&too_large).is_err());
        assert!(frame(TunnelFrameBody::Data(Vec::new())).encode().is_err());
        assert!(
            frame(TunnelFrameBody::Data(vec![0; TUNNEL_MAX_DATA + 1]))
                .encode()
                .is_err()
        );
        assert!(frame(TunnelFrameBody::Window(0)).encode().is_err());
        assert!(
            frame(TunnelFrameBody::Window(TUNNEL_INITIAL_WINDOW + 1))
                .encode()
                .is_err()
        );
        assert!(
            frame(TunnelFrameBody::RenewReply(vec![
                0;
                TUNNEL_MAX_HANDSHAKE + 1
            ]))
            .encode()
            .is_err()
        );
        assert!(
            frame(TunnelFrameBody::RenewStart(TunnelRenewStart {
                certificate_jws: "cert".into(),
                data: "====".into()
            }))
            .encode()
            .is_err()
        );
        let mut malformed = frame(TunnelFrameBody::Opened).encode().unwrap();
        malformed[5] = 1;
        malformed.extend_from_slice(
            br#"{"placement_id":"p","service_id":"hosting","host":"127.0.0.1"}"#,
        );
        assert!(TunnelFrame::decode(&malformed).is_err());
        let mut control = frame(TunnelFrameBody::Ping([0; 8]));
        control.stream_id = 1;
        assert!(control.encode().is_err());
    }

    #[test]
    fn data_open_binds_request_fields_and_artifact_destination() {
        let request = serde_json::json!({"kind":"request","request":{"operation_id":"operation","device_id":"device","issued_at":100,"expires_at":160,"command":{"type":"logs","placement_id":"placement","after":0,"limit":100}}});
        let artifact = serde_json::json!({"kind":"artifact","project_id":"project","transfer_id":"12345678-1234-1234-1234-123456789abc","file_index":null,"offset":0});
        let encode = |value: &serde_json::Value, stream_id: u32| {
            let mut encoded = frame(TunnelFrameBody::Opened).encode().unwrap();
            encoded[5] = 13;
            encoded[8..12].copy_from_slice(&stream_id.to_be_bytes());
            encoded.extend_from_slice(&serde_json::to_vec(value).unwrap());
            TunnelFrame::decode(&encoded)
        };
        assert!(encode(&request, 1).is_ok());
        assert!(encode(&artifact, 1).is_ok());
        let mut large_offset = artifact.clone();
        large_offset["offset"] = serde_json::json!(u64::MAX);
        assert!(encode(&large_offset, 1).is_ok());
        assert!(encode(&request, 0).is_err());
        assert!(encode(&artifact, 2).is_err());
        for (field, value) in [
            ("issued_at", serde_json::json!(0)),
            ("expires_at", serde_json::json!(401)),
            ("device_id", serde_json::json!("device/path")),
        ] {
            let mut changed = request.clone();
            changed["request"][field] = value;
            assert!(encode(&changed, 1).is_err());
        }
        let mut unknown = request.clone();
        unknown["request"]["command"]["host"] = serde_json::json!("127.0.0.1");
        assert!(encode(&unknown, 1).is_err());
        for (field, value) in [
            ("project_id", serde_json::json!("..")),
            ("transfer_id", serde_json::json!("not-a-transfer")),
            ("file_index", serde_json::json!(-1)),
            ("host", serde_json::json!("127.0.0.1")),
        ] {
            let mut changed = artifact.clone();
            changed[field] = value;
            assert!(encode(&changed, 1).is_err());
        }
    }

    #[test]
    fn envelopes_reject_truncation_and_unbounded_payloads() {
        for body in [
            TunnelEnvelopeBody::Hello(TunnelHello {
                grant_id: "grant".into(),
                certificate_jws: "cert".into(),
                data: URL_SAFE_NO_PAD.encode([1; 32]),
            }),
            TunnelEnvelopeBody::Handshake(vec![0; 96]),
            TunnelEnvelopeBody::Message(vec![0; TUNNEL_MAX_FRAME + 16]),
            TunnelEnvelopeBody::Close,
        ] {
            let envelope = TunnelEnvelope {
                session_id: "session:1".into(),
                body,
            };
            let bytes = envelope.encode().unwrap();
            assert_eq!(TunnelEnvelope::decode(&bytes).unwrap(), envelope);
            for length in 0..15 {
                assert!(TunnelEnvelope::decode(&bytes[..length]).is_err());
            }
        }
        assert!(TunnelEnvelope::decode(b"FLTE\x04\x00").is_err());
        assert!(TunnelEnvelope::decode(b"FLTE\x04\x01\xff").is_err());
        assert!(TunnelEnvelope::decode(b"FLTE\x04\x01xextra").is_err());
        assert!(
            TunnelEnvelope {
                session_id: "session".into(),
                body: TunnelEnvelopeBody::Message(vec![0; TUNNEL_MAX_FRAME + 17])
            }
            .encode()
            .is_err()
        );
    }

    #[test]
    fn credits_only_return_after_consumption_and_never_overflow() {
        let mut flow = TunnelFlowControl::default();
        assert!(flow.consume(1).is_err());
        assert!(flow.grant_send_credit(1).is_err());
        let mut remaining = TUNNEL_INITIAL_WINDOW as usize;
        while remaining > 0 {
            let chunk = remaining.min(TUNNEL_MAX_DATA);
            flow.send(chunk).unwrap();
            flow.receive(chunk).unwrap();
            remaining -= chunk;
        }
        assert_eq!(flow.send_credit(), 0);
        assert_eq!(flow.receive_credit(), 0);
        assert!(flow.send(1).is_err());
        assert!(flow.receive(1).is_err());
        assert!(flow.consume(TUNNEL_INITIAL_WINDOW + 1).is_err());
        assert!(flow.grant_send_credit(u32::MAX).is_err());
        flow.consume(TUNNEL_INITIAL_WINDOW).unwrap();
        flow.grant_send_credit(TUNNEL_INITIAL_WINDOW).unwrap();
        assert_eq!(flow.send_credit(), TUNNEL_INITIAL_WINDOW);
        assert_eq!(flow.receive_credit(), TUNNEL_INITIAL_WINDOW);
        assert!(flow.consume(1).is_err());
        assert!(flow.grant_send_credit(1).is_err());
    }

    #[test]
    fn debug_does_not_print_application_plaintext() {
        let frame = frame(TunnelFrameBody::Data(b"application-password".to_vec()));
        assert!(!format!("{frame:?}").contains("application-password"));
        assert!(!format!("{frame:?}").contains("97, 112, 112"));
    }
}
