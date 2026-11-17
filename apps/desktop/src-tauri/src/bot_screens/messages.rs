use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const MAX_SEALED_BYTES: usize = 48 * 1024;
const MAX_FIELD_BYTES: usize = 512;
const MAX_SKEW_MS: i64 = 300_000;

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Incoming {
    Hello {
        from: String,
        bot_id: String,
        at: i64,
    },
    Offer {
        from: String,
        to: String,
        bot_id: String,
        session: String,
        sealed: serde_json::Value,
        at: i64,
    },
    Pair {
        from: String,
        to: String,
        session: String,
        key_id: String,
        public_key: String,
        label: String,
        mac: String,
        at: i64,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Outgoing {
    Here {
        from: String,
        to: String,
        bot_id: String,
        active: bool,
        key_id: String,
        label: String,
        at: i64,
    },
    Paired {
        from: String,
        to: String,
        session: String,
        mac: String,
        at: i64,
    },
    Answer {
        from: String,
        to: String,
        session: String,
        sealed: serde_json::Value,
        at: i64,
    },
    Refused {
        from: String,
        to: String,
        session: String,
        code: RefusedCode,
        message: String,
        at: i64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum RefusedCode {
    Busy,
    Failed,
    NoKey,
    NotPaired,
}

pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn token(value: &str, min: usize, max: usize, extra: &[char]) -> bool {
    (min..=max).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || extra.contains(&c))
}

fn peer_id(value: &str) -> bool {
    token(value, 8, 64, &[])
}

fn bot_id(value: &str) -> bool {
    token(value, 1, 128, &['.'])
}

fn fresh(at: i64, now: i64) -> bool {
    (now - at).abs() <= MAX_SKEW_MS
}

impl Incoming {
    pub fn parse(plaintext: &[u8]) -> Option<Self> {
        serde_json::from_slice(plaintext).ok()
    }

    /// A message that is well formed and fresh; anything else is dropped without an answer.
    pub fn valid(&self, now: i64) -> bool {
        match self {
            Self::Hello {
                from,
                bot_id: bot,
                at,
            } => peer_id(from) && bot_id(bot) && fresh(*at, now),
            Self::Offer {
                from,
                to,
                bot_id: bot,
                session,
                sealed,
                at,
            } => {
                peer_id(from)
                    && peer_id(to)
                    && peer_id(session)
                    && bot_id(bot)
                    && sealed.is_object()
                    && sealed.to_string().len() <= MAX_SEALED_BYTES
                    && fresh(*at, now)
            }
            Self::Pair {
                from,
                to,
                session,
                key_id,
                public_key,
                label,
                mac,
                at,
            } => {
                peer_id(from)
                    && peer_id(to)
                    && peer_id(session)
                    && token(key_id, 1, 64, &['.'])
                    && public_key.len() <= MAX_FIELD_BYTES
                    && label.len() <= MAX_FIELD_BYTES
                    && mac.len() <= MAX_FIELD_BYTES
                    && fresh(*at, now)
            }
            Self::Other => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_763_337_600_000;

    fn hello(from: &str, bot: &str, at: i64) -> Incoming {
        Incoming::Hello {
            from: from.into(),
            bot_id: bot.into(),
            at,
        }
    }

    fn offer(sealed: serde_json::Value, at: i64) -> Incoming {
        Incoming::Offer {
            from: "viewer0123456789".into(),
            to: "host01234567890".into(),
            bot_id: "bot-1".into(),
            session: "session-12345".into(),
            sealed,
            at,
        }
    }

    #[test]
    fn parses_the_vector_plaintext() {
        let plain =
            br#"{"type":"hello","from":"viewer0123456789","bot_id":"bot-1","at":1763337600000}"#;
        let parsed = Incoming::parse(plain).unwrap();
        assert_eq!(parsed, hello("viewer0123456789", "bot-1", NOW));
        assert!(parsed.valid(NOW));
    }

    #[test]
    fn unknown_types_and_garbage_are_not_valid() {
        assert!(
            !Incoming::parse(br#"{"type":"answer","from":"x"}"#)
                .unwrap()
                .valid(NOW)
        );
        assert!(Incoming::parse(b"not json").is_none());
        assert!(Incoming::parse(br#"{"type":"hello"}"#).is_none());
    }

    #[test]
    fn ids_are_bounded() {
        assert!(hello("viewer01", "b", NOW).valid(NOW));
        assert!(!hello("short", "bot", NOW).valid(NOW));
        assert!(!hello(&"a".repeat(65), "bot", NOW).valid(NOW));
        assert!(!hello("viewer01", "", NOW).valid(NOW));
        assert!(!hello("viewer01", &"b".repeat(129), NOW).valid(NOW));
        assert!(hello("viewer01", "bot.v1_x-2", NOW).valid(NOW));
        assert!(!hello("viewer01", "bot/../x", NOW).valid(NOW));
        assert!(!hello("view er01", "bot", NOW).valid(NOW));
    }

    #[test]
    fn the_time_window_is_five_minutes_either_way() {
        assert!(hello("viewer01", "bot", NOW - 300_000).valid(NOW));
        assert!(hello("viewer01", "bot", NOW + 300_000).valid(NOW));
        assert!(!hello("viewer01", "bot", NOW - 300_001).valid(NOW));
        assert!(!hello("viewer01", "bot", NOW + 300_001).valid(NOW));
    }

    #[test]
    fn the_sealed_offer_is_an_object_capped_at_48_kib() {
        let sized = |bytes: usize| serde_json::json!({ "ct": "v".repeat(bytes - 9) });
        assert!(offer(sized(MAX_SEALED_BYTES), NOW).valid(NOW));
        assert!(!offer(sized(MAX_SEALED_BYTES + 1), NOW).valid(NOW));
        assert!(!offer(serde_json::json!("sdp"), NOW).valid(NOW));
        assert!(!offer(serde_json::Value::Null, NOW).valid(NOW));
    }

    #[test]
    fn an_offer_parses_with_its_sealed_body() {
        let plain = br#"{"type":"offer","from":"viewer0123456789","to":"host01234567890","bot_id":"bot-1","session":"session-12345","sealed":{"v":1},"at":1763337600000}"#;
        assert!(Incoming::parse(plain).unwrap().valid(NOW));
    }

    #[test]
    fn outgoing_messages_carry_their_type_tag() {
        let refused = serde_json::to_value(Outgoing::Refused {
            from: "h".into(),
            to: "v".into(),
            session: "s".into(),
            code: RefusedCode::Busy,
            message: "m".into(),
            at: 1,
        })
        .unwrap();
        assert_eq!(refused["type"], "refused");
        assert_eq!(refused["code"], "busy");
        assert_eq!(serde_json::to_value(RefusedCode::NoKey).unwrap(), "no_key");
        assert_eq!(
            serde_json::to_value(RefusedCode::NotPaired).unwrap(),
            "not_paired"
        );
    }

    #[test]
    fn a_pair_message_parses_and_is_bounded() {
        let plain = |label: &str| {
            format!(
                r#"{{"type":"pair","from":"viewer0123456789","to":"host01234567890","session":"session-12345","key_id":"ik_a1a2a3a4a5a6a7a8","public_key":"pk","label":"{label}","mac":"m","at":1763337600000}}"#
            )
        };
        assert!(
            Incoming::parse(plain("Phone").as_bytes())
                .unwrap()
                .valid(NOW)
        );
        let long = "x".repeat(MAX_FIELD_BYTES + 1);
        assert!(!Incoming::parse(plain(&long).as_bytes()).unwrap().valid(NOW));
    }
}
