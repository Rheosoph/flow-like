//! Which audit actions a deployment records. Every action name maps to the
//! lowest `AuditLevel` that includes it; unknown actions count as content
//! changes so a new hook is recorded by default without being verbose.

use flow_like::hub::AuditConfig;
pub use flow_like::hub::AuditLevel;

pub const REQUEST_ATTEMPT_ACTION: &str = "api.request.attempt";
pub const REQUEST_FINISH_ACTION: &str = "api.request.finish";

/// Identity, access, credentials, ingress configuration and platform administration.
const MINIMAL_PREFIXES: &[&str] = &[
    "admin.",
    "pat.",
    "apikey.",
    "role.",
    "membership.",
    "payments.",
    "marketplace.",
    "invite.",
    "team.",
    "app_connection.",
    "app_group.member.",
    "app_group.request.",
    "sink.",
];

/// Lifecycle, publication and irreversible deletions of primary resources.
const MINIMAL_ACTIONS: &[&str] = &[
    "app.create",
    "app.delete",
    "app.visibility",
    "app.visibility.request",
    "app.settings.forking",
    "app_group.create",
    "app_group.delete",
    "app_group.visibility",
    "app_group.visibility.request",
    "audit.export.webhook.set",
    "audit.export.webhook.rotate",
    "audit.export.webhook.delete",
    "registry.publish",
    "registry.access.purchase",
    "board.delete",
    "event.delete",
    "page.delete",
    "widget.delete",
    "template.delete",
    "route.delete",
    "database.table.drop",
    "graph.overlay.delete",
];

/// High-frequency records whose content is already persisted elsewhere.
const VERBOSE_PREFIXES: &[&str] = &["api.request.", "board.commands.", "execution."];

const VERBOSE_ACTIONS: &[&str] = &[
    "graph.nodes.upsert",
    "graph.edges.upsert",
    "file.access.authorize",
    "database.rows.offline_replay",
    "storage.files.offline_replay",
];

fn matches(action: &str, prefixes: &[&str], exact: &[&str]) -> bool {
    exact.contains(&action) || prefixes.iter().any(|prefix| action.starts_with(prefix))
}

/// The lowest level that records `action`.
pub fn required_level(action: &str) -> AuditLevel {
    if matches(action, MINIMAL_PREFIXES, MINIMAL_ACTIONS) {
        AuditLevel::Minimal
    } else if matches(action, VERBOSE_PREFIXES, VERBOSE_ACTIONS) {
        AuditLevel::Verbose
    } else {
        AuditLevel::Standard
    }
}

/// Which retention schedule a record follows. Actions only recorded at `verbose` are
/// operational and age out of the database; everything else is evidence and is archived.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RetentionClass {
    Evidence,
    Activity,
}

impl RetentionClass {
    pub fn of(action: &str) -> Self {
        if required_level(action) == AuditLevel::Verbose {
            Self::Activity
        } else {
            Self::Evidence
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Evidence => "evidence",
            Self::Activity => "activity",
        }
    }
}

/// Whether the deployment records `action`. The execution switch also applies to
/// custom actors recorded through the generic writer, such as anonymous frontends.
pub fn records(config: &AuditConfig, action: impl AsRef<str>) -> bool {
    let action = action.as_ref();
    config.enabled
        && (config.level >= required_level(action)
            || (config.log_executions && action.starts_with("execution.")))
}

/// Execution lifecycle records are the largest audit volume. `verbose`
/// includes them; lower levels need the explicit `log_executions` switch.
pub fn records_executions(config: &AuditConfig) -> bool {
    config.enabled && (config.log_executions || config.level >= AuditLevel::Verbose)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(level: AuditLevel, enabled: bool, log_executions: bool) -> AuditConfig {
        AuditConfig {
            enabled,
            level,
            log_executions,
            ..AuditConfig::default()
        }
    }

    /// Every action name emitted by the API with the level that records it.
    /// Adding a hook with a new name means adding it here.
    const CLASSIFIED: &[(&str, AuditLevel)] = &[
        ("admin.model.upsert", AuditLevel::Minimal),
        ("admin.package.delete", AuditLevel::Minimal),
        ("admin.package.ensure_wasm_artifacts", AuditLevel::Minimal),
        ("admin.package.review", AuditLevel::Minimal),
        ("admin.package.update", AuditLevel::Minimal),
        ("admin.profile.delete", AuditLevel::Minimal),
        ("admin.publication.review", AuditLevel::Minimal),
        ("admin.sink.register", AuditLevel::Minimal),
        ("admin.sink.revoke", AuditLevel::Minimal),
        ("admin.solution.log", AuditLevel::Minimal),
        ("admin.solution.update", AuditLevel::Minimal),
        ("admin.user.update", AuditLevel::Minimal),
        ("apikey.create", AuditLevel::Minimal),
        ("apikey.delete", AuditLevel::Minimal),
        ("app.create", AuditLevel::Minimal),
        ("app.delete", AuditLevel::Minimal),
        ("app.settings.appearance", AuditLevel::Standard),
        ("app.settings.forking", AuditLevel::Minimal),
        ("app.update", AuditLevel::Standard),
        ("app.visibility", AuditLevel::Minimal),
        ("app.visibility.request", AuditLevel::Minimal),
        ("app_connection.accept", AuditLevel::Minimal),
        ("app_connection.create", AuditLevel::Minimal),
        ("app_connection.reject", AuditLevel::Minimal),
        ("app_connection.remove", AuditLevel::Minimal),
        ("app_connection.request", AuditLevel::Minimal),
        ("app_connection.update", AuditLevel::Minimal),
        ("app_group.create", AuditLevel::Minimal),
        ("app_group.delete", AuditLevel::Minimal),
        ("app_group.member.add", AuditLevel::Minimal),
        ("app_group.member.leave", AuditLevel::Minimal),
        ("app_group.member.remove", AuditLevel::Minimal),
        ("app_group.request.accept", AuditLevel::Minimal),
        ("app_group.request.decline", AuditLevel::Minimal),
        ("app_group.update", AuditLevel::Standard),
        ("app_group.visibility", AuditLevel::Minimal),
        ("app_group.visibility.request", AuditLevel::Minimal),
        ("api.request.attempt", AuditLevel::Verbose),
        ("api.request.finish", AuditLevel::Verbose),
        ("audit.export.webhook.delete", AuditLevel::Minimal),
        ("audit.export.webhook.rotate", AuditLevel::Minimal),
        ("audit.export.webhook.set", AuditLevel::Minimal),
        ("board.commands.execute", AuditLevel::Verbose),
        ("board.commands.redo", AuditLevel::Verbose),
        ("board.commands.undo", AuditLevel::Verbose),
        ("board.delete", AuditLevel::Minimal),
        ("board.flow_ir.commit", AuditLevel::Standard),
        ("board.flowscript.apply", AuditLevel::Standard),
        ("board.update", AuditLevel::Standard),
        ("board.version", AuditLevel::Standard),
        ("database.rows.delete", AuditLevel::Standard),
        ("database.rows.insert", AuditLevel::Standard),
        ("database.rows.offline_replay", AuditLevel::Verbose),
        ("database.rows.update", AuditLevel::Standard),
        ("database.table.create", AuditLevel::Standard),
        ("database.table.drop", AuditLevel::Minimal),
        ("event.alias.delete", AuditLevel::Standard),
        ("event.alias.upsert", AuditLevel::Standard),
        ("event.canary.abort", AuditLevel::Standard),
        ("event.canary.promote", AuditLevel::Standard),
        ("event.canary.share", AuditLevel::Standard),
        ("event.canary.variants", AuditLevel::Standard),
        ("event.delete", AuditLevel::Minimal),
        ("event.regression.fixture.delete", AuditLevel::Standard),
        ("event.regression.fixture.promote", AuditLevel::Standard),
        ("event.regression.run", AuditLevel::Standard),
        ("event.regression.suite.update", AuditLevel::Standard),
        ("event.restore", AuditLevel::Standard),
        ("event.setup", AuditLevel::Standard),
        ("event.upsert", AuditLevel::Standard),
        ("execution.board.complete", AuditLevel::Verbose),
        ("execution.board.reject", AuditLevel::Verbose),
        ("execution.board.start", AuditLevel::Verbose),
        ("execution.event.fail", AuditLevel::Verbose),
        ("execution.event.start", AuditLevel::Verbose),
        ("file.access.authorize", AuditLevel::Verbose),
        ("file.delete", AuditLevel::Standard),
        ("file.upload.authorize", AuditLevel::Standard),
        ("graph.edges.upsert", AuditLevel::Verbose),
        ("graph.nodes.upsert", AuditLevel::Verbose),
        ("graph.objects.update", AuditLevel::Standard),
        ("graph.overlay.create", AuditLevel::Standard),
        ("graph.overlay.delete", AuditLevel::Minimal),
        ("graph.overlay.update", AuditLevel::Standard),
        ("graph.relationships.update", AuditLevel::Standard),
        ("invite.create", AuditLevel::Minimal),
        ("invite.delete", AuditLevel::Minimal),
        ("membership.accept", AuditLevel::Minimal),
        ("membership.invite", AuditLevel::Minimal),
        ("membership.invite.revoke", AuditLevel::Minimal),
        ("membership.join", AuditLevel::Minimal),
        ("membership.purchase", AuditLevel::Minimal),
        ("membership.comp", AuditLevel::Minimal),
        ("payments.settings.updated", AuditLevel::Minimal),
        ("payments.seller.blocked", AuditLevel::Minimal),
        ("payments.account.deauthorized", AuditLevel::Minimal),
        ("payments.account.onboarding", AuditLevel::Minimal),
        ("payments.account.disconnected", AuditLevel::Minimal),
        ("payments.account.reconnected", AuditLevel::Minimal),
        ("payments.terms.accepted", AuditLevel::Minimal),
        ("membership.purchase.approve", AuditLevel::Minimal),
        ("payments.effect.resumed", AuditLevel::Minimal),
        ("marketplace.purchase.paid", AuditLevel::Minimal),
        ("marketplace.purchase.withdrawn", AuditLevel::Minimal),
        ("marketplace.dispute.updated", AuditLevel::Minimal),
        ("membership.reject", AuditLevel::Minimal),
        ("membership.remove", AuditLevel::Minimal),
        ("membership.request", AuditLevel::Minimal),
        ("meta.upsert", AuditLevel::Standard),
        ("page.delete", AuditLevel::Minimal),
        ("page.upsert", AuditLevel::Standard),
        ("pat.create", AuditLevel::Minimal),
        ("pat.delete", AuditLevel::Minimal),
        ("process_note.create", AuditLevel::Standard),
        ("process_note.delete", AuditLevel::Standard),
        ("process_note.update", AuditLevel::Standard),
        ("registry.access.purchase", AuditLevel::Minimal),
        ("registry.publish", AuditLevel::Minimal),
        ("role.assign", AuditLevel::Minimal),
        ("role.create", AuditLevel::Minimal),
        ("role.default", AuditLevel::Minimal),
        ("role.delete", AuditLevel::Minimal),
        ("role.update", AuditLevel::Minimal),
        ("route.create", AuditLevel::Standard),
        ("route.delete", AuditLevel::Minimal),
        ("route.reassign", AuditLevel::Standard),
        ("route.update", AuditLevel::Standard),
        ("sink.toggle", AuditLevel::Minimal),
        ("solution.deposit.paid", AuditLevel::Standard),
        ("storage.files.offline_replay", AuditLevel::Verbose),
        ("sink.update", AuditLevel::Minimal),
        ("team.invite.accept", AuditLevel::Minimal),
        ("team.invite.reject", AuditLevel::Minimal),
        ("template.delete", AuditLevel::Minimal),
        ("template.update", AuditLevel::Standard),
        ("widget.create", AuditLevel::Standard),
        ("widget.delete", AuditLevel::Minimal),
        ("widget.update", AuditLevel::Standard),
        ("widget.version", AuditLevel::Standard),
    ];

    #[test]
    fn every_known_action_has_its_documented_level() {
        for (action, expected) in CLASSIFIED {
            assert_eq!(required_level(action), *expected, "{action}");
        }
    }

    #[test]
    fn verbose_only_actions_are_activity() {
        for (action, level) in CLASSIFIED {
            let expected = if *level == AuditLevel::Verbose {
                RetentionClass::Activity
            } else {
                RetentionClass::Evidence
            };
            assert_eq!(RetentionClass::of(action), expected, "{action}");
        }
    }

    #[test]
    fn unknown_actions_are_content_changes() {
        assert_eq!(required_level("something.new"), AuditLevel::Standard);
        assert_eq!(required_level(""), AuditLevel::Standard);
    }

    #[test]
    fn levels_nest_and_the_master_switch_wins() {
        assert!(AuditLevel::Minimal < AuditLevel::Standard);
        assert!(AuditLevel::Standard < AuditLevel::Verbose);
        let minimal = config(AuditLevel::Minimal, true, false);
        assert!(records(&minimal, "pat.delete"));
        assert!(!records(&minimal, "board.update"));
        assert!(!records(&minimal, REQUEST_ATTEMPT_ACTION));
        let standard = config(AuditLevel::Standard, true, false);
        assert!(records(&standard, "pat.delete"));
        assert!(records(&standard, "board.update".to_string()));
        assert!(!records(&standard, REQUEST_FINISH_ACTION));
        let verbose = config(AuditLevel::Verbose, true, false);
        assert!(records(&verbose, "board.commands.execute"));
        assert!(!records(
            &config(AuditLevel::Verbose, false, true),
            "pat.delete"
        ));
    }

    #[test]
    fn executions_follow_the_switch_below_verbose() {
        assert!(!records_executions(&config(
            AuditLevel::Standard,
            true,
            false
        )));
        assert!(records_executions(&config(AuditLevel::Minimal, true, true)));
        assert!(records_executions(&config(
            AuditLevel::Verbose,
            true,
            false
        )));
        assert!(!records_executions(&config(
            AuditLevel::Verbose,
            false,
            true
        )));
    }

    #[test]
    fn execution_switch_covers_generic_writes_without_enabling_other_activity() {
        for level in [
            AuditLevel::Minimal,
            AuditLevel::Standard,
            AuditLevel::Verbose,
        ] {
            let enabled = config(level, true, true);
            for action in ["execution.event.start", "execution.board.complete"] {
                assert!(records(&enabled, action), "{level:?}: {action}");
                assert!(!records(&config(level, false, true), action));
                assert_eq!(
                    records(&config(level, true, false), action),
                    records_executions(&config(level, true, false))
                );
            }
            assert_eq!(
                records(&enabled, REQUEST_ATTEMPT_ACTION),
                level == AuditLevel::Verbose
            );
            assert_eq!(
                RetentionClass::of("execution.event.start"),
                RetentionClass::Activity
            );
        }
    }

    #[test]
    fn default_level_is_standard() {
        assert_eq!(AuditConfig::default().level, AuditLevel::Standard);
        let parsed: AuditConfig = serde_json::from_str(r#"{"enabled": true}"#).unwrap();
        assert_eq!(parsed.level, AuditLevel::Standard);
        let parsed: AuditConfig = serde_json::from_str(r#"{"level": "minimal"}"#).unwrap();
        assert_eq!(parsed.level, AuditLevel::Minimal);
    }
}
