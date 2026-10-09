#![recursion_limit = "256"]

pub mod acme;
pub mod archives;
pub mod broker;
pub mod certificate_inventory;
pub mod certificate_issuers;
pub mod certificate_requests;
pub mod certificates;
pub mod config;
pub mod crypto;
pub mod diagnostics;
pub mod enrollment;
pub mod event_kind;
#[cfg(feature = "runtime")]
pub(crate) mod execution_logs;
pub mod fleet;
pub mod host;
#[cfg(unix)]
pub mod ipc;
pub mod isolation;
pub mod management;
pub mod models;
pub(crate) mod operational;
pub mod outbox;
pub mod placement_data;
pub(crate) mod process_spawner;
pub mod project_artifacts;
pub mod release;
pub mod rollout;
pub mod secrets;
pub mod service;
pub mod state;
pub mod supervisor;
pub mod telemetry;
pub mod telemetry_groups;
pub mod transport;
pub mod usage;
pub mod vault;

#[cfg(all(feature = "runtime", feature = "bots"))]
pub mod bots;
#[cfg(feature = "runtime")]
pub mod dependencies;
#[cfg(feature = "runtime")]
pub mod deployment;
#[cfg(feature = "runtime")]
pub mod environment;
#[cfg(feature = "runtime")]
pub mod hosting;
#[cfg(all(feature = "runtime", feature = "on-demand"))]
pub mod on_demand;
#[cfg(feature = "runtime")]
pub mod online;
#[cfg(feature = "runtime")]
pub mod run_once;
#[cfg(feature = "runtime")]
pub mod runtime;
#[cfg(feature = "runtime")]
pub mod schedule;

pub mod replicas;

#[cfg(all(feature = "runtime", unix))]
pub mod service_tls;

/// The most a placement process logs of these targets, whatever `RUST_LOG` says: every
/// Telegram request carries its bot's token in the address, the HTTP client stack logs
/// addresses below `info`, and Discord's gateway logs a payload it cannot read, message
/// text included, at `warn`.
const PLACEMENT_LOG_LIMITS: [(&str, &str); 11] = [
    ("teloxide", "warn"),
    ("teloxide_core", "warn"),
    ("serenity", "warn"),
    ("serenity::gateway::ws", "error"),
    ("reqwest", "info"),
    ("hyper", "info"),
    ("hyper_util", "info"),
    ("h2", "info"),
    ("rustls", "info"),
    ("tungstenite", "info"),
    ("tokio_tungstenite", "info"),
];

/// The log filter of an agent process: the directives of `RUST_LOG` (`environment`), else
/// `info`. A placement process adds `PLACEMENT_LOG_LIMITS` after them, so a limit replaces
/// what the environment set for its target; an environment directive that would still
/// reach a limited target (a longer target, or a span without a target) is left out.
pub fn log_filter(environment: Option<&str>, placement: bool) -> tracing_subscriber::EnvFilter {
    let directives = environment.map(|directives| {
        directives
            .split(',')
            .filter(|directive| !placement || !reaches_limited_target(directive))
            .collect::<Vec<_>>()
            .join(",")
    });
    let filter = directives
        .and_then(|directives| tracing_subscriber::EnvFilter::try_new(directives).ok())
        .unwrap_or_else(|| "info".into());
    if !placement {
        return filter;
    }
    PLACEMENT_LOG_LIMITS
        .iter()
        .fold(filter, |filter, (target, level)| {
            filter.add_directive(
                format!("{target}={level}")
                    .parse()
                    .expect("a placement log limit parses"),
            )
        })
}

/// A directive's target matches every target it is a prefix of, and the most specific
/// directive wins.
fn reaches_limited_target(directive: &str) -> bool {
    let directive = directive.trim();
    let target = &directive[..directive.find(['[', '=']).unwrap_or(directive.len())];
    if target.is_empty() {
        return directive.starts_with('[');
    }
    let bare_level = target.len() == directive.len()
        && target
            .parse::<tracing_subscriber::filter::LevelFilter>()
            .is_ok();
    !bare_level
        && PLACEMENT_LOG_LIMITS
            .iter()
            .any(|(limited, _)| target.starts_with(limited))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Clone, Default)]
    struct Seen(Arc<Mutex<Vec<(String, Level)>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Seen {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let metadata = event.metadata();
            self.0
                .lock()
                .unwrap()
                .push((metadata.target().to_owned(), *metadata.level()));
        }
    }

    /// What a debug, info and warn event of each limited target, and a trace event of the
    /// agent, leave behind under `filter`.
    fn events_under(filter: tracing_subscriber::EnvFilter) -> Vec<(String, Level)> {
        let seen = Seen::default();
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(seen.clone());
        tracing::subscriber::with_default(subscriber, || {
            macro_rules! probe {
                ($($target:literal),*) => {$(
                    tracing::debug!(target: $target, "probe");
                    tracing::info!(target: $target, "probe");
                    tracing::warn!(target: $target, "probe");
                )*};
            }
            probe!(
                "teloxide::dispatching",
                "teloxide_core::requests",
                "serenity::gateway",
                "serenity::gateway::ws",
                "reqwest::connect",
                "hyper::proto",
                "hyper_util::client::legacy",
                "h2::codec",
                "rustls::client",
                "tungstenite::protocol",
                "tokio_tungstenite"
            );
            tracing::trace!(target: "flow_like_standalone::runtime", "probe");
        });
        let seen = seen.0.lock().unwrap().clone();
        seen
    }

    fn limited(seen: &[(String, Level)]) -> Vec<(String, Level)> {
        seen.iter()
            .filter(|(target, _)| !target.starts_with("flow_like_standalone"))
            .cloned()
            .collect()
    }

    /// Each probed target with the most verbose level that still reaches the log.
    fn expected_limited() -> Vec<(String, Level)> {
        [
            ("teloxide::dispatching", Some(Level::WARN)),
            ("teloxide_core::requests", Some(Level::WARN)),
            ("serenity::gateway", Some(Level::WARN)),
            ("serenity::gateway::ws", None),
            ("reqwest::connect", Some(Level::INFO)),
            ("hyper::proto", Some(Level::INFO)),
            ("hyper_util::client::legacy", Some(Level::INFO)),
            ("h2::codec", Some(Level::INFO)),
            ("rustls::client", Some(Level::INFO)),
            ("tungstenite::protocol", Some(Level::INFO)),
            ("tokio_tungstenite", Some(Level::INFO)),
        ]
        .into_iter()
        .flat_map(|(target, most)| {
            [Level::INFO, Level::WARN]
                .into_iter()
                .filter(move |level| most.is_some_and(|most| *level <= most))
                .map(move |level| (target.to_owned(), level))
        })
        .collect()
    }

    #[test]
    fn a_placement_keeps_its_limits_whatever_rust_log_says() {
        for environment in [
            "trace",
            "trace,reqwest=trace,teloxide=trace",
            "reqwest::connect=trace,hyper_util::client=debug,h2=trace,flow_like_standalone=trace",
            "[request]=trace,[{token}]=trace,trace",
            "TRACE,hyper_util[conn]=trace",
        ] {
            let seen = events_under(log_filter(Some(environment), true));
            assert_eq!(limited(&seen), expected_limited(), "{environment}");
            assert!(
                seen.contains(&("flow_like_standalone::runtime".into(), Level::TRACE)),
                "{environment}"
            );
        }
        let seen = events_under(log_filter(None, true));
        assert_eq!(limited(&seen), expected_limited());
        assert!(
            seen.iter()
                .all(|(target, _)| !target.starts_with("flow_like"))
        );
    }

    #[test]
    fn the_agent_parent_keeps_what_rust_log_says() {
        let seen = events_under(log_filter(Some("trace"), false));
        assert!(seen.contains(&("reqwest::connect".into(), Level::DEBUG)));
        assert!(seen.contains(&("teloxide::dispatching".into(), Level::DEBUG)));
        assert!(seen.contains(&("flow_like_standalone::runtime".into(), Level::TRACE)));
        let seen = events_under(log_filter(None, false));
        assert!(seen.contains(&("reqwest::connect".into(), Level::INFO)));
        assert!(!seen.contains(&("reqwest::connect".into(), Level::DEBUG)));
    }
}
