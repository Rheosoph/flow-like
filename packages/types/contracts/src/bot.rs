//! Bots: the capability, location and built-in bot ids every Flow-Like process shares, and
//! the model policy hosts resolve. The ids are the wire format.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Something a bot may do. Every capability is off until the owner switches it on.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BotCapability {
    /// Researches the public web: search and read pages through a sealed researcher.
    Internet,
    /// Calls web APIs and fetches URLs (feeds, JSON, pages) itself, asking the owner before each
    /// new site.
    WebRequests,
    /// Every app in one of the owner's profiles; the bot names the profile.
    ProfileApps,
    /// The apps the owner attached to the bot.
    SelectedApps,
    /// Run code.
    CodeExecution,
    /// Operate a desktop. Needs a desktop or device host.
    ComputerUse,
    /// Drive a browser. Needs a desktop or device host.
    BrowserUse,
    /// A flow-like.me mail address.
    Email,
    /// Push notifications to the owner.
    PushNotifications,
    /// One-time and recurring runs.
    Scheduling,
    /// Files, stored like app storage.
    Storage,
    /// Tables, stored like app databases.
    DataStudio,
    /// Remembers across chats: recall and the memory tools. FlowPilot bots always have it.
    Memory,
    /// Reads, summarizes and extracts files; creates Word, Excel, PowerPoint and PDF.
    Documents,
    /// Connects to external databases like Postgres, MySQL or ODBC.
    DataConnections,
    /// Builds searchable indexes over files.
    Knowledge,
    /// Uses the owner's connected services and MCP servers.
    Connectors,
    /// Talks with people on Slack, Teams, Telegram, Discord or email.
    Channels,
    /// Follows and runs installed skills.
    Skills,
    /// Creates images with the bot's image model.
    ImageGeneration,
    /// Splits work into parallel sub-tasks.
    Delegation,
    /// A shell. Needs a desktop or device host.
    CommandLine,
    /// End-to-end encryption.
    Encryption,
    /// A webhook that wakes the bot.
    Webhook,
    /// Requests current context from a person who chooses what to share in this conversation.
    UserContext,
    /// FlowPilot's complete toolset: builds, edits and debugs apps and flows, FlowScript
    /// included. Flux holds it, and so do duplicates of a bot that holds it; no other bot gets it.
    Flowpilot,
}

impl BotCapability {
    /// Every capability, in the order the settings list them.
    pub const ALL: [Self; 26] = [
        Self::Internet,
        Self::WebRequests,
        Self::ProfileApps,
        Self::SelectedApps,
        Self::CodeExecution,
        Self::ComputerUse,
        Self::BrowserUse,
        Self::Email,
        Self::PushNotifications,
        Self::Scheduling,
        Self::Storage,
        Self::DataStudio,
        Self::Memory,
        Self::Documents,
        Self::DataConnections,
        Self::Knowledge,
        Self::Connectors,
        Self::Channels,
        Self::Skills,
        Self::ImageGeneration,
        Self::Delegation,
        Self::CommandLine,
        Self::Encryption,
        Self::Webhook,
        Self::UserContext,
        Self::Flowpilot,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Internet => "internet",
            Self::WebRequests => "web_requests",
            Self::ProfileApps => "profile_apps",
            Self::SelectedApps => "selected_apps",
            Self::CodeExecution => "code_execution",
            Self::ComputerUse => "computer_use",
            Self::BrowserUse => "browser_use",
            Self::Email => "email",
            Self::PushNotifications => "push_notifications",
            Self::Scheduling => "scheduling",
            Self::Storage => "storage",
            Self::DataStudio => "data_studio",
            Self::Memory => "memory",
            Self::Documents => "documents",
            Self::DataConnections => "data_connections",
            Self::Knowledge => "knowledge",
            Self::Connectors => "connectors",
            Self::Channels => "channels",
            Self::Skills => "skills",
            Self::ImageGeneration => "image_generation",
            Self::Delegation => "delegation",
            Self::CommandLine => "command_line",
            Self::Encryption => "encryption",
            Self::Webhook => "webhook",
            Self::UserContext => "user_context",
            Self::Flowpilot => "flowpilot",
        }
    }

    /// The capability with id `value`, ignoring case and surrounding whitespace.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|capability| capability.as_str().eq_ignore_ascii_case(value))
    }

    /// Needs a screen, a shell or a private key only its host holds, so a bot on the server
    /// can't hold it: the server must never read end-to-end encrypted content.
    pub fn requires_local_host(self) -> bool {
        matches!(
            self,
            Self::ComputerUse | Self::BrowserUse | Self::CommandLine | Self::Encryption
        )
    }

    /// Can't be switched on or off: a bot gets it only by being Flux or a duplicate of a bot that
    /// holds it, and a bot that holds it keeps the capabilities it has.
    pub fn is_fixed(self) -> bool {
        matches!(self, Self::Flowpilot)
    }
}

/// Where a bot's host runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BotLocation {
    /// A hosted worker.
    #[default]
    Server,
    /// The owner's desktop client.
    Desktop,
    /// A managed device the owner registered.
    Device,
}

impl BotLocation {
    pub const ALL: [Self; 3] = [Self::Server, Self::Desktop, Self::Device];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Desktop => "desktop",
            Self::Device => "device",
        }
    }

    /// The location named `value`, ignoring case and surrounding whitespace.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|location| location.as_str().eq_ignore_ascii_case(value))
    }

    /// Whether a bot running here may hold `capability`.
    pub fn allows(self, capability: BotCapability) -> bool {
        self != Self::Server || !capability.requires_local_host()
    }

    /// Whether the owner's desktop answers the bot's turns while it is online: a server bot that
    /// holds `flowpilot`, whose build tools only a desktop runs without a window.
    pub fn hands_off_to_desktop(self, capabilities: &[BotCapability]) -> bool {
        self == Self::Server && capabilities.contains(&BotCapability::Flowpilot)
    }
}

/// The built-in bots every person has and can't delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BotSystemKind {
    /// The general bot, "Assistant".
    General,
    /// Flux, the former FlowPilot, with every FlowPilot tool.
    Flux,
}

impl BotSystemKind {
    pub const ALL: [Self; 2] = [Self::General, Self::Flux];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Flux => "flux",
        }
    }

    /// The built-in bot with key `value`, ignoring case and surrounding whitespace.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(value))
    }

    /// Flux keeps every FlowPilot tool, so its capabilities can't be changed.
    pub fn capabilities_fixed(self) -> bool {
        matches!(self, Self::Flux)
    }

    /// What the built-in bot holds whatever its stored row says: Flux holds `flowpilot`.
    pub fn fixed_capabilities(self) -> &'static [BotCapability] {
        match self {
            Self::General => &[],
            Self::Flux => &[BotCapability::Flowpilot],
        }
    }
}

/// The model a bot runs on. Hosts that can't run the primary try the fallbacks in order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BotModelPolicy {
    /// Bit id of the preferred model; unset lets the host choose.
    pub primary: Option<String>,
    /// Bit ids to try, in order, where the primary is unavailable.
    pub fallbacks: Vec<String>,
    /// System One Bit for response decisions; absent disables decision routing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn capability_ids_match_the_wire_format() {
        let ids: Vec<&str> = BotCapability::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            ids,
            [
                "internet",
                "web_requests",
                "profile_apps",
                "selected_apps",
                "code_execution",
                "computer_use",
                "browser_use",
                "email",
                "push_notifications",
                "scheduling",
                "storage",
                "data_studio",
                "memory",
                "documents",
                "data_connections",
                "knowledge",
                "connectors",
                "channels",
                "skills",
                "image_generation",
                "delegation",
                "command_line",
                "encryption",
                "webhook",
                "user_context",
                "flowpilot",
            ]
        );
        for capability in BotCapability::ALL {
            let wire = serde_json::to_value(capability).unwrap();
            assert_eq!(wire, Value::String(capability.as_str().to_string()));
            assert_eq!(
                serde_json::from_value::<BotCapability>(wire).unwrap(),
                capability
            );
        }
    }

    #[test]
    fn parses_capability_ids_case_insensitively() {
        assert_eq!(
            BotCapability::parse(" Code_Execution "),
            Some(BotCapability::CodeExecution)
        );
        assert_eq!(
            BotCapability::parse("webhook"),
            Some(BotCapability::Webhook)
        );
        assert_eq!(
            BotCapability::parse("FlowPilot"),
            Some(BotCapability::Flowpilot)
        );
        assert_eq!(
            BotCapability::parse(" Memory "),
            Some(BotCapability::Memory)
        );
        assert_eq!(BotCapability::parse("telepathy"), None);
        assert_eq!(BotCapability::parse(""), None);
        assert!(serde_json::from_value::<BotCapability>(json!("telepathy")).is_err());
    }

    #[test]
    fn only_screen_shell_and_key_capabilities_need_a_local_host() {
        let local: Vec<BotCapability> = BotCapability::ALL
            .into_iter()
            .filter(|capability| capability.requires_local_host())
            .collect();
        assert_eq!(
            local,
            [
                BotCapability::ComputerUse,
                BotCapability::BrowserUse,
                BotCapability::CommandLine,
                BotCapability::Encryption
            ]
        );
        assert!(!BotLocation::Server.allows(BotCapability::CommandLine));
        assert!(!BotLocation::Server.allows(BotCapability::Encryption));
        assert!(BotLocation::Desktop.allows(BotCapability::Encryption));
        assert!(BotLocation::Device.allows(BotCapability::Encryption));
        assert!(BotLocation::Server.allows(BotCapability::Internet));
        assert!(BotLocation::Desktop.allows(BotCapability::ComputerUse));
        assert!(BotLocation::Device.allows(BotCapability::BrowserUse));
    }

    #[test]
    fn locations_round_trip_and_read_stored_names() {
        for location in BotLocation::ALL {
            let wire = serde_json::to_value(location).unwrap();
            assert_eq!(wire, Value::String(location.as_str().to_string()));
            assert_eq!(
                serde_json::from_value::<BotLocation>(wire).unwrap(),
                location
            );
        }
        assert_eq!(BotLocation::parse("DEVICE"), Some(BotLocation::Device));
        assert_eq!(BotLocation::parse("cloud"), None);
        assert_eq!(BotLocation::default(), BotLocation::Server);
    }

    #[test]
    fn system_kinds_round_trip_and_only_flux_is_fixed() {
        for kind in BotSystemKind::ALL {
            let wire = serde_json::to_value(kind).unwrap();
            assert_eq!(wire, Value::String(kind.as_str().to_string()));
            assert_eq!(serde_json::from_value::<BotSystemKind>(wire).unwrap(), kind);
        }
        assert_eq!(BotSystemKind::parse("Flux"), Some(BotSystemKind::Flux));
        assert_eq!(BotSystemKind::parse("custom"), None);
        assert!(BotSystemKind::Flux.capabilities_fixed());
        assert!(!BotSystemKind::General.capabilities_fixed());
    }

    #[test]
    fn only_flowpilot_is_fixed_and_flux_always_holds_it() {
        let fixed: Vec<BotCapability> = BotCapability::ALL
            .into_iter()
            .filter(|capability| capability.is_fixed())
            .collect();
        assert_eq!(fixed, [BotCapability::Flowpilot]);
        assert_eq!(
            BotSystemKind::Flux.fixed_capabilities(),
            [BotCapability::Flowpilot]
        );
        assert!(BotSystemKind::General.fixed_capabilities().is_empty());
        assert!(!BotCapability::Flowpilot.requires_local_host());
        for location in BotLocation::ALL {
            assert!(location.allows(BotCapability::Flowpilot), "{location:?}");
        }
        assert_eq!(
            serde_json::from_value::<BotCapability>(json!("flowpilot")).unwrap(),
            BotCapability::Flowpilot
        );
    }

    #[test]
    fn memory_is_switchable_everywhere() {
        assert!(!BotCapability::Memory.is_fixed());
        assert!(!BotCapability::Memory.requires_local_host());
        for location in BotLocation::ALL {
            assert!(location.allows(BotCapability::Memory), "{location:?}");
        }
        assert_eq!(
            serde_json::to_value(BotCapability::Memory).unwrap(),
            json!("memory")
        );
    }

    #[test]
    fn tool_capabilities_are_switchable_everywhere_and_round_trip() {
        let added = [
            (BotCapability::Documents, "documents"),
            (BotCapability::DataConnections, "data_connections"),
            (BotCapability::Knowledge, "knowledge"),
            (BotCapability::Connectors, "connectors"),
            (BotCapability::Channels, "channels"),
            (BotCapability::Skills, "skills"),
            (BotCapability::ImageGeneration, "image_generation"),
            (BotCapability::Delegation, "delegation"),
            (BotCapability::WebRequests, "web_requests"),
        ];
        for (capability, id) in added {
            assert!(!capability.is_fixed(), "{id}");
            assert!(!capability.requires_local_host(), "{id}");
            for location in BotLocation::ALL {
                assert!(location.allows(capability), "{id} {location:?}");
            }
            assert_eq!(serde_json::to_value(capability).unwrap(), json!(id));
            assert_eq!(
                serde_json::from_value::<BotCapability>(json!(id)).unwrap(),
                capability
            );
            assert_eq!(BotCapability::parse(&id.to_uppercase()), Some(capability));
        }
    }

    #[test]
    fn model_policy_reads_the_column_default_and_writes_explicit_nulls() {
        assert_eq!(
            serde_json::from_value::<BotModelPolicy>(json!({})).unwrap(),
            BotModelPolicy::default()
        );
        assert_eq!(
            serde_json::to_value(BotModelPolicy::default()).unwrap(),
            json!({ "primary": null, "fallbacks": [] })
        );
        let policy = BotModelPolicy {
            decision: None,
            primary: Some("model-a".into()),
            fallbacks: vec!["model-b".into(), "model-c".into()],
        };
        let wire = serde_json::to_value(&policy).unwrap();
        assert_eq!(
            wire,
            json!({ "primary": "model-a", "fallbacks": ["model-b", "model-c"] })
        );
        assert_eq!(
            serde_json::from_value::<BotModelPolicy>(wire).unwrap(),
            policy
        );
        let enabled = BotModelPolicy {
            decision: Some("decision-bit".into()),
            ..policy
        };
        let wire = serde_json::to_value(&enabled).unwrap();
        assert_eq!(wire["decision"], "decision-bit");
        assert_eq!(
            serde_json::from_value::<BotModelPolicy>(wire).unwrap(),
            enabled
        );
    }
}
