//! Bot teams: people and bots in nested teams with one lead bot per team. The teams under one
//! top-level team form an org; every active member of any of its teams sees the whole org, bots
//! with their job descriptions included. The API stores teams, the bot hosts hand a bot in a team
//! its [`TeamBrief`], and [`team_brief_text`] writes what that bot reads before every turn, so the
//! prompt and the "what it reads" view in the app never differ.

use crate::data_classification::DataClassification;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_TEAM_NAME_CHARS: usize = 60;
pub const MAX_TEAM_PURPOSE_CHARS: usize = 600;
/// Levels a team may sit below its top-level team.
pub const MAX_TEAM_DEPTH: usize = 5;
/// Teams in one org.
pub const MAX_ORG_TEAMS: usize = 100;
/// Active and pending bots in one team. The team room holds the team's people too, and a
/// conversation has at most 50 members.
pub const MAX_TEAM_BOTS: usize = 30;
/// Active and pending people in one team.
pub const MAX_TEAM_PEOPLE: usize = 20;
/// Teams the organization section of a brief names, those nearest the bot's team first; the rest
/// are counted and found with [`ORG_LOOKUP_TOOL`].
pub const MAX_BRIEF_TEAMS: usize = 15;
/// The tool a bot in a team searches its org with.
pub const ORG_LOOKUP_TOOL: &str = "org_lookup";
/// Teams one lookup describes in full; an empty query lists every team without their members.
pub const MAX_LOOKUP_TEAMS: usize = 8;
pub const MAX_LOOKUP_BOTS: usize = 15;
pub const MAX_LOOKUP_PEOPLE: usize = 15;
const DESCRIPTION_CHARS: usize = 140;
const LOOKUP_TEXT_CHARS: usize = 600;

/// What a person may do in a team. The wire format is lowercase; the database stores uppercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    /// Renames the team, sets its purpose, lead and place in the org, invites and removes.
    Admin,
    /// Works with the team's bots and adds their own bots.
    Member,
}

impl TeamRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "ADMIN",
            Self::Member => "MEMBER",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "ADMIN" => Some(Self::Admin),
            "MEMBER" => Some(Self::Member),
            _ => None,
        }
    }
}

/// Whether a person or bot is in a team or only invited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TeamMemberStatus {
    /// Invited; a person accepts for themselves, a bot's owner for the bot.
    Pending,
    Active,
}

impl TeamMemberStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Active => "ACTIVE",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "PENDING" => Some(Self::Pending),
            "ACTIVE" => Some(Self::Active),
            _ => None,
        }
    }
}

/// A person in a team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TeamPerson {
    pub user_id: String,
    pub role: TeamRole,
    pub status: TeamMemberStatus,
    #[serde(default)]
    pub invited_by: Option<String>,
}

/// A bot in a team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TeamBot {
    pub bot_id: String,
    pub owner_id: String,
    pub status: TeamMemberStatus,
    /// The bot's owner agreed to direct asks in this team.
    pub direct_agreed: bool,
    #[serde(default)]
    pub invited_by: Option<String>,
}

/// One team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Team {
    pub id: String,
    pub name: String,
    /// What every bot in the team reads about its work.
    #[serde(default)]
    pub purpose: Option<String>,
    /// The team it sits in; `None` for a top-level team, which names its org.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// One of the team's active bots.
    #[serde(default)]
    pub lead_bot_id: Option<String>,
    /// Teammates ask each other without the first-time Allow card once their owners agreed.
    pub direct_asks: bool,
    /// The team room.
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// The bot an admin asked to lead the team, until its owner answers. Only the team's admins
    /// and that bot's owner see it.
    #[serde(default)]
    pub lead_offer_bot_id: Option<String>,
    /// Bots that take no turn in the team room while someone in it is classified below them.
    #[serde(default)]
    pub gated_bots: Vec<String>,
    pub people: Vec<TeamPerson>,
    pub bots: Vec<TeamBot>,
    /// RFC 3339.
    pub created_at: String,
    pub updated_at: String,
}

/// A bot as every member of its org sees it: who it is and what it does, never its settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OrgBot {
    pub id: String,
    pub owner_id: String,
    /// `general` or `flux` for a built-in bot.
    #[serde(default)]
    pub system: Option<String>,
    /// `None` on a built-in bot its owner never renamed.
    #[serde(default)]
    pub name: Option<String>,
    /// Its job description.
    #[serde(default)]
    pub description: Option<String>,
    #[schemars(with = "serde_json::Value")]
    pub appearance: serde_json::Value,
    /// `server`, `desktop` or `device`.
    pub location: String,
    /// The bot's data classification, lowercase; `None` counts as internal.
    #[serde(default)]
    pub classification: Option<String>,
    /// What starts its work from outside the chats, and what it sends out.
    #[serde(default)]
    pub inputs: OrgBotInputs,
}

/// How work reaches a bot from outside its chats, as its org sees it: ways in and their recent
/// traffic, never a webhook path, a secret or a calendar account.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct OrgBotInputs {
    /// Its own mail address while the email ability is on.
    #[serde(default)]
    pub mail: Option<OrgBotMail>,
    /// Its switched-on webhooks while the webhook ability is on, by name.
    #[serde(default)]
    pub webhooks: Vec<OrgBotWebhook>,
    /// The calendar ability is on: the bot keeps its own calendar and its events wake it.
    #[serde(default)]
    pub own_calendar: bool,
    /// One provider (`google`, `microsoft`, `caldav`) per connected calendar the bot reads,
    /// while the calendar ability is on.
    #[serde(default)]
    pub connected_calendars: Vec<String>,
}

/// Days [`OrgBotMail`] counts mail over.
pub const ORG_MAIL_WINDOW_DAYS: i64 = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OrgBotMail {
    pub address: String,
    /// Mails it received in the last [`ORG_MAIL_WINDOW_DAYS`] days.
    #[serde(default)]
    pub received: u32,
    /// Mails it sent in the last [`ORG_MAIL_WINDOW_DAYS`] days.
    #[serde(default)]
    pub sent: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OrgBotWebhook {
    pub name: String,
    /// RFC 3339; `None` while nothing called it.
    #[serde(default)]
    pub last_called_at: Option<String>,
}

/// What waits for the reader's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TeamInvitationKind {
    /// The reader is invited to the team.
    Person,
    /// One of the reader's bots is invited to the team.
    Bot,
    /// The team turned on direct asks and one of the reader's bots in it waits for their agreement.
    DirectAsks,
    /// An admin asks one of the reader's bots to lead the team, which makes it the team's contact
    /// for everyone in the org.
    Lead,
    /// Someone in the org (`user_id`) asks to talk with one of the reader's bots.
    ChatRequest,
}

/// One answer the reader owes a team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TeamInvitation {
    pub kind: TeamInvitationKind,
    pub team_id: String,
    pub team_name: String,
    /// The name of the team's top-level team.
    pub org_name: String,
    /// The reader's bot, for every kind but `person`.
    #[serde(default)]
    pub bot_id: Option<String>,
    #[serde(default)]
    pub invited_by: Option<String>,
    /// For `chat_request`: who asks.
    #[serde(default)]
    pub user_id: Option<String>,
    /// For `chat_request`: why they ask.
    #[serde(default)]
    pub note: Option<String>,
    /// RFC 3339.
    pub created_at: String,
}

/// Every team of every org the reader belongs to, the bots in them, and what waits for the reader.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct OrgView {
    pub teams: Vec<Team>,
    pub bots: Vec<OrgBot>,
    pub invitations: Vec<TeamInvitation>,
    /// The viewer's own threads with teams, where their `#Team` requests went.
    #[serde(default)]
    pub threads: Vec<TeamThread>,
    /// How the reader reaches each bot in `bots`.
    #[serde(default)]
    pub reach: Vec<BotReach>,
    /// The reader's chats with other owners' bots, and other people's chats with the reader's
    /// bots; ended ones are left out.
    #[serde(default)]
    pub chats: Vec<BotChat>,
    /// The reader's own classification in each of their orgs, and everyone's in the orgs whose
    /// top-level team the reader is an admin of. A person without one counts as internal.
    #[serde(default)]
    pub classifications: Vec<PersonClassification>,
}

/// A person's thread with a team: their `#Team` requests land here and the team's lead answers.
/// Its members are the person, the current lead and the lead's owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TeamThread {
    pub team_id: String,
    pub conversation_id: String,
}

/// On the copy of a team request in the person's thread: the message it copies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TeamRequestRef {
    pub team_id: String,
    pub origin_conversation_id: String,
    pub origin_message_id: String,
    /// The lead's answer is copied back into the origin, which is the person's own chat with one
    /// of their bots. Anywhere else the answer stays in the thread.
    #[serde(default)]
    pub answer_here: bool,
}

/// A bot as a brief names it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct BriefBot {
    pub id: String,
    pub name: String,
    /// The display name of its owner.
    pub owner_name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// A team as a brief's organization section counts and names it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct BriefTeam {
    pub id: String,
    pub name: String,
    /// `None` for the top-level team, which names the org.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// 0 for the top-level team.
    pub depth: u32,
    #[serde(default)]
    pub purpose: Option<String>,
    /// The lead's name.
    #[serde(default)]
    pub lead: Option<String>,
    /// Its active bots, the lead included.
    #[serde(default)]
    pub bots: u32,
}

/// A person's place in one team, as a brief names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BriefMembership {
    pub team: String,
    pub role: TeamRole,
}

/// A person in the conversation a turn runs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BriefPerson {
    pub name: String,
    /// The bot's owner.
    #[serde(default)]
    pub owner: bool,
    /// The org's teams they are active in.
    #[serde(default)]
    pub teams: Vec<BriefMembership>,
    /// Their classification in the org; [`UNCLASSIFIED_PERSON`] without one.
    pub classification: DataClassification,
}

/// What a bot in a team knows about its team and its org. A host builds it from the team tables
/// and the bot reads [`team_brief_text`] of it before every turn.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct TeamBrief {
    pub bot_id: String,
    pub bot_name: String,
    pub owner_name: String,
    pub team_id: String,
    pub team_name: String,
    #[serde(default)]
    pub purpose: Option<String>,
    /// The team it sits in.
    #[serde(default)]
    pub parent_name: Option<String>,
    /// The bot leads its team.
    pub is_lead: bool,
    /// The team's lead, when that is another bot.
    #[serde(default)]
    pub lead: Option<BriefBot>,
    /// For a lead: the lead of the team above.
    #[serde(default)]
    pub reports_to: Option<BriefBot>,
    /// For a lead: the leads of the teams inside its team.
    #[serde(default)]
    pub sub_team_leads: Vec<BriefBot>,
    /// The other active bots of its team.
    #[serde(default)]
    pub teammates: Vec<BriefBot>,
    /// Teammates are asked without the first-time Allow card.
    pub direct_asks: bool,
    /// The team room's name, `#Security`.
    #[serde(default)]
    pub room: Option<String>,
    /// Every team of the org, depth first, the top-level team first. The brief names the teams
    /// near the bot's own and counts the rest.
    #[serde(default)]
    pub org: Vec<BriefTeam>,
    /// The people in the conversation the turn runs in; empty in the owner's own chat.
    #[serde(default)]
    pub present: Vec<BriefPerson>,
}

/// The tag a team is addressed by in chats: its name without spaces after `#`.
pub fn team_tag(name: &str) -> String {
    let compact: String = name.split_whitespace().collect();
    format!("#{compact}")
}

/// Teams one message can address.
pub const MAX_REQUEST_TEAMS: usize = 3;
/// Characters of a lead's answer copied back into the chat its request came from.
pub const TEAM_ANSWER_MAX_CHARS: usize = 4_000;

/// The `#Tag`s a message starts with, without `#` and trailing punctuation, in order and at most
/// [`MAX_REQUEST_TEAMS`]: "#Security #ITSupport, is the VPN down?" gives `Security` and
/// `ITSupport`. A tag later in the text or a markdown heading ("# Notes") addresses no team.
pub fn leading_team_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for token in text.split_whitespace() {
        let Some(tag) = token.strip_prefix('#') else {
            break;
        };
        let tag = tag.trim_end_matches([',', ':', ';', '.', '!', '?']);
        if tag.is_empty() || tag.starts_with('#') {
            break;
        }
        if !tags
            .iter()
            .any(|known| known.to_lowercase() == tag.to_lowercase())
        {
            tags.push(tag.to_string());
        }
        if tags.len() == MAX_REQUEST_TEAMS {
            break;
        }
    }
    tags
}

/// Whether `tag`, as [`leading_team_tags`] returns it, names the team called `name`.
pub fn tag_names_team(tag: &str, name: &str) -> bool {
    let full = team_tag(name);
    full.trim_start_matches('#').to_lowercase() == tag.to_lowercase()
}

fn clip(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", text[..index].trim_end()),
        None => text.to_string(),
    }
}

fn sentence(text: &str) -> String {
    if text.ends_with(['.', '!', '?', '…']) {
        text.to_string()
    } else {
        format!("{text}.")
    }
}

fn first_line(text: Option<&str>) -> Option<String> {
    text.and_then(|text| text.lines().map(str::trim).find(|line| !line.is_empty()))
        .map(|line| sentence(&clip(line, DESCRIPTION_CHARS)))
}

fn names(bots: &[BriefBot]) -> String {
    let names: Vec<&str> = bots.iter().map(|bot| bot.name.as_str()).collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn bot_line(bot: &BriefBot, role: Option<&str>) -> String {
    let role = role.map(|role| format!(" ({role})")).unwrap_or_default();
    let description = first_line(bot.description.as_deref())
        .map(|line| format!(" — {line}"))
        .unwrap_or_else(|| ".".to_string());
    format!(
        "- {}{role}, {}'s bot{description}",
        bot.name, bot.owner_name
    )
}

/// What a bot in a team reads before every turn: its team, its teammates, the org and how to
/// reach other teams.
pub fn team_brief_text(brief: &TeamBrief) -> String {
    let mut out = Vec::new();
    let mut team = vec!["## YOUR TEAM".to_string()];
    let top_level = root_team(brief).is_some_and(|root| root.id == brief.team_id);
    if top_level {
        team.push(format!(
            "You are {}, {}'s bot, in {}, the organization's top-level team.",
            brief.bot_name, brief.owner_name, brief.team_name
        ));
    } else {
        let place = brief
            .parent_name
            .as_deref()
            .map(|parent| format!(", part of {parent}"))
            .unwrap_or_default();
        team.push(format!(
            "You are {}, {}'s bot, in the team {}{place}.",
            brief.bot_name, brief.owner_name, brief.team_name
        ));
    }
    if let Some(purpose) = filled(brief.purpose.as_deref()).filter(|_| !top_level) {
        team.push(format!(
            "{}'s purpose: {}",
            brief.team_name,
            sentence(purpose)
        ));
    }
    if brief.is_lead {
        let reports = &brief.teammates;
        if reports.is_empty() {
            team.push(format!("You lead {}.", brief.team_name));
        } else {
            team.push(format!(
                "You lead {}. {} {} to you: hand out work inside the team and answer for it.",
                brief.team_name,
                names(reports),
                if reports.len() == 1 {
                    "reports"
                } else {
                    "report"
                }
            ));
        }
        if !brief.sub_team_leads.is_empty() {
            team.push(format!(
                "The leads of the teams inside {} report to you: {}.",
                brief.team_name,
                names(&brief.sub_team_leads)
            ));
        }
        if let Some(boss) = &brief.reports_to {
            team.push(format!(
                "You report to {}, the lead above your team.",
                boss.name
            ));
        }
        if let Some(room) = &brief.room {
            team.push(format!(
                "Messages in {room} reach you first: answer what you can, hand the rest to a teammate by writing @Name, and close with a short summary."
            ));
        }
    } else {
        if let Some(lead) = &brief.lead {
            team.push(format!(
                "You report to {}, the team's lead. Take decisions above you to {}.",
                lead.name, lead.name
            ));
        }
        if let Some(room) = &brief.room {
            team.push(format!(
                "In {room}, answer when someone names you or the lead hands you the word."
            ));
        }
    }
    out.push(team.join("\n"));

    if !brief.teammates.is_empty() {
        let how = if brief.direct_asks {
            "Ask them directly with ask_bot; no approval is needed inside this team."
        } else {
            "Ask them with ask_bot; your owner approves your first question to each."
        };
        let mut lines = vec!["## TEAMMATES".to_string(), how.to_string()];
        lines.extend(brief.teammates.iter().map(|bot| {
            let lead = brief.lead.as_ref().is_some_and(|lead| lead.id == bot.id);
            bot_line(bot, lead.then_some("lead"))
        }));
        out.push(lines.join("\n"));
    }

    out.extend(org_section(brief));
    out.extend(present_section(brief));
    out.push(format!(
        "## OTHER TEAMS\nFor work another team covers, ask that team's lead; the lead hands it on. Your first question to a bot outside {} needs {}'s OK, and a bot classified above you never answers you.",
        brief.team_name, brief.owner_name
    ));
    out.join("\n\n")
}

fn filled(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|text| !text.is_empty())
}

fn counted(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
}

fn role_name(role: TeamRole) -> &'static str {
    match role {
        TeamRole::Admin => "admin",
        TeamRole::Member => "member",
    }
}

fn root_team(brief: &TeamBrief) -> Option<&BriefTeam> {
    brief.org.iter().find(|team| team.parent_id.is_none())
}

fn children<'a>(org: &'a [BriefTeam], parent: Option<&str>) -> Vec<&'a str> {
    let Some(parent) = parent else {
        return Vec::new();
    };
    org.iter()
        .filter(|team| team.parent_id.as_deref() == Some(parent))
        .map(|team| team.id.as_str())
        .collect()
}

/// The teams a brief names: the bot's team and those above it, then the teams inside it, beside it
/// and on the org's first level, at most [`MAX_BRIEF_TEAMS`].
fn near_teams(brief: &TeamBrief) -> Vec<&str> {
    let by_id: HashMap<&str, &BriefTeam> = brief
        .org
        .iter()
        .map(|team| (team.id.as_str(), team))
        .collect();
    let mut chain: Vec<&str> = Vec::new();
    let mut current = by_id.get(brief.team_id.as_str()).copied();
    while let Some(team) = current {
        if chain.contains(&team.id.as_str()) || chain.len() > MAX_TEAM_DEPTH {
            break;
        }
        chain.push(team.id.as_str());
        current = team
            .parent_id
            .as_deref()
            .and_then(|parent| by_id.get(parent).copied());
    }
    let parent = by_id
        .get(brief.team_id.as_str())
        .and_then(|team| team.parent_id.as_deref());
    let root = root_team(brief).map(|team| team.id.as_str());
    let mut near: Vec<&str> = Vec::new();
    for id in chain
        .into_iter()
        .chain(children(&brief.org, Some(brief.team_id.as_str())))
        .chain(children(&brief.org, parent))
        .chain(children(&brief.org, root))
    {
        if near.len() == MAX_BRIEF_TEAMS {
            break;
        }
        if !near.contains(&id) {
            near.push(id);
        }
    }
    near
}

fn org_section(brief: &TeamBrief) -> Option<String> {
    let root = root_team(brief)?;
    let org = root.name.as_str();
    let bots: usize = brief.org.iter().map(|team| team.bots as usize).sum();
    let mut lines = vec![
        "## THE ORGANIZATION".to_string(),
        format!(
            "You work in {org}, an organization of {} and {}.",
            counted(brief.org.len(), "team"),
            counted(bots, "bot")
        ),
    ];
    if let Some(purpose) = filled(root.purpose.as_deref()) {
        lines.push(format!("What {org} does: {}", sentence(purpose)));
    }
    let near = near_teams(brief);
    lines.push(
        if near.len() < brief.org.len() {
            "Teams near yours:"
        } else {
            "Its teams:"
        }
        .to_string(),
    );
    for team in brief
        .org
        .iter()
        .filter(|team| near.contains(&team.id.as_str()))
    {
        let indent = "  ".repeat(team.depth as usize);
        let lead = team
            .lead
            .as_deref()
            .map(|lead| format!(" — lead {lead}"))
            .unwrap_or_default();
        let here = if team.id == brief.team_id {
            " (your team)"
        } else {
            ""
        };
        let purpose = team
            .parent_id
            .as_ref()
            .and_then(|_| first_line(team.purpose.as_deref()))
            .map(|line| format!(": {line}"))
            .unwrap_or_default();
        lines.push(format!("{indent}- {}{lead}{here}{purpose}", team.name));
    }
    let hidden = brief.org.len().saturating_sub(near.len());
    if hidden > 0 {
        let teams = if hidden == 1 { "team" } else { "teams" };
        lines.push(format!("…and {hidden} more {teams}."));
    }
    lines.push(format!(
        "This is your own organization. Answer questions about {org}, its teams, bots and people from this brief and `{ORG_LOOKUP_TOOL}` first; the web knows only what {org} published. `{ORG_LOOKUP_TOOL}` finds any team, bot or person of {org}: what a team does, who leads it and works in it, and the teams a person is in."
    ));
    Some(lines.join("\n"))
}

fn present_section(brief: &TeamBrief) -> Option<String> {
    if brief.present.iter().all(|person| person.owner) {
        return None;
    }
    let lowest = brief
        .present
        .iter()
        .map(|person| person.classification)
        .min()?;
    let mut lines = vec![
        "## IN THIS CONVERSATION".to_string(),
        "The people here, their teams and their classification:".to_string(),
    ];
    for person in &brief.present {
        let owner = if person.owner { " (your owner)" } else { "" };
        let teams: Vec<String> = person
            .teams
            .iter()
            .map(|membership| format!("{} ({})", membership.team, role_name(membership.role)))
            .collect();
        let teams = if teams.is_empty() {
            "in no team".to_string()
        } else {
            teams.join(", ")
        };
        lines.push(format!(
            "- {}{owner}: {teams}; classified {}",
            person.name, person.classification
        ));
    }
    lines.push(format!(
        "Everyone here reads your answer, so nothing classified above {lowest} belongs in it. Use people's teams and roles when you judge what someone may see or ask you to do."
    ));
    Some(lines.join("\n"))
}

/// One org as [`org_lookup`] searches it: what every member of the org sees in the org chart, never
/// anyone's classification.
#[derive(Debug, Clone, Default)]
pub struct OrgDirectory {
    /// The top-level team first.
    pub teams: Vec<DirectoryTeam>,
    pub bots: Vec<DirectoryBot>,
    pub people: Vec<DirectoryPerson>,
}

#[derive(Debug, Clone, Default)]
pub struct DirectoryTeam {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub purpose: Option<String>,
    pub lead_bot_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DirectoryBot {
    pub id: String,
    pub name: String,
    pub owner_name: String,
    pub description: Option<String>,
    /// The team it is active in.
    pub team_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct DirectoryPerson {
    pub name: String,
    /// The ids of the teams they are active in, with their role there.
    pub teams: Vec<(String, TeamRole)>,
}

/// What [`ORG_LOOKUP_TOOL`] answers: the org's teams, bots and people matching the query.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct OrgLookup {
    pub org: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<FoundTeam>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bots: Vec<FoundBot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<FoundPerson>,
    /// What was left out, or why nothing matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct FoundTeam {
    pub name: String,
    /// How chats address it, `#ITSupport`.
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams_inside: Vec<String>,
    /// `Patch, Jonas's bot`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bots: Vec<String>,
    /// `Anna (admin)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct FoundBot {
    pub name: String,
    pub owner: String,
    pub team: String,
    #[serde(default)]
    pub leads_team: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct FoundPerson {
    pub name: String,
    /// `Finance (admin)`.
    pub teams: Vec<String>,
}

struct LookupQuery {
    whole: String,
    words: Vec<String>,
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|candidate| candidate.starts_with(word))
}

impl LookupQuery {
    fn new(text: &str) -> Self {
        let whole = text
            .trim()
            .trim_start_matches(['#', '@'])
            .trim()
            .to_lowercase();
        let words = whole
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.chars().count() >= 2)
            .map(str::to_string)
            .collect();
        Self { whole, words }
    }

    /// 100 for the exact name (spaces ignored), then the whole query inside the name, then each
    /// word that starts a word of the name or, weaker, of `text`.
    fn score(&self, name: &str, text: Option<&str>) -> u32 {
        let name = name.to_lowercase();
        let compact: String = name.split_whitespace().collect();
        if name == self.whole || compact == self.whole {
            return 100;
        }
        let text = text.map(str::to_lowercase).unwrap_or_default();
        let mut score = if name.contains(&self.whole) { 40 } else { 0 };
        for word in &self.words {
            if has_word(&name, word) {
                score += 10;
            } else if has_word(&text, word) {
                score += 3;
            }
        }
        score
    }
}

fn ranked<T>(items: &[T], score: impl Fn(&T) -> u32) -> Vec<&T> {
    let mut scored: Vec<(u32, &T)> = items
        .iter()
        .map(|item| (score(item), item))
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, item)| item).collect()
}

fn capped<T>(mut items: Vec<T>, max: usize, left_out: &mut usize) -> Vec<T> {
    *left_out += items.len().saturating_sub(max);
    items.truncate(max);
    items
}

impl OrgDirectory {
    fn team(&self, id: &str) -> Option<&DirectoryTeam> {
        self.teams.iter().find(|team| team.id == id)
    }

    fn team_name(&self, id: &str) -> String {
        self.team(id)
            .map(|team| team.name.clone())
            .unwrap_or_default()
    }

    fn bot_name(&self, id: &str) -> Option<String> {
        self.bots
            .iter()
            .find(|bot| bot.id == id)
            .map(|bot| bot.name.clone())
    }

    fn found_team(&self, team: &DirectoryTeam, members: bool) -> FoundTeam {
        let purpose = if members {
            filled(team.purpose.as_deref()).map(|purpose| clip(purpose, LOOKUP_TEXT_CHARS))
        } else {
            first_line(team.purpose.as_deref())
        };
        let mut found = FoundTeam {
            name: team.name.clone(),
            tag: team_tag(&team.name),
            purpose,
            part_of: team
                .parent_id
                .as_deref()
                .and_then(|parent| self.team(parent))
                .map(|parent| parent.name.clone()),
            lead: team
                .lead_bot_id
                .as_deref()
                .and_then(|lead| self.bot_name(lead)),
            ..FoundTeam::default()
        };
        if members {
            found.teams_inside = self
                .teams
                .iter()
                .filter(|inner| inner.parent_id.as_deref() == Some(team.id.as_str()))
                .map(|inner| inner.name.clone())
                .collect();
            found.bots = self
                .bots
                .iter()
                .filter(|bot| bot.team_id == team.id)
                .map(|bot| format!("{}, {}'s bot", bot.name, bot.owner_name))
                .collect();
            found.people = self
                .people
                .iter()
                .filter_map(|person| {
                    let (_, role) = person.teams.iter().find(|(id, _)| *id == team.id)?;
                    Some(format!("{} ({})", person.name, role_name(*role)))
                })
                .collect();
        }
        found
    }

    fn found_bot(&self, bot: &DirectoryBot) -> FoundBot {
        FoundBot {
            name: bot.name.clone(),
            owner: bot.owner_name.clone(),
            team: self.team_name(&bot.team_id),
            leads_team: self
                .team(&bot.team_id)
                .is_some_and(|team| team.lead_bot_id.as_deref() == Some(bot.id.as_str())),
            description: filled(bot.description.as_deref())
                .map(|text| clip(text, LOOKUP_TEXT_CHARS)),
        }
    }

    fn found_person(&self, person: &DirectoryPerson) -> FoundPerson {
        FoundPerson {
            name: person.name.clone(),
            teams: person
                .teams
                .iter()
                .map(|(id, role)| format!("{} ({})", self.team_name(id), role_name(*role)))
                .collect(),
        }
    }
}

/// Searches `directory`: names first, then team purposes and bots' job descriptions. A team found
/// comes with its bots, people and the teams inside it. An empty query lists every team without
/// its members.
pub fn org_lookup(directory: &OrgDirectory, query: &str) -> OrgLookup {
    let org = directory
        .teams
        .first()
        .map(|team| team.name.clone())
        .unwrap_or_default();
    let wanted = LookupQuery::new(query);
    if wanted.whole.is_empty() {
        return OrgLookup {
            teams: directory
                .teams
                .iter()
                .map(|team| directory.found_team(team, false))
                .collect(),
            note: Some(format!(
                "Every team of {org}. Look a team up by name for its bots, people and the teams inside it."
            )),
            org,
            ..OrgLookup::default()
        };
    }
    let mut left_out = 0usize;
    let teams = capped(
        ranked(&directory.teams, |team| {
            wanted.score(&team.name, team.purpose.as_deref())
        }),
        MAX_LOOKUP_TEAMS,
        &mut left_out,
    );
    let bots = capped(
        ranked(&directory.bots, |bot| {
            wanted.score(&bot.name, bot.description.as_deref())
        }),
        MAX_LOOKUP_BOTS,
        &mut left_out,
    );
    let people = capped(
        ranked(&directory.people, |person| wanted.score(&person.name, None)),
        MAX_LOOKUP_PEOPLE,
        &mut left_out,
    );
    let note = if teams.is_empty() && bots.is_empty() && people.is_empty() {
        Some(format!(
            "Nothing in {org} matches \"{}\". Look up with an empty query to list every team.",
            query.trim()
        ))
    } else if left_out == 1 {
        Some("1 more match was left out; narrow the query.".to_string())
    } else if left_out > 1 {
        Some(format!(
            "{left_out} more matches were left out; narrow the query."
        ))
    } else {
        None
    };
    OrgLookup {
        org,
        teams: teams
            .into_iter()
            .map(|team| directory.found_team(team, true))
            .collect(),
        bots: bots
            .into_iter()
            .map(|bot| directory.found_bot(bot))
            .collect(),
        people: people
            .into_iter()
            .map(|person| directory.found_person(person))
            .collect(),
        note,
    }
}

/// What a person without a classification of their own counts as, like an unclassified bot.
pub const UNCLASSIFIED_PERSON: DataClassification = DataClassification::Internal;
/// Characters of the note a person sends with a chat request.
pub const MAX_CHAT_NOTE_CHARS: usize = 280;
/// Days a declined chat request waits before the person may ask again.
pub const CHAT_RETRY_DAYS: i64 = 7;

/// Whether a person classified `person` may talk with a bot classified `bot`; `None` counts as
/// internal on both sides. It is "no read up" for people: a bot answers only people at or above
/// its level, and a bot asks only bots at or below its own, so nothing classified above the
/// person reaches them.
pub fn person_may_talk(
    person: Option<DataClassification>,
    bot: Option<DataClassification>,
) -> bool {
    person.unwrap_or(UNCLASSIFIED_PERSON) >= bot.unwrap_or(UNCLASSIFIED_PERSON)
}

/// Whether and where the reader can talk with a bot of their org, most specific first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TalkReach {
    /// The reader's own bot.
    Own,
    /// Invited and not accepted by its owner yet: nobody in the org talks with it.
    Unavailable,
    /// Classified above the reader: it doesn't answer them.
    Classified,
    /// A bot of a team the reader is in: the team room, where the lead reads along and the others
    /// answer when named.
    TeamRoom,
    /// The lead of a team the reader isn't in: the reader's thread with that team.
    Lead,
    /// Its owner approved a chat with the reader.
    Chat,
    /// The reader asked its owner for a chat and waits for the answer.
    Requested,
    /// Its owner declined the reader's request; they may ask again from `retry_at`.
    Declined,
    /// The reader may ask its owner for a chat.
    Request,
}

/// How the reader reaches one bot of [`OrgView::bots`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotReach {
    pub bot_id: String,
    pub reach: TalkReach,
    /// Where to talk: the team room, the reader's thread with the team, or the approved chat.
    /// `None` for a lead until the reader's thread with the team exists.
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// For `declined`, RFC 3339.
    #[serde(default)]
    pub retry_at: Option<String>,
}

/// Where a person's chat with another owner's bot stands. The wire format is lowercase; the
/// database stores uppercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BotChatStatus {
    /// Asked; the bot's owner hasn't answered.
    Pending,
    /// Approved: the person talks with the bot while both are in the org.
    Active,
    /// The owner said no; the person may ask again after [`CHAT_RETRY_DAYS`].
    Declined,
    /// The person or the owner ended it, or the bot or the person left the org. The person keeps
    /// the history.
    Ended,
}

impl BotChatStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Active => "ACTIVE",
            Self::Declined => "DECLINED",
            Self::Ended => "ENDED",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "PENDING" => Some(Self::Pending),
            "ACTIVE" => Some(Self::Active),
            "DECLINED" => Some(Self::Declined),
            "ENDED" => Some(Self::Ended),
            _ => None,
        }
    }
}

/// A person's chat with another owner's bot of an org they share, as the person and the bot's
/// owner see it. Its members are the person, the bot and the owner, who watches without being
/// pinged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotChat {
    pub bot_id: String,
    /// The person who asked.
    pub user_id: String,
    /// The bot's owner, who decides and watches.
    pub owner_id: String,
    pub status: BotChatStatus,
    /// Exists once approved.
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Why the person asks, at most [`MAX_CHAT_NOTE_CHARS`] characters.
    #[serde(default)]
    pub note: Option<String>,
    /// RFC 3339.
    pub requested_at: String,
    #[serde(default)]
    pub decided_at: Option<String>,
}

/// A person's classification in an org, set by an admin of its top-level team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PersonClassification {
    /// The org's top-level team.
    pub org_id: String,
    pub user_id: String,
    pub classification: DataClassification,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bot(id: &str, name: &str, owner: &str, description: &str) -> BriefBot {
        BriefBot {
            id: id.into(),
            name: name.into(),
            owner_name: owner.into(),
            description: Some(description.into()),
        }
    }

    fn security_brief(is_lead: bool) -> TeamBrief {
        let sentinel = bot("sentinel", "Sentinel", "Mara", "Triages security alerts.");
        let patch = bot("patch", "Patch", "Jonas", "Keeps laptops patched.");
        let audit = bot("audit", "Audit", "Jonas", "Reviews access logs.");
        TeamBrief {
            bot_id: if is_lead { "sentinel" } else { "patch" }.into(),
            bot_name: if is_lead { "Sentinel" } else { "Patch" }.into(),
            owner_name: if is_lead { "Mara" } else { "Jonas" }.into(),
            team_id: "security".into(),
            team_name: "Security".into(),
            purpose: Some("Keep our systems patched".into()),
            parent_name: Some("Kestrel".into()),
            is_lead,
            lead: (!is_lead).then(|| sentinel.clone()),
            reports_to: is_lead.then(|| bot("atlas", "Atlas", "Mara", "Chief of staff.")),
            sub_team_leads: Vec::new(),
            teammates: if is_lead {
                vec![patch.clone(), audit.clone()]
            } else {
                vec![sentinel.clone(), audit.clone()]
            },
            direct_asks: true,
            room: Some(team_tag("Security")),
            org: vec![
                brief_team("kestrel", None, 0, "Atlas", 1),
                brief_team("security", Some("kestrel"), 1, "Sentinel", 3),
                brief_team("finance", Some("kestrel"), 1, "Ledger", 2),
            ],
            present: Vec::new(),
        }
    }

    fn brief_team(id: &str, parent: Option<&str>, depth: u32, lead: &str, bots: u32) -> BriefTeam {
        let mut name = id.to_string();
        name[..1].make_ascii_uppercase();
        let purpose = match id {
            "kestrel" => "Kestrel builds drones for farms",
            "security" => "Keep our systems patched",
            "finance" => "Pays the bills\nand files taxes",
            _ => "",
        };
        BriefTeam {
            id: id.into(),
            name,
            parent_id: parent.map(str::to_string),
            depth,
            purpose: Some(purpose.into()),
            lead: Some(lead.into()),
            bots,
        }
    }

    fn person(
        name: &str,
        owner: bool,
        teams: &[(&str, TeamRole)],
        level: DataClassification,
    ) -> BriefPerson {
        BriefPerson {
            name: name.into(),
            owner,
            teams: teams
                .iter()
                .map(|(team, role)| BriefMembership {
                    team: (*team).into(),
                    role: *role,
                })
                .collect(),
            classification: level,
        }
    }

    #[test]
    fn tag_drops_spaces() {
        assert_eq!(team_tag("IT Support"), "#ITSupport");
    }

    #[test]
    fn member_brief_names_lead_room_teammates_and_org() {
        let text = team_brief_text(&security_brief(false));
        assert!(
            text.contains("You are Patch, Jonas's bot, in the team Security, part of Kestrel.")
        );
        assert!(text.contains("Security's purpose: Keep our systems patched."));
        assert!(text.contains("You report to Sentinel, the team's lead."));
        assert!(text.contains("In #Security, answer when someone names you"));
        assert!(text.contains("Ask them directly with ask_bot"));
        assert!(text.contains("- Sentinel (lead), Mara's bot — Triages security alerts."));
        assert!(text.contains("- Audit, Jonas's bot — Reviews access logs."));
        assert!(!text.contains("Joins once"));
        assert!(text.contains("You work in Kestrel, an organization of 3 teams and 6 bots."));
        assert!(text.contains("What Kestrel does: Kestrel builds drones for farms."));
        assert!(text.contains("Its teams:\n- Kestrel — lead Atlas\n"));
        assert!(
            text.contains("  - Security — lead Sentinel (your team): Keep our systems patched.")
        );
        assert!(text.contains("  - Finance — lead Ledger: Pays the bills."));
        assert!(text.contains(&format!(
            "`{ORG_LOOKUP_TOOL}` finds any team, bot or person of Kestrel"
        )));
        assert!(!text.contains("## IN THIS CONVERSATION"));
    }

    #[test]
    fn a_top_level_bot_reads_the_org_purpose_once() {
        let mut brief = security_brief(true);
        brief.team_id = "kestrel".into();
        brief.team_name = "Kestrel".into();
        brief.purpose = Some("Kestrel builds drones for farms".into());
        brief.parent_name = None;
        let text = team_brief_text(&brief);
        assert!(text.contains(
            "You are Sentinel, Mara's bot, in Kestrel, the organization's top-level team."
        ));
        assert!(!text.contains("Kestrel's purpose"));
        assert_eq!(text.matches("Kestrel builds drones for farms").count(), 1);
        assert!(text.contains("- Kestrel — lead Atlas (your team)"));
    }

    #[test]
    fn a_large_org_names_the_teams_near_the_bot_and_counts_the_rest() {
        let mut org = vec![brief_team("root", None, 0, "Atlas", 1)];
        org.extend((0..30).map(|i| brief_team(&format!("dept{i:02}"), Some("root"), 1, "Lead", 2)));
        org.push(brief_team("ops", Some("dept29"), 2, "Opsy", 1));
        org.push(brief_team("security", Some("dept29"), 2, "Sentinel", 3));
        org.push(brief_team("soc", Some("security"), 3, "Watch", 1));
        let mut brief = security_brief(false);
        brief.org = org;
        let text = team_brief_text(&brief);
        let listed = text
            .lines()
            .filter(|line| line.trim_start().starts_with("- ") && line.contains(" — lead "))
            .count();
        assert!(text.contains("Teams near yours:"));
        assert_eq!(listed, MAX_BRIEF_TEAMS);
        assert!(text.contains("- Root — lead Atlas"));
        assert!(text.contains("  - Dept29 — lead Lead"));
        assert!(text.contains("    - Security — lead Sentinel (your team)"));
        assert!(text.contains("      - Soc — lead Watch"));
        assert!(text.contains("    - Ops — lead Opsy"));
        assert!(text.contains(&format!("…and {} more teams.", 34 - MAX_BRIEF_TEAMS)));
        assert!(text.contains("an organization of 34 teams and 66 bots"));
    }

    #[test]
    fn the_brief_names_the_people_in_the_conversation() {
        use DataClassification::*;
        let mut brief = security_brief(false);
        brief.present = vec![
            person(
                "Anna",
                false,
                &[("Finance", TeamRole::Admin), ("Security", TeamRole::Member)],
                Confidential,
            ),
            person("Jonas", true, &[("Security", TeamRole::Member)], Internal),
        ];
        let text = team_brief_text(&brief);
        assert!(text.contains("## IN THIS CONVERSATION"));
        assert!(
            text.contains("- Anna: Finance (admin), Security (member); classified confidential")
        );
        assert!(text.contains("- Jonas (your owner): Security (member); classified internal"));
        assert!(text.contains("nothing classified above internal belongs in it"));
        assert!(text.find("## IN THIS CONVERSATION") < text.find("## OTHER TEAMS"));

        brief.present.remove(0);
        assert!(!team_brief_text(&brief).contains("## IN THIS CONVERSATION"));
    }

    #[test]
    fn lead_brief_names_reports_and_room_duty() {
        let text = team_brief_text(&security_brief(true));
        assert!(text.contains("You lead Security. Patch and Audit report to you"));
        assert!(text.contains("You report to Atlas, the lead above your team."));
        assert!(text.contains("Messages in #Security reach you first"));
        assert!(!text.contains("Audit reports"));
    }

    #[test]
    fn approval_line_follows_direct_asks() {
        let mut brief = security_brief(false);
        brief.direct_asks = false;
        let text = team_brief_text(&brief);
        assert!(text.contains("your owner approves your first question to each"));
    }

    fn kestrel() -> OrgDirectory {
        let team =
            |id: &str, name: &str, parent: Option<&str>, purpose: &str, lead: &str| DirectoryTeam {
                id: id.into(),
                name: name.into(),
                parent_id: parent.map(str::to_string),
                purpose: Some(purpose.into()),
                lead_bot_id: Some(lead.into()),
            };
        let bot = |id: &str, name: &str, owner: &str, team: &str, description: &str| DirectoryBot {
            id: id.into(),
            name: name.into(),
            owner_name: owner.into(),
            description: Some(description.into()),
            team_id: team.into(),
        };
        let person = |name: &str, teams: &[(&str, TeamRole)]| DirectoryPerson {
            name: name.into(),
            teams: teams
                .iter()
                .map(|(id, role)| ((*id).to_string(), *role))
                .collect(),
        };
        OrgDirectory {
            teams: vec![
                team(
                    "kestrel",
                    "Kestrel",
                    None,
                    "Kestrel builds drones for farms",
                    "atlas",
                ),
                team(
                    "security",
                    "Security",
                    Some("kestrel"),
                    "Keep our systems patched",
                    "sentinel",
                ),
                team(
                    "it",
                    "IT Support",
                    Some("kestrel"),
                    "Laptops, accounts and the VPN",
                    "helpy",
                ),
                team(
                    "finance",
                    "Finance",
                    Some("kestrel"),
                    "Payroll, invoices and the budget",
                    "ledger",
                ),
            ],
            bots: vec![
                bot("atlas", "Atlas", "Mara", "kestrel", "Chief of staff."),
                bot(
                    "sentinel",
                    "Sentinel",
                    "Mara",
                    "security",
                    "Triages security alerts.",
                ),
                bot("helpy", "Helpy", "Jonas", "it", "Resets passwords."),
                bot(
                    "ledger",
                    "Ledger",
                    "Tomás",
                    "finance",
                    "Runs payroll every month.",
                ),
                bot(
                    "forecast",
                    "Forecast",
                    "Tomás",
                    "finance",
                    "Builds the cash forecast.",
                ),
            ],
            people: vec![
                person(
                    "Mara Kessler",
                    &[("kestrel", TeamRole::Admin), ("security", TeamRole::Admin)],
                ),
                person("Anna Weber", &[("finance", TeamRole::Admin)]),
                person("Jonas", &[("it", TeamRole::Member)]),
            ],
        }
    }

    #[test]
    fn a_lookup_by_tag_finds_the_team_with_its_members() {
        let found = org_lookup(&kestrel(), "#ITSupport");
        assert_eq!(found.org, "Kestrel");
        let team = &found.teams[0];
        assert_eq!(team.name, "IT Support");
        assert_eq!(team.tag, "#ITSupport");
        assert_eq!(team.part_of.as_deref(), Some("Kestrel"));
        assert_eq!(team.lead.as_deref(), Some("Helpy"));
        assert_eq!(team.bots, ["Helpy, Jonas's bot"]);
        assert_eq!(team.people, ["Jonas (member)"]);
    }

    #[test]
    fn a_lookup_searches_purposes_jobs_and_people() {
        let directory = kestrel();
        let payroll = org_lookup(&directory, "payroll");
        assert_eq!(payroll.teams[0].name, "Finance");
        assert_eq!(payroll.teams[0].people, ["Anna Weber (admin)"]);
        let ledger = &payroll.bots[0];
        assert_eq!(
            (
                ledger.name.as_str(),
                ledger.team.as_str(),
                ledger.leads_team
            ),
            ("Ledger", "Finance", true)
        );

        let anna = org_lookup(&directory, "anna");
        assert!(anna.teams.is_empty() && anna.bots.is_empty());
        assert_eq!(anna.people[0].teams, ["Finance (admin)"]);

        let top = org_lookup(&directory, "Kestrel");
        assert_eq!(
            top.teams[0].teams_inside,
            ["Security", "IT Support", "Finance"]
        );
        assert_eq!(top.teams[0].people, ["Mara Kessler (admin)"]);
    }

    #[test]
    fn an_empty_lookup_lists_every_team_without_members() {
        let found = org_lookup(&kestrel(), "  ");
        assert_eq!(found.teams.len(), 4);
        assert!(
            found
                .teams
                .iter()
                .all(|team| team.bots.is_empty() && team.people.is_empty())
        );
        assert!(found.note.unwrap().starts_with("Every team of Kestrel"));
    }

    #[test]
    fn a_lookup_says_when_nothing_matches_and_never_carries_a_classification() {
        let found = org_lookup(&kestrel(), "zebra");
        assert!(found.teams.is_empty() && found.bots.is_empty() && found.people.is_empty());
        assert!(
            found
                .note
                .unwrap()
                .contains("Nothing in Kestrel matches \"zebra\"")
        );
        let json = serde_json::to_string(&org_lookup(&kestrel(), "Kestrel")).unwrap();
        assert!(!json.contains("classif"));
    }

    #[test]
    fn a_lookup_caps_each_kind_and_says_how_many_it_left_out() {
        let mut directory = kestrel();
        directory.bots = (0..MAX_LOOKUP_BOTS + 3)
            .map(|i| DirectoryBot {
                id: format!("b{i}"),
                name: format!("Scout {i}"),
                owner_name: "Mara".into(),
                team_id: "security".into(),
                ..DirectoryBot::default()
            })
            .collect();
        let found = org_lookup(&directory, "scout");
        assert_eq!(found.bots.len(), MAX_LOOKUP_BOTS);
        assert_eq!(
            found.note.as_deref(),
            Some("3 more matches were left out; narrow the query.")
        );
    }

    #[test]
    fn enums_round_trip_their_database_form() {
        assert_eq!(
            TeamRole::parse(TeamRole::Admin.as_str()),
            Some(TeamRole::Admin)
        );
        assert_eq!(
            TeamMemberStatus::parse("active"),
            Some(TeamMemberStatus::Active)
        );
        assert_eq!(
            serde_json::to_value(TeamInvitationKind::DirectAsks).unwrap(),
            serde_json::json!("direct_asks")
        );
    }

    #[test]
    fn a_message_addresses_the_teams_it_starts_with() {
        assert_eq!(
            leading_team_tags("#Security #ITSupport, is the VPN down?"),
            ["Security", "ITSupport"]
        );
        assert_eq!(
            leading_team_tags("  #People: who approves leave?"),
            ["People"]
        );
        assert_eq!(leading_team_tags("#security #Security hi"), ["security"]);
        assert_eq!(leading_team_tags("#A #B #C #D hi"), ["A", "B", "C"]);
        assert!(leading_team_tags("I asked #Security yesterday").is_empty());
        assert!(leading_team_tags("# Notes\nnothing here").is_empty());
        assert!(leading_team_tags("## Heading").is_empty());
        assert!(leading_team_tags("").is_empty());
    }

    #[test]
    fn people_talk_only_with_bots_at_or_below_their_level() {
        use DataClassification::*;
        assert!(person_may_talk(None, None));
        assert!(person_may_talk(None, Some(Internal)));
        assert!(!person_may_talk(None, Some(Confidential)));
        assert!(person_may_talk(Some(Confidential), Some(Confidential)));
        assert!(person_may_talk(Some(Restricted), None));
        assert!(!person_may_talk(Some(Public), None));
        assert_eq!(
            BotChatStatus::parse(BotChatStatus::Declined.as_str()),
            Some(BotChatStatus::Declined)
        );
        assert_eq!(
            serde_json::to_value(TalkReach::TeamRoom).unwrap(),
            serde_json::json!("team_room")
        );
    }

    #[test]
    fn a_tag_names_a_team_without_spaces_or_case() {
        assert!(tag_names_team("ITSupport", "IT Support"));
        assert!(tag_names_team("itsupport", "IT  Support"));
        assert!(tag_names_team("R&D", "R&D"));
        assert!(!tag_names_team("IT", "IT Support"));
    }
}
