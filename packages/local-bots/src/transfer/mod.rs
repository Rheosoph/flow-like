//! Moving a bot of this device into the signed-in account (`todo/bots-local.md`, decision 8 and
//! fix round 1). The account mints every id: the bot's and each conversation's id here travel
//! as import keys, so a repeated move returns the same account records and finishes an
//! interrupted one. Assistant and Flux aren't imported: the account's own take what was changed
//! here. Every step can repeat, and the local copy goes last.
//!
//! The bot's workspace app moves with it: the account's workspace is ensured before any chat
//! moves, so handles in message bodies can name it, its files are copied and compared with
//! the account's copy, and the local app is deleted only once a second comparison, right
//! before the bot is forgotten, shows nothing missing. Segments follow their chat: the bot's
//! own summaries are written to the account's conversation once its entries are there, with
//! each range mapped from this device's positions to the account's by reading the positions
//! the account gave the moved entries back; ranges keep pointing at the same messages.

mod hub;
pub(crate) mod requests;

pub use hub::{AccountHub, HubBody, HubCall, HubError, HubMethod, WorkspaceSummary};

use crate::{
    LocalBotStore,
    error::codes,
    model::{Bot, ConversationSegment, ExportedConversation, InboxEntry, LocalBotBundle},
    validation,
};
use flow_like_types::bot::{BotLocation, BotSystemKind};
use requests::{
    AccountIds, BotSettingsRequest, ImportBotRequest, PositionMap, count_messages, entries_to_move,
    foreign_handles, import_conversation_requests, segment_requests, split_conversations,
    unmovable_segments,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

/// Export → import → forget passes before a bot that keeps changing here stops moving.
const MOVE_PASSES: u8 = 5;
/// Lies before every event, so a forget conditional on it keeps a chat that got any.
const BEFORE_EVERY_EVENT: &str = "0:0";

/// A secret this device's key no longer opens; `ref_id` is its name.
pub const BOT_SECRET_UNREADABLE: &str = "BOT_SECRET_UNREADABLE";
/// The hub could not be reached.
pub const HUB_UNAVAILABLE: &str = "HUB_UNAVAILABLE";
/// The hub refused without a code of its own.
pub const HUB_ERROR: &str = "HUB_ERROR";
/// The hub answered without what the step needs.
pub const HUB_RESPONSE_INVALID: &str = "HUB_RESPONSE_INVALID";
/// The account runs the bot where a capability it has can't work (the API's code).
pub const BOT_CAPABILITY_LOCATION: &str = "BOT_CAPABILITY_LOCATION";
/// The account's copy of the workspace lacks files the local one holds.
pub const WORKSPACE_NOT_VERIFIED: &str = "WORKSPACE_NOT_VERIFIED";

/// Something that keeps a bot from moving, from the account's check or from this device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MoveIssue {
    /// `bot`, `profile`, `device`, `app` or `capability` (the account), `secret` (this device).
    pub part: String,
    pub code: String,
    pub message: String,
    /// The app, profile, device, capability or secret the issue is about.
    pub ref_id: Option<String>,
}

/// What moving one bot takes along, and what blocks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MoveReport {
    pub bot_id: String,
    /// Empty when nothing blocks the move.
    pub issues: Vec<MoveIssue>,
    pub apps: usize,
    pub secrets: usize,
    /// Chats that move with the bot.
    pub conversations: usize,
    pub messages: usize,
    /// Chats that also hold bots of this device that stay; they move once those bots do.
    pub waiting_conversations: usize,
    /// Messages from senders neither in their chat nor in the account; they don't move.
    pub skipped_messages: usize,
    /// The bot's own summaries of the chats that move; they move with them.
    pub segments: usize,
    /// What the move leaves out: workspace handles to apps that stay on this device and
    /// summaries whose range holds no message that moves.
    pub warnings: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveStep {
    /// Export the bot, ask the account's check, list the account's bots.
    Check,
    ImportBot,
    UpdateBot,
    Apps,
    Secrets,
    /// The account's workspace for the bot, and its files copied there.
    Workspace,
    Conversations,
    /// The bot's summaries of the chats just moved, written to the account.
    Segments,
    ForgetConversations,
    /// A fresh export shows what changed here meanwhile.
    Verify,
    /// The account's copy of the workspace compared once more, then the local app deleted.
    ForgetWorkspace,
    Rebind,
    ForgetBot,
    Done,
}

/// Where a move is, told before each step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MoveProgress {
    pub bot_id: String,
    pub step: MoveStep,
    /// The export → import → forget pass, from 1; 0 before the first.
    pub pass: u8,
}

/// Told before each step of a move.
pub type MoveListener<'a> = dyn Fn(MoveProgress) + Send + Sync + 'a;

/// A chat whose id on this device now stands for its id in the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MovedConversation {
    pub local_id: String,
    pub account_id: String,
}

/// A finished move: the account's bot and where the bot's chats went.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MovedBot {
    pub bot: Value,
    pub conversations: Vec<MovedConversation>,
    /// Handles and summaries left out, as in [`MoveReport::warnings`].
    pub warnings: usize,
}

/// Why a check or move stopped. `code` is the account's, the store's or one of this module's;
/// repeating the move resumes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MoveError {
    pub code: String,
    pub message: String,
    pub step: MoveStep,
    /// The issue that blocks the move, when one does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<Box<MoveIssue>>,
    /// Chats that moved before the move stopped.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conversations: Vec<MovedConversation>,
}

impl MoveError {
    pub fn new(step: MoveStep, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            step,
            issue: None,
            conversations: Vec::new(),
        }
    }

    fn blocked(step: MoveStep, message: String, issue: MoveIssue) -> Self {
        let mut error = Self::new(step, issue.code.clone(), message);
        error.issue = Some(Box::new(issue));
        error
    }

    fn hub(step: MoveStep, operation: &str, error: HubError) -> Self {
        let code = error.code.unwrap_or_else(|| {
            match error.status {
                None => HUB_UNAVAILABLE,
                Some(_) => HUB_ERROR,
            }
            .to_string()
        });
        Self::new(
            step,
            code,
            format!("Could not {operation}: {}", error.message),
        )
    }
}

impl fmt::Display for MoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MoveError {}

fn unreadable_secret(name: &str) -> MoveIssue {
    MoveIssue {
        part: "secret".into(),
        code: BOT_SECRET_UNREADABLE.into(),
        message: format!("Secret {name} can't be read on this computer anymore"),
        ref_id: Some(name.to_string()),
    }
}

/// What this device changed on Assistant or Flux from their defaults. The account's built-in
/// takes those fields and keeps the rest, its location and device included; Flux keeps its
/// capabilities.
fn built_in_edits(bot: &Bot, kind: BotSystemKind) -> Map<String, Value> {
    let defaults = validation::built_in(kind);
    let mut edits = Map::new();
    let mut edit = |field: &str, changed: bool, value: Value| {
        if changed {
            edits.insert(field.to_string(), value);
        }
    };
    edit("name", bot.name != defaults.name, json!(bot.name));
    edit(
        "description",
        bot.description != defaults.description,
        json!(bot.description),
    );
    edit(
        "instructions",
        bot.instructions != defaults.instructions,
        json!(bot.instructions),
    );
    edit(
        "appearance",
        bot.appearance != defaults.appearance,
        bot.appearance.clone(),
    );
    edit("model", bot.model != defaults.model, json!(bot.model));
    edit(
        "embedding_model",
        bot.embedding_model != defaults.embedding_model,
        json!(bot.embedding_model),
    );
    edit(
        "stt_model",
        bot.stt_model != defaults.stt_model,
        json!(bot.stt_model),
    );
    edit(
        "tts_model",
        bot.tts_model != defaults.tts_model,
        json!(bot.tts_model),
    );
    edit(
        "image_model",
        bot.image_model != defaults.image_model,
        json!(bot.image_model),
    );
    edit(
        "vision_model",
        bot.vision_model != defaults.vision_model,
        json!(bot.vision_model),
    );
    edit(
        "classification",
        bot.classification != defaults.classification,
        json!(bot.classification),
    );
    edit(
        "capabilities",
        !kind.capabilities_fixed() && bot.capabilities != defaults.capabilities,
        json!(bot.capabilities),
    );
    edit(
        "profile_id",
        bot.profile_id != defaults.profile_id,
        json!(bot.profile_id),
    );
    edits
}

/// The capabilities switched on here that the account's built-in can't use where the account
/// runs it (a computer, a browser or the command line on the server).
fn location_issues(bot: &Bot, kind: BotSystemKind, account: &Value) -> Vec<MoveIssue> {
    if kind.capabilities_fixed() {
        return Vec::new();
    }
    let location = text(account, "location")
        .and_then(BotLocation::parse)
        .unwrap_or_default();
    bot.capabilities
        .iter()
        .filter(|capability| !location.allows(**capability))
        .map(|capability| MoveIssue {
            part: "capability".into(),
            code: BOT_CAPABILITY_LOCATION.into(),
            message: format!(
                "Your account runs this bot on the {}, which can't use '{}'",
                location.as_str(),
                capability.as_str()
            ),
            ref_id: Some(capability.as_str().to_string()),
        })
        .collect()
}

/// The array `field` of a JSON answer; nothing when it is missing.
fn records<'v>(answer: Option<&'v Value>, field: &str) -> impl Iterator<Item = &'v Value> {
    answer
        .and_then(|answer| answer.get(field))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn text<'v>(value: &'v Value, field: &str) -> Option<&'v str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn quiet(_: MoveProgress) {}

/// Moves bots of this device into the signed-in account, one at a time, so a bot's review sees
/// the bots moved before it as account bots.
#[derive(Default)]
pub struct BotMover {
    turn: tokio::sync::Mutex<()>,
}

impl BotMover {
    /// Checks every part of the bot against the account; changes nothing.
    pub async fn check<H: AccountHub>(
        &self,
        store: &Arc<LocalBotStore>,
        hub: &H,
        bot_id: &str,
    ) -> Result<MoveReport, MoveError> {
        Ok(BotMove::new(store, hub, bot_id, &quiet)
            .plan()
            .await?
            .report)
    }

    /// Moves the bot with its apps, secrets and chats, then removes it here. Refused while its
    /// check reports issues.
    pub async fn move_bot<H: AccountHub>(
        &self,
        store: &Arc<LocalBotStore>,
        hub: &H,
        bot_id: &str,
        progress: &MoveListener<'_>,
    ) -> Result<MovedBot, MoveError> {
        let _turn = self.turn.lock().await;
        BotMove::new(store, hub, bot_id, progress).run().await
    }
}

struct Plan {
    bundle: LocalBotBundle,
    account_bots: HashSet<String>,
    /// For Assistant or Flux, the account's own: it takes their edits instead of an import.
    built_in: Option<Value>,
    report: MoveReport,
}

/// What one chat has sent to the account so far.
#[derive(Default)]
struct SentChat {
    account_id: Option<String>,
    message_ids: HashSet<String>,
    /// Workspace handles each sent message lost.
    stripped: HashMap<String, usize>,
    /// Segments written to the account or left out.
    segment_ids: HashSet<String>,
    dropped_segments: usize,
}

/// One bot's way into the account. Each pass copies what changed here since the last; the
/// local copy goes once a fresh export shows nothing left to carry.
struct BotMove<'a, H> {
    store: &'a Arc<LocalBotStore>,
    hub: &'a H,
    bot_id: &'a str,
    progress: &'a MoveListener<'a>,
    step: MoveStep,
    pass: u8,
    account_bots: HashSet<String>,
    account_id: String,
    ids: AccountIds,
    chats: HashMap<String, SentChat>,
    moved: Vec<MovedConversation>,
    /// The bot's workspace app on this device, once the account's is ensured and copied.
    workspace: Option<String>,
}

impl<'a, H: AccountHub> BotMove<'a, H> {
    fn new(
        store: &'a Arc<LocalBotStore>,
        hub: &'a H,
        bot_id: &'a str,
        progress: &'a MoveListener<'a>,
    ) -> Self {
        Self {
            store,
            hub,
            bot_id,
            progress,
            step: MoveStep::Check,
            pass: 0,
            account_bots: HashSet::new(),
            account_id: String::new(),
            ids: AccountIds::new(hub.caller(), []),
            chats: HashMap::new(),
            moved: Vec::new(),
            workspace: None,
        }
    }

    fn enter(&mut self, step: MoveStep) {
        self.step = step;
        (self.progress)(MoveProgress {
            bot_id: self.bot_id.to_string(),
            step,
            pass: self.pass,
        });
    }

    async fn call(&self, operation: &str, call: HubCall<'_>) -> Result<Option<Value>, MoveError> {
        self.hub
            .call(call)
            .await
            .map_err(|error| MoveError::hub(self.step, operation, error))
    }

    /// Runs `work` on the store off the async runtime.
    async fn local<T, F>(&self, operation: &str, work: F) -> Result<T, MoveError>
    where
        T: Send + 'static,
        F: FnOnce(&LocalBotStore) -> crate::Result<T> + Send + 'static,
    {
        let store = Arc::clone(self.store);
        match tokio::task::spawn_blocking(move || work(&store)).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(MoveError::new(
                self.step,
                error.code(),
                format!("Could not {operation}: {}", error.message()),
            )),
            Err(error) => Err(MoveError::new(
                self.step,
                codes::STORAGE,
                format!("Could not {operation}: the local bots worker failed: {error}"),
            )),
        }
    }

    fn body<T: Serialize>(&self, request: &T) -> Result<Value, MoveError> {
        serde_json::to_value(request).map_err(|error| {
            MoveError::new(
                self.step,
                codes::INVALID,
                format!("Could not write a request for bot {}: {error}", self.bot_id),
            )
        })
    }

    async fn export(&self) -> Result<LocalBotBundle, MoveError> {
        let bot_id = self.bot_id.to_string();
        self.local(&format!("export local bot {}", self.bot_id), move |store| {
            store.export_bot(&bot_id)
        })
        .await
    }

    /// What the account's check says about importing the bot.
    async fn import_issues(&self, bundle: &LocalBotBundle) -> Result<Vec<MoveIssue>, MoveError> {
        let request = self.body(&ImportBotRequest::of(bundle))?;
        let checked = self
            .call(
                &format!("check bot {} against your account", self.bot_id),
                HubCall::new(HubMethod::Post, &["bots", "import", "check"])
                    .with(HubBody::Json(&request)),
            )
            .await?;
        Ok(records(checked.as_ref(), "issues")
            .filter_map(|issue| MoveIssue::deserialize(issue).ok())
            .collect())
    }

    /// The account's Assistant or Flux, which `GET bots` makes when the account has none yet.
    fn built_in(&self, account: &[&Value], kind: BotSystemKind) -> Result<Value, MoveError> {
        account
            .iter()
            .find(|bot| text(bot, "system") == Some(kind.as_str()) && text(bot, "id").is_some())
            .map(|bot| (*bot).clone())
            .ok_or_else(|| {
                MoveError::new(
                    self.step,
                    HUB_RESPONSE_INVALID,
                    format!(
                        "Could not check bot {} against your account: it lists no built-in bot '{}'",
                        self.bot_id,
                        kind.as_str()
                    ),
                )
            })
    }

    async fn plan(&mut self) -> Result<Plan, MoveError> {
        self.enter(MoveStep::Check);
        let bundle = self.export().await?;
        let system = bundle.bot.system;
        let mut issues = match system {
            None => self.import_issues(&bundle).await?,
            Some(_) => Vec::new(),
        };
        let listed = self
            .call(
                "list your account's bots",
                HubCall::new(HubMethod::Get, &["bots"]),
            )
            .await?;
        let account: Vec<&Value> = records(listed.as_ref(), "bots").collect();
        let built_in = match system {
            Some(kind) => {
                let built_in = self.built_in(&account, kind)?;
                issues.extend(location_issues(&bundle.bot, kind, &built_in));
                Some(built_in)
            }
            None => None,
        };
        let account_bots: HashSet<String> = account
            .iter()
            .filter_map(|bot| text(bot, "id"))
            .map(str::to_string)
            .collect();
        let split = split_conversations(&bundle, &account_bots);
        let (mut messages, mut skipped_messages) = (0, 0);
        let (mut segments, mut warnings) = (0, 0);
        for item in &split.movable {
            let (kept, dropped) = entries_to_move(item, &account_bots);
            messages += count_messages(&kept);
            skipped_messages += count_messages(&dropped);
            let unmovable = unmovable_segments(item, &kept);
            segments += item.segments.len() - unmovable;
            warnings += unmovable + foreign_handles(&kept, bundle.bot.workspace_app_id.as_deref());
        }
        issues.extend(
            bundle
                .secrets
                .iter()
                .filter(|secret| !secret.readable)
                .map(|secret| unreadable_secret(&secret.name)),
        );
        let report = MoveReport {
            bot_id: self.bot_id.to_string(),
            issues,
            apps: bundle.app_ids.len(),
            secrets: bundle.secrets.len(),
            conversations: split.movable.len(),
            messages,
            waiting_conversations: split.waiting.len(),
            skipped_messages,
            segments,
            warnings,
        };
        Ok(Plan {
            bundle,
            account_bots,
            built_in,
            report,
        })
    }

    async fn run(mut self) -> Result<MovedBot, MoveError> {
        let plan = self.plan().await?;
        if let Some(issue) = plan.report.issues.first() {
            return Err(MoveError::blocked(
                self.step,
                format!(
                    "Bot {} can't move to your account yet: {}",
                    self.bot_id, issue.message
                ),
                issue.clone(),
            ));
        }
        self.account_bots = plan.account_bots;
        let target = match plan.built_in {
            Some(built_in) => built_in,
            None => self.import_bot(&plan.bundle).await?,
        };
        if let Some(account_id) = text(&target, "id") {
            self.become_account_bot(account_id.to_string());
        }
        let moved = self.transfer(plan.bundle, target).await;
        moved.map_err(|error| MoveError {
            conversations: std::mem::take(&mut self.moved),
            ..error
        })
    }

    /// From here on the bot goes by `account_id` in what moves.
    fn become_account_bot(&mut self, account_id: String) {
        self.ids = AccountIds::new(
            self.hub.caller(),
            [(self.bot_id.to_string(), account_id.clone())],
        );
        self.account_id = account_id;
    }

    /// The account bot this one becomes; the account gives it its id.
    async fn import_bot(&mut self, bundle: &LocalBotBundle) -> Result<Value, MoveError> {
        self.enter(MoveStep::ImportBot);
        let operation = format!("move bot {} to your account", self.bot_id);
        let request = self.body(&ImportBotRequest::of(bundle))?;
        let imported = self
            .call(
                &operation,
                HubCall::new(HubMethod::Post, &["bots", "import"]).with(HubBody::Json(&request)),
            )
            .await?;
        imported
            .filter(|bot| text(bot, "id").is_some())
            .ok_or_else(|| {
                MoveError::new(
                    self.step,
                    HUB_RESPONSE_INVALID,
                    format!("Could not {operation}: the hub returned no bot"),
                )
            })
    }

    async fn transfer(
        &mut self,
        exported: LocalBotBundle,
        imported: Value,
    ) -> Result<MovedBot, MoveError> {
        let mut bundle = exported;
        let mut bot = self.sync_bot(&bundle).await?.unwrap_or(imported);
        for pass in 1..=MOVE_PASSES {
            self.pass = pass;
            self.move_workspace(&bundle).await?;
            self.move_chats(&bundle).await?;
            self.enter(MoveStep::Verify);
            let latest = self.export().await?;
            if latest.bot.updated_at != bundle.bot.updated_at {
                if let Some(updated) = self.sync_bot(&latest).await? {
                    bot = updated;
                }
            } else if self.forget_bot(&latest).await? {
                self.enter(MoveStep::Done);
                return Ok(MovedBot {
                    bot,
                    conversations: std::mem::take(&mut self.moved),
                    warnings: self.warnings(),
                });
            }
            bundle = latest;
        }
        Err(MoveError::new(
            self.step,
            codes::LOCAL_CHANGED,
            format!(
                "Could not finish moving bot {}: it kept changing on this computer while it moved. Try again once nothing writes to it.",
                self.bot_id
            ),
        ))
    }

    /// The account bot takes this device's settings, apps and secrets. The account's Assistant
    /// or Flux takes only what was changed here, and keeps its other apps.
    async fn sync_bot(&mut self, bundle: &LocalBotBundle) -> Result<Option<Value>, MoveError> {
        self.enter(MoveStep::UpdateBot);
        let account_id = self.account_id.clone();
        let request = match bundle.bot.system {
            None => Some(self.body(&BotSettingsRequest::of(&bundle.bot))?),
            Some(kind) => {
                let edits = built_in_edits(&bundle.bot, kind);
                (!edits.is_empty()).then_some(Value::Object(edits))
            }
        };
        let updated = match &request {
            Some(request) => {
                self.call(
                    &format!("update bot {account_id} from this computer"),
                    HubCall::new(HubMethod::Patch, &["bots", &account_id])
                        .with(HubBody::Json(request)),
                )
                .await?
            }
            None => None,
        };
        self.sync_apps(&bundle.app_ids, bundle.bot.system.is_none())
            .await?;
        self.upload_secrets(bundle).await?;
        Ok(updated.filter(Value::is_object))
    }

    /// Attaches the apps the account bot lacks and, with `detach`, detaches the ones this
    /// device's bot dropped.
    async fn sync_apps(&mut self, app_ids: &[String], detach: bool) -> Result<(), MoveError> {
        self.enter(MoveStep::Apps);
        let account_id = self.account_id.clone();
        let listed = self
            .call(
                &format!("list the apps of bot {account_id}"),
                HubCall::new(HubMethod::Get, &["bots", &account_id, "apps"]),
            )
            .await?;
        let attached: Vec<String> = records(listed.as_ref(), "apps")
            .filter_map(|app| text(app, "app_id"))
            .map(str::to_string)
            .collect();
        for app_id in app_ids.iter().filter(|app_id| !attached.contains(app_id)) {
            self.call(
                &format!("attach app {app_id} to bot {account_id}"),
                HubCall::new(HubMethod::Put, &["bots", &account_id, "apps", app_id]),
            )
            .await?;
        }
        for app_id in attached
            .iter()
            .filter(|app_id| detach && !app_ids.contains(app_id))
        {
            self.call(
                &format!("detach app {app_id} from bot {account_id}"),
                HubCall::new(HubMethod::Delete, &["bots", &account_id, "apps", app_id]),
            )
            .await?;
        }
        Ok(())
    }

    /// The values go from the store to the hub here; a secret this device's key no longer
    /// opens stops the move with its issue before any value is sent.
    async fn upload_secrets(&mut self, bundle: &LocalBotBundle) -> Result<(), MoveError> {
        self.enter(MoveStep::Secrets);
        if bundle.secrets.is_empty() {
            return Ok(());
        }
        let operation = format!("move the secrets of bot {}", self.bot_id);
        let bot_id = self.bot_id.to_string();
        let opened = self
            .local(&operation, move |store| store.open_secrets(&bot_id))
            .await?;
        if let Some(name) = opened.unreadable.first() {
            let issue = unreadable_secret(name);
            return Err(MoveError::blocked(
                self.step,
                format!("Could not {operation}: {}", issue.message),
                issue,
            ));
        }
        let account_id = self.account_id.clone();
        for secret in &opened.readable {
            self.call(
                &format!("move secret {} of bot {}", secret.name, self.bot_id),
                HubCall::new(
                    HubMethod::Put,
                    &["bots", &account_id, "secrets", &secret.name],
                )
                .with(HubBody::Secret(secret)),
            )
            .await?;
        }
        Ok(())
    }

    /// The account's workspace for the bot, made on first use, and the files of the local one
    /// copied into it. Done once per move; a bot without a workspace here has nothing to do.
    async fn move_workspace(&mut self, bundle: &LocalBotBundle) -> Result<(), MoveError> {
        let Some(local) = bundle.bot.workspace_app_id.clone() else {
            return Ok(());
        };
        if self.workspace.is_some() {
            return Ok(());
        }
        self.enter(MoveStep::Workspace);
        let account_id = self.account_id.clone();
        let operation = format!("make the workspace of bot {account_id} in your account");
        let made = self
            .call(
                &operation,
                HubCall::new(HubMethod::Post, &["bots", &account_id, "workspace"]),
            )
            .await?;
        let account = made
            .as_ref()
            .and_then(|made| text(made, "app_id"))
            .map(str::to_string)
            .ok_or_else(|| {
                MoveError::new(
                    self.step,
                    HUB_RESPONSE_INVALID,
                    format!("Could not {operation}: the hub returned no app"),
                )
            })?;
        self.ids.set_workspace(&local, &account);
        self.copy_workspace(&local, false).await?;
        self.workspace = Some(local);
        Ok(())
    }

    /// Brings the account's copy of the workspace up to date and checks it. A local workspace
    /// without files needs no upload. With `compare_first`, files already there are not sent
    /// again: the upload only runs when the copies differ.
    async fn copy_workspace(&self, local: &str, compare_first: bool) -> Result<(), MoveError> {
        let measuring = format!("measure the files of workspace {}", local);
        let summary = self
            .hub
            .summarize_workspace(local)
            .await
            .map_err(|error| MoveError::hub(self.step, &measuring, error))?;
        if summary.is_empty() {
            return Ok(());
        }
        let grant = self.workspace_grant(local, summary).await?;
        if compare_first && self.workspace_differences(local, &grant).await?.is_empty() {
            return Ok(());
        }
        self.hub
            .upload_workspace(local, &grant)
            .await
            .map_err(|error| {
                MoveError::hub(
                    self.step,
                    &format!("copy workspace {} to your account", local),
                    error,
                )
            })?;
        let missing = self.workspace_differences(local, &grant).await?;
        match missing.first() {
            None => Ok(()),
            Some(path) => Err(MoveError::new(
                self.step,
                WORKSPACE_NOT_VERIFIED,
                format!(
                    "Could not move workspace {} to your account: {} of its files are missing there or differ, such as {path}",
                    local,
                    missing.len()
                ),
            )),
        }
    }

    async fn workspace_grant(
        &self,
        local: &str,
        summary: WorkspaceSummary,
    ) -> Result<Value, MoveError> {
        let request = json!({ "summary": summary });
        let operation = format!("ask your account to take workspace {}", local);
        self.call(
            &operation,
            HubCall::new(
                HubMethod::Post,
                &["bots", &self.account_id, "workspace", "upload"],
            )
            .with(HubBody::Json(&request)),
        )
        .await?
        .ok_or_else(|| {
            MoveError::new(
                self.step,
                HUB_RESPONSE_INVALID,
                format!("Could not {operation}: the hub returned no upload"),
            )
        })
    }

    async fn workspace_differences(
        &self,
        local: &str,
        grant: &Value,
    ) -> Result<Vec<String>, MoveError> {
        self.hub
            .workspace_differences(local, grant)
            .await
            .map_err(|error| {
                MoveError::hub(
                    self.step,
                    &format!("compare workspace {} with your account's copy", local),
                    error,
                )
            })
    }

    /// Deletes the local workspace app, but only after the account's copy was compared once
    /// more and lacked nothing. Files written meanwhile are sent first.
    async fn forget_workspace(&mut self) -> Result<(), MoveError> {
        let Some(local) = self.workspace.clone() else {
            return Ok(());
        };
        self.enter(MoveStep::ForgetWorkspace);
        self.copy_workspace(&local, true).await?;
        self.hub.delete_workspace(&local).await.map_err(|error| {
            MoveError::hub(
                self.step,
                &format!("remove workspace {local} from this computer"),
                error,
            )
        })
    }

    /// Copies what each movable chat holds that the account lacks, then removes the chat here
    /// unless it changed meanwhile.
    async fn move_chats(&mut self, bundle: &LocalBotBundle) -> Result<(), MoveError> {
        let movable = split_conversations(bundle, &self.account_bots).movable;
        self.enter(MoveStep::Conversations);
        for item in &movable {
            self.import_chat(item).await?;
        }
        if movable.iter().any(|item| self.unsent_segments(item) > 0) {
            self.enter(MoveStep::Segments);
            for item in &movable {
                self.write_segments(item).await?;
            }
        }
        self.enter(MoveStep::ForgetConversations);
        for item in &movable {
            self.forget_chat(item).await?;
        }
        Ok(())
    }

    async fn import_chat(&mut self, item: &ExportedConversation) -> Result<(), MoveError> {
        let conversation = &item.conversation;
        let sent = self.chats.entry(conversation.id.clone()).or_default();
        let created = sent.account_id.is_some();
        let fresh: Vec<&InboxEntry> = entries_to_move(item, &self.account_bots)
            .0
            .into_iter()
            .filter(|entry| !sent.message_ids.contains(&entry.message_id))
            .collect();
        if created && fresh.is_empty() {
            return Ok(());
        }
        let operation = format!(
            "move conversation {} of bot {}",
            conversation.id, self.bot_id
        );
        for request in import_conversation_requests(conversation, &fresh, &self.ids) {
            let body = self.body(&request)?;
            let answer = self
                .call(
                    &operation,
                    HubCall::new(HubMethod::Post, &["inbox", "import"]).with(HubBody::Json(&body)),
                )
                .await?;
            let Some(account_id) = answer
                .as_ref()
                .and_then(|answer| answer.get("conversation"))
                .and_then(|moved| text(moved, "id"))
                .map(str::to_string)
            else {
                return Err(MoveError::new(
                    self.step,
                    HUB_RESPONSE_INVALID,
                    format!("Could not {operation}: the hub returned no conversation"),
                ));
            };
            let sent = self.chats.entry(conversation.id.clone()).or_default();
            sent.account_id = Some(account_id);
            for entry in &request.entries {
                sent.message_ids.insert(entry.message_id.to_string());
                sent.stripped
                    .insert(entry.message_id.to_string(), entry.stripped);
            }
        }
        Ok(())
    }

    fn unsent_segments(&self, item: &ExportedConversation) -> usize {
        let sent = self.chats.get(&item.conversation.id);
        item.segments
            .iter()
            .filter(|segment| sent.is_none_or(|sent| !sent.segment_ids.contains(&segment.id)))
            .count()
    }

    fn warnings(&self) -> usize {
        self.chats
            .values()
            .map(|sent| sent.stripped.values().sum::<usize>() + sent.dropped_segments)
            .sum()
    }

    /// Every `(sender, message id)` of the account's conversation with the position the account
    /// gave it, read by position.
    async fn account_positions(
        &self,
        account_chat: &str,
        operation: &str,
    ) -> Result<HashMap<(String, String), i64>, MoveError> {
        let mut found = HashMap::new();
        let mut from = 1_i64;
        loop {
            let from_seq = from.to_string();
            let page = self
                .call(
                    operation,
                    HubCall::new(
                        HubMethod::Get,
                        &["inbox", "conversations", account_chat, "entries"],
                    )
                    .query(&[("from_seq", from_seq.as_str()), ("limit", "100")]),
                )
                .await?;
            for entry in records(page.as_ref(), "entries") {
                if let (Some(sender), Some(message_id), Some(seq)) = (
                    text(entry, "sender_id"),
                    text(entry, "message_id"),
                    entry.get("seq").and_then(Value::as_i64),
                ) {
                    found.insert((sender.to_string(), message_id.to_string()), seq);
                }
            }
            let next = page
                .as_ref()
                .and_then(|page| page.get("next_from_seq"))
                .and_then(Value::as_i64);
            match next {
                Some(next) if next > from => from = next,
                _ => return Ok(found),
            }
        }
    }

    /// Writes the bot's summaries of a moved chat that the account lacks, parents first, each
    /// range mapped to the positions the account gave the entries it covers. A summary none of
    /// whose entries moved is left out.
    async fn write_segments(&mut self, item: &ExportedConversation) -> Result<(), MoveError> {
        let local_id = &item.conversation.id;
        let Some(account_chat) = self
            .chats
            .get(local_id)
            .and_then(|sent| sent.account_id.clone())
        else {
            return Ok(());
        };
        let pending: Vec<&ConversationSegment> = item
            .segments
            .iter()
            .filter(|segment| {
                self.chats
                    .get(local_id)
                    .is_some_and(|sent| !sent.segment_ids.contains(&segment.id))
            })
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        let operation = format!(
            "move the summaries of conversation {local_id} of bot {}",
            self.bot_id
        );
        let account = self.account_positions(&account_chat, &operation).await?;
        let moved = entries_to_move(item, &self.account_bots).0;
        let positions = PositionMap::new(&moved, &self.ids, &account);
        let plan = segment_requests(pending.iter().copied(), &positions);
        let account_id = self.account_id.clone();
        for request in &plan.requests {
            let body = self.body(request)?;
            self.call(
                &operation,
                HubCall::new(
                    HubMethod::Post,
                    &["inbox", "conversations", &account_chat, "segments"],
                )
                .query(&[("as", account_id.as_str())])
                .with(HubBody::Json(&body)),
            )
            .await?;
        }
        let sent = self.chats.entry(local_id.clone()).or_default();
        sent.dropped_segments += plan.dropped;
        sent.segment_ids
            .extend(pending.iter().map(|segment| segment.id.clone()));
        Ok(())
    }

    /// Kept here when it changed after this export; the next pass carries the rest.
    async fn forget_chat(&mut self, item: &ExportedConversation) -> Result<(), MoveError> {
        let local_id = item.conversation.id.clone();
        let newest = item.entries.last().map_or_else(
            || BEFORE_EVERY_EVENT.to_string(),
            |entry| entry.cursor.clone(),
        );
        let conversation_id = local_id.clone();
        let forgotten = self
            .local(
                &format!("remove local conversation {local_id} from this computer"),
                move |store| store.forget_conversation(&conversation_id, Some(newest.as_str())),
            )
            .await;
        match forgotten {
            Ok(()) => {}
            Err(error) if error.code == codes::LOCAL_CHANGED => return Ok(()),
            Err(error) => return Err(error),
        }
        if let Some(account_id) = self
            .chats
            .get(&local_id)
            .and_then(|sent| sent.account_id.clone())
        {
            self.moved.push(MovedConversation {
                local_id,
                account_id,
            });
        }
        Ok(())
    }

    /// Once a fresh export leaves no chat to carry, points the chats that stay here at the
    /// account bot and removes the bot, unless it changed since that export.
    async fn forget_bot(&mut self, bundle: &LocalBotBundle) -> Result<bool, MoveError> {
        if !split_conversations(bundle, &self.account_bots)
            .movable
            .is_empty()
        {
            return Ok(false);
        }
        self.forget_workspace().await?;
        self.enter(MoveStep::Rebind);
        let local_id = self.bot_id.to_string();
        let account_id = self.account_id.clone();
        let operation =
            format!("point this computer's chats from bot {local_id} to bot {account_id}");
        let (from, to) = (local_id.clone(), account_id);
        self.local(&operation, move |store| store.rebind_bot(&from, &to))
            .await?;
        self.enter(MoveStep::ForgetBot);
        let expected = bundle.bot.updated_at.to_rfc3339();
        let forgotten = self
            .local(
                &format!("remove local bot {local_id} from this computer"),
                move |store| store.forget_bot(&local_id, Some(expected.as_str())),
            )
            .await;
        match forgotten {
            Ok(()) => Ok(true),
            Err(error) if error.code == codes::LOCAL_CHANGED => Ok(false),
            Err(error) => Err(error),
        }
    }
}
