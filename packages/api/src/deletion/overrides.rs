//! What the foreign-key metadata cannot express about a deletion root.
//!
//! External stores, soft references and non-FK rows keyed by the root id are
//! declared here per root and folded into the plan by [`super::plan`].

use super::DeletionRoot;
use super::external::ExternalStep;

/// Rows that reference the root by value without a foreign key and go with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoftSweep {
    pub table: &'static str,
    pub column: &'static str,
}

/// A by-value reference to the root that is kept on purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoftReference {
    pub table: &'static str,
    pub column: &'static str,
    pub reason: &'static str,
}

#[derive(Clone, Debug, Default)]
pub struct RootOverrides {
    /// External cleanup that needs child rows to find its targets; runs before
    /// the first row drains.
    pub before_drain: Vec<ExternalStep>,
    /// External cleanup keyed by the root id alone; runs after the last child
    /// drain and before the root row is deleted.
    pub after_drain: Vec<ExternalStep>,
    pub soft_sweeps: Vec<SoftSweep>,
    pub keep: Vec<SoftReference>,
    /// Blocking edges (`Restrict`/`NoAction`) the plan drains as if they
    /// cascaded, because the rows belong to the root semantically.
    pub restrict_as_cascade: Vec<(&'static str, &'static str)>,
}

fn sweep(table: &'static str, column: &'static str) -> SoftSweep {
    SoftSweep { table, column }
}

fn keep(table: &'static str, column: &'static str, reason: &'static str) -> SoftReference {
    SoftReference {
        table,
        column,
        reason,
    }
}

pub fn overrides_for(root: DeletionRoot) -> RootOverrides {
    match root {
        DeletionRoot::App => RootOverrides {
            // Staged execution-event payloads are keyed by the run, not the
            // app, so the `payloadRef` on those rows is the only way to find
            // them. Both steps must therefore run before the rows drain.
            before_drain: vec![
                ExternalStep::AppSinkSchedules,
                ExternalStep::ExecutionEventPayloads,
            ],
            after_drain: vec![
                ExternalStep::AppStoragePrefixes,
                ExternalStep::AppCacheBackend,
            ],
            soft_sweeps: vec![
                sweep("AppCacheEntry", "appId"),
                sweep("UsageInvocation", "appId"),
                sweep("UsageAlert", "appId"),
                sweep("UsageLimitAuditLog", "appId"),
                sweep("FlowScriptApplyFailure", "appId"),
            ],
            keep: vec![
                keep(
                    "FileAccountingObject",
                    "appId",
                    "storage event deduplication outlives the app",
                ),
                keep("AuditEntry", "chainId", "audit trail outlives the app"),
                keep("Channel", "appId", "expires through the channel sweeper"),
                keep(
                    "ExecutionRunCallerApp",
                    "appId",
                    "belongs to the calling run, not the app it names",
                ),
            ],
            restrict_as_cascade: vec![],
        },
        DeletionRoot::User => RootOverrides {
            restrict_as_cascade: vec![
                ("WasmPackageInvitation", "invitedById"),
                ("WasmPackageInvitation", "inviteeId"),
            ],
            keep: vec![
                keep(
                    "FileAccountingObject",
                    "userId",
                    "storage event deduplication outlives the user",
                ),
                keep("UserCourseEnrollment", "userId", "learning history"),
                keep("UserLessonProgress", "userId", "learning history"),
                keep("UserChallengeAttempt", "userId", "learning history"),
                keep(
                    "Certificate",
                    "userId",
                    "issued certificates stay verifiable",
                ),
                keep("LeaderboardOptIn", "userId", "learning history"),
                keep("ErrorReport", "userId", "diagnostics"),
                keep("UsageInvocation", "userId", "billing history"),
                keep("UsageAlert", "userId", "billing history"),
                keep("UsageLimitAuditLog", "userId", "billing history"),
                keep("AuditEntry", "chainId", "audit trail"),
            ],
            ..RootOverrides::default()
        },
        DeletionRoot::WasmPackage => RootOverrides {
            before_drain: vec![ExternalStep::WasmPackageArtifacts],
            keep: vec![
                keep(
                    "AppPackage",
                    "packageId",
                    "installs are flagged stale, not removed",
                ),
                keep("AuditEntry", "chainId", "audit trail"),
            ],
            ..RootOverrides::default()
        },
        DeletionRoot::Course => RootOverrides {
            before_drain: vec![ExternalStep::CourseMedia],
            ..RootOverrides::default()
        },
        DeletionRoot::Bit => RootOverrides {
            before_drain: vec![ExternalStep::BitCdnArtifact],
            ..RootOverrides::default()
        },
        DeletionRoot::Event => RootOverrides {
            keep: vec![
                keep("ExecutionRun", "eventId", "run history"),
                keep(
                    "RegressionSuite",
                    "eventId",
                    "suite keeps its configuration",
                ),
                keep("Feedback", "eventId", "feedback history"),
            ],
            ..RootOverrides::default()
        },
        DeletionRoot::ExecutionRun => RootOverrides {
            keep: vec![
                keep("ExecutionRun", "parentRunId", "soft parent link"),
                keep("RegressionCaseResult", "replayRunId", "soft replay link"),
            ],
            ..RootOverrides::default()
        },
        // The board, versions and page payloads live under the owning app's
        // prefix and go after the last child drains, so a template that is
        // still listed is still openable.
        DeletionRoot::Template => RootOverrides {
            after_drain: vec![ExternalStep::TemplateStorage],
            ..RootOverrides::default()
        },
        DeletionRoot::CourseModule
        | DeletionRoot::Lesson
        | DeletionRoot::Challenge
        | DeletionRoot::LearningPath
        | DeletionRoot::Role
        | DeletionRoot::TechnicalUser
        | DeletionRoot::Membership
        | DeletionRoot::AppGroup => RootOverrides::default(),
    }
}
