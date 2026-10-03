//! Quick actions and forms: events a person starts. The agent parent queues each run; the
//! placement process fetches it over its supervisor channel, checks the fields against the
//! flow version it runs, runs it once and reports how it ended.

use crate::{
    config::PlacementConfig,
    hosting::PreparedInvocation,
    management::run_queue::{FinishedRun, MAX_OUTPUT_BYTES, RunBatch, StartRun},
    run_once::{RunContext, RunEnd, RunRequest, run_event_once},
};
use anyhow::{Context, Result, ensure};
use flow_like_runtime::flow::{
    event::EventInput,
    pin::{Pin, PinType, ValueType, resolve_schema},
    variable::{Variable, VariableType},
};
use flow_like_types::intercom::{InterComCallback, InterComEvent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
use tokio::{sync::oneshot, task::JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

const STATE_DIRECTORY: &str = ".standalone-run";
const CONTRACT_FILE: &str = "contract.json";
const STATE_VERSION: u64 = 1;
const PAYLOAD_PIN: &str = "payload";
const MAX_FIELDS: usize = 64;
const MAX_LABEL_CHARS: usize = 120;
const MAX_DESCRIPTION_CHARS: usize = 480;
const MAX_DEFAULT_BYTES: usize = 1024;
const MAX_OPTIONS: usize = 32;
const MAX_OPTION_CHARS: usize = 64;
const MAX_ROUTES: usize = 8;
const MAX_ROUTE_CHARS: usize = 128;
const MAX_ITEMS: usize = 16_384;
const MAX_ENTRY_BYTES: usize = 12 * 1024;
const MAX_CONTRACT_BYTES: usize = 256 * 1024;
const MAX_STATE_BYTES: usize = 64 * 1024;
const MAX_REFUSED_NAMES: usize = 16;
const MAX_REFUSED_NAME_CHARS: usize = 64;
const MAX_REFUSED_NAMES_BYTES: usize = 2 * 1024;
const MAX_KEPT_TEXT_BYTES: usize = 64 * 1024;
const MAX_TRACKED_ATTACHMENTS: usize = 1024;
const MAX_REPORTS_PER_QUESTION: usize = 32;
const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(3600);
const POLL: Duration = Duration::from_millis(500);
const STOP_GRACE: Duration = Duration::from_secs(12);
/// `run_event_once` lets a cancelled run end for 2 s before it gives up on it.
const CANCEL_GRACE: Duration = Duration::from_secs(4);
const LAST_REPORT_WAIT: Duration = Duration::from_secs(5);

/// `action` has no fields, `form` has at least one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Action,
    Form,
}

/// Which door a run came through. Only the service page can carry a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Origin {
    Management,
    ServicePage,
}

/// One input of a person-started event: an output pin of its entry node.
#[derive(Clone, Debug)]
struct Field {
    pin_id: String,
    name: String,
    label: String,
    description: String,
    data_type: VariableType,
    value_type: ValueType,
    schema: Option<String>,
    optional: bool,
    sensitive: bool,
    default: Option<Vec<u8>>,
    options: Option<Vec<String>>,
    index: u16,
}

impl Field {
    /// A saved board keeps a pin's description and schema as keys into its `refs`.
    fn of(pin: &Pin, refs: &HashMap<String, String>) -> Self {
        let resolved = |text: &str| resolve_schema(text, refs).unwrap_or(text).to_owned();
        let options = pin.options.as_ref();
        Self {
            pin_id: pin.id.clone(),
            name: pin.name.clone(),
            label: pin.friendly_name.clone(),
            description: resolved(&pin.description),
            data_type: pin.data_type.clone(),
            value_type: pin.value_type.clone(),
            schema: pin.schema.as_deref().map(resolved),
            optional: options
                .and_then(|options| options.optional)
                .unwrap_or(false),
            sensitive: options
                .and_then(|options| options.sensitive)
                .unwrap_or(false),
            default: pin.default_value.clone(),
            options: options.and_then(|options| options.valid_values.clone()),
            index: pin.index,
        }
    }

    fn takes_file(&self) -> bool {
        matches!(self.data_type, VariableType::PathBuf | VariableType::Byte)
    }

    fn data_type_word(&self) -> String {
        format!("{:?}", self.data_type)
    }

    fn value_type_word(&self) -> String {
        format!("{:?}", self.value_type)
    }

    /// The default a person may see: never a sensitive one, and only a small one.
    /// The flag says that a default exists but was left out for its size.
    fn shown_default(&self) -> (Value, bool) {
        let value = self
            .default
            .as_deref()
            .filter(|_| !self.sensitive)
            .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
            .unwrap_or(Value::Null);
        match serde_json::to_vec(&value) {
            Ok(bytes) if bytes.len() <= MAX_DEFAULT_BYTES => (value, false),
            _ => (Value::Null, true),
        }
    }

    fn shown_options(&self) -> Option<&[String]> {
        self.options.as_deref().filter(|values| {
            !values.is_empty()
                && values.len() <= MAX_OPTIONS
                && values
                    .iter()
                    .all(|value| value.chars().count() <= MAX_OPTION_CHARS)
        })
    }

    fn contract(&self) -> Value {
        let (default, omitted) = self.shown_default();
        let mut field = json!({
            "name": cut(&self.name, MAX_LABEL_CHARS),
            "label": cut(&self.label, MAX_LABEL_CHARS),
            "description": cut(&self.description, MAX_DESCRIPTION_CHARS),
            "data_type": self.data_type_word(),
            "value_type": self.value_type_word(),
            "optional": self.optional,
            "sensitive": self.sensitive,
            "default": default,
            "options": self.shown_options(),
        });
        if omitted {
            field["default_omitted"] = Value::Bool(true);
        }
        field
    }

    fn input(&self) -> EventInput {
        EventInput {
            id: self.pin_id.clone(),
            name: self.name.clone(),
            friendly_name: self.label.clone(),
            description: self.description.clone(),
            data_type: self.data_type_word(),
            value_type: self.value_type_word(),
            schema: self.schema.clone(),
            default_value: self.default.clone().filter(|_| !self.sensitive),
            optional: self.optional,
            index: self.index,
        }
    }
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// A quick action or form of this placement process, prepared from the flow version it runs.
#[derive(Clone)]
pub(crate) struct OnDemandEvent {
    invocation: PreparedInvocation,
    fields: Vec<Field>,
    navigate_to_routes: Vec<String>,
}

impl std::fmt::Debug for OnDemandEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OnDemandEvent")
            .field("event_id", &self.invocation.event.id)
            .field("fields", &self.fields.len())
            .finish()
    }
}

impl OnDemandEvent {
    pub(crate) fn id(&self) -> &str {
        &self.invocation.event.id
    }

    /// The prepared event a run of it executes.
    pub(crate) fn invocation(&self) -> &PreparedInvocation {
        &self.invocation
    }

    pub(crate) fn kind(&self) -> Kind {
        if self.fields.is_empty() {
            Kind::Action
        } else {
            Kind::Form
        }
    }

    /// Fields that take a file, which only the service page can send.
    pub(crate) fn file_fields(&self) -> usize {
        self.fields
            .iter()
            .filter(|field| field.takes_file())
            .count()
    }

    /// This event's entry of the contract file, at most `budget` bytes as JSON: fields are
    /// dropped from the end beyond it, and beyond 64.
    fn contract_entry(&self, budget: usize) -> Value {
        let event = &self.invocation.event;
        let fields: Vec<Value> = self
            .fields
            .iter()
            .take(MAX_FIELDS)
            .map(Field::contract)
            .collect();
        let mut kept = fields.len();
        loop {
            let entry = json!({
                "kind": self.kind(),
                "name": cut(&event.name, MAX_LABEL_CHARS),
                "description": cut(&event.description, MAX_DESCRIPTION_CHARS),
                "event_version": event.event_version,
                "board_version": event.board_version,
                "fields": &fields[..kept],
                "fields_truncated": kept < self.fields.len(),
                "file_fields": self.file_fields(),
                "navigate_to_routes": self.navigate_to_routes,
            });
            if kept == 0 || json_len(&entry) <= budget {
                return entry;
            }
            kept -= 1;
        }
    }
}

fn json_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

/// The fields are the entry node's output pins without execution pins and the `payload`
/// pin, in pin order: the rule of `Event::populate_inputs`, read from the compiled flow
/// this process runs rather than from the event's stored list.
pub(crate) fn prepare(invocation: PreparedInvocation) -> Result<OnDemandEvent> {
    let event = &invocation.event;
    let node = invocation
        .template
        .board
        .nodes
        .get(&event.node_id)
        .with_context(|| format!("event {} does not reference its entry node", event.id))?;
    let mut pins: Vec<&Pin> = node
        .pins
        .values()
        .filter(|pin| {
            pin.pin_type == PinType::Output
                && pin.data_type != VariableType::Execution
                && pin.name != PAYLOAD_PIN
        })
        .collect();
    pins.sort_by(|left, right| {
        left.index
            .cmp(&right.index)
            .then_with(|| left.name.cmp(&right.name))
    });
    let refs = &invocation.template.board.refs;
    let mut names = HashSet::new();
    let fields = pins
        .into_iter()
        .filter(|pin| names.insert(pin.name.as_str()))
        .map(|pin| Field::of(pin, refs))
        .collect();
    let navigate_to_routes = navigate_to_routes(&event.config);
    Ok(OnDemandEvent {
        invocation,
        fields,
        navigate_to_routes,
    })
}

fn navigate_to_routes(config: &[u8]) -> Vec<String> {
    serde_json::from_slice::<Value>(config)
        .ok()
        .and_then(|config| config.get("navigate_to_routes").cloned())
        .and_then(|routes| match routes {
            Value::Array(routes) => Some(routes),
            _ => None,
        })
        .into_iter()
        .flatten()
        .filter_map(|route| match route {
            Value::String(route) => Some(route),
            _ => None,
        })
        .filter(|route| {
            !route.is_empty()
                && route.chars().count() <= MAX_ROUTE_CHARS
                && !route.chars().any(char::is_control)
        })
        .take(MAX_ROUTES)
        .collect()
}

/// The service page's view of the event, as `GET /services` lists it: the fields derived
/// here, a config reduced to `navigate_to_routes`, and nothing a person must not see.
pub(crate) fn inventory_entry(event: &OnDemandEvent) -> Value {
    let mut public = event.invocation.event.clone();
    public.variables.clear();
    public.node_id.clear();
    public.notes = None;
    public.canary = None;
    public.variants.clear();
    public.correlation_mappings = None;
    public.config = serde_json::to_vec(&json!({"navigate_to_routes": event.navigate_to_routes}))
        .unwrap_or_default();
    public.inputs = event.fields.iter().map(Field::input).collect();
    serde_json::to_value(public).unwrap_or(Value::Null)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Door {
    Management,
    ServicePage,
}

/// The field check of the service page: a file field takes `data:` URLs there.
pub(crate) fn check_fields(event: &OnDemandEvent, payload: &Value) -> Result<(), Vec<String>> {
    refused_fields(
        &event.fields,
        &event.invocation.template.board.refs,
        payload,
        Door::ServicePage,
    )
}

/// The names a payload is refused for: fields with a value they do not take, required
/// fields without a value, and keys that are no field. Bounded as the parent stores them.
/// A payload that is not an object is refused with no names.
fn refused_fields(
    fields: &[Field],
    refs: &HashMap<String, String>,
    payload: &Value,
    door: Door,
) -> Result<(), Vec<String>> {
    let Some(values) = payload.as_object() else {
        return Err(Vec::new());
    };
    let known: HashSet<&str> = fields.iter().map(|field| field.name.as_str()).collect();
    let refused: Vec<&str> = fields
        .iter()
        .filter(|field| match values.get(&field.name) {
            None => !field.optional,
            Some(value) => !accepts(field, value, refs, door),
        })
        .map(|field| field.name.as_str())
        .chain(
            values
                .keys()
                .map(String::as_str)
                .filter(|key| !known.contains(key)),
        )
        .collect();
    if refused.is_empty() {
        Ok(())
    } else {
        Err(bounded_names(&refused))
    }
}

fn accepts(field: &Field, value: &Value, refs: &HashMap<String, String>, door: Door) -> bool {
    match field.data_type {
        VariableType::PathBuf | VariableType::Byte => {
            door == Door::ServicePage && each_item(field, value, is_data_url)
        }
        VariableType::Date => each_item(field, value, is_date),
        VariableType::Geometry if each_item(field, value, Value::is_string) => true,
        _ => {
            let mut variable = Variable::new(
                &field.name,
                field.data_type.clone(),
                field.value_type.clone(),
            );
            variable.schema = field.schema.clone();
            crate::runtime::validate_override(&variable, refs, value).is_ok()
        }
    }
}

/// `accept` for the value itself, or for every member of the field's collection.
fn each_item(field: &Field, value: &Value, accept: impl Fn(&Value) -> bool) -> bool {
    match field.value_type {
        ValueType::Normal => accept(value),
        ValueType::Array => value
            .as_array()
            .is_some_and(|items| items.len() <= MAX_ITEMS && items.iter().all(&accept)),
        ValueType::HashSet => value.as_array().is_some_and(|items| {
            let mut seen = HashSet::new();
            items.len() <= MAX_ITEMS
                && items
                    .iter()
                    .all(|item| accept(item) && seen.insert(item.to_string()))
        }),
        ValueType::HashMap => value
            .as_object()
            .is_some_and(|items| items.len() <= MAX_ITEMS && items.values().all(&accept)),
    }
}

/// `YYYY-MM-DD` of a real calendar day, or an RFC 3339 date and time.
fn is_date(value: &Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    let date_only = text.len() == 10
        && text.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    if date_only {
        chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
    } else {
        chrono::DateTime::parse_from_rfc3339(text).is_ok()
    }
}

fn is_data_url(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
            && text.contains(',')
    })
}

/// At most 16 names of 64 characters, and at most 2 KiB as a JSON array.
fn bounded_names(names: &[&str]) -> Vec<String> {
    let mut kept = Vec::new();
    let mut bytes = 2;
    for name in names.iter().take(MAX_REFUSED_NAMES) {
        let name = cut(name, MAX_REFUSED_NAME_CHARS);
        let size = serde_json::to_vec(&name).map_or(usize::MAX, |encoded| encoded.len()) + 1;
        if bytes + size > MAX_REFUSED_NAMES_BYTES {
            break;
        }
        bytes += size;
        kept.push(name);
    }
    kept
}

/// What a run gave back, as a person may read it: a JSON result, or text.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Output {
    /// `{"json": value}` or `{"text": "…"}`, at most 8 KiB as JSON.
    pub(crate) output: Option<Value>,
    /// The size of the whole result, before it was shortened.
    pub(crate) output_bytes: u64,
    pub(crate) truncated: bool,
    pub(crate) attachments: u64,
}

/// A result kept while the run goes on: small JSON as it is, anything larger as the start
/// of its text with its full size.
#[derive(Debug)]
enum Kept {
    Json(Value),
    Text { text: String, bytes: usize },
}

impl Kept {
    fn json(value: &Value) -> Self {
        let Ok(encoded) = serde_json::to_string(value) else {
            return Self::text(String::new());
        };
        if encoded.len() <= MAX_OUTPUT_BYTES {
            Self::Json(value.clone())
        } else {
            Self::text(encoded)
        }
    }

    fn text(text: String) -> Self {
        let bytes = text.len();
        let mut text = text;
        text.truncate(floor_char_boundary(&text, MAX_KEPT_TEXT_BYTES));
        Self::Text { text, bytes }
    }

    fn output(self) -> (Value, u64, bool) {
        match self {
            Self::Json(value) => {
                let bytes = json_len(&value) as u64;
                let output = json!({ "json": value });
                if json_len(&output) <= MAX_OUTPUT_BYTES {
                    return (output, bytes, false);
                }
                let encoded = value.to_string();
                let (output, _) = text_output(&encoded);
                (output, bytes, true)
            }
            Self::Text { text, bytes } => {
                let (output, cut) = text_output(&text);
                (output, bytes as u64, cut || text.len() < bytes)
            }
        }
    }
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    (0..=index)
        .rev()
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(0)
}

/// The bytes a character takes inside a JSON string.
fn escaped_len(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
        character if (character as u32) < 0x20 => 6,
        character => character.len_utf8(),
    }
}

/// `{"text": …}` with the text cut at a character so that the whole object fits 8 KiB as
/// JSON; escaping can make text up to six times longer. True when it was cut.
fn text_output(text: &str) -> (Value, bool) {
    let budget = MAX_OUTPUT_BYTES - r#"{"text":""}"#.len();
    let mut used = 0;
    let mut end = text.len();
    for (index, character) in text.char_indices() {
        let size = escaped_len(character);
        if used + size > budget {
            end = index;
            break;
        }
        used += size;
    }
    (json!({ "text": &text[..end] }), end < text.len())
}

/// Collects what a run streams into its result. Never copies a media part, whose address
/// can be a signed link: those are only counted.
#[derive(Debug, Default)]
pub(crate) struct Fold {
    result: Option<Kept>,
    response: Option<Kept>,
    text: String,
    text_bytes: usize,
    attachments: HashSet<u64>,
    question: bool,
}

impl Fold {
    pub(crate) fn add(&mut self, event: &InterComEvent) {
        let payload = &event.payload;
        match event.event_type.as_str() {
            "generic_result" => self.result = Some(Kept::json(payload)),
            "chat_out" | "chat_stream" => {
                if let Some(text) = response_text(payload) {
                    self.response = Some(Kept::text(text));
                }
                self.media(payload);
            }
            "chat_stream_partial" => self.media(payload),
            "text_output" | "stream_text" => {
                if let Some(chunk) = text_chunk(payload) {
                    self.append(chunk);
                }
            }
            "interaction_request" => self.question = true,
            _ => {}
        }
    }

    /// The flow asked a question that nobody can answer here.
    pub(crate) fn asked(&self) -> bool {
        self.question
    }

    fn append(&mut self, chunk: &str) {
        self.text_bytes = self.text_bytes.saturating_add(chunk.len());
        let room = MAX_KEPT_TEXT_BYTES.saturating_sub(self.text.len());
        self.text
            .push_str(&chunk[..floor_char_boundary(chunk, room)]);
    }

    fn media(&mut self, payload: &Value) {
        let attachments = payload
            .get("attachments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(attachment_identity);
        let parts = payload
            .pointer("/response/choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|choice| choice.pointer("/message/content_parts"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(media_part);
        for identity in attachments.chain(parts) {
            if self.attachments.len() >= MAX_TRACKED_ATTACHMENTS {
                break;
            }
            use std::hash::{Hash, Hasher};
            let mut hasher = std::hash::DefaultHasher::new();
            identity.hash(&mut hasher);
            self.attachments.insert(hasher.finish());
        }
    }

    /// The result of a run: the last `generic_result`, else the last chat answer, else the
    /// text the run streamed.
    pub(crate) fn finish(self) -> Output {
        let attachments = self.attachments.len() as u64;
        let kept = self.result.or(self.response).or_else(|| {
            (self.text_bytes > 0).then(|| Kept::Text {
                text: self.text,
                bytes: self.text_bytes,
            })
        });
        let Some(kept) = kept else {
            return Output {
                attachments,
                ..Output::default()
            };
        };
        let (output, output_bytes, truncated) = kept.output();
        Output {
            output: Some(output),
            output_bytes,
            truncated,
            attachments,
        }
    }
}

fn response_text(payload: &Value) -> Option<String> {
    let message = payload.pointer("/response/choices/0/message")?;
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        return Some(content.to_owned());
    }
    let parts = message.get("content_parts")?.as_array()?;
    let text: String = parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect();
    (!text.is_empty()).then_some(text)
}

fn text_chunk(payload: &Value) -> Option<&str> {
    payload
        .as_str()
        .or_else(|| payload.get("text").and_then(Value::as_str))
}

fn attachment_identity(attachment: &Value) -> String {
    attachment
        .as_str()
        .or_else(|| attachment.get("url").and_then(Value::as_str))
        .map_or_else(|| attachment.to_string(), str::to_owned)
}

fn media_part(part: &Value) -> Option<String> {
    part.pointer("/image_url/url")
        .or_else(|| part.get("audio_url"))
        .or_else(|| part.get("video_url"))
        .or_else(|| part.get("document_url"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The parent's side of person-started runs, as a placement process sees it.
#[async_trait::async_trait]
pub(crate) trait RunSource: Send + Sync {
    /// The periodic question: hands over runs that ended and takes new ones. Fails with
    /// [`SourceBusy`], without asking, while the channel serves another request.
    async fn exchange(&self, finished: Vec<FinishedRun>) -> Result<RunBatch>;
    /// The question of a process that stops: it takes no new run and waits for the channel.
    async fn report(&self, finished: Vec<FinishedRun>) -> Result<RunBatch>;
}

/// The periodic question was not asked because the channel was busy.
#[derive(Debug, thiserror::Error)]
#[error("The supervisor channel is busy")]
pub(crate) struct SourceBusy;

/// What the person-started events of one placement process share.
pub(crate) struct OnDemandContext {
    pub(crate) run: Arc<RunContext>,
    pub(crate) source: Option<Arc<dyn RunSource>>,
    /// `<data root>/.standalone-run/<placement id>`, see [`state_directory`].
    pub(crate) state_dir: PathBuf,
    pub(crate) placement_id: String,
    pub(crate) slot: u8,
    pub(crate) config_revision: u64,
    pub(crate) intent_revision: u64,
    /// The longest a run may take, see [`time_limit`].
    pub(crate) time_limit: Duration,
    counters: Counters,
}

impl OnDemandContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        run: Arc<RunContext>,
        source: Option<Arc<dyn RunSource>>,
        state_dir: PathBuf,
        placement_id: String,
        slot: u8,
        config_revision: u64,
        intent_revision: u64,
        time_limit: Duration,
    ) -> Self {
        let counters = Counters::new(&state_dir, slot);
        Self {
            run,
            source,
            state_dir,
            placement_id,
            slot,
            config_revision,
            intent_revision,
            time_limit,
            counters,
        }
    }

    /// The counters of this instance's state file, shared with the service page's runs.
    pub(crate) fn counters(&self) -> Counters {
        self.counters.clone()
    }
}

/// Where a placement process keeps its person-started events' files.
pub(crate) fn state_directory(data_root: &Path, placement_id: &str) -> PathBuf {
    data_root.join(STATE_DIRECTORY).join(placement_id)
}

/// The service's request time limit when it has a web endpoint, else an hour.
pub(crate) fn time_limit(config: &PlacementConfig) -> Duration {
    config
        .hosting
        .as_ref()
        .map_or(DEFAULT_TIME_LIMIT, |hosting| {
            Duration::from_secs(hosting.request_timeout_secs.into())
        })
}

/// How a run ended, as the state file counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

impl From<RunEnd> for Outcome {
    fn from(end: RunEnd) -> Self {
        match end {
            RunEnd::Succeeded => Self::Succeeded,
            RunEnd::Failed => Self::Failed,
            RunEnd::Cancelled => Self::Cancelled,
            RunEnd::TimedOut => Self::TimedOut,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LastRun {
    at: i64,
    finished_at: i64,
    outcome: Outcome,
    origin: Origin,
}

/// One event's entry of a state file. Counts only: no input, no result, no error text.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct EventCounters {
    kind: Kind,
    fields: usize,
    file_fields: usize,
    running: u64,
    runs: u64,
    failed: u64,
    last: Option<LastRun>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StateDocument {
    version: u64,
    config_revision: u64,
    intent_revision: u64,
    events: BTreeMap<String, EventCounters>,
}

/// The run counts of one instance, kept in its `state.<slot>.json`: with several instances
/// each writes its own file, and the agent parent adds them up.
#[derive(Clone)]
pub(crate) struct Counters(Arc<CountersState>);

struct CountersState {
    directory: PathBuf,
    file: String,
    document: Mutex<StateDocument>,
    /// Held while the file is written, so a later state never loses to an earlier one.
    writing: Mutex<()>,
}

impl Counters {
    fn new(directory: &Path, slot: u8) -> Self {
        Self(Arc::new(CountersState {
            directory: directory.to_path_buf(),
            file: format!("state.{slot}.json"),
            document: Mutex::default(),
            writing: Mutex::default(),
        }))
    }

    /// Before Ready: an entry per event, with the counts an earlier process of the same
    /// intent revision left. Nothing runs yet.
    fn reset(
        &self,
        events: &[OnDemandEvent],
        config_revision: u64,
        intent_revision: u64,
    ) -> Result<()> {
        let earlier = read_state(&self.0.directory.join(&self.0.file))
            .filter(|earlier| earlier.intent_revision == intent_revision)
            .map(|earlier| earlier.events)
            .unwrap_or_default();
        {
            let mut document = lock(&self.0.document);
            *document = StateDocument {
                version: STATE_VERSION,
                config_revision,
                intent_revision,
                events: events
                    .iter()
                    .map(|event| {
                        let carried = earlier.get(event.id());
                        let entry = EventCounters {
                            kind: event.kind(),
                            fields: event.fields.len(),
                            file_fields: event.file_fields(),
                            running: 0,
                            runs: carried.map_or(0, |carried| carried.runs),
                            failed: carried.map_or(0, |carried| carried.failed),
                            last: carried.and_then(|carried| carried.last),
                        };
                        (event.id().to_owned(), entry)
                    })
                    .collect(),
            };
        }
        self.persist()
    }

    /// A run started. It counts as running until its mark is ended or dropped; a dropped
    /// mark counts the run as cancelled.
    pub(crate) fn begin(&self, event_id: &str, origin: Origin) -> Mark {
        self.update(event_id, |entry| {
            entry.running = entry.running.saturating_add(1)
        });
        Mark {
            counters: self.clone(),
            event_id: event_id.to_owned(),
            origin,
            at: unix_now(),
            closed: false,
        }
    }

    fn update(&self, event_id: &str, change: impl FnOnce(&mut EventCounters)) {
        {
            let mut document = lock(&self.0.document);
            let Some(entry) = document.events.get_mut(event_id) else {
                return;
            };
            change(entry);
        }
        if let Err(error) = blocking(|| self.persist()) {
            tracing::warn!(event_id, "Run state could not be written: {error:#}");
        }
    }

    fn persist(&self) -> Result<()> {
        let _writing = lock(&self.0.writing);
        let bytes = serde_json::to_vec(&*lock(&self.0.document))?;
        write_file(&self.0.directory, &self.0.file, &bytes, MAX_STATE_BYTES)
    }
}

impl EventCounters {
    fn ended(&mut self, at: i64, finished_at: i64, outcome: Outcome, origin: Origin) {
        self.runs = self.runs.saturating_add(1);
        if matches!(outcome, Outcome::Failed | Outcome::TimedOut) {
            self.failed = self.failed.saturating_add(1);
        }
        self.last = Some(LastRun {
            at,
            finished_at,
            outcome,
            origin,
        });
    }
}

/// A run that counts as running.
pub(crate) struct Mark {
    counters: Counters,
    event_id: String,
    origin: Origin,
    at: i64,
    closed: bool,
}

impl Mark {
    pub(crate) fn end(mut self, end: RunEnd) {
        self.close(end.into());
    }

    fn close(&mut self, outcome: Outcome) {
        if std::mem::replace(&mut self.closed, true) {
            return;
        }
        let (at, origin) = (self.at, self.origin);
        self.counters.update(&self.event_id, |entry| {
            entry.running = entry.running.saturating_sub(1);
            entry.ended(at, unix_now(), outcome, origin);
        });
    }
}

impl Drop for Mark {
    fn drop(&mut self) {
        self.close(Outcome::Cancelled);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// A small write must not stall the worker it runs on; other tasks move meanwhile.
fn blocking<T>(work: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current().map(|handle| handle.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(work),
        _ => work(),
    }
}

/// This instance's own earlier state file, when it is one this agent wrote.
fn read_state(path: &Path) -> Option<StateDocument> {
    let bytes = crate::diagnostics::small_file(path, MAX_STATE_BYTES)
        .ok()
        .flatten()
        .filter(|bytes| bytes.len() <= MAX_STATE_BYTES)?;
    serde_json::from_slice::<StateDocument>(&bytes)
        .ok()
        .filter(|state| state.version == STATE_VERSION)
}

fn create_state_directory(directory: &Path) -> Result<()> {
    directory
        .parent()
        .context("Run state directory has no parent")
        .and_then(crate::runtime::private_runtime_directory)
        .and_then(|()| crate::runtime::private_runtime_directory(directory))
        .with_context(|| format!("Create the run state directory {}", directory.display()))
}

/// Replaces `name` as a whole. Instances of one service share the directory, so the
/// temporary file carries the process id.
fn write_file(directory: &Path, name: &str, bytes: &[u8], max: usize) -> Result<()> {
    ensure!(
        bytes.len() <= max,
        "Run state file {name} of {} bytes exceeds its {max} byte limit",
        bytes.len()
    );
    let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temporary, directory.join(name)))
        .and_then(|()| std::fs::File::open(directory)?.sync_all())
        .with_context(|| format!("Write run state file {name} in {}", directory.display()))
}

/// The forms of this process for the agent parent's `event_form`: the same for every
/// instance of one config revision.
fn contract(
    events: &[OnDemandEvent],
    config_revision: u64,
    intent_revision: u64,
) -> Result<Vec<u8>> {
    let document = |budget: usize| {
        json!({
            "version": STATE_VERSION,
            "config_revision": config_revision,
            "intent_revision": intent_revision,
            "events": events
                .iter()
                .map(|event| (event.id().to_owned(), event.contract_entry(budget)))
                .collect::<serde_json::Map<String, Value>>(),
        })
    };
    let mut built = document(MAX_ENTRY_BYTES);
    if json_len(&built) > MAX_CONTRACT_BYTES {
        built = document((MAX_CONTRACT_BYTES - 1024) / events.len().max(1));
    }
    Ok(serde_json::to_vec(&built)?)
}

/// Before Ready: the contract file and this instance's state file. A start fails when they
/// cannot be written.
pub(crate) fn prepare_state(events: &[OnDemandEvent], context: &OnDemandContext) -> Result<()> {
    create_state_directory(&context.state_dir)?;
    write_file(
        &context.state_dir,
        CONTRACT_FILE,
        &contract(events, context.config_revision, context.intent_revision)?,
        MAX_CONTRACT_BYTES,
    )?;
    context
        .counters
        .reset(events, context.config_revision, context.intent_revision)
}

async fn ready_or_stopped(ready: oneshot::Receiver<()>, stop: &CancellationToken) -> bool {
    tokio::select! {
        biased;
        _ = stop.cancelled() => false,
        signal = ready => {
            if signal.is_err() {
                stop.cancelled().await;
            }
            signal.is_ok()
        }
    }
}

/// Fetches the runs the agent parent queued for this service, runs them and reports how
/// they ended. Returns only on stop: runs in progress then get 12 s, the rest is cut off,
/// and what ended is reported with a last question that waits for the channel.
pub(crate) async fn run(
    events: Vec<OnDemandEvent>,
    context: OnDemandContext,
    ready: oneshot::Receiver<()>,
    stop: CancellationToken,
) -> Result<()> {
    if !ready_or_stopped(ready, &stop).await {
        return Ok(());
    }
    let source = match context.source.clone() {
        Some(source) if !events.is_empty() => source,
        _ => {
            stop.cancelled().await;
            return Ok(());
        }
    };
    let span = tracing::info_span!(
        "person_started_runs",
        placement_id = %context.placement_id,
        slot = context.slot
    );
    Worker::new(events, &context, source)
        .serve(stop)
        .instrument(span)
        .await;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cause {
    Cancelled,
    Interrupted,
}

struct Running {
    event_id: String,
    run_id: String,
    started: Instant,
    started_at: i64,
    cancel: CancellationToken,
    cause: Option<Cause>,
    mark: Mark,
}

/// What a run's task gives back.
struct Ended {
    end: RunEnd,
    asked: bool,
    output: Output,
}

struct Worker {
    events: HashMap<String, Arc<OnDemandEvent>>,
    run: Arc<RunContext>,
    source: Arc<dyn RunSource>,
    counters: Counters,
    time_limit: Duration,
    running: HashMap<String, Running>,
    tasks: JoinSet<Ended>,
    task_runs: HashMap<tokio::task::Id, String>,
    reports: Vec<FinishedRun>,
    unanswered: u32,
}

impl Worker {
    fn new(
        events: Vec<OnDemandEvent>,
        context: &OnDemandContext,
        source: Arc<dyn RunSource>,
    ) -> Self {
        Self {
            events: events
                .into_iter()
                .map(|event| (event.id().to_owned(), Arc::new(event)))
                .collect(),
            run: context.run.clone(),
            source,
            counters: context.counters(),
            time_limit: context.time_limit.max(Duration::from_secs(1)),
            running: HashMap::new(),
            tasks: JoinSet::new(),
            task_runs: HashMap::new(),
            reports: Vec::new(),
            unanswered: 0,
        }
    }

    async fn serve(mut self, stop: CancellationToken) {
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                Some(result) = self.tasks.join_next_with_id(), if !self.tasks.is_empty() => {
                    self.finished(result);
                    continue;
                }
                _ = tick.tick() => {}
            }
            // A run handed out while the question is dropped here ends as interrupted when
            // this process's channel closes: a stopping service cannot run it.
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                _ = self.ask(true) => {}
            }
        }
        self.drain().await;
    }

    /// One question to the parent. `open`: new runs are taken; otherwise the process stops
    /// and only reports.
    async fn ask(&mut self, open: bool) {
        let count = self.reports.len().min(MAX_REPORTS_PER_QUESTION);
        let finished = self.reports[..count].to_vec();
        let answer = if open {
            self.source.exchange(finished).await
        } else {
            self.source.report(finished).await
        };
        match answer {
            Ok(batch) => {
                self.reports.drain(..count);
                self.unanswered = 0;
                self.take(batch, open);
            }
            Err(error) if error.is::<SourceBusy>() => {}
            Err(error) => {
                self.unanswered = self.unanswered.saturating_add(1);
                if self.unanswered == 1 {
                    tracing::warn!(
                        "The supervisor did not answer the question for person-started runs: {error:#}"
                    );
                }
            }
        }
    }

    fn take(&mut self, batch: RunBatch, open: bool) {
        for operation_id in batch.cancel {
            if let Some(running) = self.running.get_mut(&operation_id) {
                running.cause.get_or_insert(Cause::Cancelled);
                running.cancel.cancel();
            }
        }
        for start in batch.start {
            let known = self.running.contains_key(&start.operation_id)
                || self
                    .reports
                    .iter()
                    .any(|report| report.operation_id == start.operation_id);
            if known {
                continue;
            }
            match self.events.get(&start.event_id).cloned() {
                Some(event) if open => self.start(event, start),
                _ => {
                    tracing::info!(
                        event_id = %start.event_id,
                        run_id = %start.run_id,
                        code = "not_started",
                        "Person-started run was not started"
                    );
                    self.reports
                        .push(unstarted(start.operation_id, "not_started", Vec::new()));
                }
            }
        }
    }

    fn start(&mut self, event: Arc<OnDemandEvent>, start: StartRun) {
        let StartRun {
            operation_id,
            run_id,
            event_id,
            payload,
            time_limit_secs,
        } = start;
        let payload = payload.unwrap_or_else(|| Value::Object(Default::default()));
        let refs = &event.invocation.template.board.refs;
        if let Err(fields) = refused_fields(&event.fields, refs, &payload, Door::Management) {
            tracing::info!(
                event_id,
                run_id,
                code = "invalid_fields",
                "Person-started run was refused"
            );
            self.reports
                .push(unstarted(operation_id, "invalid_fields", fields));
            return;
        }
        let time_limit = match time_limit_secs {
            0 => self.time_limit,
            secs => self.time_limit.min(Duration::from_secs(secs)),
        };
        let cancel = CancellationToken::new();
        let mark = self.counters.begin(&event_id, Origin::Management);
        tracing::info!(event_id, run_id, "Person-started run started");
        let task = self.tasks.spawn(execute(
            event,
            self.run.clone(),
            payload,
            run_id.clone(),
            cancel.clone(),
            time_limit,
        ));
        self.task_runs.insert(task.id(), operation_id.clone());
        self.running.insert(
            operation_id,
            Running {
                event_id,
                run_id,
                started: Instant::now(),
                started_at: unix_now(),
                cancel,
                cause: None,
                mark,
            },
        );
    }

    fn finished(&mut self, result: Result<(tokio::task::Id, Ended), tokio::task::JoinError>) {
        let (task, ended) = match result {
            Ok((task, ended)) => (task, Some(ended)),
            Err(error) => (error.id(), None),
        };
        let Some(operation_id) = self.task_runs.remove(&task) else {
            return;
        };
        let Some(running) = self.running.remove(&operation_id) else {
            return;
        };
        let (run, code, outcome) = conclusion(running.cause, ended.as_ref());
        let Running {
            event_id,
            run_id,
            started,
            started_at,
            mut mark,
            ..
        } = running;
        mark.close(outcome);
        tracing::info!(
            event_id,
            run_id,
            outcome = run,
            code,
            duration_ms = started.elapsed().as_millis() as u64,
            "Person-started run finished"
        );
        let output = ended
            .filter(|_| outcome == Outcome::Succeeded)
            .map(|ended| ended.output)
            .unwrap_or_default();
        self.reports.push(FinishedRun {
            operation_id,
            run: run.into(),
            code: code.map(str::to_owned),
            fields: Vec::new(),
            started_at,
            finished_at: unix_now(),
            output: output.output,
            output_bytes: output.output_bytes,
            truncated: output.truncated,
            attachments: u32::try_from(output.attachments).unwrap_or(u32::MAX),
        });
    }

    /// Stop: no new run. Runs in progress get the drain time, then they are cut off; what
    /// ended is reported with a question that waits for the channel.
    async fn drain(&mut self) {
        self.settle(STOP_GRACE, true).await;
        for running in self.running.values_mut() {
            running.cause.get_or_insert(Cause::Interrupted);
            running.cancel.cancel();
        }
        self.settle(CANCEL_GRACE, false).await;
        self.tasks.abort_all();
        while let Some(result) = self.tasks.join_next_with_id().await {
            self.finished(result);
        }
        self.last_report().await;
    }

    /// Takes runs as they end, for at most `grace`. While `asking` it reports them and takes
    /// cancellations as it goes, without accepting a new run.
    async fn settle(&mut self, grace: Duration, asking: bool) {
        let deadline = tokio::time::Instant::now() + grace;
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        while !self.tasks.is_empty() {
            tokio::select! {
                biased;
                Some(result) = self.tasks.join_next_with_id() => self.finished(result),
                _ = tokio::time::sleep_until(deadline) => break,
                _ = tick.tick(), if asking => {
                    let _ = tokio::time::timeout(POLL, self.ask(false)).await;
                }
            }
        }
    }

    async fn last_report(&mut self) {
        let deadline = tokio::time::Instant::now() + LAST_REPORT_WAIT;
        while !self.reports.is_empty() {
            let count = self.reports.len().min(MAX_REPORTS_PER_QUESTION);
            let finished = self.reports[..count].to_vec();
            match tokio::time::timeout_at(deadline, self.source.report(finished)).await {
                Ok(Ok(_)) => {
                    self.reports.drain(..count);
                }
                _ => {
                    tracing::warn!(
                        runs = self.reports.len(),
                        "The ends of person-started runs could not be reported before the service stopped"
                    );
                    return;
                }
            }
        }
    }
}

/// `run`, `code` and the counted outcome of a run that ended. A run that succeeded is
/// reported as such also when it was told to stop a moment later.
fn conclusion(
    cause: Option<Cause>,
    ended: Option<&Ended>,
) -> (&'static str, Option<&'static str>, Outcome) {
    const FAILED: &str = "failed";
    match ended.map(|ended| (ended.end, ended.asked)) {
        Some((RunEnd::Succeeded, _)) => ("succeeded", None, Outcome::Succeeded),
        Some((_, true)) => (FAILED, Some("needs_interaction"), Outcome::Failed),
        Some((RunEnd::Failed, _)) => (FAILED, Some("flow_failed"), Outcome::Failed),
        Some((RunEnd::TimedOut, _)) => ("timed_out", Some("timed_out"), Outcome::TimedOut),
        Some((RunEnd::Cancelled, _)) | None => match cause {
            Some(Cause::Interrupted) => (FAILED, Some("interrupted"), Outcome::Cancelled),
            Some(Cause::Cancelled) => ("cancelled", Some("cancelled"), Outcome::Cancelled),
            None if ended.is_some() => ("cancelled", Some("cancelled"), Outcome::Cancelled),
            None => (FAILED, Some("flow_failed"), Outcome::Failed),
        },
    }
}

fn unstarted(operation_id: String, code: &str, fields: Vec<String>) -> FinishedRun {
    let now = unix_now();
    FinishedRun {
        operation_id,
        run: "failed".into(),
        code: Some(code.into()),
        fields,
        started_at: now,
        finished_at: now,
        output: None,
        output_bytes: 0,
        truncated: false,
        attachments: 0,
    }
}

async fn execute(
    event: Arc<OnDemandEvent>,
    run: Arc<RunContext>,
    payload: Value,
    run_id: String,
    cancel: CancellationToken,
    time_limit: Duration,
) -> Ended {
    let fold = Arc::new(Mutex::new(Fold::default()));
    let request_bytes = serde_json::to_vec(&payload).map_or(0, |bytes| bytes.len() as u64);
    let end = run_event_once(
        &run,
        &event.invocation,
        RunRequest {
            payload: Some(payload),
            callback: folding(fold.clone(), cancel.clone()),
            cancel,
            run_id: Some(run_id),
            time_limit,
            request_bytes,
        },
    )
    .await;
    let fold = std::mem::take(&mut *lock(&fold));
    Ended {
        end,
        asked: fold.asked(),
        output: fold.finish(),
    }
}

/// Folds every event of a run; a question cancels the run at once.
fn folding(fold: Arc<Mutex<Fold>>, cancel: CancellationToken) -> InterComCallback {
    use futures_util::FutureExt;
    Some(Arc::new(move |event: InterComEvent| {
        let mut fold = lock(&fold);
        fold.add(&event);
        if fold.asked() {
            cancel.cancel();
        }
        futures_util::future::ready(Ok(())).boxed()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ValueType as V;
    use VariableType as T;
    use flow_like_runtime::{
        app::App,
        flow::{
            board::Board,
            compiled::TemplateCache,
            execution::context::ExecutionContext,
            node::{Node, NodeLogic},
            pin::PinOptions,
        },
    };
    use flow_like_storage::Path as StorePath;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn field(name: &str, data_type: VariableType, value_type: ValueType) -> Field {
        Field {
            pin_id: format!("pin-{name}"),
            name: name.into(),
            label: name.into(),
            description: String::new(),
            data_type,
            value_type,
            schema: None,
            optional: false,
            sensitive: false,
            default: None,
            options: None,
            index: 0,
        }
    }

    fn optional(mut field: Field) -> Field {
        field.optional = true;
        field
    }

    fn one(name: &str, value: Value) -> Value {
        Value::Object([(name.to_owned(), value)].into_iter().collect())
    }

    fn check(fields: &[Field], payload: Value, door: Door) -> Result<(), Vec<String>> {
        refused_fields(fields, &HashMap::new(), &payload, door)
    }

    #[test]
    fn each_field_type_takes_the_values_of_the_table() {
        let cases = [
            (
                field("text", T::String, V::Normal),
                vec![json!("Hello"), json!("")],
                vec![json!(7), json!(null), json!(["a"])],
            ),
            (
                field("place", T::Geometry, V::Normal),
                vec![
                    json!("POINT (1 2)"),
                    json!({"type": "Point", "coordinates": [1.0, 2.0]}),
                ],
                vec![json!(7), json!({"type": "Nowhere"})],
            ),
            (
                field("count", T::Integer, V::Normal),
                vec![json!(3), json!(-4)],
                vec![json!(2.5), json!("3"), json!(true)],
            ),
            (
                field("ratio", T::Float, V::Normal),
                vec![json!(2.5), json!(3)],
                vec![json!("2.5"), json!(null)],
            ),
            (
                field("urgent", T::Boolean, V::Normal),
                vec![json!(true), json!(false)],
                vec![json!("true"), json!(1)],
            ),
            (
                field("due", T::Date, V::Normal),
                vec![
                    json!("2026-10-03"),
                    json!("2026-10-03T10:00:00Z"),
                    json!("2026-10-03T10:00:00+02:00"),
                ],
                vec![
                    json!("2026-02-30"),
                    json!("2026-1-3"),
                    json!("03.10.2026"),
                    json!("2026-10-03 10:00"),
                    json!(20261003),
                ],
            ),
            (
                field("data", T::Generic, V::Normal),
                vec![json!(null), json!(1), json!({"a": [1]})],
                vec![],
            ),
            (
                field("labels", T::String, V::Array),
                vec![json!(["a", "b"]), json!([])],
                vec![json!("a"), json!([1])],
            ),
            (
                field("ids", T::Integer, V::HashSet),
                vec![json!([1, 2])],
                vec![json!([1, 1]), json!({"a": 1})],
            ),
            (
                field("weights", T::Float, V::HashMap),
                vec![json!({"a": 1.5})],
                vec![json!([1.5]), json!({"a": "x"})],
            ),
            (
                field("days", T::Date, V::Array),
                vec![json!(["2026-10-03", "2026-10-04T00:00:00Z"])],
                vec![json!(["2026-13-01"]), json!("2026-10-03")],
            ),
        ];
        for (field, accepted, refused) in cases {
            let fields = [field.clone()];
            for value in accepted {
                assert_eq!(
                    check(&fields, one(&field.name, value.clone()), Door::Management),
                    Ok(()),
                    "{} takes {value}",
                    field.name
                );
            }
            for value in refused {
                assert_eq!(
                    check(&fields, one(&field.name, value.clone()), Door::Management),
                    Err(vec![field.name.clone()]),
                    "{} refuses {value}",
                    field.name
                );
            }
        }
    }

    #[test]
    fn a_struct_field_follows_its_schema_and_never_fetches_one() {
        let mut limit = field("limit", T::Struct, V::Normal);
        limit.schema = Some(
            json!({"type": "object", "properties": {"max": {"type": "integer", "minimum": 1}}, "required": ["max"]})
                .to_string(),
        );
        let limit = [limit];
        assert_eq!(
            check(&limit, json!({"limit": {"max": 2}}), Door::Management),
            Ok(())
        );
        for refused in [
            json!({"max": 0}),
            json!({"max": "2"}),
            json!([]),
            json!("x"),
        ] {
            assert_eq!(
                check(&limit, one("limit", refused), Door::Management),
                Err(vec!["limit".to_owned()])
            );
        }
        let mut remote = field("remote", T::Struct, V::Normal);
        remote.schema = Some(json!({"$ref": "https://schemas.example/form.json"}).to_string());
        assert_eq!(
            check(&[remote], json!({"remote": {"any": 1}}), Door::Management),
            Err(vec!["remote".to_owned()])
        );
    }

    #[test]
    fn files_come_only_through_the_service_page_as_data_urls() {
        let fields = [
            field("photo", T::PathBuf, V::Normal),
            optional(field("scans", T::Byte, V::Array)),
        ];
        let photo = "data:image/png;base64,iVBORw0KGgo=";
        assert_eq!(
            check(&fields, json!({"photo": photo}), Door::Management),
            Err(vec!["photo".to_owned()])
        );
        assert_eq!(
            check(&fields, json!({"photo": photo}), Door::ServicePage),
            Ok(())
        );
        assert_eq!(
            check(
                &fields,
                json!({"photo": photo, "scans": [photo, photo]}),
                Door::ServicePage
            ),
            Ok(())
        );
        for refused in [
            "file:///etc/passwd",
            "/tmp/photo.png",
            "https://files.example/a.png",
            "data:no-comma",
        ] {
            assert_eq!(
                check(&fields, json!({"photo": refused}), Door::ServicePage),
                Err(vec!["photo".to_owned()]),
                "{refused}"
            );
        }
    }

    #[test]
    fn missing_fields_and_unknown_keys_are_named() {
        let fields = [
            field("title", T::String, V::Normal),
            optional(field("note", T::String, V::Normal)),
            field("count", T::Integer, V::Normal),
        ];
        assert_eq!(
            check(&fields, json!({"title": "a", "count": 1}), Door::Management),
            Ok(())
        );
        let Err(names) = check(
            &fields,
            json!({"count": "x", "extra": 1, "Title": "a"}),
            Door::Management,
        ) else {
            panic!("an unknown key and a missing field are refused");
        };
        assert_eq!(names[..2], ["title", "count"]);
        let mut unknown = names[2..].to_vec();
        unknown.sort();
        assert_eq!(unknown, ["Title", "extra"]);
        for not_an_object in [json!(null), json!([1]), json!("title")] {
            assert_eq!(
                check(&fields, not_an_object, Door::Management),
                Err(Vec::new())
            );
        }
    }

    #[test]
    fn refused_names_stay_within_their_bounds() {
        for filler in ["x", "\u{1}", "\"", "ü"] {
            let keys: serde_json::Map<String, Value> = (0..100)
                .map(|index| (format!("{index:03}{}", filler.repeat(497)), json!(1)))
                .collect();
            let Err(names) = check(&[], Value::Object(keys), Door::Management) else {
                panic!("unknown keys are refused");
            };
            assert!(!names.is_empty() && names.len() <= MAX_REFUSED_NAMES);
            assert!(
                names
                    .iter()
                    .all(|name| name.chars().count() <= MAX_REFUSED_NAME_CHARS)
            );
            assert!(serde_json::to_vec(&names).unwrap().len() <= MAX_REFUSED_NAMES_BYTES);
        }
    }

    fn folded(events: &[(&str, Value)]) -> (Output, bool) {
        let mut fold = Fold::default();
        for (event_type, payload) in events {
            fold.add(&InterComEvent::with_type(*event_type, payload.clone()));
        }
        let asked = fold.asked();
        (fold.finish(), asked)
    }

    fn sent(output: &Output) -> usize {
        serde_json::to_vec(output.output.as_ref().expect("an output"))
            .unwrap()
            .len()
    }

    #[test]
    fn the_result_is_the_last_generic_result_then_the_answer_then_the_text() {
        let (output, asked) = folded(&[
            ("text_output", json!("ignored")),
            ("generic_result", json!({"id": 1})),
            ("generic_result", json!({"id": 42})),
        ]);
        assert!(!asked);
        assert_eq!(
            output,
            Output {
                output: Some(json!({"json": {"id": 42}})),
                output_bytes: 9,
                truncated: false,
                attachments: 0,
            }
        );
        let answer = |text: &str| json!({"response": {"choices": [{"message": {"role": "assistant", "content": text}}]}});
        let (output, _) = folded(&[
            ("chat_stream", answer("Sav")),
            ("chat_out", answer("Saved.")),
            ("text_output", json!("streamed")),
        ]);
        assert_eq!(output.output, Some(json!({"text": "Saved."})));
        let (output, _) = folded(&[
            ("text_output", json!("Hel")),
            ("stream_text", json!({"text": "lo"})),
            ("a2ui", json!({"type": "createElement"})),
        ]);
        assert_eq!(output.output, Some(json!({"text": "Hello"})));
        assert_eq!((output.output_bytes, output.truncated), (5, false));
        assert_eq!(
            folded(&[("run_initiated", json!({"run_id": "r"}))]).0,
            Output::default()
        );
    }

    #[test]
    fn a_large_result_is_cut_until_it_fits_8_kib_as_it_is_sent() {
        for text in [
            "a".repeat(20_000),
            "ä".repeat(10_000),
            "😀".repeat(5_000),
            "\"".repeat(10_000),
            "\\".repeat(10_000),
            "\u{1}".repeat(10_000),
            "line\n".repeat(4_000),
        ] {
            for (event_type, payload) in [
                ("text_output", json!(text)),
                ("generic_result", json!(text)),
                ("generic_result", json!({"text": text})),
                (
                    "chat_out",
                    json!({"response": {"choices": [{"message": {"content": text}}]}}),
                ),
            ] {
                let (output, _) = folded(&[(event_type, payload)]);
                let size = sent(&output);
                assert!(size <= MAX_OUTPUT_BYTES, "{event_type}: {size} bytes");
                assert!(
                    size + 6 > MAX_OUTPUT_BYTES,
                    "{event_type}: only {size} bytes kept"
                );
                assert!(output.truncated, "{event_type}");
                assert!(output.output_bytes >= text.len() as u64, "{event_type}");
                assert!(output.output.as_ref().unwrap()["text"].is_string());
            }
        }
        let fits = "a".repeat(MAX_OUTPUT_BYTES - r#"{"json":""}"#.len());
        let (output, _) = folded(&[("generic_result", json!(fits))]);
        assert_eq!((sent(&output), output.truncated), (MAX_OUTPUT_BYTES, false));
        assert!(output.output.as_ref().unwrap()["json"].is_string());
        let (output, _) = folded(&[("generic_result", json!(format!("{fits}a")))]);
        assert!(output.truncated && output.output.as_ref().unwrap()["text"].is_string());
    }

    #[test]
    fn media_are_counted_once_and_never_copied() {
        let signed = "https://files.example/a.png?X-Amz-Signature=secret";
        let other = "https://files.example/b.pdf?sig=secret";
        let (output, _) = folded(&[
            ("chat_stream_partial", json!({"attachments": [signed]})),
            (
                "chat_out",
                json!({
                    "response": {"choices": [{"message": {"content": "Here", "content_parts": [
                        {"type": "image_url", "image_url": {"url": signed}},
                        {"type": "text", "text": "Here"}
                    ]}}]},
                    "attachments": [{"url": other, "name": "b.pdf"}]
                }),
            ),
        ]);
        assert_eq!(output.attachments, 2);
        assert_eq!(output.output, Some(json!({"text": "Here"})));
        let sent = serde_json::to_string(&output.output).unwrap();
        assert!(!sent.contains("secret") && !sent.contains("files.example"));
    }

    #[test]
    fn a_question_is_noticed() {
        let (output, asked) = folded(&[(
            "interaction_request",
            json!({"id": "question", "prompt": "Are you sure?"}),
        )]);
        assert!(asked);
        assert_eq!(output.output, None);
    }

    /// A board with one entry node of `kind` and the pins `add` gives it, as version 1.0.0.
    async fn prepared(
        root: &Path,
        kind: &str,
        config: Value,
        add: impl FnOnce(&mut flow_like_runtime::flow::node::Node),
    ) -> Result<(OnDemandEvent, Vec<EventInput>)> {
        let (placement, state, _, _) = crate::runtime::tests::fixture(root).await;
        let app = App::load(placement.project_id.clone(), state.clone()).await?;
        let mut board = Board::new(
            Some("form-board".into()),
            StorePath::from("apps/project"),
            state.clone(),
        );
        let mut node = {
            let registry = state.node_registry.read().await;
            let definition = registry.get_node(kind)?;
            registry.instantiate(&definition)?.get_node()
        };
        node.id = "form".into();
        add(&mut node);
        board.nodes.insert(node.id.clone(), node);
        board.snapshot_at_version((1, 0, 0), None).await?;
        let mut event = app.get_event("event", Some((1, 0, 0))).await?;
        event.id = "form".into();
        event.name = "New note".into();
        event.board_id = board.id.clone();
        event.node_id = "form".into();
        event.event_type = "generic_form".into();
        event.config = serde_json::to_vec(&config)?;
        let template = TemplateCache::default()
            .resolve(&state, &app.id, &board.id, Some((1, 0, 0)), None, "")
            .await?;
        event.populate_inputs(&app).await?;
        let stored = event.inputs.clone();
        let event = prepare(PreparedInvocation {
            event,
            template,
            action_admission: None,
        })?;
        Ok((event, stored))
    }

    fn note_form(node: &mut flow_like_runtime::flow::node::Node) {
        node.add_output_pin("title", "Title", "What the note is about", T::String);
        node.add_output_pin("count", "Count", "", T::Integer)
            .set_default_value(Some(json!(3)))
            .set_options(PinOptions::new().set_optional(true).build());
        node.add_output_pin("secret", "Secret", "", T::String)
            .set_default_value(Some(json!("hush")))
            .set_options(PinOptions::new().set_sensitive(true).build());
        node.add_output_pin("color", "Color", "", T::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["red".into(), "green".into()])
                    .build(),
            );
        node.add_output_pin("photo", "Photo", "", T::PathBuf);
    }

    #[tokio::test]
    async fn the_fields_are_the_pins_populate_inputs_reads_without_the_payload() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let config = json!({"navigate_to_routes": ["/notes", 7, ""]});
        let (event, stored) =
            prepared(directory.path(), "events_generic", config, note_form).await?;
        // The stored list keeps a description as the key of the board's text for it.
        let refs = &event.invocation.template.board.refs;
        let stored: Vec<_> = stored
            .iter()
            .filter(|input| input.name != PAYLOAD_PIN)
            .map(|input| {
                (
                    input.name.clone(),
                    input.friendly_name.clone(),
                    resolve_schema(&input.description, refs).unwrap().to_owned(),
                    input.data_type.clone(),
                    input.value_type.clone(),
                    input.optional,
                )
            })
            .collect();
        let derived: Vec<_> = event
            .fields
            .iter()
            .map(|field| {
                (
                    field.name.clone(),
                    field.label.clone(),
                    field.description.clone(),
                    field.data_type_word(),
                    field.value_type_word(),
                    field.optional,
                )
            })
            .collect();
        assert_eq!(derived.len(), 5);
        assert_eq!(derived, stored);
        assert_eq!((event.kind(), event.file_fields()), (Kind::Form, 1));

        let entry = event.contract_entry(MAX_ENTRY_BYTES);
        let mut keys: Vec<_> = entry.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "board_version",
                "description",
                "event_version",
                "fields",
                "fields_truncated",
                "file_fields",
                "kind",
                "name",
                "navigate_to_routes"
            ]
        );
        assert_eq!(entry["kind"], "form");
        assert_eq!(entry["name"], "New note");
        assert_eq!(entry["event_version"], json!([1, 0, 0]));
        assert_eq!(entry["board_version"], json!([1, 0, 0]));
        assert_eq!(entry["navigate_to_routes"], json!(["/notes"]));
        assert_eq!(entry["fields_truncated"], false);
        assert_eq!(
            entry["fields"][0],
            json!({"name": "title", "label": "Title", "description": "What the note is about",
                "data_type": "String", "value_type": "Normal", "optional": false,
                "sensitive": false, "default": null, "options": null})
        );
        assert_eq!(entry["fields"][1]["default"], 3);
        assert_eq!(entry["fields"][2]["sensitive"], true);
        assert_eq!(entry["fields"][2]["default"], Value::Null);
        assert_eq!(entry["fields"][3]["options"], json!(["red", "green"]));
        assert_eq!(entry["fields"][4]["data_type"], "PathBuf");
        assert!(!entry.to_string().contains("hush"));

        let listed = inventory_entry(&event);
        assert_eq!(listed["inputs"].as_array().unwrap().len(), 5);
        assert_eq!(listed["inputs"][1]["default_value"], json!(b"3".to_vec()));
        assert_eq!(listed["inputs"][2]["name"], "secret");
        assert_eq!(listed["inputs"][2]["default_value"], Value::Null);
        let config: Value = serde_json::from_slice(&serde_json::from_value::<Vec<u8>>(
            listed["config"].clone(),
        )?)?;
        assert_eq!(config, json!({"navigate_to_routes": ["/notes"]}));
        assert_eq!(listed["node_id"], "");
        Ok(())
    }

    #[tokio::test]
    async fn a_quick_action_has_no_fields_and_takes_an_empty_payload() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (event, stored) =
            prepared(directory.path(), "events_simple", json!({}), |_| {}).await?;
        assert!(stored.is_empty());
        assert_eq!((event.kind(), event.file_fields()), (Kind::Action, 0));
        let refs = &event.invocation.template.board.refs;
        assert_eq!(
            refused_fields(&event.fields, refs, &json!({}), Door::Management),
            Ok(())
        );
        assert_eq!(
            refused_fields(&event.fields, refs, &json!({"x": 1}), Door::Management),
            Err(vec!["x".to_owned()])
        );
        assert_eq!(check_fields(&event, &json!({})), Ok(()));
        assert_eq!(event.contract_entry(MAX_ENTRY_BYTES)["kind"], "action");
        Ok(())
    }

    #[tokio::test]
    async fn a_form_with_many_large_fields_is_cut_to_its_budget() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (event, _) = prepared(directory.path(), "events_generic", json!({}), |node| {
            for index in 0..80 {
                node.add_output_pin(
                    &format!("field_{index:02}"),
                    &"L".repeat(300),
                    &"D".repeat(900),
                    T::String,
                );
            }
        })
        .await?;
        assert_eq!(event.fields.len(), 80);
        let entry = event.contract_entry(MAX_ENTRY_BYTES);
        assert!(json_len(&entry) <= MAX_ENTRY_BYTES);
        assert_eq!(entry["fields_truncated"], true);
        let fields = entry["fields"].as_array().unwrap();
        assert!(!fields.is_empty() && fields.len() < MAX_FIELDS);
        assert_eq!(
            fields[0]["label"].as_str().unwrap().chars().count(),
            MAX_LABEL_CHARS
        );
        assert_eq!(
            fields[0]["description"].as_str().unwrap().chars().count(),
            MAX_DESCRIPTION_CHARS
        );
        Ok(())
    }

    /// A form whose `mode` field says what a run of it does.
    #[derive(Default)]
    struct Probe {
        runs: AtomicUsize,
    }

    #[flow_like_types::async_trait]
    impl NodeLogic for Probe {
        fn get_node(&self) -> Node {
            let mut node = Node::new("standalone_test_on_demand", "Form", "", "Tests");
            node.set_start(true);
            node.add_output_pin("exec_out", "Out", "", T::Execution);
            node.add_output_pin("mode", "Mode", "", T::String);
            node.add_output_pin("note", "Note", "", T::String)
                .set_options(PinOptions::new().set_optional(true).build());
            node
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            let payload = context
                .get_payload()
                .await?
                .payload
                .clone()
                .unwrap_or_default();
            match payload["mode"].as_str() {
                Some("succeed") => {
                    let result = json!({"id": 42, "note": payload["note"]});
                    context.stream_response("generic_result", result).await?;
                }
                Some("slow") => {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    context
                        .stream_response("generic_result", json!({"id": 7}))
                        .await?;
                }
                Some("fail") => return Err(flow_like_types::anyhow!("the probe failed")),
                Some("ask") => {
                    let question = json!({"id": "question", "prompt": "Sure?"});
                    context
                        .stream_response("interaction_request", question)
                        .await?;
                    std::future::pending::<()>().await;
                }
                Some("hang") => std::future::pending::<()>().await,
                _ => {}
            }
            Ok(())
        }
    }

    /// The probe's form, prepared, with what its runs share.
    async fn probe_service(root: &Path) -> Result<(OnDemandEvent, Arc<RunContext>, Arc<Probe>)> {
        let (placement, state, _, _) = crate::runtime::tests::fixture(root).await;
        let probe = Arc::new(Probe::default());
        state.node_registry.write().await.push_node(probe.clone());
        let app = App::load(placement.project_id.clone(), state.clone()).await?;
        let mut board = Board::new(
            Some("probe-board".into()),
            StorePath::from("apps/project"),
            state.clone(),
        );
        let mut node = probe.get_node();
        node.id = "form".into();
        board.nodes.insert(node.id.clone(), node);
        board.snapshot_at_version((1, 0, 0), None).await?;
        let mut event = app.get_event("event", Some((1, 0, 0))).await?;
        event.id = "form".into();
        event.board_id = board.id.clone();
        event.node_id = "form".into();
        event.event_type = "generic_form".into();
        let template = TemplateCache::default()
            .resolve(&state, &app.id, &board.id, Some((1, 0, 0)), None, "")
            .await?;
        let event = prepare(PreparedInvocation {
            event,
            template,
            action_admission: None,
        })?;
        let run = Arc::new(RunContext {
            project_id: app.id.clone(),
            state,
            profile: Default::default(),
            visibility: app.visibility.clone(),
            execution_sub: None,
        });
        Ok((event, run, probe))
    }

    /// The agent parent as a placement process sees it: it hands out what was queued to the
    /// first that asks, and keeps every report.
    #[derive(Default)]
    struct Parent {
        waiting: Mutex<Vec<StartRun>>,
        cancels: Mutex<Vec<String>>,
        reports: Mutex<Vec<FinishedRun>>,
        last_reports: AtomicUsize,
        busy: AtomicBool,
    }

    impl Parent {
        fn queue(&self, operation_id: &str, payload: Value) {
            self.queue_for(operation_id, "form", payload, 30);
        }

        fn queue_for(&self, operation_id: &str, event_id: &str, payload: Value, limit: u64) {
            lock(&self.waiting).push(StartRun {
                operation_id: operation_id.into(),
                run_id: format!("run-{operation_id}"),
                event_id: event_id.into(),
                payload: Some(payload),
                time_limit_secs: limit,
            });
        }

        fn reported(&self, operation_id: &str) -> Vec<FinishedRun> {
            lock(&self.reports)
                .iter()
                .filter(|report| report.operation_id == operation_id)
                .cloned()
                .collect()
        }

        async fn ended(&self, operation_id: &str) -> FinishedRun {
            until(&format!("{operation_id} is reported"), || {
                !self.reported(operation_id).is_empty()
            })
            .await;
            self.reported(operation_id).remove(0)
        }
    }

    #[async_trait::async_trait]
    impl RunSource for Parent {
        async fn exchange(&self, finished: Vec<FinishedRun>) -> Result<RunBatch> {
            if self.busy.load(Ordering::SeqCst) {
                return Err(SourceBusy.into());
            }
            lock(&self.reports).extend(finished);
            Ok(RunBatch {
                start: std::mem::take(&mut *lock(&self.waiting)),
                cancel: std::mem::take(&mut *lock(&self.cancels)),
            })
        }

        async fn report(&self, finished: Vec<FinishedRun>) -> Result<RunBatch> {
            self.last_reports.fetch_add(1, Ordering::SeqCst);
            lock(&self.reports).extend(finished);
            Ok(RunBatch {
                start: Vec::new(),
                cancel: std::mem::take(&mut *lock(&self.cancels)),
            })
        }
    }

    async fn until(what: &str, condition: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(30), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("Timed out waiting until {what}"));
    }

    struct Service {
        stop: CancellationToken,
        task: tokio::task::JoinHandle<Result<()>>,
        state_dir: PathBuf,
        slot: u8,
    }

    impl Service {
        fn start(
            event: &OnDemandEvent,
            run_context: &Arc<RunContext>,
            parent: &Arc<Parent>,
            root: &Path,
            slot: u8,
        ) -> Result<Self> {
            let state_dir = state_directory(root, "placement");
            let source: Arc<dyn RunSource> = parent.clone();
            let context = OnDemandContext::new(
                run_context.clone(),
                Some(source),
                state_dir.clone(),
                "placement".into(),
                slot,
                1,
                1,
                Duration::from_secs(30),
            );
            let events = vec![event.clone()];
            prepare_state(&events, &context)?;
            let (ready, accepted) = oneshot::channel();
            ready.send(()).unwrap();
            let stop = CancellationToken::new();
            let task = tokio::spawn(super::run(events, context, accepted, stop.clone()));
            Ok(Self {
                stop,
                task,
                state_dir,
                slot,
            })
        }

        fn counters(&self) -> Value {
            let file = self.state_dir.join(format!("state.{}.json", self.slot));
            serde_json::from_slice::<Value>(&std::fs::read(file).unwrap()).unwrap()["events"]
                ["form"]
                .clone()
        }

        async fn stop(self) {
            self.stop.cancel();
            tokio::time::timeout(Duration::from_secs(30), self.task)
                .await
                .expect("the task ends on stop")
                .unwrap()
                .unwrap();
        }
    }

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_run_reports_its_result_and_keeps_inputs_and_results_out_of_files_and_logs()
    -> Result<()> {
        let captured = Captured::default();
        let writer = captured.clone();
        let capture = tracing::Dispatch::new(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::INFO)
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .finish(),
        );
        // With a single registered subscriber, tracing asks the subscriber of whichever
        // thread meets a log site first, and a parallel test's thread has none. A second one
        // makes it ask both, for every site.
        let _second = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let _logs = tracing::dispatcher::set_default(&capture);
        tracing::callsite::rebuild_interest_cache();
        let directory = tempfile::tempdir()?;
        let (event, run_context, probe) = probe_service(directory.path()).await?;
        let parent = Arc::new(Parent::default());
        let service = Service::start(&event, &run_context, &parent, directory.path(), 0)?;
        parent.queue(
            "op-1",
            json!({"mode": "succeed", "note": "TOP-SECRET-INPUT"}),
        );
        let report = parent.ended("op-1").await;
        assert_eq!((report.run.as_str(), report.code), ("succeeded", None));
        assert_eq!(
            report.output,
            Some(json!({"json": {"id": 42, "note": "TOP-SECRET-INPUT"}}))
        );
        assert!(report.output_bytes > 0 && !report.truncated);
        assert!(report.started_at <= report.finished_at);
        let counters = service.counters();
        assert_eq!(
            (&counters["runs"], &counters["failed"], &counters["running"]),
            (&json!(1), &json!(0), &json!(0))
        );
        assert_eq!(counters["last"]["outcome"], "succeeded");
        assert_eq!(counters["last"]["origin"], "management");
        assert_eq!(
            (&counters["kind"], &counters["fields"]),
            (&json!("form"), &json!(2))
        );
        let state_dir = service.state_dir.clone();
        service.stop().await;
        assert_eq!(probe.runs.load(Ordering::SeqCst), 1);
        for entry in std::fs::read_dir(&state_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(name == CONTRACT_FILE || name == "state.0.json", "{name}");
            let text = std::fs::read_to_string(entry.path())?;
            assert!(!text.contains("TOP-SECRET-INPUT"), "{name}");
        }
        let logs = String::from_utf8(lock(&captured.0).clone())?;
        assert!(logs.contains("Person-started run started"), "{logs}");
        assert!(logs.contains("Person-started run finished"), "{logs}");
        assert!(!logs.contains("TOP-SECRET-INPUT"), "{logs}");
        Ok(())
    }

    #[tokio::test]
    async fn each_way_a_run_ends_has_its_code() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (event, run_context, probe) = probe_service(directory.path()).await?;
        let parent = Arc::new(Parent::default());
        let service = Service::start(&event, &run_context, &parent, directory.path(), 0)?;
        parent.queue("fails", json!({"mode": "fail"}));
        parent.queue("asks", json!({"mode": "ask"}));
        parent.queue_for("hangs", "form", json!({"mode": "hang"}), 1);
        parent.queue_for("elsewhere", "other", json!({"mode": "succeed"}), 30);
        parent.queue("wrong", json!({"mode": 7, "extra": true}));
        let code = |report: &FinishedRun| (report.run.clone(), report.code.clone());
        let failed = |code: &str| ("failed".to_owned(), Some(code.to_owned()));
        assert_eq!(code(&parent.ended("fails").await), failed("flow_failed"));
        assert_eq!(
            code(&parent.ended("asks").await),
            failed("needs_interaction")
        );
        assert_eq!(
            code(&parent.ended("elsewhere").await),
            failed("not_started")
        );
        let wrong = parent.ended("wrong").await;
        assert_eq!(code(&wrong), failed("invalid_fields"));
        let mut fields = wrong.fields.clone();
        fields.sort();
        assert_eq!(fields, ["extra", "mode"]);
        let timed_out = parent.ended("hangs").await;
        assert_eq!(
            code(&timed_out),
            ("timed_out".to_owned(), Some("timed_out".to_owned()))
        );
        assert!(timed_out.output.is_none());

        parent.queue("stopped", json!({"mode": "hang"}));
        until("the run started", || probe.runs.load(Ordering::SeqCst) == 4).await;
        lock(&parent.cancels).push("stopped".into());
        assert_eq!(
            code(&parent.ended("stopped").await),
            ("cancelled".to_owned(), Some("cancelled".to_owned()))
        );
        // Refused and unknown runs are no runs; the others are counted as they ended.
        let counters = service.counters();
        assert_eq!(
            (&counters["runs"], &counters["failed"], &counters["running"]),
            (&json!(4), &json!(3), &json!(0))
        );
        assert_eq!(counters["last"]["outcome"], "cancelled");
        service.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_run_that_ended_before_a_stop_is_reported_as_it_ended() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (event, run_context, probe) = probe_service(directory.path()).await?;
        let parent = Arc::new(Parent::default());
        let service = Service::start(&event, &run_context, &parent, directory.path(), 0)?;
        parent.queue("op-1", json!({"mode": "slow"}));
        until("the run started", || probe.runs.load(Ordering::SeqCst) == 1).await;
        // The channel is busy from now on: the periodic question cannot report the end.
        parent.busy.store(true, Ordering::SeqCst);
        until("the run ended", || {
            service.counters()["last"]["outcome"] == "succeeded"
        })
        .await;
        assert!(parent.reported("op-1").is_empty());
        service.stop().await;
        let reported = parent.reported("op-1");
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].run, "succeeded");
        assert_eq!(reported[0].output, Some(json!({"json": {"id": 7}})));
        assert!(parent.last_reports.load(Ordering::SeqCst) >= 1);
        Ok(())
    }

    #[tokio::test]
    async fn a_run_cut_off_by_a_stop_ends_as_interrupted() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (event, run_context, probe) = probe_service(directory.path()).await?;
        let parent = Arc::new(Parent::default());
        let service = Service::start(&event, &run_context, &parent, directory.path(), 0)?;
        parent.queue("op-1", json!({"mode": "hang"}));
        until("the run started", || probe.runs.load(Ordering::SeqCst) == 1).await;
        let state_dir = service.state_dir.clone();
        let stopped = Instant::now();
        service.stop().await;
        assert!(stopped.elapsed() >= STOP_GRACE);
        let report = parent.ended("op-1").await;
        assert_eq!(
            (report.run.as_str(), report.code.as_deref()),
            ("failed", Some("interrupted"))
        );
        let file: Value = serde_json::from_slice(&std::fs::read(state_dir.join("state.0.json"))?)?;
        let counters = &file["events"]["form"];
        assert_eq!(
            (&counters["runs"], &counters["running"]),
            (&json!(1), &json!(0))
        );
        assert_eq!(counters["last"]["outcome"], "cancelled");
        Ok(())
    }

    #[tokio::test]
    async fn two_instances_keep_their_own_state_files_and_each_run_runs_once() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (event, run_context, probe) = probe_service(directory.path()).await?;
        let parent = Arc::new(Parent::default());
        let first = Service::start(&event, &run_context, &parent, directory.path(), 0)?;
        let second = Service::start(&event, &run_context, &parent, directory.path(), 1)?;
        for index in 0..6 {
            parent.queue(&format!("op-{index}"), json!({"mode": "succeed"}));
            parent.ended(&format!("op-{index}")).await;
        }
        let runs = |service: &Service| service.counters()["runs"].as_u64().unwrap();
        assert_eq!(runs(&first) + runs(&second), 6);
        assert_eq!(probe.runs.load(Ordering::SeqCst), 6);
        for index in 0..6 {
            assert_eq!(parent.reported(&format!("op-{index}")).len(), 1);
        }
        let state_dir = first.state_dir.clone();
        first.stop().await;
        second.stop().await;
        let mut names: Vec<String> = std::fs::read_dir(&state_dir)?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<std::io::Result<_>>()?;
        names.sort();
        assert_eq!(names, [CONTRACT_FILE, "state.0.json", "state.1.json"]);
        let contract: Value =
            serde_json::from_slice(&std::fs::read(state_dir.join(CONTRACT_FILE))?)?;
        assert_eq!(
            (
                &contract["version"],
                &contract["config_revision"],
                &contract["intent_revision"]
            ),
            (&json!(1), &json!(1), &json!(1))
        );
        assert_eq!(contract["events"]["form"]["kind"], "form");
        Ok(())
    }

    /// What two instances wrote is what the agent parent reads: the row fact adds up both
    /// state files, and the form comes from the contract.
    #[tokio::test]
    async fn the_agent_parent_reads_what_two_instances_wrote() -> Result<()> {
        use crate::diagnostics::{Detail, Diagnostics, Rows, test_support};
        let directory = tempfile::tempdir()?;
        let (event, run_context, _) = probe_service(directory.path()).await?;
        let parent = Arc::new(Parent::default());
        let first = Service::start(&event, &run_context, &parent, directory.path(), 0)?;
        let second = Service::start(&event, &run_context, &parent, directory.path(), 1)?;
        for (index, mode) in ["succeed", "fail", "succeed"].into_iter().enumerate() {
            let operation = format!("op-{index}");
            parent.queue(&operation, json!({"mode": mode}));
            parent.ended(&operation).await;
        }
        let written = first.state_dir.clone();
        first.stop().await;
        second.stop().await;

        let agent = tempfile::tempdir()?;
        for file in [CONTRACT_FILE, "state.0.json", "state.1.json"] {
            let bytes = std::fs::read(written.join(file))?;
            test_support::write_placement_file(
                agent.path(),
                "api",
                ".standalone-run",
                file,
                &bytes,
            );
        }
        let contract: Value = serde_json::from_slice(&std::fs::read(written.join(CONTRACT_FILE))?)?;
        let entry = &contract["events"]["form"];
        let mut record = test_support::running_record(&["form"]);
        record.config["events"][0]["event_version"] = entry["event_version"].clone();
        record.config["events"][0]["board_version"] = entry["board_version"].clone();
        let mut replica = record.replicas[0].clone();
        replica.slot = 1;
        record.replicas.push(replica);
        record.running_replicas = 2;
        record.ready_replicas = 2;

        let row = Rows::new(&Diagnostics::default(), agent.path(), false).placement(
            &record,
            true,
            Detail::Full,
        );
        let action = &row["actions"][0];
        assert_eq!(
            (&action["event_id"], &action["kind"]),
            (&json!("form"), &json!("form"))
        );
        assert_eq!(
            (&action["runs"], &action["failed"], &action["running"]),
            (&json!(3), &json!(1), &json!(0))
        );
        let form = crate::management::event_contract(agent.path(), &record, "form")?;
        let names = |fields: &Value| -> Vec<Value> {
            fields
                .as_array()
                .unwrap()
                .iter()
                .map(|field| field["name"].clone())
                .collect()
        };
        assert_eq!(form["kind"], "form");
        assert_eq!(names(&form["fields"]), names(&entry["fields"]));
        assert!(!names(&form["fields"]).is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn a_new_process_of_the_same_intent_keeps_the_counts() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (event, run_context, _) = probe_service(directory.path()).await?;
        let parent = Arc::new(Parent::default());
        let service = Service::start(&event, &run_context, &parent, directory.path(), 0)?;
        parent.queue("op-1", json!({"mode": "fail"}));
        parent.ended("op-1").await;
        service.stop().await;
        let again = Service::start(&event, &run_context, &parent, directory.path(), 0)?;
        let counters = again.counters();
        assert_eq!(
            (&counters["runs"], &counters["failed"]),
            (&json!(1), &json!(1))
        );
        assert_eq!(counters["last"]["outcome"], "failed");
        again.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_service_with_only_a_form_runs_what_its_parent_hands_it() -> Result<()> {
        use crate::runtime::tests::{TestClock, schedule_context, start_supervised_with};
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let (mut config, state, _, _) = crate::runtime::tests::fixture(root).await;
        let probe = Arc::new(Probe::default());
        state.node_registry.write().await.push_node(probe.clone());
        let app = App::load(config.project_id.clone(), state.clone()).await?;
        let mut board = Board::new(
            Some("probe-board".into()),
            StorePath::from("apps/project"),
            state.clone(),
        );
        let mut node = probe.get_node();
        node.id = "form".into();
        board.nodes.insert(node.id.clone(), node);
        board.snapshot_at_version((1, 0, 0), None).await?;
        let mut event = app.get_event("event", Some((1, 0, 0))).await?;
        event.id = "form".into();
        event.board_id = board.id.clone();
        event.node_id = "form".into();
        event.event_type = "generic_form".into();
        event.save(&app, Some((1, 0, 0))).await?;
        config.events[0].event_id = "form".into();

        let parent = Arc::new(Parent::default());
        let source: Arc<dyn RunSource> = parent.clone();
        let context = schedule_context(root, TestClock::at("2026-10-03T10:00:00Z"), None);
        let (ready, is_ready) = oneshot::channel();
        let (stop, task) = start_supervised_with(
            config,
            state,
            context,
            crate::runtime::PersonStartedRuns {
                source: Some(source),
            },
            || async move {
                let _ = ready.send(());
                Ok(())
            },
        );
        tokio::time::timeout(Duration::from_secs(20), is_ready).await??;
        parent.queue("succeeds", json!({"mode": "succeed"}));
        parent.queue("fails", json!({"mode": "fail"}));
        parent.queue("asks", json!({"mode": "ask"}));
        parent.queue_for("slow", "form", json!({"mode": "hang"}), 1);
        parent.queue_for("not-here", "event", json!({}), 30);
        let ended = |report: FinishedRun| (report.run, report.code);
        let failed = |code: &str| ("failed".to_owned(), Some(code.to_owned()));
        let succeeded = parent.ended("succeeds").await;
        assert_eq!(
            succeeded.output,
            Some(json!({"json": {"id": 42, "note": null}}))
        );
        assert_eq!(ended(succeeded), ("succeeded".to_owned(), None));
        assert_eq!(ended(parent.ended("fails").await), failed("flow_failed"));
        assert_eq!(
            ended(parent.ended("asks").await),
            failed("needs_interaction")
        );
        assert_eq!(ended(parent.ended("not-here").await), failed("not_started"));
        assert_eq!(
            ended(parent.ended("slow").await),
            ("timed_out".to_owned(), Some("timed_out".to_owned()))
        );
        parent.queue("stopped", json!({"mode": "hang"}));
        until("the run started", || probe.runs.load(Ordering::SeqCst) == 5).await;
        lock(&parent.cancels).push("stopped".into());
        assert_eq!(
            ended(parent.ended("stopped").await),
            ("cancelled".to_owned(), Some("cancelled".to_owned()))
        );
        assert!(
            !task.is_finished(),
            "a service with only a form keeps running"
        );
        let counters: Value = serde_json::from_slice(&std::fs::read(
            root.join(".standalone-run/placement/state.0.json"),
        )?)?;
        assert_eq!(
            (
                &counters["events"]["form"]["runs"],
                &counters["events"]["form"]["failed"]
            ),
            (&json!(5), &json!(3))
        );
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(30), task).await???;
        Ok(())
    }

    #[tokio::test]
    async fn nothing_is_asked_before_ready_and_a_state_directory_that_is_a_link_fails_the_start()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (event, run_context, _) = probe_service(directory.path()).await?;
        let parent = Arc::new(Parent::default());
        parent.queue("op-1", json!({"mode": "succeed"}));
        let source: Arc<dyn RunSource> = parent.clone();
        let state_dir = state_directory(directory.path(), "placement");
        let context = OnDemandContext::new(
            run_context.clone(),
            Some(source.clone()),
            state_dir.clone(),
            "placement".into(),
            0,
            1,
            1,
            Duration::from_secs(30),
        );
        prepare_state(std::slice::from_ref(&event), &context)?;
        let (never, waiting) = oneshot::channel::<()>();
        drop(never);
        let stop = CancellationToken::new();
        let task = tokio::spawn(super::run(
            vec![event.clone()],
            context,
            waiting,
            stop.clone(),
        ));
        tokio::time::sleep(POLL * 3).await;
        assert_eq!(lock(&parent.waiting).len(), 1, "nothing was asked");
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), task).await???;

        let elsewhere = tempfile::tempdir()?;
        let linked = directory.path().join("linked");
        std::fs::create_dir(&linked)?;
        std::os::unix::fs::symlink(elsewhere.path(), linked.join(STATE_DIRECTORY))?;
        let context = OnDemandContext::new(
            run_context,
            Some(source),
            state_directory(&linked, "placement"),
            "placement".into(),
            0,
            1,
            1,
            Duration::from_secs(30),
        );
        assert!(prepare_state(&[event], &context).is_err());
        assert_eq!(std::fs::read_dir(elsewhere.path())?.count(), 0);
        Ok(())
    }
}
