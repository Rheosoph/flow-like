//! Bounded iOS inbox delivery and status refresh using the native login authority.

use reqwest::{Client, Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

const MAX_RESPONSE: usize = 1024 * 1024;

#[derive(Debug)]
struct Scope {
    raw: String,
    origin: String,
    profile_id: String,
    subject: String,
}

impl Scope {
    fn parse(raw: &str) -> Result<Self, ()> {
        if raw.len() > 8192 {
            return Err(());
        }
        let [origin, profile_id, subject]: [String; 3] =
            serde_json::from_str(raw).map_err(|_| ())?;
        let url = Url::parse(&origin).map_err(|_| ())?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || origin.contains(['%', '\\'])
            || origin.chars().any(char::is_control)
            || profile_id.is_empty()
            || profile_id.len() > 256
            || profile_id.chars().any(char::is_control)
            || subject.is_empty()
            || subject == "local"
            || subject.len() > 512
            || subject.chars().any(char::is_control)
        {
            return Err(());
        }
        Ok(Self {
            raw: raw.into(),
            origin: origin.trim_end_matches('/').into(),
            profile_id,
            subject,
        })
    }

    fn url(&self, path: &str) -> String {
        let base = self.origin.strip_suffix("/api/v1").unwrap_or(&self.origin);
        format!("{base}/api/v1/{path}")
    }
}

#[derive(Deserialize)]
struct Payload {
    scope: String,
    #[serde(default)]
    bots: Vec<Value>,
    #[serde(default)]
    actions: Vec<Value>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshResult {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    turns: Option<Vec<Value>>,
    delivered_ids: Vec<String>,
    failed_actions: Vec<FailedAction>,
}

#[derive(Serialize)]
struct FailedAction {
    id: String,
    error: String,
}

#[derive(Debug, PartialEq)]
enum Delivery {
    Reply {
        bot: String,
        conversation: String,
        text: String,
    },
    Approval {
        bot: String,
        turn: String,
        approval: String,
        decision: String,
    },
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value.get(name)?.as_str()
}

fn delivery(value: &Value, payload: &Payload, now: i64) -> Option<Delivery> {
    if value.get("backgroundDelivery")?.as_bool()? != true
        || field(value, "scope")? != payload.scope
        || value.get("responseMode").is_some_and(|v| !v.is_null())
        || value
            .get("executionMode")
            .is_some_and(|v| !v.is_null() && v != "foreground")
        || uuid::Uuid::parse_str(field(value, "id")?).is_err()
    {
        return None;
    }
    let created = chrono::DateTime::parse_from_rfc3339(field(value, "createdAt")?)
        .ok()?
        .timestamp();
    if created > now + 60 || created <= now - 86_400 {
        return None;
    }
    if let Some(deadline) = field(value, "responseDeadline") {
        if chrono::DateTime::parse_from_rfc3339(deadline)
            .ok()?
            .timestamp()
            <= now
        {
            return None;
        }
    }
    let action = value.get("action")?;
    if action
        .get("voice")
        .is_some_and(|v| !v.is_null() && v != false)
        || action
            .get("files")
            .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
    {
        return None;
    }
    let bot = field(action, "botId")?;
    if !identifier(bot) {
        return None;
    }
    let cached = payload
        .bots
        .iter()
        .find(|candidate| field(candidate, "id") == Some(bot))?;
    if cached.get("local")?.as_bool()?
        || cached.get("encrypted")?.as_bool()?
        || cached.get("enabled").is_some_and(|v| v == false)
    {
        return None;
    }
    match field(action, "kind")? {
        "ask_bot" => {
            let conversation = field(action, "conversationId")?;
            let text = field(action, "prompt")
                .or_else(|| field(action, "text"))?
                .trim();
            if !identifier(conversation) || text.is_empty() || text.len() > 60_000 {
                return None;
            }
            Some(Delivery::Reply {
                bot: bot.into(),
                conversation: conversation.into(),
                text: text.into(),
            })
        }
        "respond_bot_approval" => {
            let turn = field(action, "turnId")?;
            let approval = field(action, "approvalId")?;
            let decision = field(action, "decision")?;
            if !identifier(turn) || !identifier(approval) || !matches!(decision, "once" | "deny") {
                return None;
            }
            Some(Delivery::Approval {
                bot: bot.into(),
                turn: turn.into(),
                approval: approval.into(),
                decision: decision.into(),
            })
        }
        _ => None,
    }
}

fn fresh_bot(bot: &Value, id: &str, subject: &str) -> bool {
    field(bot, "id") == Some(id)
        && field(bot, "owner_id") == Some(subject)
        && matches!(
            field(bot, "location"),
            Some("server" | "desktop" | "device")
        )
        && bot
            .get("capabilities")
            .and_then(Value::as_array)
            .is_some_and(|caps| !caps.iter().any(|cap| cap == "encryption"))
}

fn plain_conversation(value: &Value, id: &str, bot: &str, subject: &str) -> bool {
    let Some(members) = value.get("members").and_then(Value::as_array) else {
        return false;
    };
    field(value, "id") == Some(id)
        && field(value, "encryption") == Some("plain")
        && members
            .iter()
            .any(|m| field(m, "kind") == Some("bot") && field(m, "principal_id") == Some(bot))
        && members
            .iter()
            .any(|m| field(m, "kind") == Some("user") && field(m, "principal_id") == Some(subject))
}

#[async_trait::async_trait]
trait Authority: Sync {
    fn active(&self) -> bool;
    async fn token(&self) -> Result<String, ()>;
}

#[derive(Debug, PartialEq)]
enum Failure {
    Retry,
    Terminal,
}

fn failure(status: StatusCode) -> Failure {
    if status == StatusCode::UNAUTHORIZED
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
    {
        Failure::Retry
    } else {
        Failure::Terminal
    }
}

async fn request(
    authority: &impl Authority,
    client: &Client,
    method: Method,
    url: String,
    body: Option<Value>,
) -> Result<Value, Failure> {
    if !authority.active() {
        return Err(Failure::Retry);
    }
    let token = authority.token().await.map_err(|_| Failure::Retry)?;
    if !authority.active() {
        return Err(Failure::Retry);
    }
    let mut request = client
        .request(method, url)
        .bearer_auth(token)
        .header("Accept", "application/json");
    if let Some(body) = body {
        request = request.json(&body);
    }
    let mut response = request.send().await.map_err(|_| Failure::Retry)?;
    if !response.status().is_success() {
        return Err(failure(response.status()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE as u64)
    {
        return Err(Failure::Retry);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Retry)? {
        if !authority.active() || bytes.len() + chunk.len() > MAX_RESPONSE {
            return Err(Failure::Retry);
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| Failure::Retry)
}

async fn refresh(
    authority: &impl Authority,
    payload: &Payload,
    scope: &Scope,
    result: &mut RefreshResult,
) -> Result<(), ()> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| ())?;
    let status = request(
        authority,
        &client,
        Method::GET,
        scope.url("bots/mobile-status"),
        None,
    )
    .await
    .map_err(|_| ())?;
    let turns = status.get("turns").and_then(Value::as_array).ok_or(())?;
    result.turns = Some(
        turns
            .iter()
            .take(50)
            .filter(|turn| field(turn, "owner_id") == Some(&scope.subject))
            .cloned()
            .collect(),
    );
    for value in payload.actions.iter().take(8) {
        if !authority.active() {
            return Err(());
        }
        let Some(id) = field(value, "id").filter(|id| uuid::Uuid::parse_str(id).is_ok()) else {
            continue;
        };
        if field(value, "scope") != Some(&scope.raw) {
            continue;
        }
        let Some(delivery) = delivery(value, payload, chrono::Utc::now().timestamp()) else {
            result.failed_actions.push(FailedAction {
                id: id.into(),
                error: "Open Flow Like to review this notification action.".into(),
            });
            continue;
        };
        let bot = match &delivery {
            Delivery::Reply { bot, .. } | Delivery::Approval { bot, .. } => bot,
        };
        let outcome = async {
            let current_bot = request(authority, &client, Method::GET, scope.url(&format!("bots/{bot}")), None).await?;
            if !fresh_bot(&current_bot, bot, &scope.subject) { return Err(Failure::Terminal); }
            let (path, body) = match &delivery {
                Delivery::Reply { conversation, text, .. } => {
                    let current = request(authority, &client, Method::GET, scope.url(&format!("inbox/conversations/{conversation}")), None).await?;
                    if !plain_conversation(&current, conversation, bot, &scope.subject) { return Err(Failure::Terminal); }
                    (format!("inbox/conversations/{conversation}/entries"), json!({"message_id":id,"kind":"message","envelope":{"v":1,"enc":"none","body":{"text":text,"mentions":[bot]}}}))
                }
                Delivery::Approval { turn, approval, decision, .. } => {
                    let Some(current) = turns.iter().find(|value| field(value, "id") == Some(turn)) else { return Err(Failure::Terminal); };
                    if field(current, "owner_id") != Some(&scope.subject) || field(current, "bot_id") != Some(bot)
                        || field(current, "status") != Some("waiting") || field(current, "reply_message_id") != Some(approval)
                    { return Err(Failure::Terminal); }
                    (format!("bots/{bot}/turns/{turn}/approval"), json!({"approval_id":approval,"decision":decision}))
                }
            };
            request(authority, &client, Method::POST, scope.url(&path), Some(body)).await?;
            Ok::<_, Failure>(())
        }.await;
        match outcome {
            Ok(()) => result.delivered_ids.push(id.into()),
            Err(Failure::Terminal) => result.failed_actions.push(FailedAction {
                id: id.into(),
                error:
                    "This notification action is no longer available. Open Flow Like to review it."
                        .into(),
            }),
            Err(Failure::Retry) => return Err(()),
        }
    }
    if !result.delivered_ids.is_empty() {
        let latest = request(
            authority,
            &client,
            Method::GET,
            scope.url("bots/mobile-status"),
            None,
        )
        .await
        .map_err(|_| ())?;
        result.turns = Some(
            latest
                .get("turns")
                .and_then(Value::as_array)
                .ok_or(())?
                .iter()
                .take(50)
                .filter(|turn| field(turn, "owner_id") == Some(&scope.subject))
                .cloned()
                .collect(),
        );
    }
    Ok(())
}

#[cfg(target_os = "ios")]
mod ios {
    use super::*;
    use std::{
        ffi::{CStr, CString, c_char},
        sync::OnceLock,
    };
    use tauri::AppHandle;

    static APP: OnceLock<AppHandle> = OnceLock::new();
    static WORK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    unsafe extern "C" {
        fn flow_like_native_set_bot_refresh_callback(
            callback: extern "C" fn(*const c_char, *const c_char),
        );
        fn flow_like_native_bot_refresh_active(id: *const c_char, scope: *const c_char) -> i32;
        fn flow_like_native_complete_bot_refresh(id: *const c_char, result: *const c_char);
    }

    struct NativeAuthority {
        app: AppHandle,
        id: CString,
        scope: Scope,
        raw_scope: CString,
        session: crate::native_auth::SessionBinding,
    }
    #[async_trait::async_trait]
    impl Authority for NativeAuthority {
        fn active(&self) -> bool {
            self.session.is_current()
                && unsafe {
                    flow_like_native_bot_refresh_active(self.id.as_ptr(), self.raw_scope.as_ptr())
                        == 1
                }
        }
        async fn token(&self) -> Result<String, ()> {
            if !self.active() {
                return Err(());
            }
            let settings = crate::state::TauriSettingsState::construct(&self.app)
                .await
                .map_err(|_| ())?;
            let hub = {
                let settings = settings.lock().await;
                let profile = settings
                    .profile_by_id(&self.scope.profile_id)
                    .map_err(|_| ())?;
                flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
                    .ok_or(())?
            };
            if hub.trim_end_matches('/') != self.scope.origin {
                return Err(());
            }
            if crate::native_auth::current_subject(&self.scope.profile_id, &hub)
                .await
                .map_err(|_| ())?
                .as_deref()
                != Some(&self.scope.subject)
            {
                return Err(());
            }
            let token = self.session.access_token(None).await.map_err(|_| ())?;
            if !self.active() {
                return Err(());
            }
            Ok(token)
        }
    }

    extern "C" fn begin(id: *const c_char, payload: *const c_char) {
        if id.is_null() || payload.is_null() {
            return;
        }
        let id = unsafe { CStr::from_ptr(id) }.to_owned();
        let bytes = unsafe { CStr::from_ptr(payload) }.to_bytes();
        if id.to_bytes().len() > 128 || bytes.len() > 2 * 1024 * 1024 {
            return;
        }
        let payload = serde_json::from_slice::<Payload>(bytes);
        let Some(app) = APP.get().cloned() else {
            return;
        };
        tauri::async_runtime::spawn(async move {
            let mut result = RefreshResult::default();
            let work = async {
                let _lock = WORK.lock().await;
                let payload = payload.map_err(|_| ())?;
                let scope = Scope::parse(&payload.scope)?;
                let raw_scope = CString::new(payload.scope.as_str()).map_err(|_| ())?;
                if unsafe { flow_like_native_bot_refresh_active(id.as_ptr(), raw_scope.as_ptr()) }
                    != 1
                {
                    return Err(());
                }
                let session =
                    crate::native_auth::SessionBinding::capture(&scope.profile_id, &scope.origin)
                        .await
                        .map_err(|_| ())?
                        .ok_or(())?;
                if session.subject != scope.subject {
                    return Err(());
                }
                let authority = NativeAuthority {
                    app,
                    id: id.clone(),
                    scope,
                    raw_scope,
                    session,
                };
                refresh(&authority, &payload, &authority.scope, &mut result).await
            };
            // Leave time for Swift to apply acknowledgements before its 25-second lease expires.
            result.success = matches!(
                tokio::time::timeout(Duration::from_secs(23), work).await,
                Ok(Ok(()))
            );
            if let Ok(encoded) = CString::new(serde_json::to_string(&result).unwrap_or_default()) {
                unsafe {
                    flow_like_native_complete_bot_refresh(id.as_ptr(), encoded.as_ptr());
                }
            }
        });
    }

    pub(super) fn setup(app: &AppHandle) {
        let _ = APP.set(app.clone());
        unsafe {
            flow_like_native_set_bot_refresh_callback(begin);
        }
    }
}

#[cfg(target_os = "ios")]
pub(super) fn setup(app: &tauri::AppHandle) {
    ios::setup(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;
    fn payload() -> Payload {
        Payload {
            scope: json!(["https://hub.example", "profile", "alice"]).to_string(),
            bots: vec![json!({"id":"bot_1","local":false,"encrypted":false,"enabled":true})],
            actions: vec![],
        }
    }
    fn reply(payload: &Payload) -> Value {
        json!({"id":"60f6ea36-0391-4e4a-a036-82e50e1ca814","scope":payload.scope,
            "createdAt":chrono::DateTime::from_timestamp(NOW,0).unwrap().to_rfc3339(),"backgroundDelivery":true,
            "action":{"kind":"ask_bot","botId":"bot_1","conversationId":"chat_1","prompt":"Continue"}})
    }
    #[test]
    fn scope_rejects_credentials_insecure_or_ambiguous_origins() {
        for origin in [
            "http://hub.example",
            "https://a@hub.example",
            "https://hub.example?x=1",
            "https://hub.example/#x",
            "https://hub.example/%2e%2e",
            "https://hub.example\\evil",
        ] {
            assert!(
                Scope::parse(&json!([origin, "profile", "alice"]).to_string()).is_err(),
                "{origin}"
            );
        }
        assert!(
            Scope::parse(&json!(["https://hub.example", "profile", "local"]).to_string()).is_err()
        );
        let scope = Scope::parse(&payload().scope).unwrap();
        assert_eq!(
            scope.url("bots/mobile-status"),
            "https://hub.example/api/v1/bots/mobile-status"
        );
    }
    #[test]
    fn only_explicit_current_scoped_notifications_are_replayed() {
        let payload = payload();
        let accepted = reply(&payload);
        assert!(matches!(
            delivery(&accepted, &payload, NOW),
            Some(Delivery::Reply { .. })
        ));
        for (key, value) in [
            ("scope", json!("other")),
            ("backgroundDelivery", json!(false)),
            ("responseMode", json!("text")),
            ("executionMode", json!("background")),
            ("id", json!("not-a-uuid")),
        ] {
            let mut item = accepted.clone();
            item[key] = value;
            assert_eq!(delivery(&item, &payload, NOW), None, "{key}");
        }
        assert_eq!(delivery(&accepted, &payload, NOW + 86_400), None);
        assert_eq!(delivery(&accepted, &payload, NOW - 61), None);
    }
    #[test]
    fn local_encrypted_file_voice_and_task_requests_stay_in_foreground() {
        let mut payload = payload();
        let accepted = reply(&payload);
        for key in ["local", "encrypted"] {
            payload.bots[0][key] = json!(true);
            assert_eq!(delivery(&accepted, &payload, NOW), None);
            payload.bots[0][key] = json!(false);
        }
        for (key, value) in [
            ("voice", json!(true)),
            ("files", json!(["photo.jpg"])),
            ("kind", json!("run_bot_task")),
            ("conversationId", json!("../foreign")),
        ] {
            let mut item = accepted.clone();
            item["action"][key] = value;
            assert_eq!(delivery(&item, &payload, NOW), None, "{key}");
        }
    }
    #[test]
    fn grants_are_limited_to_one_time_or_denial() {
        let payload = payload();
        let mut item = reply(&payload);
        item["action"] = json!({"kind":"respond_bot_approval","botId":"bot_1","turnId":"bt_1","approvalId":"ap-1","decision":"once"});
        assert!(matches!(
            delivery(&item, &payload, NOW),
            Some(Delivery::Approval { .. })
        ));
        item["action"]["decision"] = json!("deny");
        assert!(matches!(
            delivery(&item, &payload, NOW),
            Some(Delivery::Approval { .. })
        ));
        item["action"]["decision"] = json!("always");
        assert_eq!(delivery(&item, &payload, NOW), None);
    }
    #[test]
    fn latest_conversation_must_be_plain_and_include_bot_and_owner() {
        let mut conversation = json!({"id":"chat_1","encryption":"plain","members":[{"kind":"bot","principal_id":"bot_1"},{"kind":"user","principal_id":"alice"}]});
        assert!(plain_conversation(
            &conversation,
            "chat_1",
            "bot_1",
            "alice"
        ));
        assert!(!plain_conversation(&conversation, "chat_1", "bot_1", "bob"));
        assert!(!plain_conversation(
            &conversation,
            "chat_1",
            "bot_2",
            "alice"
        ));
        conversation["encryption"] = json!("e2e");
        assert!(!plain_conversation(
            &conversation,
            "chat_1",
            "bot_1",
            "alice"
        ));
    }
    #[test]
    fn latest_bot_must_still_belong_to_the_scoped_owner_without_encryption() {
        let mut bot =
            json!({"id":"bot_1","owner_id":"alice","location":"server","capabilities":[]});
        assert!(fresh_bot(&bot, "bot_1", "alice"));
        assert!(!fresh_bot(&bot, "bot_1", "bob"));
        bot["capabilities"] = json!(["encryption"]);
        assert!(!fresh_bot(&bot, "bot_1", "alice"));
    }
    #[test]
    fn transient_failures_preserve_replay_and_terminal_failures_do_not_retry() {
        for code in [401, 408, 429, 500, 503] {
            assert_eq!(failure(StatusCode::from_u16(code).unwrap()), Failure::Retry);
        }
        for code in [301, 400, 403, 404, 409, 410, 422] {
            assert_eq!(
                failure(StatusCode::from_u16(code).unwrap()),
                Failure::Terminal
            );
        }
    }
    struct Cancelled;
    #[async_trait::async_trait]
    impl Authority for Cancelled {
        fn active(&self) -> bool {
            false
        }
        async fn token(&self) -> Result<String, ()> {
            panic!("Cancelled work must not read credentials")
        }
    }
    #[tokio::test]
    async fn cancelled_refresh_never_reads_credentials_or_connects() {
        let payload = payload();
        let scope = Scope::parse(&payload.scope).unwrap();
        let mut result = RefreshResult::default();
        assert!(
            refresh(&Cancelled, &payload, &scope, &mut result)
                .await
                .is_err()
        );
        assert!(result.delivered_ids.is_empty());
        assert!(result.turns.is_none());
    }

    struct SignedIn;
    #[async_trait::async_trait]
    impl Authority for SignedIn {
        fn active(&self) -> bool {
            true
        }
        async fn token(&self) -> Result<String, ()> {
            Ok("test-bearer".into())
        }
    }

    fn server(responses: Vec<(u16, Value)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, value) in responses {
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "Expected background request was not sent"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    let text = String::from_utf8_lossy(&bytes);
                    if let Some(head) = text.find("\r\n\r\n") {
                        let length = text[..head]
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= head + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                let body = value.to_string();
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nLocation: https://foreign.invalid/steal\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (origin, worker)
    }

    async fn send_reply(encryption: &str, post_status: u16) -> (RefreshResult, Vec<String>, bool) {
        let mut payload = payload();
        let mut item = reply(&payload);
        item["createdAt"] = json!(chrono::Utc::now().to_rfc3339());
        payload.actions.push(item);
        let mut responses = vec![
            (200, json!({"turns":[]})),
            (
                200,
                json!({"id":"bot_1","owner_id":"alice","location":"server","capabilities":[]}),
            ),
            (
                200,
                json!({"id":"chat_1","encryption":encryption,"members":[{"kind":"bot","principal_id":"bot_1"},{"kind":"user","principal_id":"alice"}]}),
            ),
        ];
        if encryption == "plain" {
            responses.push((post_status, json!({})));
            if post_status == 200 {
                responses.push((200, json!({"turns":[{"id":"turn_1","owner_id":"alice","bot_id":"bot_1","status":"queued"},{"id":"foreign","owner_id":"bob"}]})));
            }
        }
        let (origin, worker) = server(responses);
        // Production parses HTTPS scopes; this local server exercises the same HTTP client and body.
        let mut scope = Scope::parse(&payload.scope).unwrap();
        scope.origin = origin;
        let mut result = RefreshResult::default();
        let ok = refresh(&SignedIn, &payload, &scope, &mut result)
            .await
            .is_ok();
        (result, worker.join().unwrap(), ok)
    }

    #[tokio::test]
    async fn reply_uses_stable_message_identity_mentions_and_refreshes_status() {
        let (result, requests, ok) = send_reply("plain", 200).await;
        assert!(ok);
        assert_eq!(
            result.delivered_ids,
            ["60f6ea36-0391-4e4a-a036-82e50e1ca814"]
        );
        assert_eq!(result.turns.unwrap().len(), 1);
        let post = &requests[3];
        assert!(post.starts_with("POST /api/v1/inbox/conversations/chat_1/entries "));
        assert!(
            post.to_ascii_lowercase()
                .contains("authorization: bearer test-bearer")
        );
        let body: Value = serde_json::from_str(post.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["message_id"], "60f6ea36-0391-4e4a-a036-82e50e1ca814");
        assert_eq!(body["envelope"]["body"]["mentions"], json!(["bot_1"]));
        assert_eq!(body["envelope"]["body"]["text"], "Continue");
    }

    #[tokio::test]
    async fn encryption_change_prevents_post_and_preserves_action_for_review() {
        let (result, requests, ok) = send_reply("e2e", 200).await;
        assert!(ok);
        assert_eq!(requests.len(), 3);
        assert!(result.delivered_ids.is_empty());
        assert_eq!(result.failed_actions.len(), 1);
    }

    #[tokio::test]
    async fn server_retry_and_redirect_responses_never_acknowledge_delivery() {
        let (result, _, ok) = send_reply("plain", 503).await;
        assert!(!ok);
        assert!(result.delivered_ids.is_empty());
        assert!(result.failed_actions.is_empty());
        let (result, requests, ok) = send_reply("plain", 307).await;
        assert!(ok);
        assert_eq!(requests.len(), 4);
        assert!(result.delivered_ids.is_empty());
        assert_eq!(result.failed_actions.len(), 1);
    }
}
