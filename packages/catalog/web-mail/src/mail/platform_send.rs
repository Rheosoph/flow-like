use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like::{
    flow::{
        execution::{ExecutionEnvironment, context::ExecutionContext},
        node::{Node, NodeLogic, NodeScores},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
    hub::{HubRetry, hub_api_url, hub_response_error, send_hub_request},
};
use flow_like_catalog_core::{
    FlowPath, MAX_MAIL_ATTACHMENT_BYTES, MAX_MAIL_ATTACHMENTS, MailAttachment, MailSession,
    read_file_attachments,
};
use flow_like_types::{
    Value, anyhow, async_trait, authorization::ResourceAudience, json::json, reqwest,
};
use serde::Deserialize;

#[crate::register_node]
#[derive(Default)]
pub struct PlatformSendMailNode;

impl PlatformSendMailNode {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Deserialize)]
pub(super) struct SendMailResponse {
    accepted: bool,
    pub request_id: String,
    pub session: MailSession,
}

fn throttled(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

const MAIL_RETRY: HubRetry = HubRetry {
    retryable: throttled,
    retries: 3,
    max_wait: Duration::from_secs(60),
};

/// Carries the code a regression suite reads to grade a replay that failed
/// only on refused side effects as skipped instead of regressed.
const SEND_MAIL_REFUSED: &str =
    "SHADOW_SIDE_EFFECT_REFUSED: Shadow and regression runs cannot send platform mail";

fn mail_url(
    hub: &str,
    secure: bool,
    app_id: &str,
    action: &str,
    execution: bool,
) -> flow_like_types::Result<reqwest::Url> {
    if execution {
        hub_api_url(hub, secure, &["execution", "apps", app_id, "mail", action])
    } else {
        hub_api_url(hub, secure, &["apps", app_id, "mail", action])
    }
}

pub(super) fn optional_text(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn recipients(addresses: Vec<String>) -> Vec<String> {
    addresses
        .into_iter()
        .map(|address| address.trim().to_owned())
        .filter(|address| !address.is_empty())
        .collect()
}

fn validate_recipients_and_subject(to: &[String], subject: &str) -> flow_like_types::Result<()> {
    if to.is_empty() {
        return Err(anyhow!("Send Platform Email needs at least one To address"));
    }
    if subject.trim().is_empty() {
        return Err(anyhow!("Send Platform Email needs a subject"));
    }
    Ok(())
}

async fn message_body(
    context: &mut ExecutionContext,
    session: &MailSession,
) -> flow_like_types::Result<Value> {
    let to = recipients(context.evaluate_pin::<Vec<String>>("to").await?);
    let cc = recipients(context.evaluate_pin::<Vec<String>>("cc").await?);
    let bcc = recipients(context.evaluate_pin::<Vec<String>>("bcc").await?);
    let subject = context.evaluate_pin::<String>("subject").await?;
    validate_recipients_and_subject(&to, &subject)?;
    let text = optional_text(context.evaluate_pin::<String>("text").await?);
    let html = optional_text(context.evaluate_pin::<String>("html").await?);
    Ok(json!({
        "session": session,
        "to": to,
        "cc": cc,
        "bcc": bcc,
        "subject": subject,
        "text": text,
        "html": html,
    }))
}

pub(super) fn add_attachments_pin(node: &mut Node) {
    node.add_input_pin(
        "attachments",
        "Attachments",
        "Files to attach: at most 10, together at most 3 MiB",
        VariableType::Struct,
    )
    .set_value_type(ValueType::Array)
    .set_schema::<FlowPath>()
    .set_options(PinOptions::new().set_enforce_schema(true).build())
    .set_default_value(Some(json!([])));
}

pub(super) fn add_sender_name_pin(node: &mut Node) {
    node.add_input_pin(
        "sender_name",
        "Sender Name",
        "Name recipients see as the sender, at most 64 characters; empty uses the email event's name",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

/// An empty name is omitted, so an API that predates sender names still accepts the message.
pub(super) async fn name_sender(
    context: &mut ExecutionContext,
    body: &mut Value,
) -> flow_like_types::Result<()> {
    if let Some(name) = optional_text(context.evaluate_pin::<String>("sender_name").await?) {
        body["sender_name"] = json!(name.trim());
    }
    Ok(())
}

/// Adds the attachment pin's files to `body`. Messages without files omit the field, so
/// an API that predates attachments still accepts them.
pub(super) async fn attach_files(
    context: &mut ExecutionContext,
    body: &mut Value,
) -> flow_like_types::Result<()> {
    let files = context.evaluate_pin::<Vec<FlowPath>>("attachments").await?;
    if files.is_empty() {
        return Ok(());
    }
    if files.len() > MAX_MAIL_ATTACHMENTS {
        return Err(anyhow!(
            "Platform email accepts at most {MAX_MAIL_ATTACHMENTS} attachments, got {}",
            files.len()
        ));
    }
    let attachments = read_file_attachments(context, &files, MAX_MAIL_ATTACHMENT_BYTES)
        .await?
        .into_iter()
        .map(|file| MailAttachment {
            filename: file.filename.unwrap_or_default(),
            content_type: file.content_type,
            content: STANDARD.encode(file.data),
        })
        .collect::<Vec<_>>();
    body["attachments"] = json!(attachments);
    Ok(())
}

fn validate_session(session: &MailSession, app_id: &str) -> flow_like_types::Result<()> {
    if session.app_id != app_id || session.event_id.is_empty() {
        return Err(anyhow!(
            "Mail session does not belong to this app execution"
        ));
    }
    Ok(())
}

fn mail_request_id(run_id: &str, node_id: &str, trace_id: &str) -> flow_like_types::Result<String> {
    if run_id.is_empty() {
        return Err(anyhow!("Platform email requires an execution run ID"));
    }
    // Trace counters restart in each executor process. Include the run so a new
    // execution cannot replay another run's successful submission.
    let identity = json!([run_id, node_id, trace_id]).to_string();
    Ok(flow_like::utils::hash::hash_string(&identity))
}

/// The executor's execution route and credential on server runs; the user
/// route on the profile hub otherwise.
fn mail_endpoint<'a>(
    context: &'a ExecutionContext,
    app_id: &str,
    action: &str,
) -> flow_like_types::Result<(reqwest::Url, Option<&'a str>)> {
    let executor = context.executor_api_auth.as_ref();
    if context.execution_environment() == ExecutionEnvironment::Server && executor.is_none() {
        return Err(anyhow!(
            "Server email requires an authenticated executor session"
        ));
    }
    let token = executor
        .map(|auth| auth.token())
        .or(context.token.as_deref())
        .filter(|token| !token.trim().is_empty());
    if token.is_none() && context.request_authorizer().is_none() {
        return Err(anyhow!(
            "Platform email requires an authenticated server session"
        ));
    }
    let secure = context.profile.secure;
    let url = match executor {
        Some(auth) => mail_url(auth.api_url(), secure, app_id, action, true)?,
        None => mail_url(&context.profile.hub, secure, app_id, action, false)?,
    };
    let bearer = token
        .filter(|_| executor.is_some() || context.request_authorizer().is_none())
        .map(str::trim);
    Ok((url, bearer))
}

async fn post_mail(
    context: &ExecutionContext,
    url: &reqwest::Url,
    bearer: Option<&str>,
    body: &Value,
) -> flow_like_types::Result<reqwest::Response> {
    // An executor's API base comes from its verified callback claim and may
    // address a private service in Docker or Kubernetes.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(330))
        .build()?;
    let executor = context.executor_api_auth.is_some();
    let client_ref = &client;
    send_hub_request(&client, MAIL_RETRY, move || async move {
        let mut request = client_ref.post(url.clone()).json(body);
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let request = request.build()?;
        if executor {
            return Ok(request);
        }
        context
            .authorize_request(request, ResourceAudience::ProjectApi)
            .await
    })
    .await
}

pub(super) async fn submit(
    context: &ExecutionContext,
    session: &MailSession,
    action: &str,
    mut body: Value,
) -> flow_like_types::Result<SendMailResponse> {
    let cache = context
        .execution_cache
        .as_ref()
        .ok_or_else(|| anyhow!("Platform email requires an app execution"))?;
    if cache.shadow {
        return Err(anyhow!(SEND_MAIL_REFUSED));
    }
    validate_session(session, &cache.app_id)?;
    let (url, bearer) = mail_endpoint(context, &cache.app_id, action)?;
    body["request_id"] = json!(mail_request_id(
        context.run_id(),
        &context.id,
        &context.trace.id,
    )?);
    let response = post_mail(context, &url, bearer, &body).await?;
    if !response.status().is_success() {
        return Err(hub_response_error("Platform email", response).await);
    }
    let result: SendMailResponse = response.json().await?;
    if !result.accepted || result.request_id.is_empty() || result.session != *session {
        return Err(anyhow!(
            "The server did not confirm this mail session's submission"
        ));
    }
    Ok(result)
}

pub(super) fn mail_scores() -> NodeScores {
    NodeScores::new()
        .set_privacy(4)
        .set_security(6)
        .set_performance(6)
        .set_governance(6)
        .set_reliability(6)
        .set_cost(7)
        .build()
}

pub(super) fn add_result_pins(node: &mut Node) {
    node.add_output_pin(
        "exec_out",
        "Out",
        "The mail provider accepted the email",
        VariableType::Execution,
    );
    node.add_output_pin(
        "request_id",
        "Request ID",
        "Server identifier for this submission",
        VariableType::String,
    );
    node.add_output_pin(
        "session_out",
        "Session",
        "Mail session used for this submission",
        VariableType::Struct,
    )
    .set_schema::<MailSession>();
}

pub(super) async fn finish(
    context: &mut ExecutionContext,
    result: SendMailResponse,
) -> flow_like_types::Result<()> {
    context
        .set_pin_value("request_id", json!(result.request_id))
        .await?;
    context
        .set_pin_value("session_out", json!(result.session))
        .await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

#[async_trait]
impl NodeLogic for PlatformSendMailNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_platform_send",
            "Send Platform Email",
            "Send email from an authorized event's address",
            "Email",
        );
        node.set_version(4);
        node.set_flowscript_name("email", "sendPlatform");
        node.set_receiver("session");
        node.add_icon("/flow/icons/mail.svg");
        node.set_scores(mail_scores());
        node.add_input_pin("exec_in", "In", "Send the email", VariableType::Execution);
        node.add_input_pin(
            "session",
            "Session",
            "Mail session from an inbound email event",
            VariableType::Struct,
        )
        .set_schema::<MailSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        for (name, label, description) in [
            ("to", "To", "Recipient email addresses"),
            ("cc", "Cc", "Visible copy recipients"),
            ("bcc", "Bcc", "Hidden copy recipients"),
        ] {
            node.add_input_pin(name, label, description, VariableType::String)
                .set_value_type(ValueType::Array)
                .set_default_value(Some(json!([])));
        }
        for (name, label, description) in [
            ("subject", "Subject", "Email subject"),
            ("text", "Text", "Plain text body"),
            ("html", "HTML", "HTML body"),
        ] {
            node.add_input_pin(name, label, description, VariableType::String)
                .set_default_value(Some(json!("")));
        }
        add_attachments_pin(&mut node);
        add_sender_name_pin(&mut node);
        add_result_pins(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session = context.evaluate_pin::<MailSession>("session").await?;
        let mut body = message_body(context, &session).await?;
        attach_files(context, &mut body).await?;
        name_sender(context, &mut body).await?;
        let result = submit(context, &session, "send", body).await?;
        finish(context, result).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{LogLevel, context::ExecutionContextCache, internal_node::InternalNode},
        },
        flow_like_storage::Path,
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_types::sync::{Mutex, RwLock};
    use std::sync::{Arc, Weak};

    #[test]
    fn executor_runs_use_the_execution_route_and_users_the_app_route() {
        for hub in ["example.com", "https://example.com/api/v1"] {
            for action in ["send", "reply"] {
                assert_eq!(
                    mail_url(hub, true, "app-1", action, false)
                        .unwrap()
                        .as_str(),
                    format!("https://example.com/api/v1/apps/app-1/mail/{action}")
                );
                assert_eq!(
                    mail_url(hub, true, "app-1", action, true).unwrap().as_str(),
                    format!("https://example.com/api/v1/execution/apps/app-1/mail/{action}")
                );
            }
        }
        assert_eq!(
            mail_url("https://example.com", true, "app/other", "send", false)
                .unwrap()
                .as_str(),
            "https://example.com/api/v1/apps/app%2Fother/mail/send"
        );
    }

    #[test]
    fn retries_only_throttled_responses() {
        assert!(throttled(reqwest::StatusCode::TOO_MANY_REQUESTS));
        for status in [400, 409, 422, 500, 503] {
            assert!(!throttled(reqwest::StatusCode::from_u16(status).unwrap()));
        }
        assert_eq!(MAIL_RETRY.retries, 3);
        assert!(MAIL_RETRY.max_wait <= Duration::from_secs(60));
    }

    #[test]
    fn mail_idempotency_separates_runs_and_preserves_retries() {
        let first = mail_request_id("run-1", "reply-node", "t4").unwrap();
        let retry = mail_request_id("run-1", "reply-node", "t4").unwrap();
        let next_run = mail_request_id("run-2", "reply-node", "t4").unwrap();
        let next_iteration = mail_request_id("run-1", "reply-node", "t5").unwrap();
        let other_node = mail_request_id("run-1", "send-node", "t4").unwrap();

        assert_eq!(first, retry, "HTTP retries must replay the same submission");
        assert_ne!(first, next_run, "new executors may reuse trace counters");
        assert_ne!(first, next_iteration, "loop iterations may send new mail");
        assert_ne!(first, other_node, "separate mail nodes may send new mail");
        assert!(mail_request_id("", "reply-node", "t4").is_err());
    }

    #[test]
    fn mail_idempotency_accepts_long_ids_without_ambiguous_boundaries() {
        let request_id = mail_request_id(&"run".repeat(100), "node/with spaces", "t4").unwrap();
        assert!(request_id.len() <= 128);
        assert!(request_id.bytes().all(|byte| byte.is_ascii_alphanumeric()));
        assert_ne!(
            mail_request_id("run:node", "part", "t4").unwrap(),
            mail_request_id("run", "node:part", "t4").unwrap(),
        );
    }

    #[test]
    fn requires_a_recipient_and_a_subject() {
        let to = recipients(vec![" ".into(), " ops@example.com ".into(), String::new()]);
        assert_eq!(to, ["ops@example.com"]);
        assert!(validate_recipients_and_subject(&to, "Report").is_ok());
        assert!(validate_recipients_and_subject(&recipients(vec!["  ".into()]), "Report").is_err());
        assert!(validate_recipients_and_subject(&to, " \n").is_err());
    }

    #[test]
    fn session_input_is_required_and_caller_names_but_never_addresses_the_sender() {
        let node = PlatformSendMailNode::new().get_node();
        assert_eq!(node.version, Some(4));
        let sender_name = node.get_pin_by_name("sender_name").unwrap();
        assert_eq!(sender_name.data_type, VariableType::String);
        assert_eq!(
            sender_name.default_value,
            Some(flow_like_types::json::to_vec(&json!("")).unwrap())
        );
        assert!(node.scores.is_some());
        let session = node.get_pin_by_name("session").unwrap();
        assert_eq!(session.data_type, VariableType::Struct);
        assert_eq!(session.options.as_ref().unwrap().enforce_schema, Some(true));
        assert!(session.default_value.is_none());
        let schema: Value =
            flow_like_types::json::from_str(session.schema.as_deref().unwrap()).unwrap();
        assert_eq!(schema["title"], "MailSession");
        assert!(node.get_pin_by_name("from").is_none());
        assert!(node.get_pin_by_name("reply_to").is_none());
        assert!(node.get_pin_by_name("headers").is_none());
        assert_eq!(
            node.get_pin_by_name("session_out").unwrap().schema,
            session.schema
        );
    }

    #[test]
    fn rejects_cross_app_or_missing_event_sessions() {
        let session = MailSession {
            app_id: "app-1".into(),
            event_id: "event-1".into(),
        };
        assert!(validate_session(&session, "app-1").is_ok());
        assert!(validate_session(&session, "other").is_err());
        assert!(
            validate_session(
                &MailSession {
                    event_id: String::new(),
                    ..session
                },
                "app-1"
            )
            .is_err()
        );
    }

    async fn replay_context() -> ExecutionContext {
        let logic: Arc<dyn NodeLogic> = Arc::new(PlatformSendMailNode);
        let node = Arc::new(InternalNode::new(
            logic.get_node(),
            Default::default(),
            logic,
            Default::default(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let mut context = ExecutionContext::new(
            Arc::new(Default::default()),
            &Weak::new(),
            &state,
            &node,
            &Arc::new(Mutex::new(Default::default())),
            &Arc::new(RwLock::new(Default::default())),
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            Some("executor-jwt".into()),
            Arc::new(Default::default()),
            None,
        )
        .await;
        context.execution_cache = Some(ExecutionContextCache {
            stores: state.config.read().await.stores.clone(),
            app_id: "app-1".into(),
            model_usage_app_id: Some("app-1".into()),
            board_dir: Path::from("apps/app-1"),
            board_id: "board".into(),
            node_id: "node".into(),
            sub: "alice".into(),
            shadow: true,
        });
        context
    }

    #[tokio::test]
    async fn replays_are_refused_with_the_graded_refusal() {
        let context = replay_context().await;
        let session = MailSession {
            app_id: "app-1".into(),
            event_id: "event-1".into(),
        };
        for action in ["send", "reply"] {
            let error = submit(&context, &session, action, json!({}))
                .await
                .err()
                .unwrap();
            assert_eq!(
                error.to_string(),
                "SHADOW_SIDE_EFFECT_REFUSED: Shadow and regression runs cannot send platform mail"
            );
        }
    }

    #[test]
    fn omits_empty_optional_bodies() {
        assert_eq!(optional_text("  ".into()), None);
        assert_eq!(optional_text("Hello\n".into()), Some("Hello\n".into()));
    }

    #[test]
    fn attachments_are_an_optional_list_of_files() {
        let node = PlatformSendMailNode::new().get_node();
        let pin = node.get_pin_by_name("attachments").unwrap();
        assert_eq!(pin.data_type, VariableType::Struct);
        assert_eq!(pin.value_type, ValueType::Array);
        assert_eq!(pin.options.as_ref().unwrap().enforce_schema, Some(true));
        let schema: Value =
            flow_like_types::json::from_str(pin.schema.as_deref().unwrap()).unwrap();
        assert_eq!(schema["title"], "FlowPath");
        assert_eq!(
            pin.default_value,
            Some(flow_like_types::json::to_vec(&json!([])).unwrap())
        );
    }

    #[test]
    fn catalog_upgrade_preserves_message_and_execution_wires() {
        use flow_like::flow::board::cleanup::sync_node_schema::sync_node_with_catalog;
        let catalog = PlatformSendMailNode::new().get_node();
        let mut placed = catalog.clone();
        placed.set_version(1);
        placed.pins.retain(|_, pin| {
            !matches!(
                pin.name.as_str(),
                "session" | "session_out" | "attachments" | "sender_name"
            )
        });
        placed.add_input_pin("reply_to", "Reply To", "", VariableType::String);
        let mut original_pins = Vec::new();
        for name in ["exec_in", "to", "cc", "bcc", "subject", "text", "html"] {
            let pin = placed.get_pin_mut_by_name(name).unwrap();
            pin.depends_on.insert(format!("source-{name}"));
            original_pins.push((name, pin.id.clone()));
        }
        for name in ["exec_out", "request_id"] {
            let pin = placed.get_pin_mut_by_name(name).unwrap();
            pin.connected_to.insert(format!("target-{name}"));
            original_pins.push((name, pin.id.clone()));
        }
        placed
            .get_pin_mut_by_name("subject")
            .unwrap()
            .set_default_value(Some(json!("Existing subject")));
        sync_node_with_catalog(&mut placed, &catalog);
        assert_eq!(placed.version, Some(4));
        assert!(placed.get_pin_by_name("reply_to").is_none());
        assert!(placed.get_pin_by_name("attachments").is_some());
        assert!(placed.get_pin_by_name("sender_name").is_some());
        assert!(
            placed
                .get_pin_by_name("session")
                .unwrap()
                .default_value
                .is_none()
        );
        assert!(placed.get_pin_by_name("session_out").is_some());
        for (name, id) in original_pins {
            let pin = placed.get_pin_by_name(name).unwrap();
            assert_eq!(pin.id, id);
            assert!(
                pin.depends_on.contains(&format!("source-{name}"))
                    || pin.connected_to.contains(&format!("target-{name}"))
            );
        }
        assert_eq!(
            placed.get_pin_by_name("subject").unwrap().default_value,
            Some(flow_like_types::json::to_vec(&json!("Existing subject")).unwrap())
        );
    }
}
