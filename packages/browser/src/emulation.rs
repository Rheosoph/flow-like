use serde_json::Value;

use crate::page::Page;

const NETWORK_ENABLE: &str = "Network.enable";
const NETWORK_DISABLE: &str = "Network.disable";
const NETWORK_DOMAIN: &str = "Network.";
const SET_GEOLOCATION: &str = "Emulation.setGeolocationOverride";
const CLEAR_GEOLOCATION: &str = "Emulation.clearGeolocationOverride";
const SET_IDLE: &str = "Emulation.setIdleOverride";
const CLEAR_IDLE: &str = "Emulation.clearIdleOverride";

// Overrides set on the page session do not reach OOPIF sessions, so these are replayed on every
// iframe session. Viewport, focus, touch and scrollbar emulation stay top-level only.
const REPLAYED: [&str; 14] = [
    "Emulation.setTimezoneOverride",
    "Emulation.setLocaleOverride",
    SET_GEOLOCATION,
    CLEAR_GEOLOCATION,
    "Emulation.setUserAgentOverride",
    "Emulation.setEmulatedMedia",
    SET_IDLE,
    CLEAR_IDLE,
    "Emulation.setCPUThrottlingRate",
    NETWORK_ENABLE,
    "Network.setUserAgentOverride",
    "Network.setExtraHTTPHeaders",
    "Network.setBlockedURLs",
    "Network.emulateNetworkConditions",
];

pub(crate) struct EmulationState {
    entries: Vec<(&'static str, Value)>,
}

impl EmulationState {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(crate) fn record(&mut self, method: &str, params: &serde_json::Value) {
        if method == NETWORK_DISABLE {
            self.entries
                .retain(|(recorded, _)| !recorded.starts_with(NETWORK_DOMAIN));
            return;
        }
        let Some(method) = replayed(method) else {
            return;
        };
        if let Some(cleared) = cleared_override(method) {
            self.entries.retain(|(recorded, _)| *recorded != cleared);
            return;
        }
        match self
            .entries
            .iter_mut()
            .find(|(recorded, _)| *recorded == method)
        {
            Some((_, recorded)) => *recorded = params.clone(),
            None => self.entries.push((method, params.clone())),
        }
    }

    fn commands(&self) -> Vec<(String, Value)> {
        let network = self
            .entries
            .iter()
            .filter(|(method, _)| *method == NETWORK_ENABLE);
        let rest = self
            .entries
            .iter()
            .filter(|(method, _)| *method != NETWORK_ENABLE);
        network
            .chain(rest)
            .map(|(method, params)| ((*method).to_owned(), params.clone()))
            .collect()
    }
}

fn cleared_override(method: &str) -> Option<&'static str> {
    match method {
        CLEAR_GEOLOCATION => Some(SET_GEOLOCATION),
        CLEAR_IDLE => Some(SET_IDLE),
        _ => None,
    }
}

fn replayed(method: &str) -> Option<&'static str> {
    REPLAYED.iter().copied().find(|known| *known == method)
}

pub(crate) fn is_replayed(method: &str) -> bool {
    replayed(method).is_some()
}

/// Replayed overrides, plus `Network.disable`: it turns the domain off on every iframe session
/// that got the replayed `Network.enable`, and drops the recorded `Network.*` entries.
pub(crate) fn reaches_iframes(method: &str) -> bool {
    is_replayed(method) || method == NETWORK_DISABLE
}

pub(crate) fn replay_commands(page: &Page) -> Vec<(String, serde_json::Value)> {
    page.inner.lock_state().emulation.commands()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{PageHarness, default_auto_reply};
    use serde_json::json;

    const TOP_LEVEL_ONLY: [&str; 8] = [
        "Emulation.setDeviceMetricsOverride",
        "Emulation.clearDeviceMetricsOverride",
        "Emulation.setVisibleSize",
        "Emulation.setFocusEmulationEnabled",
        "Emulation.setTouchEmulationEnabled",
        "Emulation.setEmitTouchEventsForMouse",
        "Emulation.setScrollbarsHidden",
        "Emulation.setDefaultBackgroundColorOverride",
    ];

    fn methods(commands: &[(String, Value)]) -> Vec<&str> {
        commands.iter().map(|(method, _)| method.as_str()).collect()
    }

    #[test]
    fn only_the_allowlist_is_replayed() {
        for method in REPLAYED {
            assert!(is_replayed(method), "{method} must be replayed");
        }
        for method in TOP_LEVEL_ONLY {
            assert!(!is_replayed(method), "{method} is top-level only");
        }
        for method in [
            "Network.disable",
            "Runtime.evaluate",
            "emulation.settimezoneoverride",
        ] {
            assert!(
                !is_replayed(method),
                "{method} is not an emulation override"
            );
        }
    }

    #[test]
    fn top_level_overrides_are_never_recorded() {
        let mut state = EmulationState::new();
        for method in TOP_LEVEL_ONLY {
            state.record(method, &json!({"width": 390, "height": 844}));
        }
        state.record("Network.disable", &json!({}));
        assert!(state.commands().is_empty());
    }

    #[test]
    fn network_enable_replays_first_and_the_rest_in_first_recorded_order() {
        let mut state = EmulationState::new();
        state.record(
            "Emulation.setTimezoneOverride",
            &json!({"timezoneId": "Asia/Tokyo"}),
        );
        state.record(
            "Network.setExtraHTTPHeaders",
            &json!({"headers": {"x-a": "1"}}),
        );
        state.record("Emulation.setLocaleOverride", &json!({"locale": "de-DE"}));
        state.record("Network.enable", &json!({}));
        state.record("Emulation.setCPUThrottlingRate", &json!({"rate": 4}));
        assert_eq!(
            methods(&state.commands()),
            [
                "Network.enable",
                "Emulation.setTimezoneOverride",
                "Network.setExtraHTTPHeaders",
                "Emulation.setLocaleOverride",
                "Emulation.setCPUThrottlingRate",
            ]
        );
    }

    #[test]
    fn a_later_call_replaces_the_params_in_place() {
        let mut state = EmulationState::new();
        state.record(
            "Emulation.setTimezoneOverride",
            &json!({"timezoneId": "Asia/Tokyo"}),
        );
        state.record("Emulation.setLocaleOverride", &json!({"locale": "de-DE"}));
        state.record(
            "Emulation.setTimezoneOverride",
            &json!({"timezoneId": "UTC"}),
        );
        let commands = state.commands();
        assert_eq!(
            methods(&commands),
            [
                "Emulation.setTimezoneOverride",
                "Emulation.setLocaleOverride"
            ]
        );
        assert_eq!(commands[0].1, json!({"timezoneId": "UTC"}));
    }

    #[test]
    fn clearing_the_geolocation_drops_the_recorded_override() {
        let mut state = EmulationState::new();
        let berlin = json!({"latitude": 52.52, "longitude": 13.4, "accuracy": 1});
        state.record(SET_GEOLOCATION, &berlin);
        state.record("Emulation.setEmulatedMedia", &json!({"media": "print"}));
        state.record(CLEAR_GEOLOCATION, &json!({}));
        assert_eq!(methods(&state.commands()), ["Emulation.setEmulatedMedia"]);
        let paris = json!({"latitude": 48.85, "longitude": 2.35, "accuracy": 1});
        state.record(SET_GEOLOCATION, &paris);
        let commands = state.commands();
        assert_eq!(
            methods(&commands),
            ["Emulation.setEmulatedMedia", SET_GEOLOCATION]
        );
        assert_eq!(commands[1].1, paris);
    }

    #[test]
    fn clearing_the_idle_state_drops_the_recorded_override() {
        let mut state = EmulationState::new();
        let idle = json!({"isUserActive": false, "isScreenUnlocked": false});
        state.record(SET_IDLE, &idle);
        state.record(SET_GEOLOCATION, &json!({"latitude": 52.52}));
        state.record(CLEAR_IDLE, &json!({}));
        assert!(is_replayed(CLEAR_IDLE));
        assert_eq!(methods(&state.commands()), [SET_GEOLOCATION]);
    }

    #[test]
    fn network_disable_drops_every_recorded_network_entry() {
        let mut state = EmulationState::new();
        state.record("Network.enable", &json!({}));
        state.record(
            "Emulation.setTimezoneOverride",
            &json!({"timezoneId": "Asia/Tokyo"}),
        );
        state.record("Network.setBlockedURLs", &json!({"urls": ["*/ads/*"]}));
        state.record(
            "Network.setExtraHTTPHeaders",
            &json!({"headers": {"x-a": "1"}}),
        );
        state.record(NETWORK_DISABLE, &json!({}));
        assert_eq!(
            methods(&state.commands()),
            ["Emulation.setTimezoneOverride"]
        );
        assert!(reaches_iframes(NETWORK_DISABLE));
        assert!(!is_replayed(NETWORK_DISABLE));
        state.record("Network.enable", &json!({}));
        assert_eq!(
            methods(&state.commands()),
            ["Network.enable", "Emulation.setTimezoneOverride"]
        );
    }

    #[tokio::test]
    async fn network_disable_through_cdp_reaches_iframe_sessions_and_clears_the_replay() {
        let harness = PageHarness::new().await;
        harness.attach_child("S2", "F2", "T1", "https://cross.test/", "LF2");
        harness.control.set_auto_reply(|command| {
            command
                .method
                .starts_with(NETWORK_DOMAIN)
                .then(|| json!({}))
                .or_else(|| default_auto_reply(command))
        });
        let blocked = json!({"urls": ["*/ads/*"]});
        harness.page.cdp("Network.enable", json!({})).await.unwrap();
        harness
            .page
            .cdp("Network.setBlockedURLs", blocked)
            .await
            .unwrap();
        harness.page.cdp(NETWORK_DISABLE, json!({})).await.unwrap();
        let on_child: Vec<String> = harness
            .control
            .commands_seen()
            .into_iter()
            .filter(|command| {
                command.session.as_deref() == Some("S2")
                    && command.method.starts_with(NETWORK_DOMAIN)
            })
            .map(|command| command.method)
            .collect();
        assert_eq!(
            on_child,
            ["Network.enable", "Network.setBlockedURLs", NETWORK_DISABLE]
        );
        assert!(replay_commands(&harness.page).is_empty());
    }

    #[tokio::test]
    async fn replay_commands_read_the_page_state() {
        let harness = PageHarness::new().await;
        assert!(replay_commands(&harness.page).is_empty());
        {
            let mut state = harness.page.inner.lock_state();
            let agent = json!({"userAgent": "flow-like", "acceptLanguage": "de-DE"});
            state
                .emulation
                .record("Emulation.setUserAgentOverride", &agent);
            state
                .emulation
                .record("Emulation.setDeviceMetricsOverride", &json!({"width": 390}));
            state.emulation.record("Network.enable", &json!({}));
        }
        let commands = replay_commands(&harness.page);
        assert_eq!(
            methods(&commands),
            ["Network.enable", "Emulation.setUserAgentOverride"]
        );
        assert_eq!(commands[1].1["acceptLanguage"], "de-DE");
    }
}
