//! Access to the account-scoped signaling topic where desktop apps answer viewers of a bot's screen.

use crate::{
    backend_jwt::{self, TokenType, issuer, make_time_claims},
    error::ApiError,
    middleware::jwt::AppUser,
    realtime_ice::RealtimeIceServer,
    routes::inbox::presence::length_prefixed,
    state::AppState,
};
use axum::{Extension, Json, extract::State, http::header, response::IntoResponse};
use chrono::NaiveDate;
use flow_like_types::{
    anyhow,
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD},
    create_id,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const ROOM_CONTEXT: &str = "flow-like.bot-screens-room/v1";
const KEY_CONTEXT: &str = "flow-like.bot-screens-key/v1";
const ROOM_HEX: usize = 32;
const SCOPE: &str = "realtime.read";
const NO_SIGNALING: &str =
    "Live bot screens need a signaling server; none is configured on this hub";

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BotScreenClaims {
    pub sub: String,
    pub bot_screens: String,
    pub scope: String,
    #[serde(rename = "typ")]
    pub token_type: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotScreenKey {
    /// UTC date the key belongs to, `YYYY-MM-DD`.
    #[schema(example = "2026-11-17")]
    pub id: String,
    /// Base64url (no padding) 256-bit key.
    pub key: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotScreenAccess {
    /// JWT authorizing the signaling connection for this account's bot-screen topic.
    pub jwt: String,
    /// 32 lowercase hex characters, stable per account.
    pub room: String,
    /// `bot-screens:{room}:v1`.
    pub topic: String,
    /// Today's key first, then yesterday's.
    pub keys: Vec<BotScreenKey>,
    pub signaling_servers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ice_servers: Option<Vec<RealtimeIceServer>>,
    /// Expiry of the ICE credentials as a Unix timestamp in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ice_expires_at: Option<i64>,
    /// Expiry of the JWT as a Unix timestamp in seconds.
    pub expires_at: i64,
}

pub(crate) fn room_for(sub: &str) -> String {
    let derived = blake3::derive_key(ROOM_CONTEXT, sub.as_bytes());
    blake3::Hash::from_bytes(derived).to_hex()[..ROOM_HEX].to_string()
}

pub(crate) fn key_for(master: &[u8; 32], sub: &str, date: NaiveDate) -> BotScreenKey {
    let id = date.format("%Y-%m-%d").to_string();
    let mut material = blake3::Hasher::new();
    material.update(master);
    length_prefixed(&mut material, sub);
    length_prefixed(&mut material, &id);
    let key = blake3::derive_key(KEY_CONTEXT, material.finalize().as_bytes());
    BotScreenKey {
        id,
        key: URL_SAFE_NO_PAD.encode(key),
    }
}

pub(crate) fn keys_for(master: &[u8; 32], sub: &str, today: NaiveDate) -> Vec<BotScreenKey> {
    [Some(today), today.pred_opt()]
        .into_iter()
        .flatten()
        .map(|date| key_for(master, sub, date))
        .collect()
}

pub(crate) fn claims_for(sub: String, room: &str, jti: String) -> BotScreenClaims {
    let time = make_time_claims(TokenType::Realtime, None);
    BotScreenClaims {
        sub,
        bot_screens: room.to_string(),
        scope: SCOPE.to_string(),
        token_type: TokenType::Realtime,
        iss: issuer().to_string(),
        aud: TokenType::Realtime.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti,
    }
}

pub(crate) fn person_sub(user: &AppUser) -> Result<String, ApiError> {
    if !matches!(user, AppUser::OpenID(_) | AppUser::PAT(_)) {
        return Err(ApiError::forbidden(
            "Only people get live bot screen access, not bots, API keys or running flows",
        ));
    }
    user.sub()
}

#[utoipa::path(
    post,
    path = "/realtime/bot-screens",
    tag = "Realtime",
    responses(
        (status = 200, description = "Access to your account's live bot screen topic: a short-lived token, the room, today's and yesterday's keys and the signaling servers. Lets the app you are using watch the screen a bot drives on one of your desktop apps.", body = BotScreenAccess),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people get live bot screen access, not bots, API keys or running flows"),
        (status = 502, description = "The connection server provider is unavailable"),
        (status = 503, description = "This hub has no signaling server configured")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /realtime/bot-screens", skip(state, user))]
pub async fn bot_screens(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<impl IntoResponse, ApiError> {
    let sub = person_sub(&user)?;
    let signaling_servers = state
        .platform_config
        .signaling
        .clone()
        .filter(|servers| !servers.is_empty())
        .ok_or_else(|| ApiError::service_unavailable(NO_SIGNALING))?;

    let jti = create_id();
    let issued = state.realtime_ice.issue(&jti).await.map_err(|error| {
        tracing::error!(error = %error, "Failed to issue realtime ICE credentials");
        ApiError::bad_gateway("Realtime ICE credentials are temporarily unavailable")
    })?;
    let (ice_servers, ice_expires_at) = match issued {
        Some(issued) => (Some(issued.ice_servers), Some(issued.expires_at)),
        None => (None, None),
    };

    let room = room_for(&sub);
    let keys = keys_for(&state.encryption_key, &sub, chrono::Utc::now().date_naive());
    let claims = claims_for(sub, &room, jti);
    let expires_at = claims.exp;
    let jwt = backend_jwt::sign(&claims)
        .map_err(|e| ApiError::internal_error(anyhow!("Realtime not configured: {}", e)))?;

    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(BotScreenAccess {
            topic: format!("bot-screens:{room}:v1"),
            jwt,
            room,
            keys,
            signaling_servers,
            ice_servers,
            ice_expires_at,
            expires_at,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::{BotUser, OpenIDUser};
    use axum::http::StatusCode;

    const MASTER: [u8; 32] = [7; 32];

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    #[test]
    fn the_room_is_32_lowercase_hex_stable_per_sub_and_distinct_across_subs() {
        let room = room_for("user-a");
        assert_eq!(room.len(), 32);
        assert!(room.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        assert_eq!(room, room_for("user-a"));
        assert_ne!(room, room_for("user-b"));
    }

    #[test]
    fn keys_differ_per_date_per_sub_and_per_master() {
        let a = key_for(&MASTER, "user-a", day(17));
        assert_eq!(a, key_for(&MASTER, "user-a", day(17)));
        assert_eq!(a.id, "2026-11-17");
        assert_ne!(a.key, key_for(&MASTER, "user-a", day(16)).key);
        assert_ne!(a.key, key_for(&MASTER, "user-b", day(17)).key);
        assert_ne!(a.key, key_for(&[8; 32], "user-a", day(17)).key);
        let raw = URL_SAFE_NO_PAD.decode(&a.key).unwrap();
        assert_eq!(raw.len(), 32);
        assert!(!a.key.contains('='));
    }

    #[test]
    fn keys_list_today_then_yesterday() {
        let keys = keys_for(&MASTER, "user-a", day(1));
        let ids: Vec<_> = keys.iter().map(|k| k.id.as_str()).collect();
        assert_eq!(ids, ["2026-11-01", "2026-10-31"]);
        assert_eq!(keys[0], key_for(&MASTER, "user-a", day(1)));
    }

    #[test]
    fn the_claims_carry_bot_screens_and_no_other_kind() {
        let room = room_for("user-a");
        let wire = serde_json::to_value(claims_for("user-a".into(), &room, "jti".into())).unwrap();
        assert_eq!(wire["bot_screens"], room.as_str());
        assert_eq!(wire["sub"], "user-a");
        assert_eq!(wire["scope"], "realtime.read");
        assert_eq!(wire["typ"], "realtime");
        for absent in [
            "conversation_id",
            "app_id",
            "board_id",
            "board_format_version",
        ] {
            assert!(wire.get(absent).is_none(), "{absent}");
        }
    }

    #[test]
    fn only_people_get_access() {
        let person = AppUser::OpenID(OpenIDUser {
            sub: "user-a".into(),
            access_token: String::new(),
        });
        assert_eq!(person_sub(&person).unwrap(), "user-a");
        let bot = AppUser::Bot(BotUser {
            bot_id: "bot_1".into(),
            turn_id: "bt_1".into(),
            conv_id: None,
        });
        assert_eq!(
            person_sub(&bot).unwrap_err().status(),
            StatusCode::FORBIDDEN
        );
        assert!(person_sub(&AppUser::Unauthorized).is_err());
    }
}
