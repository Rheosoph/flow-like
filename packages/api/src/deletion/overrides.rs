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
                ExternalStep::AppQuotaPayloads,
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
                sweep("AppRollingContribution", "appId"),
                sweep("AppRollingUsage", "appId"),
                sweep("AuditExportTarget", "appId"),
            ],
            keep: vec![
                keep(
                    "AppPurchase",
                    "appId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "AppPaymentSettings",
                    "appId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentRequest",
                    "appId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentOrder",
                    "itemId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "LegacyCheckout",
                    "itemId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentAttempt",
                    "appId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentLedgerEntry",
                    "appId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "AccessGrant",
                    "itemId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentEntitlement",
                    "itemId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "ProjectCapacity",
                    "appId",
                    "retained fork provenance and storage payer",
                ),
                keep(
                    "StorageUploadGrant",
                    "appId",
                    "valid storage grants expire through reconciliation",
                ),
                keep(
                    "QuotaOperation",
                    "appId",
                    "billing and in-flight settlement survive deletion",
                ),
                keep("QuotaDailyUsage", "appId", "billing history"),
                keep(
                    "FileAccountingObject",
                    "appId",
                    "storage event deduplication outlives the app",
                ),
                keep("AuditEntry", "chainId", "audit trail outlives the app"),
                keep("AuditRecord", "chainId", "audit trail outlives the app"),
                keep("AuditSeal", "chainId", "audit trail outlives the app"),
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
            before_drain: vec![ExternalStep::UserQuotaPayloads],
            soft_sweeps: vec![sweep("QuotaWarningState", "payerId")],
            restrict_as_cascade: vec![
                ("WasmPackageInvitation", "invitedById"),
                ("WasmPackageInvitation", "inviteeId"),
            ],
            keep: vec![
                keep(
                    "AppPurchase",
                    "userId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "WasmPackagePurchase",
                    "userId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentAccountBinding",
                    "userId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "ConnectedAccount",
                    "userId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "AppPaymentSettings",
                    "ownerUserId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentsBlock",
                    "userId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentRequest",
                    "payerUserId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentRequest",
                    "payeeUserId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentOrder",
                    "userId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentOrder",
                    "payeeUserId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "LegacyCheckout",
                    "userId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentAttempt",
                    "payerUserId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentAttempt",
                    "payeeUserId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentRefund",
                    "requestedBy",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentLedgerEntry",
                    "payerUserId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentLedgerEntry",
                    "payeeUserId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "AccessGrant",
                    "userId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentEntitlement",
                    "userId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "LegalConsent",
                    "userId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "AccountCapacity",
                    "payerId",
                    "late object events reconcile current storage",
                ),
                keep(
                    "ProjectCapacity",
                    "payerId",
                    "retained storage payer and fork provenance",
                ),
                keep(
                    "StorageUploadGrant",
                    "payerId",
                    "valid storage grants expire through reconciliation",
                ),
                keep(
                    "FileAccountingObject",
                    "payerId",
                    "late object events retain payer identity",
                ),
                keep(
                    "QuotaAccount",
                    "payerId",
                    "in-flight settlement survives deletion",
                ),
                keep("QuotaPeriod", "payerId", "billing history"),
                keep(
                    "QuotaOperation",
                    "payerId",
                    "billing and in-flight settlement survive deletion",
                ),
                keep("QuotaEvent", "payerId", "billing history"),
                keep("QuotaDailyUsage", "payerId", "billing history"),
                keep("ComputeAttempt", "payerId", "infrastructure cost history"),
                keep(
                    "CloudDispatchIntent",
                    "payerId",
                    "durable dispatch recovery expires outstanding work",
                ),
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
                keep("AuditRecord", "chainId", "audit trail"),
            ],
            ..RootOverrides::default()
        },
        DeletionRoot::WasmPackage => RootOverrides {
            before_drain: vec![ExternalStep::WasmPackageArtifacts],
            keep: vec![
                keep(
                    "WasmPackagePurchase",
                    "packageId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentOrder",
                    "itemId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "LegacyCheckout",
                    "itemId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentAttempt",
                    "packageId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentLedgerEntry",
                    "packageId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "AccessGrant",
                    "itemId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "PaymentEntitlement",
                    "itemId",
                    "payment history and servicing survive deletion",
                ),
                keep(
                    "AppPackage",
                    "packageId",
                    "installs are flagged stale, not removed",
                ),
                keep("AuditEntry", "chainId", "audit trail"),
                keep("AuditRecord", "chainId", "audit trail"),
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
