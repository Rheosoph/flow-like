//! Bot teams: people and bots in nested teams with one lead bot per team. The teams under one
//! top-level team form an org; every active member of any of its teams sees the whole org, bots
//! with their job descriptions included. The API stores teams, the bot hosts hand a bot in a team
//! its [`TeamBrief`], and [`team_brief_text`] writes what that bot reads before every turn, so the
//! prompt and the "what it reads" view in the app never differ.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
/// Bots the organization section of a brief names before it summarizes the rest.
pub const MAX_BRIEF_ORG_BOTS: usize = 40;
const DESCRIPTION_CHARS: usize = 140;

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
}

/// One answer the reader owes a team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TeamInvitation {
    pub kind: TeamInvitationKind,
    pub team_id: String,
    pub team_name: String,
    /// The name of the team's top-level team.
    pub org_name: String,
    /// The reader's bot, for `bot` and `direct_asks`.
    #[serde(default)]
    pub bot_id: Option<String>,
    #[serde(default)]
    pub invited_by: Option<String>,
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
    /// Invited and not yet accepted by its owner.
    #[serde(default)]
    pub joining: bool,
}

/// A team as a brief's organization section lists it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct BriefTeam {
    pub id: String,
    pub name: String,
    /// 0 for the top-level team.
    pub depth: u32,
    /// The lead's name.
    #[serde(default)]
    pub lead: Option<String>,
    /// The team's active bots other than its lead.
    #[serde(default)]
    pub members: Vec<BriefBot>,
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
    /// The other bots of its team, joining ones included.
    #[serde(default)]
    pub teammates: Vec<BriefBot>,
    /// Teammates are asked without the first-time Allow card.
    pub direct_asks: bool,
    /// The team room's name, `#Security`.
    #[serde(default)]
    pub room: Option<String>,
    /// The whole org, depth first.
    #[serde(default)]
    pub org: Vec<BriefTeam>,
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
    let joining = if bot.joining {
        format!(" Joins once {} accepts.", bot.owner_name)
    } else {
        String::new()
    };
    format!(
        "- {}{role}, {}'s bot{description}{joining}",
        bot.name, bot.owner_name
    )
}

/// What a bot in a team reads before every turn: its team, its teammates, the org and how to
/// reach other teams.
pub fn team_brief_text(brief: &TeamBrief) -> String {
    let mut out = Vec::new();
    let mut team = vec!["## YOUR TEAM".to_string()];
    let place = brief
        .parent_name
        .as_deref()
        .map(|parent| format!(", part of {parent}"))
        .unwrap_or_default();
    team.push(format!(
        "You are {}, {}'s bot, in the team {}{place}.",
        brief.bot_name, brief.owner_name, brief.team_name
    ));
    if let Some(purpose) = brief
        .purpose
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        team.push(format!(
            "{}'s purpose: {}",
            brief.team_name,
            sentence(purpose)
        ));
    }
    if brief.is_lead {
        let reports: Vec<BriefBot> = brief
            .teammates
            .iter()
            .filter(|bot| !bot.joining)
            .cloned()
            .collect();
        if reports.is_empty() {
            team.push(format!("You lead {}.", brief.team_name));
        } else {
            team.push(format!(
                "You lead {}. {} {} to you: hand out work inside the team and answer for it.",
                brief.team_name,
                names(&reports),
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

    if !brief.org.is_empty() {
        let mut lines = vec![
            "## THE ORGANIZATION".to_string(),
            "Every team, its lead and its bots:".to_string(),
        ];
        let mut named = 0usize;
        let mut skipped = 0usize;
        for team in &brief.org {
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
            lines.push(format!("{indent}- {}{lead}{here}", team.name));
            for member in &team.members {
                if named >= MAX_BRIEF_ORG_BOTS {
                    skipped += 1;
                    continue;
                }
                named += 1;
                let description = first_line(member.description.as_deref())
                    .map(|line| format!(" — {line}"))
                    .unwrap_or_default();
                lines.push(format!(
                    "{indent}  - {}, {}'s bot{description}",
                    member.name, member.owner_name
                ));
            }
        }
        if skipped > 0 {
            lines.push(format!("…and {skipped} more bots."));
        }
        out.push(lines.join("\n"));
    }

    out.push(format!(
        "## OTHER TEAMS\nFor work another team covers, ask that team's lead; the lead hands it on. Your first question to a bot outside {} needs {}'s OK, and a bot classified above you never answers you.",
        brief.team_name, brief.owner_name
    ));
    out.join("\n\n")
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
            joining: false,
        }
    }

    fn security_brief(is_lead: bool) -> TeamBrief {
        let sentinel = bot("sentinel", "Sentinel", "Mara", "Triages security alerts.");
        let patch = bot("patch", "Patch", "Jonas", "Keeps laptops patched.");
        let mut audit = bot("audit", "Audit", "Jonas", "Reviews access logs.");
        audit.joining = true;
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
                BriefTeam {
                    id: "kestrel".into(),
                    name: "Kestrel".into(),
                    depth: 0,
                    lead: Some("Atlas".into()),
                    members: Vec::new(),
                },
                BriefTeam {
                    id: "security".into(),
                    name: "Security".into(),
                    depth: 1,
                    lead: Some("Sentinel".into()),
                    members: vec![patch],
                },
                BriefTeam {
                    id: "finance".into(),
                    name: "Finance".into(),
                    depth: 1,
                    lead: Some("Ledger".into()),
                    members: vec![bot(
                        "forecast",
                        "Forecast",
                        "Tomás",
                        "Builds the cash forecast",
                    )],
                },
            ],
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
        assert!(text.contains("You report to Sentinel, the team's lead."));
        assert!(text.contains("In #Security, answer when someone names you"));
        assert!(text.contains("Ask them directly with ask_bot"));
        assert!(text.contains("- Sentinel (lead), Mara's bot — Triages security alerts."));
        assert!(
            text.contains("- Audit, Jonas's bot — Reviews access logs. Joins once Jonas accepts.")
        );
        assert!(text.contains("  - Finance — lead Ledger"));
        assert!(text.contains("    - Forecast, Tomás's bot — Builds the cash forecast."));
        assert!(text.contains("- Security — lead Sentinel (your team)"));
    }

    #[test]
    fn lead_brief_names_reports_and_room_duty() {
        let text = team_brief_text(&security_brief(true));
        assert!(text.contains("You lead Security. Patch reports to you"));
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

    #[test]
    fn org_listing_is_capped() {
        let mut brief = security_brief(false);
        brief.org[2].members = (0..MAX_BRIEF_ORG_BOTS + 5)
            .map(|i| bot(&format!("b{i}"), &format!("Bot{i}"), "Tomás", "Works."))
            .collect();
        let text = team_brief_text(&brief);
        assert!(text.contains("more bots."));
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
    fn a_tag_names_a_team_without_spaces_or_case() {
        assert!(tag_names_team("ITSupport", "IT Support"));
        assert!(tag_names_team("itsupport", "IT  Support"));
        assert!(tag_names_team("R&D", "R&D"));
        assert!(!tag_names_team("IT", "IT Support"));
    }
}
