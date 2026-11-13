//! A container can ask its parent to sign only its own bot's lifecycle requests.

use anyhow::{Result, bail, ensure};
use flow_like_types::bot::BotExecutionMode;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Poll,
    Claim(String),
    Turn { id: String, terminal: bool },
    PublishKey,
    Forget,
}

/// This parser deliberately accepts canonical paths only, without percent encoding or extras.
pub(super) fn action(method: &str, path: &str, device: &str) -> Result<Action> {
    let prefix = format!("/devices/{device}/bots/");
    let tail = path
        .strip_prefix(&prefix)
        .ok_or_else(|| anyhow::anyhow!("The container cannot call that device route"))?;
    match (method, tail) {
        ("POST", "poll") => return Ok(Action::Poll),
        ("POST", "inbox-keys") => return Ok(Action::PublishKey),
        ("POST", "hosts/forget") => return Ok(Action::Forget),
        _ => {}
    }
    let (route, query) = tail
        .split_once('?')
        .map_or((tail, None), |(a, b)| (a, Some(b)));
    let parts: Vec<_> = route.split('/').collect();
    ensure!(
        parts.len() == 3 && parts[0] == "turns",
        "The container cannot call that device route"
    );
    let id = parts[1];
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "Invalid container turn id"
    );
    if method == "GET" && parts[2] == "checkpoint" {
        let mut seen = std::collections::HashSet::new();
        for (key, value) in url::form_urlencoded::parse(query.unwrap_or("").as_bytes()) {
            ensure!(
                seen.insert(key.to_string()),
                "Repeated checkpoint query parameter"
            );
            match key.as_ref() {
                "host_id" => ensure!(value == super::host_id(device), "Wrong container host id"),
                "attempt" => {
                    value.parse::<u32>()?;
                }
                "monitor_baseline" => ensure!(value == "true", "Invalid checkpoint query"),
                _ => bail!("The container cannot set that checkpoint query parameter"),
            }
        }
        ensure!(
            seen.contains("host_id") && seen.contains("attempt"),
            "Incomplete checkpoint query"
        );
        return Ok(Action::Turn {
            id: id.into(),
            terminal: false,
        });
    }
    ensure!(query.is_none(), "Unexpected container query parameters");
    match (method, parts[2]) {
        ("POST", "claim") => Ok(Action::Claim(id.into())),
        ("POST", "heartbeat") | ("PUT", "checkpoint") => Ok(Action::Turn {
            id: id.into(),
            terminal: false,
        }),
        ("POST", "reply" | "silent" | "park" | "release" | "wait") => Ok(Action::Turn {
            id: id.into(),
            terminal: true,
        }),
        _ => bail!("The container cannot call that device route"),
    }
}

pub(super) fn body_for(
    action: &Action,
    body: Option<&Value>,
    bot: &str,
    device: &str,
    mode: BotExecutionMode,
) -> Result<Option<Value>> {
    let Some(body) = body else {
        ensure!(
            matches!(action, Action::Turn { .. }),
            "Missing container request body"
        );
        return Ok(None);
    };
    let mut body = body
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Container request body must be an object"))?;
    if matches!(action, Action::PublishKey) {
        ensure!(
            body.get("principal_id").and_then(Value::as_str) == Some(bot),
            "A container can publish only its own inbox key"
        );
        ensure!(
            body.keys().all(|key| matches!(
                key.as_str(),
                "principal_id" | "key_id" | "algorithm" | "public_key"
            )),
            "Unexpected inbox key field"
        );
    } else {
        let host = super::host_id(device);
        ensure!(
            body.get("host_id").and_then(Value::as_str) == Some(host.as_str()),
            "Wrong container host id"
        );
        if matches!(action, Action::Claim(_)) {
            body.insert("execution_modes".into(), json!([mode]));
            body.remove("docker");
        }
    }
    Ok(Some(Value::Object(body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_device_admin_and_ambiguous_paths() {
        for path in [
            "/devices/other/bots/poll",
            "/devices/dev/reboot",
            "/devices/dev/bots/../poll",
            "/devices/dev/bots/turns/a%2fb/claim",
            "/devices/dev/bots/turns/a/claim?x=1",
            "/devices/dev/bots/turns/a/checkpoint?host_id=device%3Adev&attempt=1&attempt=2",
            "/devices/dev/bots/turns/a/checkpoint?host_id=device%3Adev&attempt=1&url=http://host",
        ] {
            assert!(
                action(
                    if path.contains("checkpoint") {
                        "GET"
                    } else {
                        "POST"
                    },
                    path,
                    "dev"
                )
                .is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn checks_host_and_key_identity_and_fixes_claim_mode() {
        assert!(
            body_for(
                &Action::PublishKey,
                Some(&json!({"principal_id":"other"})),
                "bot",
                "dev",
                BotExecutionMode::Docker
            )
            .is_err()
        );
        assert!(
            body_for(
                &Action::Claim("a".into()),
                Some(&json!({"host_id":"other"})),
                "bot",
                "dev",
                BotExecutionMode::Docker
            )
            .is_err()
        );
        let fixed = body_for(
            &Action::Claim("a".into()),
            Some(&json!({"host_id":"device:dev","execution_modes":["native"]})),
            "bot",
            "dev",
            BotExecutionMode::Docker,
        )
        .unwrap()
        .unwrap();
        assert_eq!(fixed["execution_modes"], json!(["docker"]));
        assert!(matches!(
            action(
                "GET",
                "/devices/dev/bots/turns/a/checkpoint?host_id=device%3Adev&attempt=1",
                "dev"
            )
            .unwrap(),
            Action::Turn {
                terminal: false,
                ..
            }
        ));
    }
}
