use aes_gcm::aead::{OsRng, rand_core::RngCore};
use flow_like_device_crypto::inbox::{
    InboxSecret, Member, RecipientKey, Scope, open_with, seal_all,
};
use flow_like_types::{
    Result, anyhow,
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::records::SECRET_BYTES;

#[derive(Deserialize)]
struct OfferBody {
    sdp: String,
    secret: String,
    bot_id: String,
    key_id: String,
}

#[derive(Serialize)]
struct AnswerBody<'a> {
    sdp: &'a str,
    secret: String,
}

pub(super) struct Offer {
    pub sdp: String,
    pub secret: Vec<u8>,
    pub bot_id: String,
    pub key_id: String,
}

pub(super) enum OfferError {
    NoKey,
    Invalid(String),
}

fn conversation(session: &str) -> String {
    format!("bot-screen:{session}")
}

fn scope<'a>(conversation: &'a str, message_id: &'a str, sub: &'a str) -> Scope<'a> {
    Scope {
        conversation_id: conversation,
        message_id,
        sender_id: sub,
    }
}

pub(super) fn random_secret() -> [u8; SECRET_BYTES] {
    let mut secret = [0u8; SECRET_BYTES];
    OsRng.fill_bytes(&mut secret);
    secret
}

/// The viewer's offer, opened with this person's keys. The sealed bot id must be the outer one.
pub(super) fn open_offer(
    secrets: &[InboxSecret],
    sub: &str,
    session: &str,
    outer_bot_id: &str,
    sealed: &Value,
) -> Result<Offer, OfferError> {
    let conversation = conversation(session);
    let plaintext = open_with(secrets, &scope(&conversation, "offer", sub), sub, sealed)
        .map_err(|_| OfferError::NoKey)?;
    let body: OfferBody = serde_json::from_slice(&plaintext)
        .map_err(|error| OfferError::Invalid(format!("The sealed offer is malformed: {error}")))?;
    let secret = B64
        .decode(&body.secret)
        .ok()
        .filter(|secret| secret.len() == SECRET_BYTES)
        .ok_or_else(|| {
            OfferError::Invalid(format!(
                "The sealed offer secret is not {SECRET_BYTES} base64url bytes"
            ))
        })?;
    if body.bot_id != outer_bot_id {
        return Err(OfferError::Invalid(
            "The sealed offer names another bot than the message".to_owned(),
        ));
    }
    Ok(Offer {
        sdp: body.sdp,
        secret,
        bot_id: body.bot_id,
        key_id: body.key_id,
    })
}

pub(super) fn seal_answer(
    sub: &str,
    viewer_key: &RecipientKey,
    session: &str,
    sdp: &str,
    host_secret: &[u8],
) -> Result<Value> {
    let conversation = conversation(session);
    let body = serde_json::to_vec(&AnswerBody {
        sdp,
        secret: B64.encode(host_secret),
    })?;
    let member = Member {
        principal_id: sub.to_owned(),
        keys: vec![viewer_key.clone()],
    };
    seal_all(&scope(&conversation, "answer", sub), &[member], &body)
        .map_err(|error| anyhow!("Seal the bot screen answer: {error}"))?
        .remove(sub)
        .ok_or_else(|| anyhow!("Sealing the bot screen answer produced no envelope for {sub}"))
}

/// The `sub` claim of a JWT. The token is the hub's own for this device, so it is not verified.
pub(super) fn sub_of_jwt(jwt: &str) -> Option<String> {
    let payload = B64.decode(jwt.split('.').nth(1)?).ok()?;
    serde_json::from_slice::<Value>(&payload)
        .ok()?
        .get("sub")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SUB: &str = "user-1";
    const SESSION: &str = "session-12345";

    fn owner_key(secret: &InboxSecret) -> RecipientKey {
        let public = secret.public();
        RecipientKey {
            key_id: public.key_id.clone(),
            public_key: public.public_key.clone(),
        }
    }

    fn sealed_offer(secret: &InboxSecret, session: &str, body: Value) -> Value {
        let conversation = conversation(session);
        let member = Member {
            principal_id: SUB.to_owned(),
            keys: vec![owner_key(secret)],
        };
        seal_all(
            &scope(&conversation, "offer", SUB),
            &[member],
            body.to_string().as_bytes(),
        )
        .unwrap()
        .remove(SUB)
        .unwrap()
    }

    fn offer_body(bot: &str) -> Value {
        json!({ "sdp": "v=0", "secret": B64.encode([7u8; 32]), "bot_id": bot, "key_id": "ik_viewer" })
    }

    #[test]
    fn opens_an_offer_sealed_to_this_devices_key() {
        let secret = InboxSecret::generate();
        let sealed = sealed_offer(&secret, SESSION, offer_body("bot-1"));
        let offer = open_offer(&[secret], SUB, SESSION, "bot-1", &sealed)
            .ok()
            .unwrap();
        assert_eq!(offer.sdp, "v=0");
        assert_eq!(offer.secret, vec![7u8; 32]);
        assert_eq!(offer.bot_id, "bot-1");
        assert_eq!(offer.key_id, "ik_viewer");
    }

    #[test]
    fn another_devices_key_cannot_open_it() {
        let sealed = sealed_offer(&InboxSecret::generate(), SESSION, offer_body("bot-1"));
        let result = open_offer(&[InboxSecret::generate()], SUB, SESSION, "bot-1", &sealed);
        assert!(matches!(result, Err(OfferError::NoKey)));
    }

    #[test]
    fn the_offer_is_bound_to_its_session() {
        let secret = InboxSecret::generate();
        let sealed = sealed_offer(&secret, SESSION, offer_body("bot-1"));
        let result = open_offer(&[secret], SUB, "session-other-1", "bot-1", &sealed);
        assert!(matches!(result, Err(OfferError::NoKey)));
    }

    #[test]
    fn a_sealed_bot_that_differs_from_the_outer_one_is_invalid() {
        let secret = InboxSecret::generate();
        let sealed = sealed_offer(&secret, SESSION, offer_body("bot-1"));
        let result = open_offer(&[secret], SUB, SESSION, "bot-2", &sealed);
        assert!(matches!(result, Err(OfferError::Invalid(_))));
    }

    #[test]
    fn a_short_secret_is_invalid() {
        let secret = InboxSecret::generate();
        let mut body = offer_body("bot-1");
        body["secret"] = json!(B64.encode([1u8; 8]));
        let sealed = sealed_offer(&secret, SESSION, body);
        let result = open_offer(&[secret], SUB, SESSION, "bot-1", &sealed);
        assert!(matches!(result, Err(OfferError::Invalid(_))));
    }

    #[test]
    fn the_answer_opens_with_the_owners_key_and_carries_the_host_secret() {
        let secret = InboxSecret::generate();
        let sealed =
            seal_answer(SUB, &owner_key(&secret), SESSION, "v=0 answer", &[9u8; 32]).unwrap();
        let conversation = conversation(SESSION);
        let opened = open_with(
            &[secret],
            &scope(&conversation, "answer", SUB),
            SUB,
            &sealed,
        )
        .unwrap();
        let body: Value = serde_json::from_slice(&opened).unwrap();
        assert_eq!(body["sdp"], "v=0 answer");
        assert_eq!(body["secret"], B64.encode([9u8; 32]));
    }

    #[test]
    fn the_answer_is_sealed_only_to_the_given_key() {
        let pinned = InboxSecret::generate();
        let other = InboxSecret::generate();
        let sealed = seal_answer(SUB, &owner_key(&pinned), SESSION, "v=0", &[9u8; 32]).unwrap();
        let conversation = conversation(SESSION);
        let scope = scope(&conversation, "answer", SUB);
        assert!(open_with(&[other], &scope, SUB, &sealed).is_err());
        assert!(open_with(&[pinned], &scope, SUB, &sealed).is_ok());
    }

    #[test]
    fn sub_comes_from_the_jwt_payload() {
        let payload = B64.encode(json!({ "sub": "user-1" }).to_string());
        assert_eq!(
            sub_of_jwt(&format!("h.{payload}.s")).as_deref(),
            Some("user-1")
        );
        assert_eq!(sub_of_jwt("garbage"), None);
    }
}
