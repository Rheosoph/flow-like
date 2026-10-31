use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_types::channel::now_unix;
use flow_like_types::{Result, anyhow, bail};
use nkeys::KeyPair;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Client,
    Executor,
}

pub fn subject_for(channel_id: &str) -> Result<String> {
    if !super::valid_delivery_id(channel_id) {
        bail!(
            "NATS channel id must contain 1 to 128 ASCII letters, digits, underscores or hyphens"
        );
    }
    Ok(format!("channel.{channel_id}.inbox"))
}

pub fn validate_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url).map_err(|_| anyhow!("invalid NATS channel URL"))?;
    if !matches!(parsed.scheme(), "nats" | "tls" | "ws" | "wss")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || (matches!(parsed.scheme(), "nats" | "tls") && !matches!(parsed.path(), "" | "/"))
        || parsed.port() == Some(0)
    {
        bail!(
            "NATS channel URL must use nats, tls, ws or wss without credentials, query or fragment"
        );
    }
    Ok(())
}

pub fn validate_account_seed(seed: &str) -> Result<()> {
    account_key(seed).map(|_| ())
}

fn account_key(seed: &str) -> Result<KeyPair> {
    let key = KeyPair::from_seed(seed).map_err(|_| anyhow!("invalid NATS account signing seed"))?;
    if !key.public_key().starts_with('A') {
        bail!("NATS channel signing seed must belong to an account");
    }
    Ok(key)
}

pub fn mint_user_token(
    account_seed: &str,
    channel_id: &str,
    role: Role,
    ttl_secs: i64,
) -> Result<String> {
    if ttl_secs <= 0 || ttl_secs > 9 * 60 * 60 {
        bail!("NATS channel credential lifetime must be between 1 second and 9 hours");
    }
    mint_user_token_until(account_seed, channel_id, role, now_unix() + ttl_secs)
}

/// Preserve the original deadline when refreshing a credential.
pub fn mint_user_token_until(
    account_seed: &str,
    channel_id: &str,
    role: Role,
    expires_at: i64,
) -> Result<String> {
    let now = now_unix();
    if expires_at <= now || expires_at > now + 9 * 60 * 60 {
        bail!("NATS channel credential deadline must be within the next 9 hours");
    }
    let key = account_key(account_seed)?;
    let subject = subject_for(channel_id)?;
    let ack = format!("{subject}.ack.>");
    let (publish, subscribe) = match role {
        Role::Client => (subject, ack),
        Role::Executor => (ack, subject),
    };
    sign(
        &key,
        json!({
            "iss": key.public_key(), "sub": KeyPair::new_user().public_key(),
            "iat": now, "exp": expires_at,
            "nats": {
                "pub": {"allow": [publish]}, "sub": {"allow": [subscribe]},
                "subs": 32, "data": -1, "payload": super::MAX_PAYLOAD_BYTES,
                "bearer_token": true, "type": "user", "version": 2
            }
        }),
    )
}

pub(crate) fn sign(key: &KeyPair, mut claims: Value) -> Result<String> {
    // NATS uses a base32 digest of claims before jti is inserted.
    claims["jti"] = data_encoding::BASE32_NOPAD
        .encode(&Sha256::digest(serde_json::to_vec(&claims)?))
        .into();
    let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ed25519-nkey"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?);
    let unsigned = format!("{header}.{payload}");
    let signature = key
        .sign(unsigned.as_bytes())
        .map_err(|_| anyhow!("NATS credential signing failed"))?;
    Ok(format!("{unsigned}.{}", URL_SAFE_NO_PAD.encode(signature)))
}
