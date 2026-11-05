use super::{NativePlatformSession, err};
use crate::{
    event_sink::manager::LocalSinkPlan,
    functions::{
        ai::{frontend_tool_bridge::FrontendToolApproval, native_tool_bridge},
        automation_approval::{self, ApprovalScope},
        flow::event::{self, SinkRegistration},
    },
};
use flow_like::flow::event::{Event, EventExecutionMode};
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

pub(super) struct TriggerAuthorization {
    pub mode: SinkRegistration,
    pub pat: Option<String>,
}

fn approval(title: &str, description: &str) -> FrontendToolApproval {
    FrontendToolApproval {
        kind: "execute".into(),
        title: title.into(),
        description: description.into(),
        // These approvals always reach the host. Auto mode and a previous tool approval
        // cannot authorize a persistent trigger or this workflow revision's capabilities.
        session_key: flow_like_types::create_id(),
    }
}

async fn dialog(
    session: &NativePlatformSession,
    title: &str,
    message: String,
    allow: &str,
    decline: Option<&str>,
) -> Result<&'static str, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let allow = allow.to_owned();
    let decline = decline.map(str::to_owned);
    let buttons = match &decline {
        Some(decline) => {
            MessageDialogButtons::YesNoCancelCustom(allow.clone(), decline.clone(), "Cancel".into())
        }
        None => MessageDialogButtons::OkCancelCustom(allow.clone(), "Cancel".into()),
    };
    session
        .app_handle
        .dialog()
        .message(message)
        .title(title)
        .kind(MessageDialogKind::Warning)
        .buttons(buttons)
        .show_with_result(move |result| {
            let answer = match result {
                MessageDialogResult::Yes | MessageDialogResult::Ok => "allow",
                MessageDialogResult::No if decline.is_some() => "decline",
                MessageDialogResult::Custom(value) if value == allow => "allow",
                MessageDialogResult::Custom(value) if decline.as_ref() == Some(&value) => "decline",
                _ => "dismiss",
            };
            let _ = sender.send(answer);
        });
    let cancelled = native_tool_bridge::execution_cancellation();
    tokio::select! {
        biased;
        _ = cancelled.cancelled() => Err("The owning FlowPilot run was cancelled.".into()),
        response = tokio::time::timeout(Duration::from_secs(600), receiver) => {
            response.map_err(|_| "Authorization timed out; the Event was not saved.")?
                .map_err(|_| "The authorization dialog closed; the Event was not saved.".into())
        }
    }
}

async fn approve_automation(
    session: &NativePlatformSession,
    app: &str,
    event: &Event,
) -> Result<(), String> {
    if !event.active
        || event.board_id.is_empty()
        || matches!(event.execution_mode, EventExecutionMode::Remote)
    {
        return Ok(());
    }
    let requirements = automation_approval::get_rpa_requirements(
        session.app_handle.clone(),
        app.into(),
        event.board_id.clone(),
        event.board_version,
        Some(event.id.clone()),
    )
    .await
    .map_err(err)?;
    if requirements.approved {
        return Ok(());
    }
    let capabilities = serde_json::to_value(&requirements.required).map_err(err)?;
    let description = format!(
        "Event '{}' can automate this computer. Approve these capabilities for this Event's current workflow revision: {}. Later workflow revisions require review again.",
        event.name, capabilities,
    );
    session.assert_current_profile().await?;
    let response = native_tool_bridge::request_interaction(
        &session.app_handle, session.context.as_ref(), "rpa_automation_consent",
        json!({"app_id":app,"event_id":event.id,"board_id":event.board_id,"version":event.board_version,
            "capabilities":capabilities,"revision":requirements.revision,"scope":"event"}),
        approval("Allow computer automation", &description),
    ).await;
    let allowed = match response {
        Ok(response) => {
            response.approved
                && response.error.is_none()
                && response.result.as_ref().and_then(|v| v["answer"].as_str()) == Some("allow")
        }
        Err(error) if error == "interaction_required" => {
            dialog(
                session,
                "Allow computer automation",
                description,
                "Allow automation",
                None,
            )
            .await?
                == "allow"
        }
        Err(error) => return Err(error),
    };
    if !allowed {
        return Err("Computer automation was not approved; the Event was not saved.".into());
    }
    session.assert_current_profile().await?;
    automation_approval::grant_native_automation(
        &session.app_handle,
        app,
        &event.board_id,
        event.board_version,
        Some(&event.id),
        &requirements.revision,
        &requirements.identity,
        ApprovalScope::Event,
    )
    .await
    .map_err(err)?;
    session.assert_current_profile().await
}

fn token_request(value: &Value, now: i64) -> Result<Value, String> {
    let name = value["name"]
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.chars().count() <= 256)
        .ok_or("A new trigger token needs a name of 1 to 256 characters.")?;
    let permission = value.get("permissions").map(Value::as_u64).unwrap_or(Some(2))
        .filter(|permission| matches!(permission, 2 | 4))
        .ok_or("A trigger token requires the explicitly selected Read & Write (2) or Admin (4) permission.")?;
    let valid_until =
        match value.get("valid_until") {
            None => Some(now.saturating_add(30 * 24 * 60 * 60)),
            Some(Value::Null) => None,
            Some(value) => Some(value.as_i64().filter(|until| *until > now).ok_or(
                "The trigger token expiration must be a future Unix timestamp in seconds.",
            )?),
        };
    Ok(json!({"name":name,"valid_until":valid_until,"permissions":permission}))
}

enum Decision {
    Allow {
        pat: Option<String>,
        create_token: Option<Value>,
    },
    Decline,
    Dismiss,
}

fn decision(value: &Value, needs_token: bool, now: i64) -> Result<Decision, String> {
    match value["answer"].as_str() {
        Some("decline") => Ok(Decision::Decline),
        Some("dismiss") => Ok(Decision::Dismiss),
        Some("allow") => {
            let pat = match value.get("pat") {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|token| {
                            token.starts_with("pat_")
                                && token.len() <= 8192
                                && !token.chars().any(char::is_whitespace)
                        })
                        .ok_or("The selected credential is not a personal access token.")?
                        .to_owned(),
                ),
            };
            let create_token = value
                .get("create_token")
                .filter(|v| !v.is_null())
                .map(|value| token_request(value, now))
                .transpose()?;
            if pat.is_some() && create_token.is_some() {
                return Err("Choose an existing token or request a new token, not both.".into());
            }
            if needs_token && pat.is_none() && create_token.is_none() {
                return Err("Select a personal access token or explicitly approve creating one before enabling this trigger.".into());
            }
            Ok(Decision::Allow { pat, create_token })
        }
        _ => Err(
            "Trigger authorization requires an explicit allow, decline, or dismiss answer.".into(),
        ),
    }
}

pub(super) async fn authorize(
    session: &NativePlatformSession,
    app: &str,
    event: &Event,
) -> Result<TriggerAuthorization, String> {
    let plan = event::local_trigger_plan(session.app_handle.clone(), app.into(), event.clone())
        .await
        .map_err(err)?;
    if plan.needs_token {
        // Sign-in must succeed before displaying consent. This does not use the session's
        // access token as the durable credential the trigger stores.
        let _ = session.token().await?;
    }
    approve_automation(session, app, event).await?;
    if !plan.needs_token && !matches!(plan.registration, LocalSinkPlan::New) {
        return Ok(TriggerAuthorization {
            mode: SinkRegistration::Keep,
            pat: None,
        });
    }
    let default_token = json!({"name":format!("FlowPilot: {}", event.name.chars().take(220).collect::<String>()),
        "permissions":2,"valid_until":chrono::Utc::now().timestamp() + 30 * 24 * 60 * 60});
    let mut description = format!(
        "Allow '{}' ({}) to run a persistent trigger on this device? Saving without a trigger removes any previous trigger for this Event on this device.",
        event.name, event.event_type
    );
    if let Some(previous_app) = &plan.moves_from_app {
        description.push_str(&format!(
            " This transfers the existing trigger from app '{previous_app}' to '{app}'."
        ));
    }
    if plan.needs_token {
        description.push_str(&format!(" Allow creates a Read & Write personal access token named '{}' on {} for the signed-in account. It expires in 30 days and is stored with this device's trigger.", default_token["name"].as_str().unwrap_or("FlowPilot"), session.hub));
    }
    session.assert_current_profile().await?;
    let response = native_tool_bridge::request_interaction(
        &session.app_handle, session.context.as_ref(), "local_trigger_consent",
        json!({"app_id":app,"event_id":event.id,"event_name":event.name,"event_type":event.event_type,
            "needs_token":plan.needs_token,"registration":plan.registration,"moves_from_app":plan.moves_from_app,
            "default_token":if plan.needs_token {Some(&default_token)} else {None}}),
        approval("Allow local trigger", &description),
    ).await;
    let answer = match response {
        Ok(response) if response.approved && response.error.is_none() => response
            .result
            .ok_or("Trigger authorization did not include a decision.")?,
        Ok(_) => {
            return Err("Trigger authorization was dismissed; the Event was not saved.".into());
        }
        Err(error) if error == "interaction_required" => {
            let answer = dialog(
                session,
                "Allow local trigger",
                description,
                "Allow",
                Some("Save without trigger"),
            )
            .await?;
            json!({"answer":answer,"create_token":if answer == "allow" && plan.needs_token {Some(&default_token)} else {None}})
        }
        Err(error) => return Err(error),
    };
    session.assert_current_profile().await?;
    match decision(&answer, plan.needs_token, chrono::Utc::now().timestamp())? {
        Decision::Dismiss => {
            Err("Trigger authorization was dismissed; the Event was not saved.".into())
        }
        Decision::Decline => Ok(TriggerAuthorization {
            mode: SinkRegistration::Skip,
            pat: None,
        }),
        Decision::Allow { pat, create_token } => {
            let pat = match create_token {
                Some(request) => {
                    // This endpoint is invoked once, only from the host's explicit choice.
                    let response = session
                        .request(Method::PUT, "user/pat", Some(&request))
                        .await?;
                    let token = response["pat"]
                        .as_str()
                        .filter(|pat| pat.starts_with("pat_"))
                        .ok_or("The hub did not return a personal access token.")?;
                    Some(token.to_owned())
                }
                None => pat,
            };
            if let Some(pat) = &pat {
                session.assert_current_profile().await?;
                event::verify_trigger_token(session.app_handle.clone(), app.into(), pat.clone())
                    .await
                    .map_err(err)?;
                session.assert_current_profile().await?;
            }
            Ok(TriggerAuthorization {
                mode: SinkRegistration::Register,
                pat,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_trigger_consent_never_implies_token_creation() {
        assert!(decision(&json!({"answer":"allow"}), true, 1).is_err());
        assert!(decision(&json!({"approved":true}), false, 1).is_err());
        assert!(matches!(
            decision(&json!({"answer":"decline"}), true, 1),
            Ok(Decision::Decline)
        ));
        assert!(matches!(
            decision(&json!({"answer":"dismiss"}), true, 1),
            Ok(Decision::Dismiss)
        ));
        assert!(decision(&json!({"answer":"allow","pat":"eyJ.access-token"}), true, 1).is_err());
        assert!(decision(&json!({"answer":"allow","pat":"pat_id.secret","create_token":{"name":"duplicate"}}), true, 1).is_err());
        assert!(matches!(
            decision(&json!({"answer":"allow","pat":"pat_id.secret"}), true, 1),
            Ok(Decision::Allow {
                pat: Some(_),
                create_token: None
            })
        ));
    }

    #[test]
    fn token_creation_preserves_only_explicit_name_expiry_and_write_permission() {
        assert_eq!(token_request(&json!({"name":" chosen ","permissions":2,"valid_until":101,"access_token":"ignored"}), 100).unwrap(), json!({"name":"chosen","permissions":2,"valid_until":101}));
        assert!(token_request(&json!({"name":"read only","permissions":1}), 100).is_err());
        assert!(token_request(&json!({"name":"expired","valid_until":100}), 100).is_err());
        assert!(token_request(&json!({"name":"","permissions":4}), 100).is_err());
        assert_eq!(
            token_request(&json!({"name":"default"}), 100).unwrap()["valid_until"],
            2_592_100
        );
        assert!(token_request(&json!({"name":"unlimited","valid_until":null}), 100).unwrap()["valid_until"].is_null());
    }
}
