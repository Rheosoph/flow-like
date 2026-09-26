use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

pub(super) const PROTOCOL: &str = "flowlike.device-management.v1";
pub(super) const MAX_PAYLOAD: usize = 32 * 1024;
pub(super) const MAX_FRAME: usize = 48 * 1024;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ServerFrame {
    Ready {
        participant_id: String,
        role: String,
        expires_at: i64,
    },
    Pong,
    Frame {
        to: String,
        channel: Channel,
        payload: String,
        from: String,
        from_role: String,
    },
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Channel {
    Signal,
    Noise,
}

#[derive(Serialize)]
pub(super) struct ClientFrame<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    to: &'a str,
    channel: Channel,
    payload: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum NoiseEnvelope {
    Hello {
        session_id: String,
        grant_id: String,
        certificate_jws: String,
        data: String,
    },
    Handshake {
        session_id: String,
        data: String,
    },
    Message {
        session_id: String,
        data: String,
    },
}

impl NoiseEnvelope {
    pub(super) fn session_id(&self) -> &str {
        match self {
            Self::Hello { session_id, .. }
            | Self::Handshake { session_id, .. }
            | Self::Message { session_id, .. } => session_id,
        }
    }
    pub(super) fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_PAYLOAD, "Management envelope too large");
        let value: Self = serde_json::from_slice(bytes)?;
        identifier(value.session_id())?;
        if let Self::Hello {
            grant_id,
            certificate_jws,
            ..
        } = &value
        {
            identifier(grant_id)?;
            ensure!(
                !certificate_jws.is_empty() && certificate_jws.len() <= 8192,
                "Invalid controller certificate size"
            );
        }
        Ok(value)
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum SignalEnvelope {
    Offer {
        session_id: String,
        sdp: String,
        grant_id: String,
        certificate_jws: String,
    },
}

impl SignalEnvelope {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_PAYLOAD, "Signaling envelope too large");
        let value: Self = serde_json::from_slice(bytes)?;
        let Self::Offer {
            session_id,
            sdp,
            grant_id,
            certificate_jws,
        } = &value;
        identifier(session_id)?;
        identifier(grant_id)?;
        ensure!(
            !certificate_jws.is_empty() && certificate_jws.len() <= 8192,
            "Invalid controller certificate size"
        );
        validate_offer(sdp)?;
        Ok(value)
    }
}

pub(super) fn validate_offer(sdp: &str) -> Result<()> {
    ensure!(
        sdp.len() <= 24 * 1024 && !sdp.contains('\0'),
        "Invalid SDP size"
    );
    let media = sdp
        .lines()
        .filter(|line| line.starts_with("m="))
        .collect::<Vec<_>>();
    ensure!(
        media.len() == 1
            && media[0].starts_with("m=application ")
            && media[0].contains("UDP/DTLS/SCTP"),
        "Only one DTLS data channel transport is accepted"
    );
    ensure!(
        sdp.lines()
            .filter(|line| line.starts_with("a=candidate:"))
            .count()
            <= 32,
        "Too many ICE candidates"
    );
    Ok(())
}

pub(super) fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'_' | b'-' | b'.')),
        "Invalid management identifier"
    );
    Ok(())
}

pub(super) fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}
pub(super) fn decode(value: &str) -> Result<Vec<u8>> {
    ensure!(
        !value.is_empty() && value.len() <= MAX_PAYLOAD.div_ceil(3) * 4,
        "Invalid management payload size"
    );
    let bytes = URL_SAFE_NO_PAD.decode(value)?;
    ensure!(
        bytes.len() <= MAX_PAYLOAD && encode(&bytes) == value,
        "Invalid management payload encoding"
    );
    Ok(bytes)
}

pub(super) fn serialize(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= MAX_PAYLOAD, "Management response too large");
    Ok(bytes)
}

pub(super) fn outbound(to: &str, channel: Channel, payload: &[u8]) -> Result<String> {
    identifier(to)?;
    ensure!(
        !payload.is_empty() && payload.len() <= MAX_PAYLOAD,
        "Management frame too large"
    );
    Ok(serde_json::to_string(&ClientFrame {
        kind: "frame",
        to,
        channel,
        payload: encode(payload),
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_ambiguous_and_cross_profile_envelopes() {
        assert!(decode(&encode(&vec![1; MAX_PAYLOAD])).is_ok());
        assert!(decode(&encode(&vec![1; MAX_PAYLOAD + 1])).is_err());
        assert!(decode("YQ==").is_err());
        assert!(decode("YR").is_err());
        for value in [
            r#"{"session_id":"s","kind":"message","data":"YQ","extra":1}"#,
            r#"{"session_id":"s","session_id":"t","kind":"message","data":"YQ"}"#,
            r#"{"session_id":"s","kind":"message","grant_id":"owner","data":"YQ"}"#,
            r#"{"session_id":"../s","kind":"message","data":"YQ"}"#,
        ] {
            assert!(NoiseEnvelope::parse(value.as_bytes()).is_err());
        }
    }

    #[test]
    fn only_data_channel_offers_and_bounded_canonical_frames() {
        assert!(
            validate_offer("v=0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n").is_ok()
        );
        assert!(validate_offer("v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\n").is_err());
        assert!(validate_offer("m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n").is_err());
        let frame = outbound("controller", Channel::Noise, b"encrypted").unwrap();
        let value: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(value["to"], "controller");
        assert_eq!(value["payload"], encode(b"encrypted"));
        assert!(value.get("from").is_none());
    }
}
