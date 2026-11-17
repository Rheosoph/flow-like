use super::crypto::{Key, RawKey};
use flow_like_types::{Result, anyhow, bail};
use serde::Deserialize;
use webrtc::peer_connection::RTCIceServer;

const MAX_ICE_SERVERS: usize = 16;
const ICE_MARGIN_SECS: i64 = 30;

#[derive(Deserialize)]
struct IceServer {
    urls: Vec<String>,
    username: Option<String>,
    credential: Option<String>,
}

#[derive(Deserialize)]
struct Wire {
    jwt: String,
    room: String,
    topic: String,
    keys: Vec<RawKey>,
    signaling_servers: Vec<String>,
    #[serde(default)]
    ice_servers: Vec<IceServer>,
    ice_expires_at: Option<i64>,
    expires_at: i64,
}

/// What `POST realtime/bot-screens` grants this account: a topic, its day keys and the ICE servers.
#[derive(Clone, Default)]
pub(super) struct Access {
    pub jwt: String,
    pub room: String,
    pub topic: String,
    pub keys: Vec<Key>,
    pub server: String,
    pub ice_servers: Vec<RTCIceServer>,
    pub ice_expires_at: Option<i64>,
    pub expires_at: i64,
}

impl Access {
    pub fn parse(value: serde_json::Value) -> Result<Self> {
        let wire: Wire = serde_json::from_value(value)
            .map_err(|error| anyhow!("Read the bot screen access from the hub: {error}"))?;
        let valid_room = wire.room.len() == 32
            && wire
                .room
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid_room {
            bail!("The hub sent a bot screen room that is not 32 lowercase hex characters");
        }
        if wire.topic != format!("bot-screens:{}:v1", wire.room) {
            bail!(
                "The hub sent bot screen topic {} that does not match its room",
                wire.topic
            );
        }
        let keys = wire
            .keys
            .iter()
            .map(Key::parse)
            .collect::<Result<Vec<_>>>()?;
        if keys.is_empty() {
            bail!("The hub sent no bot screen keys");
        }
        let server = wire
            .signaling_servers
            .into_iter()
            .next()
            .filter(|server| server.starts_with("wss://"))
            .ok_or_else(|| anyhow!("The hub sent no wss signaling server for bot screens"))?;
        if wire.ice_servers.len() > MAX_ICE_SERVERS {
            bail!(
                "The hub sent {} ICE servers, at most {MAX_ICE_SERVERS} are used",
                wire.ice_servers.len()
            );
        }
        let ice_servers = wire
            .ice_servers
            .into_iter()
            .map(|server| {
                let server = RTCIceServer {
                    urls: server.urls,
                    username: server.username.unwrap_or_default(),
                    credential: server.credential.unwrap_or_default(),
                };
                server.urls()?;
                Ok(server)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            jwt: wire.jwt,
            room: wire.room,
            topic: wire.topic,
            keys,
            server,
            ice_servers,
            ice_expires_at: wire.ice_expires_at,
            expires_at: wire.expires_at,
        })
    }

    pub fn key(&self, id: &str) -> Option<&Key> {
        self.keys.iter().find(|key| key.id == id)
    }

    pub fn sealing_key(&self) -> &Key {
        &self.keys[0]
    }

    pub fn usable_ice(&self, now_secs: i64) -> Vec<RTCIceServer> {
        match self.ice_expires_at {
            Some(expires) if expires <= now_secs + ICE_MARGIN_SECS => Vec::new(),
            _ => self.ice_servers.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body() -> serde_json::Value {
        json!({
            "jwt": "j",
            "room": "0123456789abcdef0123456789abcdef",
            "topic": "bot-screens:0123456789abcdef0123456789abcdef:v1",
            "keys": [{ "id": "2026-11-17", "key": "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA" }],
            "signaling_servers": ["wss://signaling.example"],
            "ice_servers": [{ "urls": ["turn:relay.example:3478"], "username": "u", "credential": "c" }],
            "ice_expires_at": 1000,
            "expires_at": 2000
        })
    }

    #[test]
    fn parses_a_grant_and_drops_stale_ice() {
        let access = Access::parse(body()).unwrap();
        assert_eq!(access.server, "wss://signaling.example");
        assert!(access.key("2026-11-17").is_some() && access.key("2026-11-16").is_none());
        assert_eq!(access.usable_ice(900).len(), 1);
        assert!(access.usable_ice(970).is_empty());
    }

    #[test]
    fn rejects_inconsistent_grants() {
        let mut wrong_topic = body();
        wrong_topic["topic"] = json!("bot-screens:other:v1");
        assert!(Access::parse(wrong_topic).is_err());
        let mut plain_ws = body();
        plain_ws["signaling_servers"] = json!(["ws://signaling.example"]);
        assert!(Access::parse(plain_ws).is_err());
        let mut no_keys = body();
        no_keys["keys"] = json!([]);
        assert!(Access::parse(no_keys).is_err());
        let mut bad_ice = body();
        bad_ice["ice_servers"] = json!([{ "urls": ["https://not-ice.example"] }]);
        assert!(Access::parse(bad_ice).is_err());
    }
}
