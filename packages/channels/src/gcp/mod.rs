//! Firebase Realtime Database transport.
//!
//! Layout under the database root (rules in `database.rules.json`, exported as
//! [`DATABASE_RULES`]):
//!
//! ```text
//! /channels/{channel_id}/inbox/{request_id}   { "payload": "<ChannelPush JSON>" }   client create-only, replies
//! /channels/{channel_id}/inbound/{push_id}    { "payload": "<ChannelPush JSON>" }   client create-only, inbound/cancel
//! /channels/{channel_id}/meta                 { "owner", "created_at" }             API only (bypasses rules)
//! ```
//!
//! Clients sign in with a custom token whose claims are [`client_claims`]; the executor signs in
//! as [`EXECUTOR_UID`] with [`executor_claims`] and streams both collections over REST SSE. The
//! server role may only read its own run's collections and delete its own run node; the client
//! may only create single-`payload` children. The database has no TTL: the executor deletes its
//! channel on close and the API sweeps stale runs by `meta/created_at`.

mod auth;
mod channel;
mod forwarder;
mod router;
mod stream;
mod token;

use flow_like_types::anyhow;
use reqwest::Url;
use serde_json::{Value, json};

pub use channel::FirebaseRtdbChannel;
pub use forwarder::{AccessTokenProvider, FirebaseRtdbForwarder};
pub use token::{
    FIREBASE_CUSTOM_TOKEN_AUDIENCE, MAX_CUSTOM_CLAIMS_BYTES, MAX_CUSTOM_TOKEN_TTL_SECS,
    MAX_UID_CHARS, RESERVED_CLAIMS, custom_token,
};

/// `uid` the executor signs in with; rules key on the `role` claim, not the uid.
pub const EXECUTOR_UID: &str = "svc";
/// Security rules implementing the layout above; deploy verbatim as the database's rules.
pub const DATABASE_RULES: &str = include_str!("database.rules.json");
/// Realtime Database key limit.
pub const MAX_CHANNEL_ID_BYTES: usize = 768;
/// Upper bound the rules enforce on the `payload` string of a client-created node.
pub const MAX_PAYLOAD_CHARS: usize = 16384;

const FORBIDDEN_KEY_CHARS: &[char] = &['.', '$', '#', '[', ']', '/'];

pub fn client_claims(channel_id: &str) -> Value {
    json!({ "run_id": channel_id, "role": "client" })
}

pub fn executor_claims(channel_id: &str) -> Value {
    json!({ "run_id": channel_id, "role": "server" })
}

pub fn validate_channel_id(channel_id: &str) -> flow_like_types::Result<()> {
    validate_key("channel id", channel_id)
}

pub fn channel_path(channel_id: &str) -> flow_like_types::Result<String> {
    validate_channel_id(channel_id)?;
    Ok(format!("/channels/{channel_id}"))
}

pub fn inbox_path(channel_id: &str) -> flow_like_types::Result<String> {
    Ok(format!("{}/inbox", channel_path(channel_id)?))
}

pub fn inbound_path(channel_id: &str) -> flow_like_types::Result<String> {
    Ok(format!("{}/inbound", channel_path(channel_id)?))
}

pub fn meta_path(channel_id: &str) -> flow_like_types::Result<String> {
    Ok(format!("{}/meta", channel_path(channel_id)?))
}

pub(crate) fn validate_key(label: &str, key: &str) -> flow_like_types::Result<()> {
    if key.is_empty() {
        return Err(anyhow!("firebase {label} is empty"));
    }
    if key.len() > MAX_CHANNEL_ID_BYTES {
        return Err(anyhow!(
            "firebase {label} is {} bytes, the database key limit is {MAX_CHANNEL_ID_BYTES}",
            key.len()
        ));
    }
    if let Some(bad) = key
        .chars()
        .find(|c| FORBIDDEN_KEY_CHARS.contains(c) || c.is_control())
    {
        return Err(anyhow!(
            "firebase {label} '{key}' contains {bad:?}, which is not allowed in a database key"
        ));
    }
    Ok(())
}

pub(crate) fn database_root(database_url: &str) -> flow_like_types::Result<Url> {
    let url = Url::parse(database_url.trim())
        .map_err(|err| anyhow!("firebase database url is invalid: {err}"))?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err(anyhow!(
            "firebase database url must be an http(s) url with a host"
        ));
    }
    Ok(url)
}

/// `{root}/{segments...}.json`, every segment percent-encoded.
pub(crate) fn json_url(root: &Url, segments: &[&str]) -> flow_like_types::Result<Url> {
    let (last, head) = segments
        .split_last()
        .ok_or_else(|| anyhow!("firebase node path is empty"))?;
    let mut url = root.clone();
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow!("firebase database url cannot carry a path"))?;
        path.pop_if_empty();
        path.extend(head);
        path.push(&format!("{last}.json"));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

pub(crate) fn path_segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_the_layout() {
        assert_eq!(inbox_path("run-1").unwrap(), "/channels/run-1/inbox");
        assert_eq!(inbound_path("run-1").unwrap(), "/channels/run-1/inbound");
        assert_eq!(meta_path("run-1").unwrap(), "/channels/run-1/meta");
        assert_eq!(channel_path("run-1").unwrap(), "/channels/run-1");
    }

    #[test]
    fn channel_ids_are_validated() {
        for bad in [
            "", "a.b", "a$b", "a#b", "a[b", "a]b", "a/b", "a\nb", "a\u{7}b",
        ] {
            assert!(
                validate_channel_id(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
        assert!(validate_channel_id(&"x".repeat(769)).is_err());
        assert!(validate_channel_id(&"x".repeat(768)).is_ok());
        assert!(validate_channel_id("clq7x2y0000008l4h5g6z7k8").is_ok());
        assert!(inbox_path("a/b").is_err());
    }

    #[test]
    fn claims_carry_run_and_role() {
        assert_eq!(
            client_claims("run-9"),
            json!({ "run_id": "run-9", "role": "client" })
        );
        assert_eq!(
            executor_claims("run-9"),
            json!({ "run_id": "run-9", "role": "server" })
        );
        assert_eq!(EXECUTOR_UID, "svc");
    }

    #[test]
    fn json_urls_are_encoded_and_suffixed() {
        let root = database_root("https://demo.europe-west1.firebasedatabase.app/").unwrap();
        let url = json_url(&root, &["channels", "run 1", "inbox", "r?d"]).unwrap();
        assert_eq!(
            url.as_str(),
            "https://demo.europe-west1.firebasedatabase.app/channels/run%201/inbox/r%3Fd.json"
        );
        let url = json_url(&root, &path_segments("/channels/run-1/inbox")).unwrap();
        assert_eq!(
            url.as_str(),
            "https://demo.europe-west1.firebasedatabase.app/channels/run-1/inbox.json"
        );
        assert!(json_url(&root, &[]).is_err());
        assert!(database_root("ftp://x").is_err());
        assert!(database_root("not a url").is_err());
    }

    #[test]
    fn invalid_database_urls_do_not_expose_secrets() {
        for url in [
            "https://secret-user:secret-password@[invalid/?auth=secret-token",
            "ftp://secret-user:secret-password@example.com/?auth=secret-token",
            "not-a-url?auth=secret-token",
        ] {
            let error = database_root(url).unwrap_err();
            let diagnostic = format!("{error:?}");
            assert!(diagnostic.contains("firebase database url"));
            assert!(!diagnostic.contains("secret-"));
            assert!(!diagnostic.contains(url));
        }

        let opaque_root = Url::parse("data:secret-token").unwrap();
        let error = json_url(&opaque_root, &["channels"]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "firebase database url cannot carry a path"
        );
    }

    fn assert_no_open_grants(node: &Value, path: &str) {
        let Value::Object(map) = node else { return };
        for (key, value) in map {
            if (key == ".read" || key == ".write") && value == &Value::Bool(true) {
                panic!("{path}/{key} grants unconditional access");
            }
            assert_no_open_grants(value, &format!("{path}/{key}"));
        }
    }

    #[test]
    fn rules_are_scoped_to_the_run() {
        let rules: Value = serde_json::from_str(DATABASE_RULES).expect("rules must be JSON");
        assert_no_open_grants(&rules, "");
        let root = &rules["rules"];
        assert_eq!(root[".read"], Value::Bool(false));
        assert_eq!(root[".write"], Value::Bool(false));
        assert!(root["channels"].get(".read").is_none());
        assert!(root["channels"].get(".write").is_none());

        let run = &root["channels"]["$channel_id"];
        let run_write = run[".write"].as_str().unwrap();
        assert!(run_write.contains("auth.token.role === 'server'"));
        assert!(run_write.contains("auth.token.run_id === $channel_id"));
        assert!(run_write.contains("!newData.exists()"));
        assert!(run.get("meta").is_none());

        for (collection, key) in [("inbox", "$request_id"), ("inbound", "$push_id")] {
            let node = &run[collection];
            let read = node[".read"].as_str().unwrap();
            assert!(read.contains("auth.token.role === 'server'"));
            assert!(read.contains("auth.token.run_id === $channel_id"));
            assert!(node.get(".write").is_none());
            let child = &node[key];
            let write = child[".write"].as_str().unwrap();
            assert!(write.contains("auth.token.role === 'client'"));
            assert!(write.contains("auth.token.run_id === $channel_id"));
            assert!(write.contains("!data.exists()"));
            let validate = child[".validate"].as_str().unwrap();
            assert!(validate.contains("hasChildren(['payload'])"));
            assert!(validate.contains(&MAX_PAYLOAD_CHARS.to_string()));
            assert!(
                child["payload"][".validate"]
                    .as_str()
                    .unwrap()
                    .contains("isString()")
            );
            assert_eq!(child["$other"][".validate"], Value::Bool(false));
        }
    }
}
