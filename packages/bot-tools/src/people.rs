//! A bot writing to a person its owner already chats with. The message goes into one room per bot
//! and person (the owner is in it too), where the person's answer reaches the bot. The owner
//! approves the first message to each person; a standing grant is keyed by the person's id
//! (`person:<id>`), never by a name, so it can't cover another contact of the same name.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_types::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::outcome::ToolOutput;
use crate::spec::{Approval, ToolSpec, schema_of};

pub const LIST_PEOPLE: &str = "list_people";
pub const MESSAGE_PERSON: &str = "message_person";

pub const TEXT_MAX_CHARS: usize = 4_000;
/// Messages one bot may send to people per [`WINDOW_SECS`], all people together.
pub const MAX_PER_WINDOW: usize = 20;
pub const WINDOW_SECS: i64 = 3_600;
/// People `list_people` returns at most.
pub const MAX_CONTACTS: usize = 50;
pub const SCOPE_PREFIX: &str = "person:";
const MAX_ID_CHARS: usize = 200;

const LIST_DESCRIPTION: &str = "List the people your owner already chats with directly on Flow-Like, newest conversation first (at most 50), with the `id` that `message_person` takes. `query` keeps only people whose name contains it.";
const MESSAGE_DESCRIPTION: &str = "Send a message as yourself to a person your owner already chats with: `person` is an `id` from `list_people`. It goes into a room with that person and your owner, where their answer reaches you. You write on your owner's behalf: say who you are and never pretend to be your owner. Your owner approves the first message to each person. Not for your owner (use message_owner) or other bots (ask_bot). At most 20 per hour.";

/// A person the owner has an accepted direct chat with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PersonMessage {
    /// The room of the bot and the person.
    pub conversation_id: String,
    pub message_id: String,
    pub name: String,
    /// The person got a push; `false` when they muted the room.
    pub notified: bool,
}

/// What became of one message to a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Sent(PersonMessage),
    /// The owner has no accepted direct chat with this id.
    NotAContact,
    /// The person left the bot's room with them; nothing is sent until someone adds them again.
    Left {
        name: String,
    },
}

/// How a host reaches the people its bot's owner chats with: the server in process, a desktop
/// through the account API. Counting what the bot sent lets the tool enforce its hourly limit
/// across turns.
#[async_trait]
pub trait PeopleBackend: Send + Sync {
    async fn contacts(&self, query: Option<&str>) -> Result<Vec<Contact>>;
    async fn sent_since(&self, since_ms: i64) -> Result<usize>;
    async fn message(&self, person_id: &str, text: &str) -> Result<Delivery>;
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListPeopleArgs {
    /// Part of a name, ignoring case; leave it out to list everyone.
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MessagePersonArgs {
    /// The person's `id` from `list_people`.
    pub person: String,
    /// What the person should read. At most 4000 characters.
    pub text: String,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: LIST_PEOPLE,
            description: LIST_DESCRIPTION,
            capability: Some("channels"),
            module: "people",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<ListPeopleArgs>(),
        },
        ToolSpec {
            name: MESSAGE_PERSON,
            description: MESSAGE_DESCRIPTION,
            capability: Some("channels"),
            module: "people",
            approval: Approval::Ask,
            approval_args: &[],
            mutating: true,
            input_schema: schema_of::<MessagePersonArgs>(),
        },
    ]
}

/// A usable person id: what a grant scope and the backends accept.
pub fn person_id(value: &str) -> Option<&str> {
    let value = value.trim();
    let usable = !value.is_empty()
        && value.chars().count() <= MAX_ID_CHARS
        && !value
            .chars()
            .any(|char| char.is_control() || char.is_whitespace() || char == ',');
    usable.then_some(value)
}

pub fn scope_of(person_id: &str) -> String {
    format!("{SCOPE_PREFIX}{person_id}")
}

/// The grant scope of a `message_person` call: `person:<id>`; a call without a usable id is never
/// remembered.
pub fn grant_scopes(args: &Value) -> Option<Vec<String>> {
    let person = args.get("person").and_then(Value::as_str)?;
    person_id(person).map(|id| vec![scope_of(id)])
}

pub(crate) fn window_ms() -> i64 {
    WINDOW_SECS * 1000
}

fn backend(env: &dyn ToolEnv) -> ToolResult<Arc<dyn PeopleBackend>> {
    env.people().ok_or_else(|| {
        ToolError::new(
            code::NOT_YET,
            "Messaging people needs a bot on a Flow-Like account; this bot lives only on this computer",
        )
    })
}

fn checked_text(text: &str) -> ToolResult<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ToolError::invalid("'text' is empty"));
    }
    let chars = text.chars().count();
    if chars > TEXT_MAX_CHARS {
        return Err(ToolError::invalid(format!(
            "'text' has {chars} characters; at most {TEXT_MAX_CHARS} are allowed"
        )));
    }
    Ok(text.to_string())
}

pub async fn list_people(env: &dyn ToolEnv, args: ListPeopleArgs) -> ToolResult<ToolOutput> {
    let people = backend(env)?;
    let query = args
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty());
    let mut contacts = people.contacts(query).await?;
    contacts.truncate(MAX_CONTACTS);
    if !contacts.is_empty() {
        return Ok(ToolOutput::new(json!({ "people": contacts })));
    }
    let message = match query {
        Some(query) => format!("Your owner chats with nobody whose name contains '{query}'."),
        None => "Your owner has no direct chats with people yet.".to_string(),
    };
    Ok(ToolOutput::new(json!({ "people": [], "message": message })))
}

/// Messages the bot sent to people this hour; refused once [`MAX_PER_WINDOW`] are spent.
async fn sent_this_hour(people: &dyn PeopleBackend) -> ToolResult<usize> {
    let since = chrono::Utc::now().timestamp_millis() - window_ms();
    let sent = people.sent_since(since).await?;
    if sent < MAX_PER_WINDOW {
        return Ok(sent);
    }
    Err(ToolError::new(
        code::RATE_LIMITED,
        format!(
            "This bot already sent {sent} messages to people in the last hour; at most {MAX_PER_WINDOW} are allowed. Tell your owner what is left instead."
        ),
    ))
}

fn delivered(person: &str, sent: usize, delivery: Delivery) -> ToolResult<ToolOutput> {
    match delivery {
        Delivery::Sent(message) => Ok(ToolOutput::new(json!({
            "sent": true,
            "person": { "id": person, "name": message.name },
            "conversation_id": message.conversation_id,
            "message_id": message.message_id,
            "notified": message.notified,
            "remaining_this_hour": MAX_PER_WINDOW.saturating_sub(sent + 1),
        }))),
        Delivery::NotAContact => Err(ToolError::not_found(format!(
            "'{person}' is not someone your owner chats with directly. Call list_people and pass one of its ids. Nothing was sent."
        ))),
        Delivery::Left { name } => Err(ToolError::new(
            code::SCOPE_REFUSED,
            format!(
                "{name} left your conversation with them, so nothing was sent. Tell your owner instead."
            ),
        )),
    }
}

pub async fn message_person(env: &dyn ToolEnv, args: MessagePersonArgs) -> ToolResult<ToolOutput> {
    let people = backend(env)?;
    let person = person_id(&args.person)
        .ok_or_else(|| ToolError::invalid("'person' must be an id from list_people"))?;
    let text = checked_text(&args.text)?;
    let sent = sent_this_hour(people.as_ref()).await?;
    let delivery = people.message(person, &text).await?;
    delivered(person, sent, delivery)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::testing::FakeEnv;

    struct FakePeople {
        contacts: Vec<Contact>,
        prior: AtomicUsize,
        left: bool,
        queries: Mutex<Vec<Option<String>>>,
        sent: Mutex<Vec<(String, String)>>,
    }

    impl Default for FakePeople {
        fn default() -> Self {
            Self {
                contacts: vec![
                    Contact {
                        id: "user-anna".into(),
                        name: "Anna".into(),
                    },
                    Contact {
                        id: "user-ben".into(),
                        name: "Ben".into(),
                    },
                ],
                prior: AtomicUsize::new(0),
                left: false,
                queries: Mutex::default(),
                sent: Mutex::default(),
            }
        }
    }

    #[async_trait]
    impl PeopleBackend for FakePeople {
        async fn contacts(&self, query: Option<&str>) -> Result<Vec<Contact>> {
            self.queries.lock().unwrap().push(query.map(str::to_string));
            Ok(self.contacts.clone())
        }

        async fn sent_since(&self, _since_ms: i64) -> Result<usize> {
            Ok(self.prior.load(Ordering::SeqCst) + self.sent.lock().unwrap().len())
        }

        async fn message(&self, person_id: &str, text: &str) -> Result<Delivery> {
            let Some(contact) = self.contacts.iter().find(|contact| contact.id == person_id) else {
                return Ok(Delivery::NotAContact);
            };
            if self.left {
                return Ok(Delivery::Left {
                    name: contact.name.clone(),
                });
            }
            self.sent
                .lock()
                .unwrap()
                .push((person_id.to_string(), text.to_string()));
            Ok(Delivery::Sent(PersonMessage {
                conversation_id: "room-1".into(),
                message_id: "bp_1".into(),
                name: contact.name.clone(),
                notified: true,
            }))
        }
    }

    async fn env_with(people: Arc<FakePeople>) -> FakeEnv {
        let env = FakeEnv::new().await;
        *env.people.lock().unwrap() = Some(people);
        env
    }

    #[test]
    fn both_tools_need_channels_and_only_sending_asks() {
        let list = crate::spec(LIST_PEOPLE).expect("registered");
        let send = crate::spec(MESSAGE_PERSON).expect("registered");
        assert_eq!(list.capability, Some("channels"));
        assert_eq!(send.capability, Some("channels"));
        assert_eq!(list.approval, Approval::None);
        assert!(!list.mutating);
        assert_eq!(send.approval, Approval::Ask);
        assert!(send.mutating);
    }

    #[test]
    fn the_grant_scope_is_the_person_id_and_never_a_bad_one() {
        assert_eq!(
            grant_scopes(&json!({ "person": " user-anna ", "text": "hi" })),
            Some(vec!["person:user-anna".to_string()])
        );
        for person in ["", "  ", "Anna Schmidt", "a,b", "x\ny"] {
            assert_eq!(
                grant_scopes(&json!({ "person": person })),
                None,
                "{person:?}"
            );
        }
        assert_eq!(grant_scopes(&json!({ "text": "hi" })), None);
    }

    #[tokio::test]
    async fn listing_passes_the_trimmed_query_and_returns_ids() {
        let people = Arc::new(FakePeople::default());
        let env = env_with(people.clone()).await;
        let outcome = crate::run(LIST_PEOPLE, json!({ "query": "  an " }), &env).await;
        assert_eq!(outcome.status, crate::Status::Ok, "{:?}", outcome.error);
        assert_eq!(outcome.result["people"][0]["id"], "user-anna");
        assert_eq!(outcome.result["people"][1]["name"], "Ben");
        let blank = crate::run(LIST_PEOPLE, json!({ "query": " " }), &env).await;
        assert_eq!(blank.status, crate::Status::Ok);
        assert_eq!(
            *people.queries.lock().unwrap(),
            [Some("an".to_string()), None]
        );
    }

    #[tokio::test]
    async fn an_empty_list_says_why() {
        let people = Arc::new(FakePeople {
            contacts: Vec::new(),
            ..Default::default()
        });
        let env = env_with(people).await;
        let outcome = crate::run(LIST_PEOPLE, json!({}), &env).await;
        assert_eq!(outcome.result["people"], json!([]));
        assert!(
            outcome.result["message"]
                .as_str()
                .is_some_and(|message| message.contains("no direct chats")),
            "{}",
            outcome.result
        );
    }

    #[tokio::test]
    async fn a_message_reaches_the_person_trimmed() {
        let people = Arc::new(FakePeople::default());
        let env = env_with(people.clone()).await;
        let outcome = crate::run(
            MESSAGE_PERSON,
            json!({ "person": "user-anna", "text": "  Felix asks whether Friday works. " }),
            &env,
        )
        .await;
        assert_eq!(outcome.status, crate::Status::Ok, "{:?}", outcome.error);
        assert_eq!(outcome.result["person"]["name"], "Anna");
        assert_eq!(outcome.result["conversation_id"], "room-1");
        assert_eq!(outcome.result["remaining_this_hour"], MAX_PER_WINDOW - 1);
        assert_eq!(
            *people.sent.lock().unwrap(),
            [(
                "user-anna".to_string(),
                "Felix asks whether Friday works.".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn a_stranger_or_a_name_is_refused_and_nothing_is_sent() {
        let people = Arc::new(FakePeople::default());
        let env = env_with(people.clone()).await;
        let stranger = crate::run(
            MESSAGE_PERSON,
            json!({ "person": "user-zoe", "text": "hi" }),
            &env,
        )
        .await;
        let error = stranger.error.expect("refused");
        assert_eq!(error.code, code::NOT_FOUND);
        assert!(error.message.contains("list_people"), "{}", error.message);
        let named = crate::run(
            MESSAGE_PERSON,
            json!({ "person": "Anna Schmidt", "text": "hi" }),
            &env,
        )
        .await;
        assert_eq!(named.error.expect("refused").code, code::INVALID_ARGS);
        assert!(people.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_person_who_left_is_not_written_to() {
        let people = Arc::new(FakePeople {
            left: true,
            ..Default::default()
        });
        let env = env_with(people.clone()).await;
        let outcome = crate::run(
            MESSAGE_PERSON,
            json!({ "person": "user-ben", "text": "hi" }),
            &env,
        )
        .await;
        let error = outcome.error.expect("refused");
        assert_eq!(error.code, code::SCOPE_REFUSED);
        assert!(error.message.starts_with("Ben left"), "{}", error.message);
        assert!(people.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_twentieth_message_of_an_hour_passes_and_the_next_is_refused() {
        let people = Arc::new(FakePeople::default());
        people.prior.store(MAX_PER_WINDOW - 1, Ordering::SeqCst);
        let env = env_with(people.clone()).await;
        let args = json!({ "person": "user-anna", "text": "hi" });
        let last = crate::run(MESSAGE_PERSON, args.clone(), &env).await;
        assert_eq!(last.status, crate::Status::Ok, "{:?}", last.error);
        assert_eq!(last.result["remaining_this_hour"], 0);
        let over = crate::run(MESSAGE_PERSON, args, &env).await;
        assert_eq!(over.error.expect("limited").code, code::RATE_LIMITED);
        assert_eq!(people.sent.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn bad_text_never_reaches_the_backend() {
        let people = Arc::new(FakePeople::default());
        let env = env_with(people.clone()).await;
        for text in [" ".to_string(), "x".repeat(TEXT_MAX_CHARS + 1)] {
            let outcome = crate::run(
                MESSAGE_PERSON,
                json!({ "person": "user-anna", "text": text }),
                &env,
            )
            .await;
            assert_eq!(outcome.error.expect("refused").code, code::INVALID_ARGS);
        }
        assert!(people.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_host_without_people_says_the_bot_must_be_on_an_account() {
        let env = FakeEnv::new().await;
        for (tool, args) in [
            (LIST_PEOPLE, json!({})),
            (
                MESSAGE_PERSON,
                json!({ "person": "user-anna", "text": "hi" }),
            ),
        ] {
            let error = crate::run(tool, args, &env).await.error.expect("refused");
            assert_eq!(error.code, code::NOT_YET, "{tool}");
            assert!(error.message.contains("account"), "{}", error.message);
        }
    }
}
