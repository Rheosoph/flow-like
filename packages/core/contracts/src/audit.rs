use serde::{Deserialize, Serialize};

/// How much of the mutation surface the audit trail records. Each level includes
/// everything below it. Execution lifecycle records additionally follow
/// `AuditConfig::log_executions`.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AuditLevel {
    /// Identity, access, credentials, publication, deletions and platform administration.
    Minimal,
    /// Minimal plus content changes: boards, events, pages, widgets, files, tables, settings.
    #[default]
    Standard,
    /// Standard plus every request attempt and outcome, editor commands, graph row
    /// writes, file read grants and execution lifecycle records.
    Verbose,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AuditConfig {
    /// Master switch. When false, no audit entries are recorded.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Which action families are recorded. Defaults to `standard`.
    #[serde(default)]
    pub level: AuditLevel,
    /// Whether to capture client IP addresses in audit entries (GDPR consideration)
    #[serde(default)]
    pub log_ip: bool,
    /// Reverse proxies under the deployment's control that append to
    /// `X-Forwarded-For`. The recorded client IP is taken that many entries from
    /// the right, which a client cannot forge. Unset records the leftmost entry,
    /// which the client chooses.
    #[serde(default)]
    pub trusted_proxy_hops: Option<u32>,
    /// Refuse to run without keys: the API without an entry key, the audit worker
    /// without an audit signing key.
    #[serde(default)]
    pub require_signing: bool,
    /// Record execution lifecycle transitions at any level.
    /// The `verbose` level records them regardless of this switch.
    #[serde(default)]
    pub log_executions: bool,
    /// How long each part of the trail is kept, and when the audit worker seals.
    #[serde(default)]
    pub retention: AuditRetention,
}

/// Retention, sealing and signing cadence. Evidence is pruned only once its month is
/// archived to the audit bucket, so a deployment without `AUDIT_BUCKET` keeps every
/// evidence record; activity records are deleted after their window.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AuditRetention {
    /// Days security and content records stay in the database after their month
    /// closes. Older months are read from the monthly archive.
    #[serde(default = "default_evidence_hot_days")]
    pub evidence_hot_days: u32,
    /// Informational horizon written into archive manifests: the end of this many
    /// calendar years after the month. The bucket's retention lock enforces it.
    #[serde(default = "default_archive_years")]
    pub archive_years_after_year_end: u32,
    /// Days records of verbose-only actions stay in the database. They are never
    /// archived.
    #[serde(default = "default_activity_days")]
    pub activity_days: u32,
    /// Minimum days for execution records of apps assessed as high-risk AI systems
    /// (EU AI Act art. 19 and 26(6)).
    #[serde(default = "default_high_risk_activity_days")]
    pub high_risk_activity_days: u32,
    /// Days a recorded client IP is kept before it is removed from its record.
    #[serde(default = "default_ip_days")]
    pub ip_days: u32,
    /// Days record details are kept. Unset keeps them as long as the record.
    #[serde(default)]
    pub details_days: Option<u32>,
    /// Seal a chain once this many records are pending.
    #[serde(default = "default_seal_after_records")]
    pub seal_after_records: u32,
    /// Seal a chain once its oldest pending record is this old.
    #[serde(default = "default_seal_after_seconds")]
    pub seal_after_seconds: u32,
    /// Largest seal the worker writes in one transaction.
    #[serde(default = "default_max_records_per_seal")]
    pub max_records_per_seal: u32,
    /// Sign an epoch once the oldest unanchored seal is this old, or earlier when 20,000
    /// seals wait. Each epoch is one request to the key service, so this sets its cost:
    /// 300 s means at most 12 routine signatures an hour.
    #[serde(default = "default_epoch_interval_seconds")]
    pub epoch_interval_seconds: u32,
    /// Log an alert when the oldest pending record is older than this.
    #[serde(default = "default_pending_alert_seconds")]
    pub pending_alert_seconds: u32,
    /// Days after a month closes before it is archived.
    #[serde(default = "default_archive_grace_days")]
    pub archive_grace_days: u32,
}

fn default_evidence_hot_days() -> u32 {
    396
}

fn default_archive_years() -> u32 {
    3
}

fn default_activity_days() -> u32 {
    90
}

fn default_high_risk_activity_days() -> u32 {
    183
}

fn default_ip_days() -> u32 {
    7
}

fn default_seal_after_records() -> u32 {
    500
}

fn default_seal_after_seconds() -> u32 {
    300
}

fn default_max_records_per_seal() -> u32 {
    1000
}

fn default_epoch_interval_seconds() -> u32 {
    300
}

fn default_pending_alert_seconds() -> u32 {
    900
}

fn default_archive_grace_days() -> u32 {
    3
}

impl Default for AuditRetention {
    fn default() -> Self {
        Self {
            evidence_hot_days: default_evidence_hot_days(),
            archive_years_after_year_end: default_archive_years(),
            activity_days: default_activity_days(),
            high_risk_activity_days: default_high_risk_activity_days(),
            ip_days: default_ip_days(),
            details_days: None,
            seal_after_records: default_seal_after_records(),
            seal_after_seconds: default_seal_after_seconds(),
            max_records_per_seal: default_max_records_per_seal(),
            epoch_interval_seconds: default_epoch_interval_seconds(),
            pending_alert_seconds: default_pending_alert_seconds(),
            archive_grace_days: default_archive_grace_days(),
        }
    }
}

fn default_true() -> bool {
    true
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: AuditLevel::default(),
            log_ip: false,
            trusted_proxy_hops: None,
            require_signing: false,
            log_executions: false,
            retention: AuditRetention::default(),
        }
    }
}
