-- #834: every DateTime column -> TIMESTAMPTZ(3) so the entity crate can type them as
-- DateTimeWithTimeZone. sqlx maps NaiveDateTime to TIMESTAMP and DateTime<Tz> to TIMESTAMPTZ
-- with no fallback, so one entity type cannot serve both column types — the API only serves
-- databases whose columns are timestamptz once this file has run.
--
-- Run by prisma/db-push.ts before `prisma db push`: Prisma emits the type change without a
-- USING clause, and Postgres' implicit timestamp -> timestamptz cast reads the value in the
-- session TimeZone. Every stored value is UTC, so the USING clause states that explicitly
-- rather than trusting whatever TimeZone the push session happens to carry.
--
-- PostgreSQL only. CockroachDB is deliberately left untouched (frozen legacy source) and
-- Aurora DSQL rejects ALTER COLUMN ... SET DATA TYPE outright — it starts from a migration
-- that already declares timestamptz.
--
-- db-push.ts skips each statement once the column is already timestamptz (or does not exist
-- yet), so a re-run is a no-op.

-- @dialect postgresql
ALTER TABLE "AiActAssessment" ALTER COLUMN "submittedAt" TYPE TIMESTAMPTZ(3) USING "submittedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AiActAssessment" ALTER COLUMN "reviewedAt" TYPE TIMESTAMPTZ(3) USING "reviewedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AiActAssessment" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AiActAssessment" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AiActModelObservation" ALTER COLUMN "firstSeenAt" TYPE TIMESTAMPTZ(3) USING "firstSeenAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AiActModelObservation" ALTER COLUMN "lastSeenAt" TYPE TIMESTAMPTZ(3) USING "lastSeenAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AiActModelRegistry" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AiActModelRegistry" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "App" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "App" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "App" ALTER COLUMN "forkedAt" TYPE TIMESTAMPTZ(3) USING "forkedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppAnalyticsDaily" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppAnalyticsDaily" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppBoardScore" ALTER COLUMN "computedAt" TYPE TIMESTAMPTZ(3) USING "computedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppBoardScore" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppCacheEntry" ALTER COLUMN "expiresAt" TYPE TIMESTAMPTZ(3) USING "expiresAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppCacheEntry" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppCacheEntry" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppConnection" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppConnection" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppDiscount" ALTER COLUMN "startsAt" TYPE TIMESTAMPTZ(3) USING "startsAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppDiscount" ALTER COLUMN "expiresAt" TYPE TIMESTAMPTZ(3) USING "expiresAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppDiscount" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppDiscount" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppGroup" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppGroup" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppGroupMember" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppGroupMember" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppPackage" ALTER COLUMN "addedAt" TYPE TIMESTAMPTZ(3) USING "addedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppProcessNote" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppProcessNote" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppPurchase" ALTER COLUMN "completedAt" TYPE TIMESTAMPTZ(3) USING "completedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppPurchase" ALTER COLUMN "refundedAt" TYPE TIMESTAMPTZ(3) USING "refundedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppPurchase" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppPurchase" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppSalesDaily" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppSalesDaily" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppUsageLimit" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AppUsageLimit" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "AuditEntry" ALTER COLUMN "timestamp" TYPE TIMESTAMPTZ(3) USING "timestamp" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Bit" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Bit" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "BitCache" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "BitCache" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "BitTreeCache" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "BitTreeCache" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "BoardSync" ALTER COLUMN "lastSyncedAt" TYPE TIMESTAMPTZ(3) USING "lastSyncedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "BoardSync" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "BoardSync" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Certificate" ALTER COLUMN "issuedAt" TYPE TIMESTAMPTZ(3) USING "issuedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Challenge" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Challenge" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Channel" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Comment" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Comment" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Course" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Course" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "CourseAppLink" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "CourseAppLink" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "CourseAsset" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "CourseAsset" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "CourseModule" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "CourseModule" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "DeletionJob" ALTER COLUMN "leaseUntil" TYPE TIMESTAMPTZ(3) USING "leaseUntil" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "DeletionJob" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "DeletionJob" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EmbeddingUsageTracking" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EmbeddingUsageTracking" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ErrorReport" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ErrorReport" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Event" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Event" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Event" ALTER COLUMN "lastSetupAt" TYPE TIMESTAMPTZ(3) USING "lastSetupAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EventAlias" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EventAlias" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EventRemoteAuth" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EventRemoteAuth" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EventRemoteRegistration" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EventSetup" ALTER COLUMN "lastSetupAt" TYPE TIMESTAMPTZ(3) USING "lastSetupAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EventSetup" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EventSetup" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EventSink" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "EventSink" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ExecutionEvent" ALTER COLUMN "expiresAt" TYPE TIMESTAMPTZ(3) USING "expiresAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ExecutionEvent" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ExecutionRun" ALTER COLUMN "startedAt" TYPE TIMESTAMPTZ(3) USING "startedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ExecutionRun" ALTER COLUMN "completedAt" TYPE TIMESTAMPTZ(3) USING "completedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ExecutionRun" ALTER COLUMN "expiresAt" TYPE TIMESTAMPTZ(3) USING "expiresAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ExecutionRun" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ExecutionRun" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ExecutionUsageTracking" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ExecutionUsageTracking" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Feedback" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Feedback" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "FlowScriptApplyFailure" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ForkJob" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ForkJob" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "ForkJob" ALTER COLUMN "expiresAt" TYPE TIMESTAMPTZ(3) USING "expiresAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Invitation" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Invitation" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "InviteLink" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "InviteLink" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "InviteLink" ALTER COLUMN "expiresAt" TYPE TIMESTAMPTZ(3) USING "expiresAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "JoinQueue" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "JoinQueue" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LLMUsageTracking" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LLMUsageTracking" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LandingPage" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LandingPage" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LeaderboardOptIn" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LearningPath" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LearningPath" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Lesson" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Lesson" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LessonAppRef" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LessonAppRef" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LlmModel" ALTER COLUMN "releaseDate" TYPE TIMESTAMPTZ(3) USING "releaseDate" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LlmModel" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "LlmModel" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Membership" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Membership" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Meta" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Meta" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "MutationLock" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "MutationLock" ALTER COLUMN "expiresAt" TYPE TIMESTAMPTZ(3) USING "expiresAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Node" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Node" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Notification" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Notification" ALTER COLUMN "readAt" TYPE TIMESTAMPTZ(3) USING "readAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PAT" ALTER COLUMN "validUntil" TYPE TIMESTAMPTZ(3) USING "validUntil" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PAT" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PAT" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Page" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Page" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Profile" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Profile" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Profile" ALTER COLUMN "deletedAt" TYPE TIMESTAMPTZ(3) USING "deletedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PublicationLog" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PublicationLog" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PublicationRequest" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PublicationRequest" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PushNotificationTarget" ALTER COLUMN "lastRegisteredAt" TYPE TIMESTAMPTZ(3) USING "lastRegisteredAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PushNotificationTarget" ALTER COLUMN "lastSeenAt" TYPE TIMESTAMPTZ(3) USING "lastSeenAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PushNotificationTarget" ALTER COLUMN "invalidatedAt" TYPE TIMESTAMPTZ(3) USING "invalidatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PushNotificationTarget" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "PushNotificationTarget" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "RegressionCaseResult" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "RegressionSuite" ALTER COLUMN "nextRunAt" TYPE TIMESTAMPTZ(3) USING "nextRunAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "RegressionSuite" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "RegressionSuite" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "RegressionSuiteRun" ALTER COLUMN "startedAt" TYPE TIMESTAMPTZ(3) USING "startedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "RegressionSuiteRun" ALTER COLUMN "completedAt" TYPE TIMESTAMPTZ(3) USING "completedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "RegressionSuiteRun" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Role" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Role" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "SinkToken" ALTER COLUMN "revokedAt" TYPE TIMESTAMPTZ(3) USING "revokedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "SinkToken" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "SinkToken" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "SolutionLog" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "SolutionRequest" ALTER COLUMN "deliveredAt" TYPE TIMESTAMPTZ(3) USING "deliveredAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "SolutionRequest" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "SolutionRequest" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "StripeEvent" ALTER COLUMN "processedAt" TYPE TIMESTAMPTZ(3) USING "processedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TechnicalUser" ALTER COLUMN "validUntil" TYPE TIMESTAMPTZ(3) USING "validUntil" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TechnicalUser" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TechnicalUser" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryAlertEvent" ALTER COLUMN "acknowledgedAt" TYPE TIMESTAMPTZ(3) USING "acknowledgedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryAlertEvent" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryAlertRule" ALTER COLUMN "lastEvaluatedAt" TYPE TIMESTAMPTZ(3) USING "lastEvaluatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryAlertRule" ALTER COLUMN "lastTriggeredAt" TYPE TIMESTAMPTZ(3) USING "lastTriggeredAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryAlertRule" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryAlertRule" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryDashboard" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryDashboard" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryDimensionDaily" ALTER COLUMN "day" TYPE TIMESTAMPTZ(3) USING "day" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryDimensionDaily" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryDimensionDaily" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryErrorEvent" ALTER COLUMN "clientTs" TYPE TIMESTAMPTZ(3) USING "clientTs" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryErrorEvent" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryEvent" ALTER COLUMN "clientTs" TYPE TIMESTAMPTZ(3) USING "clientTs" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryEvent" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryEventDaily" ALTER COLUMN "day" TYPE TIMESTAMPTZ(3) USING "day" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryEventDaily" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryEventDaily" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryFlowpilotDaily" ALTER COLUMN "day" TYPE TIMESTAMPTZ(3) USING "day" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryFlowpilotDaily" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryFlowpilotDaily" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryFlowpilotFailureDaily" ALTER COLUMN "day" TYPE TIMESTAMPTZ(3) USING "day" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryFlowpilotFailureDaily" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryFlowpilotFailureDaily" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryInstallDaily" ALTER COLUMN "day" TYPE TIMESTAMPTZ(3) USING "day" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryInstallDaily" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryInstallDaily" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryIssue" ALTER COLUMN "firstSeen" TYPE TIMESTAMPTZ(3) USING "firstSeen" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryIssue" ALTER COLUMN "lastSeen" TYPE TIMESTAMPTZ(3) USING "lastSeen" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryIssue" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryIssue" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryLlmCall" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryLlmDaily" ALTER COLUMN "day" TYPE TIMESTAMPTZ(3) USING "day" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryLlmDaily" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryLlmDaily" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryPerfDaily" ALTER COLUMN "day" TYPE TIMESTAMPTZ(3) USING "day" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryPerfDaily" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryPerfDaily" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryPerfMetric" ALTER COLUMN "clientTs" TYPE TIMESTAMPTZ(3) USING "clientTs" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryPerfMetric" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryRelease" ALTER COLUMN "firstSeenAt" TYPE TIMESTAMPTZ(3) USING "firstSeenAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetryRelease" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySavedQuery" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySavedQuery" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySession" ALTER COLUMN "startedAt" TYPE TIMESTAMPTZ(3) USING "startedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySession" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySession" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySessionDaily" ALTER COLUMN "day" TYPE TIMESTAMPTZ(3) USING "day" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySessionDaily" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySessionDaily" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySourceMap" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySpan" ALTER COLUMN "startedAt" TYPE TIMESTAMPTZ(3) USING "startedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TelemetrySpan" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Template" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Template" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TemplateProfile" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "TemplateProfile" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Transaction" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Transaction" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UsageAlert" ALTER COLUMN "acknowledgedAt" TYPE TIMESTAMPTZ(3) USING "acknowledgedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UsageAlert" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UsageAlert" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UsageInvocation" ALTER COLUMN "startedAt" TYPE TIMESTAMPTZ(3) USING "startedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UsageInvocation" ALTER COLUMN "completedAt" TYPE TIMESTAMPTZ(3) USING "completedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UsageInvocation" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UsageInvocation" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UsageLimitAuditLog" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "User" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "User" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UserBit" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UserBit" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UserChallengeAttempt" ALTER COLUMN "attemptedAt" TYPE TIMESTAMPTZ(3) USING "attemptedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UserCourseEnrollment" ALTER COLUMN "startedAt" TYPE TIMESTAMPTZ(3) USING "startedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UserCourseEnrollment" ALTER COLUMN "lastSeenAt" TYPE TIMESTAMPTZ(3) USING "lastSeenAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UserCourseEnrollment" ALTER COLUMN "completedAt" TYPE TIMESTAMPTZ(3) USING "completedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UserLessonProgress" ALTER COLUMN "completedAt" TYPE TIMESTAMPTZ(3) USING "completedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UserLessonProgress" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "UserLessonProgress" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackage" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackage" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackage" ALTER COLUMN "publishedAt" TYPE TIMESTAMPTZ(3) USING "publishedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackageAuthor" ALTER COLUMN "addedAt" TYPE TIMESTAMPTZ(3) USING "addedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackageInvitation" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackageInvitation" ALTER COLUMN "expiresAt" TYPE TIMESTAMPTZ(3) USING "expiresAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackageJoinQueue" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackageJoinQueue" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackagePurchase" ALTER COLUMN "completedAt" TYPE TIMESTAMPTZ(3) USING "completedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackagePurchase" ALTER COLUMN "refundedAt" TYPE TIMESTAMPTZ(3) USING "refundedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackagePurchase" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackagePurchase" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackageReview" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackageUser" ALTER COLUMN "grantedAt" TYPE TIMESTAMPTZ(3) USING "grantedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackageVersion" ALTER COLUMN "publishedAt" TYPE TIMESTAMPTZ(3) USING "publishedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WasmPackageVersion" ALTER COLUMN "approvedAt" TYPE TIMESTAMPTZ(3) USING "approvedAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WeeklyChallenge" ALTER COLUMN "expiresAt" TYPE TIMESTAMPTZ(3) USING "expiresAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "WeeklyChallenge" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Widget" ALTER COLUMN "createdAt" TYPE TIMESTAMPTZ(3) USING "createdAt" AT TIME ZONE 'UTC';
-- @dialect postgresql
ALTER TABLE "Widget" ALTER COLUMN "updatedAt" TYPE TIMESTAMPTZ(3) USING "updatedAt" AT TIME ZONE 'UTC';
