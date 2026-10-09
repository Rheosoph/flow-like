//! What a device does with an event, decided in one place. Every list of event types in the
//! agent asks this module; the client (`eventKind` in `lib/device-management/deployment.ts`)
//! and the hub (`DEVICE_EVENT_TYPES`) apply the same table. It names no runtime type, so the
//! agent parent, which is built without the runtime, can read it.

use serde::Serialize;

/// Whether this build runs `api` events, and so the flag `api_events`.
pub const API_EVENTS: bool = cfg!(all(feature = "runtime", feature = "api-events"));
/// Whether this build runs one-time schedules, and so the flag `scheduled_once`.
pub const SCHEDULED_ONCE: bool = cfg!(all(feature = "runtime", feature = "scheduled-once"));
/// Whether this build runs quick actions and forms, and so the flag `on_demand_events`.
pub const ON_DEMAND_EVENTS: bool = cfg!(all(feature = "runtime", feature = "on-demand"));
/// Whether this build runs Telegram bots, and so the flag `telegram_bots`.
pub const TELEGRAM_BOTS: bool = cfg!(all(feature = "runtime", feature = "bots-telegram"));
/// Whether this build runs Discord bots, and so the flag `discord_bots`.
pub const DISCORD_BOTS: bool = cfg!(all(feature = "runtime", feature = "bots-discord"));

/// The first matching row wins: an event with a default Page is served as a Page whatever
/// its type, so every other row means "and no default Page".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A Page, chat or route of the service's own listener.
    Served,
    /// A REST or MCP server the flow binds itself.
    OwnServer,
    /// A daemon that reports its own readiness.
    Background,
    /// A repeating or one-time schedule.
    Scheduled,
    /// A quick action or form that a person starts.
    OnDemand,
    /// A Telegram or Discord bot.
    Bot,
}

impl EventKind {
    /// The kind of an event a device may be asked to run, whatever this build carries.
    pub fn of(event_type: &str, has_page: bool) -> Option<Self> {
        if has_page {
            return Some(Self::Served);
        }
        Some(match event_type {
            "http" | "api" | "simple_chat" => Self::Served,
            "rest" | "mcp" => Self::OwnServer,
            "daemon" => Self::Background,
            "cron" => Self::Scheduled,
            "quick_action" | "generic_form" => Self::OnDemand,
            "telegram" | "discord" => Self::Bot,
            _ => return None,
        })
    }

    /// The kind of an event this build runs: `None` also when the part that runs its type
    /// was left out of the build.
    pub fn runnable(event_type: &str, has_page: bool) -> Option<Self> {
        Self::of(event_type, has_page).filter(|_| has_page || built_in(event_type))
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Served => "served",
            Self::OwnServer => "own_server",
            Self::Background => "background",
            Self::Scheduled => "scheduled",
            Self::OnDemand => "on_demand",
            Self::Bot => "bot",
        }
    }

    /// Whether the service's own listener serves it.
    pub fn hosted(self) -> bool {
        self == Self::Served
    }

    /// Whether a service that holds it runs one instance.
    pub fn limits_instances(self) -> bool {
        matches!(
            self,
            Self::OwnServer | Self::Background | Self::Scheduled | Self::Bot
        )
    }

    /// Whether it fires by itself and so runs in one place only: the hub's claim decides,
    /// and one service holds at most 64 of them.
    pub fn self_firing(self) -> bool {
        matches!(self, Self::Scheduled | Self::Bot)
    }

    /// What the supervisor waits for, as discovery names it.
    pub fn readiness_kind(self) -> &'static str {
        match self {
            Self::Served | Self::OwnServer => "listener",
            Self::Background | Self::Scheduled | Self::OnDemand | Self::Bot => "explicit",
        }
    }
}

/// The node through which a service of its own process says that it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnReadiness {
    RestServer,
    McpServer,
    ServiceReady,
}

impl OwnReadiness {
    pub fn of(event_type: &str) -> Option<Self> {
        match event_type {
            "rest" => Some(Self::RestServer),
            "mcp" => Some(Self::McpServer),
            "daemon" => Some(Self::ServiceReady),
            _ => None,
        }
    }

    pub fn node_name(self) -> &'static str {
        match self {
            Self::RestServer => "rest_server",
            Self::McpServer => "mcp_server",
            Self::ServiceReady => "service_ready",
        }
    }
}

/// The types whose route comes from their config: `http` and `api`.
pub fn serves_requests(event_type: &str) -> bool {
    matches!(event_type, "http" | "api")
}

/// The `secret_overrides` key that names the token of a bot event.
pub fn bot_token_key(event_id: &str) -> String {
    format!("event.{event_id}.bot_token")
}

/// The event a bot token key names. A key of this form is never applied to a flow variable.
pub fn bot_token_event(key: &str) -> Option<&str> {
    key.strip_prefix("event.")?
        .strip_suffix(".bot_token")
        .filter(|event_id| !event_id.is_empty())
}

/// Whether this build carries the part that runs an event of this type without a Page.
pub fn built_in(event_type: &str) -> bool {
    match event_type {
        "http" | "simple_chat" | "rest" | "mcp" | "daemon" | "cron" => true,
        "api" => API_EVENTS,
        "quick_action" | "generic_form" => ON_DEMAND_EVENTS,
        "telegram" => TELEGRAM_BOTS,
        "discord" => DISCORD_BOTS,
        _ => false,
    }
}

/// The agent flags of the parts this build carries, for its `features` map.
pub fn flags() -> impl Iterator<Item = &'static str> {
    [
        ("api_events", API_EVENTS),
        ("scheduled_once", SCHEDULED_ONCE),
        ("on_demand_events", ON_DEMAND_EVENTS),
        ("execution_logs", cfg!(feature = "runtime")),
        ("telegram_bots", TELEGRAM_BOTS),
        ("discord_bots", DISCORD_BOTS),
    ]
    .into_iter()
    .filter_map(|(flag, built)| built.then_some(flag))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TYPES: [(&str, EventKind); 11] = [
        ("http", EventKind::Served),
        ("api", EventKind::Served),
        ("simple_chat", EventKind::Served),
        ("rest", EventKind::OwnServer),
        ("mcp", EventKind::OwnServer),
        ("daemon", EventKind::Background),
        ("cron", EventKind::Scheduled),
        ("quick_action", EventKind::OnDemand),
        ("generic_form", EventKind::OnDemand),
        ("telegram", EventKind::Bot),
        ("discord", EventKind::Bot),
    ];
    const UNKNOWN: [&str; 6] = ["email", "teams", "webhook", "", "HTTP", "Cron"];

    #[test]
    fn every_type_has_its_kind_and_a_page_wins() {
        for (event_type, kind) in TYPES {
            assert_eq!(EventKind::of(event_type, false), Some(kind), "{event_type}");
        }
        for unknown in UNKNOWN {
            assert_eq!(EventKind::of(unknown, false), None, "{unknown}");
            assert_eq!(EventKind::runnable(unknown, false), None, "{unknown}");
            assert!(!built_in(unknown), "{unknown}");
        }
        for event_type in TYPES
            .map(|(event_type, _)| event_type)
            .into_iter()
            .chain(UNKNOWN)
        {
            assert_eq!(
                EventKind::of(event_type, true),
                Some(EventKind::Served),
                "{event_type}"
            );
            assert_eq!(
                EventKind::runnable(event_type, true),
                Some(EventKind::Served),
                "{event_type}"
            );
        }
    }

    #[test]
    fn wire_names_and_facts_follow_the_table() {
        let rows = [
            (EventKind::Served, "served", true, false, false, "listener"),
            (
                EventKind::OwnServer,
                "own_server",
                false,
                true,
                false,
                "listener",
            ),
            (
                EventKind::Background,
                "background",
                false,
                true,
                false,
                "explicit",
            ),
            (
                EventKind::Scheduled,
                "scheduled",
                false,
                true,
                true,
                "explicit",
            ),
            (
                EventKind::OnDemand,
                "on_demand",
                false,
                false,
                false,
                "explicit",
            ),
            (EventKind::Bot, "bot", false, true, true, "explicit"),
        ];
        for (kind, name, hosted, limits, self_firing, readiness) in rows {
            assert_eq!(kind.as_str(), name);
            assert_eq!(serde_json::to_value(kind).unwrap(), name);
            assert_eq!(kind.hosted(), hosted, "{name}");
            assert_eq!(kind.limits_instances(), limits, "{name}");
            assert_eq!(kind.self_firing(), self_firing, "{name}");
            assert_eq!(kind.readiness_kind(), readiness, "{name}");
        }
    }

    #[test]
    fn only_http_and_api_take_their_route_from_the_config() {
        for (event_type, _) in TYPES {
            assert_eq!(
                serves_requests(event_type),
                matches!(event_type, "http" | "api"),
                "{event_type}"
            );
        }
    }

    #[test]
    fn only_servers_and_daemons_have_a_readiness_node() {
        for (event_type, kind) in TYPES {
            let node = OwnReadiness::of(event_type).map(OwnReadiness::node_name);
            assert_eq!(
                node.is_some(),
                matches!(kind, EventKind::OwnServer | EventKind::Background),
                "{event_type}"
            );
        }
        assert_eq!(
            ["rest", "mcp", "daemon"]
                .map(|event_type| OwnReadiness::of(event_type).unwrap().node_name()),
            ["rest_server", "mcp_server", "service_ready"]
        );
    }

    #[test]
    fn a_bot_token_key_names_its_event() {
        assert_eq!(bot_token_key("evt_helper"), "event.evt_helper.bot_token");
        assert_eq!(
            bot_token_event("event.evt_helper.bot_token"),
            Some("evt_helper")
        );
        assert_eq!(bot_token_event("event.a.b.bot_token"), Some("a.b"));
        for other in [
            "event..bot_token",
            "event.evt_helper.bot_token.x",
            "events.evt_helper.bot_token",
            "Event.evt_helper.bot_token",
            "credential",
            "evt_helper.bot_token",
        ] {
            assert_eq!(bot_token_event(other), None, "{other}");
        }
    }

    #[test]
    fn round_one_types_are_built_into_every_build() {
        for base in ["http", "simple_chat", "rest", "mcp", "daemon", "cron"] {
            assert!(built_in(base), "{base}");
            assert_eq!(
                EventKind::runnable(base, false),
                EventKind::of(base, false),
                "{base}"
            );
        }
    }

    #[test]
    fn built_in_parts_and_flags_follow_the_cargo_features() {
        let built = |feature: bool| cfg!(feature = "runtime") && feature;
        let on_demand = built(cfg!(feature = "on-demand"));
        let parts = [
            ("api", "api_events", built(cfg!(feature = "api-events"))),
            ("quick_action", "on_demand_events", on_demand),
            ("generic_form", "on_demand_events", on_demand),
            (
                "telegram",
                "telegram_bots",
                built(cfg!(feature = "bots-telegram")),
            ),
            (
                "discord",
                "discord_bots",
                built(cfg!(feature = "bots-discord")),
            ),
        ];
        let flags: Vec<_> = flags().collect();
        for (event_type, flag, built) in parts {
            assert_eq!(built_in(event_type), built, "{event_type}");
            let runnable = EventKind::runnable(event_type, false).is_some();
            assert_eq!(runnable, built, "{event_type}");
            assert_eq!(flags.contains(&flag), built, "{flag}");
        }
        let once = built(cfg!(feature = "scheduled-once"));
        assert_eq!(flags.contains(&"scheduled_once"), once);
    }

    #[test]
    fn every_flag_is_named_once() {
        let mut flags: Vec<_> = flags().collect();
        let count = flags.len();
        flags.sort_unstable();
        flags.dedup();
        assert_eq!(flags.len(), count);
    }
}
