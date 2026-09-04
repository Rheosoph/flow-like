-- #834: enum columns -> TEXT, scalar arrays -> JSONB, then drop the enum types.
-- Run by prisma/db-push.ts before `prisma db push` on existing PostgreSQL / CockroachDB
-- databases: Prisma emits these type changes without the USING clause they need.
-- Aurora DSQL never runs this file; it starts from the migrated schema.
--
-- Format: one statement per line, executed in autocommit. Guard comments of the form
-- `-- @<guard> ...` directly above a statement restrict it (db-push.ts evaluates them
-- against information_schema.columns):
--   `dialect cockroachdb|postgresql`        run only on that engine
--   `if-type "Table"."column" <type>`       run only while the column has that data_type
--   `unless-type "Table"."column" <type>`   skip while the column has that data_type
-- ALTER COLUMN ... TYPE statements are additionally skipped once the column already
-- has the target type, so a second run is a no-op.
--
-- CockroachDB cannot rewrite the type of an indexed column (crdb issue 47636): the
-- indexes over converted columns are dropped first and `prisma db push` recreates them.
-- The AppCacheEntry primary key column is swapped through a copy column instead.

-- CockroachDB: indexes over columns whose type changes
-- @dialect cockroachdb
-- @if-type "AiActAssessment"."riskCategory" USER-DEFINED
DROP INDEX IF EXISTS "AiActAssessment_riskCategory_idx";
-- @dialect cockroachdb
-- @if-type "AiActAssessment"."status" USER-DEFINED
DROP INDEX IF EXISTS "AiActAssessment_status_idx";
-- @dialect cockroachdb
-- @if-type "AiActModelObservation"."posture" USER-DEFINED
DROP INDEX IF EXISTS "AiActModelObservation_posture_idx";
-- @dialect cockroachdb
-- @if-type "AiActModelRegistry"."posture" USER-DEFINED
DROP INDEX IF EXISTS "AiActModelRegistry_posture_idx";
-- @dialect cockroachdb
-- @if-type "AppConnection"."status" USER-DEFINED
DROP INDEX IF EXISTS "AppConnection_targetAppId_status_idx";
-- @dialect cockroachdb
-- @if-type "AppConnection"."status" USER-DEFINED
DROP INDEX IF EXISTS "AppConnection_sourceAppId_status_idx";
-- @dialect cockroachdb
-- @if-type "PushNotificationTarget"."provider" USER-DEFINED
ALTER TABLE "PushNotificationTarget" DROP CONSTRAINT IF EXISTS "PushNotificationTarget_userId_deviceId_provider_key";
-- @dialect cockroachdb
-- @if-type "PushNotificationTarget"."provider" USER-DEFINED
DROP INDEX IF EXISTS "PushNotificationTarget_provider_platform_idx";
-- @dialect cockroachdb
-- @if-type "AppGroup"."visibility" USER-DEFINED
DROP INDEX IF EXISTS "AppGroup_visibility_idx";
-- @dialect cockroachdb
-- @if-type "AppGroupMember"."status" USER-DEFINED
DROP INDEX IF EXISTS "AppGroupMember_groupId_status_idx";
-- @dialect cockroachdb
-- @if-type "AppGroupMember"."status" USER-DEFINED
DROP INDEX IF EXISTS "AppGroupMember_appId_status_idx";
-- @dialect cockroachdb
-- @if-type "App"."status" USER-DEFINED
DROP INDEX IF EXISTS "App_status_idx";
-- @dialect cockroachdb
-- @if-type "AppPurchase"."status" USER-DEFINED
DROP INDEX IF EXISTS "AppPurchase_appId_status_idx";
-- @dialect cockroachdb
-- @if-type "Channel"."kind" USER-DEFINED
DROP INDEX IF EXISTS "Channel_channelId_kind_idx";
-- @dialect cockroachdb
-- @if-type "PublicationRequest"."status" USER-DEFINED
DROP INDEX IF EXISTS "PublicationRequest_status_idx";
-- @dialect cockroachdb
-- @if-type "Bit"."type" USER-DEFINED
DROP INDEX IF EXISTS "Bit_type_idx";
-- @dialect cockroachdb
-- @if-type "Course"."category" USER-DEFINED
DROP INDEX IF EXISTS "Course_category_idx";
-- @dialect cockroachdb
-- @if-type "Meta"."tags" ARRAY
DROP INDEX IF EXISTS "Meta_tags_idx";
-- @dialect cockroachdb
-- @if-type "SolutionRequest"."status" USER-DEFINED
DROP INDEX IF EXISTS "SolutionRequest_status_idx";
-- @dialect cockroachdb
-- @if-type "UserBit"."type" USER-DEFINED
DROP INDEX IF EXISTS "UserBit_userId_type_idx";
-- @dialect cockroachdb
-- @if-type "ExecutionRun"."status" USER-DEFINED
DROP INDEX IF EXISTS "ExecutionRun_appId_status_idx";
-- @dialect cockroachdb
-- @if-type "ExecutionRun"."status" USER-DEFINED
DROP INDEX IF EXISTS "ExecutionRun_userId_status_idx";
-- @dialect cockroachdb
-- @if-type "ExecutionRun"."callerAppChain" ARRAY
DROP INDEX IF EXISTS "ExecutionRun_callerAppChain_idx";
-- @dialect cockroachdb
-- @if-type "ExecutionRun"."runVariant" USER-DEFINED
DROP INDEX IF EXISTS "ExecutionRun_eventId_runVariant_createdAt_idx";
-- @dialect cockroachdb
-- @if-type "WasmPackage"."status" USER-DEFINED
DROP INDEX IF EXISTS "WasmPackage_status_idx";
-- @dialect cockroachdb
-- @if-type "WasmPackage"."visibility" USER-DEFINED
DROP INDEX IF EXISTS "WasmPackage_visibility_idx";
-- @dialect cockroachdb
-- @if-type "WasmPackage"."keywords" ARRAY
DROP INDEX IF EXISTS "WasmPackage_keywords_idx";
-- @dialect cockroachdb
-- @if-type "WasmPackage"."primaryCategory" USER-DEFINED
DROP INDEX IF EXISTS "WasmPackage_primaryCategory_idx";
-- @dialect cockroachdb
-- @if-type "WasmPackage"."secondaryCategory" USER-DEFINED
DROP INDEX IF EXISTS "WasmPackage_secondaryCategory_idx";
-- @dialect cockroachdb
-- @if-type "WasmPackageVersion"."status" USER-DEFINED
DROP INDEX IF EXISTS "WasmPackageVersion_status_idx";
-- @dialect cockroachdb
-- @if-type "WasmPackageVersion"."compilationStatus" USER-DEFINED
DROP INDEX IF EXISTS "WasmPackageVersion_compilationStatus_idx";
-- @dialect cockroachdb
-- @if-type "WasmPackageReview"."action" USER-DEFINED
DROP INDEX IF EXISTS "WasmPackageReview_action_idx";
-- @dialect cockroachdb
-- @if-type "WasmPackagePurchase"."status" USER-DEFINED
DROP INDEX IF EXISTS "WasmPackagePurchase_packageId_status_idx";

-- CockroachDB: AppCacheEntry.scope is part of the primary key
-- @dialect cockroachdb
-- @if-type "AppCacheEntry"."scope" USER-DEFINED
ALTER TABLE "AppCacheEntry" ADD COLUMN IF NOT EXISTS "scope_text" TEXT;
-- @dialect cockroachdb
-- @if-type "AppCacheEntry"."scope" USER-DEFINED
UPDATE "AppCacheEntry" SET "scope_text" = "scope"::TEXT WHERE "scope_text" IS NULL;
-- @dialect cockroachdb
-- @if-type "AppCacheEntry"."scope" USER-DEFINED
ALTER TABLE "AppCacheEntry" ALTER COLUMN "scope_text" SET NOT NULL;
-- @dialect cockroachdb
-- @if-type "AppCacheEntry"."scope" USER-DEFINED
ALTER TABLE "AppCacheEntry" ALTER PRIMARY KEY USING COLUMNS ("appId", "scope_text", "userId", "namespace", "key");
-- @dialect cockroachdb
-- @if-type "AppCacheEntry"."scope" USER-DEFINED
ALTER TABLE "AppCacheEntry" DROP COLUMN "scope";
-- @dialect cockroachdb
-- @if-type "AppCacheEntry"."scope_text" text
ALTER TABLE "AppCacheEntry" RENAME COLUMN "scope_text" TO "scope";

-- defaults typed with the old column type cannot be cast; prisma db push restores them
-- @if-type "AiActAssessment"."status" USER-DEFINED
ALTER TABLE "AiActAssessment" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "AiActAssessment"."riskCategory" USER-DEFINED
ALTER TABLE "AiActAssessment" ALTER COLUMN "riskCategory" DROP DEFAULT;
-- @if-type "AiActModelObservation"."source" USER-DEFINED
ALTER TABLE "AiActModelObservation" ALTER COLUMN "source" DROP DEFAULT;
-- @if-type "AiActModelObservation"."posture" USER-DEFINED
ALTER TABLE "AiActModelObservation" ALTER COLUMN "posture" DROP DEFAULT;
-- @if-type "AiActModelRegistry"."posture" USER-DEFINED
ALTER TABLE "AiActModelRegistry" ALTER COLUMN "posture" DROP DEFAULT;
-- @if-type "AppConnection"."status" USER-DEFINED
ALTER TABLE "AppConnection" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "Notification"."type" USER-DEFINED
ALTER TABLE "Notification" ALTER COLUMN "type" DROP DEFAULT;
-- @if-type "AppGroup"."status" USER-DEFINED
ALTER TABLE "AppGroup" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "AppGroup"."visibility" USER-DEFINED
ALTER TABLE "AppGroup" ALTER COLUMN "visibility" DROP DEFAULT;
-- @if-type "AppGroupMember"."kind" USER-DEFINED
ALTER TABLE "AppGroupMember" ALTER COLUMN "kind" DROP DEFAULT;
-- @if-type "AppGroupMember"."status" USER-DEFINED
ALTER TABLE "AppGroupMember" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "App"."status" USER-DEFINED
ALTER TABLE "App" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "App"."visibility" USER-DEFINED
ALTER TABLE "App" ALTER COLUMN "visibility" DROP DEFAULT;
-- @if-type "App"."executionMode" USER-DEFINED
ALTER TABLE "App" ALTER COLUMN "executionMode" DROP DEFAULT;
-- @if-type "App"."bits" ARRAY
ALTER TABLE "App" ALTER COLUMN "bits" DROP DEFAULT;
-- @if-type "AppPurchase"."status" USER-DEFINED
ALTER TABLE "AppPurchase" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "Channel"."kind" USER-DEFINED
ALTER TABLE "Channel" ALTER COLUMN "kind" DROP DEFAULT;
-- @if-type "Channel"."status" USER-DEFINED
ALTER TABLE "Channel" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "Course"."difficulty" USER-DEFINED
ALTER TABLE "Course" ALTER COLUMN "difficulty" DROP DEFAULT;
-- @if-type "Course"."category" USER-DEFINED
ALTER TABLE "Course" ALTER COLUMN "category" DROP DEFAULT;
-- @if-type "CourseAppLink"."purpose" USER-DEFINED
ALTER TABLE "CourseAppLink" ALTER COLUMN "purpose" DROP DEFAULT;
-- @if-type "CourseAsset"."kind" USER-DEFINED
ALTER TABLE "CourseAsset" ALTER COLUMN "kind" DROP DEFAULT;
-- @if-type "UserLessonProgress"."status" USER-DEFINED
ALTER TABLE "UserLessonProgress" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "SolutionRequest"."status" USER-DEFINED
ALTER TABLE "SolutionRequest" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "User"."status" USER-DEFINED
ALTER TABLE "User" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "User"."tier" USER-DEFINED
ALTER TABLE "User" ALTER COLUMN "tier" DROP DEFAULT;
-- @if-type "ExecutionRun"."status" USER-DEFINED
ALTER TABLE "ExecutionRun" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "ExecutionRun"."mode" USER-DEFINED
ALTER TABLE "ExecutionRun" ALTER COLUMN "mode" DROP DEFAULT;
-- @if-type "ExecutionRun"."runVariant" USER-DEFINED
ALTER TABLE "ExecutionRun" ALTER COLUMN "runVariant" DROP DEFAULT;
-- @if-type "WasmPackage"."status" USER-DEFINED
ALTER TABLE "WasmPackage" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "WasmPackage"."visibility" USER-DEFINED
ALTER TABLE "WasmPackage" ALTER COLUMN "visibility" DROP DEFAULT;
-- @if-type "WasmPackageVersion"."status" USER-DEFINED
ALTER TABLE "WasmPackageVersion" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "WasmPackageVersion"."compilationStatus" USER-DEFINED
ALTER TABLE "WasmPackageVersion" ALTER COLUMN "compilationStatus" DROP DEFAULT;
-- @if-type "WasmPackageVersion"."compiledPlatforms" ARRAY
ALTER TABLE "WasmPackageVersion" ALTER COLUMN "compiledPlatforms" DROP DEFAULT;
-- @if-type "WasmPackageVersion"."supportedWasmtimeVersions" ARRAY
ALTER TABLE "WasmPackageVersion" ALTER COLUMN "supportedWasmtimeVersions" DROP DEFAULT;
-- @if-type "WasmPackageInvitation"."status" USER-DEFINED
ALTER TABLE "WasmPackageInvitation" ALTER COLUMN "status" DROP DEFAULT;
-- @if-type "WasmPackagePurchase"."status" USER-DEFINED
ALTER TABLE "WasmPackagePurchase" ALTER COLUMN "status" DROP DEFAULT;

-- enum columns (56)
ALTER TABLE "AiActAssessment" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "AiActAssessment" ALTER COLUMN "riskCategory" TYPE TEXT USING "riskCategory"::TEXT;
ALTER TABLE "AiActModelObservation" ALTER COLUMN "source" TYPE TEXT USING "source"::TEXT;
ALTER TABLE "AiActModelObservation" ALTER COLUMN "posture" TYPE TEXT USING "posture"::TEXT;
ALTER TABLE "AiActModelRegistry" ALTER COLUMN "posture" TYPE TEXT USING "posture"::TEXT;
ALTER TABLE "AppConnection" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "Notification" ALTER COLUMN "type" TYPE TEXT USING "type"::TEXT;
ALTER TABLE "PushNotificationTarget" ALTER COLUMN "platform" TYPE TEXT USING "platform"::TEXT;
ALTER TABLE "PushNotificationTarget" ALTER COLUMN "provider" TYPE TEXT USING "provider"::TEXT;
ALTER TABLE "AppGroup" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "AppGroup" ALTER COLUMN "visibility" TYPE TEXT USING "visibility"::TEXT;
ALTER TABLE "AppGroupMember" ALTER COLUMN "kind" TYPE TEXT USING "kind"::TEXT;
ALTER TABLE "AppGroupMember" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "App" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "App" ALTER COLUMN "visibility" TYPE TEXT USING "visibility"::TEXT;
ALTER TABLE "App" ALTER COLUMN "primaryCategory" TYPE TEXT USING "primaryCategory"::TEXT;
ALTER TABLE "App" ALTER COLUMN "secondaryCategory" TYPE TEXT USING "secondaryCategory"::TEXT;
ALTER TABLE "App" ALTER COLUMN "appType" TYPE TEXT USING "appType"::TEXT;
ALTER TABLE "App" ALTER COLUMN "executionMode" TYPE TEXT USING "executionMode"::TEXT;
ALTER TABLE "AppCacheEntry" ALTER COLUMN "scope" TYPE TEXT USING "scope"::TEXT;
ALTER TABLE "AppPurchase" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "AppDiscount" ALTER COLUMN "discountType" TYPE TEXT USING "discountType"::TEXT;
ALTER TABLE "AuditEntry" ALTER COLUMN "actorType" TYPE TEXT USING "actorType"::TEXT;
ALTER TABLE "Channel" ALTER COLUMN "kind" TYPE TEXT USING "kind"::TEXT;
ALTER TABLE "Channel" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "PublicationRequest" ALTER COLUMN "targetVisibility" TYPE TEXT USING "targetVisibility"::TEXT;
ALTER TABLE "PublicationRequest" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "PublicationLog" ALTER COLUMN "visibility" TYPE TEXT USING "visibility"::TEXT;
ALTER TABLE "Bit" ALTER COLUMN "type" TYPE TEXT USING "type"::TEXT;
ALTER TABLE "Swimlane" ALTER COLUMN "type" TYPE TEXT USING "type"::TEXT;
ALTER TABLE "Swimlane" ALTER COLUMN "size" TYPE TEXT USING "size"::TEXT;
ALTER TABLE "Course" ALTER COLUMN "difficulty" TYPE TEXT USING "difficulty"::TEXT;
ALTER TABLE "Course" ALTER COLUMN "category" TYPE TEXT USING "category"::TEXT;
ALTER TABLE "Challenge" ALTER COLUMN "kind" TYPE TEXT USING "kind"::TEXT;
ALTER TABLE "CourseAppLink" ALTER COLUMN "purpose" TYPE TEXT USING "purpose"::TEXT;
ALTER TABLE "CourseAsset" ALTER COLUMN "kind" TYPE TEXT USING "kind"::TEXT;
ALTER TABLE "LessonAppRef" ALTER COLUMN "kind" TYPE TEXT USING "kind"::TEXT;
ALTER TABLE "UserLessonProgress" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "SolutionRequest" ALTER COLUMN "pricingTier" TYPE TEXT USING "pricingTier"::TEXT;
ALTER TABLE "SolutionRequest" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "User" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "User" ALTER COLUMN "tier" TYPE TEXT USING "tier"::TEXT;
ALTER TABLE "UserBit" ALTER COLUMN "type" TYPE TEXT USING "type"::TEXT;
ALTER TABLE "ExecutionUsageTracking" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "ExecutionRun" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "ExecutionRun" ALTER COLUMN "mode" TYPE TEXT USING "mode"::TEXT;
ALTER TABLE "ExecutionRun" ALTER COLUMN "runVariant" TYPE TEXT USING "runVariant"::TEXT;
ALTER TABLE "WasmPackage" ALTER COLUMN "primaryCategory" TYPE TEXT USING "primaryCategory"::TEXT;
ALTER TABLE "WasmPackage" ALTER COLUMN "secondaryCategory" TYPE TEXT USING "secondaryCategory"::TEXT;
ALTER TABLE "WasmPackage" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "WasmPackage" ALTER COLUMN "visibility" TYPE TEXT USING "visibility"::TEXT;
ALTER TABLE "WasmPackageVersion" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "WasmPackageVersion" ALTER COLUMN "compilationStatus" TYPE TEXT USING "compilationStatus"::TEXT;
ALTER TABLE "WasmPackageReview" ALTER COLUMN "action" TYPE TEXT USING "action"::TEXT;
ALTER TABLE "WasmPackageInvitation" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;
ALTER TABLE "WasmPackagePurchase" ALTER COLUMN "status" TYPE TEXT USING "status"::TEXT;

-- enum types (46)
DROP TYPE IF EXISTS "AiActAssessmentStatus";
DROP TYPE IF EXISTS "AiGpaiPosture";
DROP TYPE IF EXISTS "AiModelSource";
DROP TYPE IF EXISTS "AiRiskCategory";
DROP TYPE IF EXISTS "AppConnectionStatus";
DROP TYPE IF EXISTS "AppGroupMemberKind";
DROP TYPE IF EXISTS "AppGroupMemberStatus";
DROP TYPE IF EXISTS "AppType";
DROP TYPE IF EXISTS "AssetKind";
DROP TYPE IF EXISTS "AuditActorType";
DROP TYPE IF EXISTS "BitType";
DROP TYPE IF EXISTS "CacheScope";
DROP TYPE IF EXISTS "Category";
DROP TYPE IF EXISTS "ChallengeKind";
DROP TYPE IF EXISTS "ChannelMessageKind";
DROP TYPE IF EXISTS "ChannelMessageStatus";
DROP TYPE IF EXISTS "CourseAppPurpose";
DROP TYPE IF EXISTS "CourseCategory";
DROP TYPE IF EXISTS "CourseDifficulty";
DROP TYPE IF EXISTS "DiscountType";
DROP TYPE IF EXISTS "ExecutionMode";
DROP TYPE IF EXISTS "ExecutionStatus";
DROP TYPE IF EXISTS "InvitationStatus";
DROP TYPE IF EXISTS "LessonAppRefKind";
DROP TYPE IF EXISTS "LessonStatus";
DROP TYPE IF EXISTS "NotificationType";
DROP TYPE IF EXISTS "PublicationRequestStatus";
DROP TYPE IF EXISTS "PurchaseStatus";
DROP TYPE IF EXISTS "PushNotificationTargetPlatform";
DROP TYPE IF EXISTS "PushNotificationTargetProvider";
DROP TYPE IF EXISTS "RunMode";
DROP TYPE IF EXISTS "RunStatus";
DROP TYPE IF EXISTS "RunVariant";
DROP TYPE IF EXISTS "SolutionPricingTier";
DROP TYPE IF EXISTS "SolutionStatus";
DROP TYPE IF EXISTS "Status";
DROP TYPE IF EXISTS "SwimlaneSize";
DROP TYPE IF EXISTS "SwimlaneType";
DROP TYPE IF EXISTS "UserStatus";
DROP TYPE IF EXISTS "UserTier";
DROP TYPE IF EXISTS "Visibility";
DROP TYPE IF EXISTS "WasmCompilationStatus";
DROP TYPE IF EXISTS "WasmPackageCategory";
DROP TYPE IF EXISTS "WasmPackageStatus";
DROP TYPE IF EXISTS "WasmPackageVisibility";
DROP TYPE IF EXISTS "WasmReviewAction";

-- scalar array columns (20)
ALTER TABLE "Role" ALTER COLUMN "attributes" TYPE JSONB USING to_jsonb("attributes");
ALTER TABLE "App" ALTER COLUMN "bits" TYPE JSONB USING to_jsonb("bits");
ALTER TABLE "Bit" ALTER COLUMN "authors" TYPE JSONB USING to_jsonb("authors");
ALTER TABLE "Bit" ALTER COLUMN "dependencies" TYPE JSONB USING to_jsonb("dependencies");
ALTER TABLE "TemplateProfile" ALTER COLUMN "interests" TYPE JSONB USING to_jsonb("interests");
ALTER TABLE "TemplateProfile" ALTER COLUMN "tags" TYPE JSONB USING to_jsonb("tags");
ALTER TABLE "TemplateProfile" ALTER COLUMN "bitIds" TYPE JSONB USING to_jsonb("bitIds");
ALTER TABLE "TemplateProfile" ALTER COLUMN "hubs" TYPE JSONB USING to_jsonb("hubs");
ALTER TABLE "Swimlane" ALTER COLUMN "tags" TYPE JSONB USING to_jsonb("tags");
ALTER TABLE "Course" ALTER COLUMN "tags" TYPE JSONB USING to_jsonb("tags");
ALTER TABLE "Meta" ALTER COLUMN "tags" TYPE JSONB USING to_jsonb("tags");
ALTER TABLE "Meta" ALTER COLUMN "previewMedia" TYPE JSONB USING to_jsonb("previewMedia");
ALTER TABLE "Profile" ALTER COLUMN "interests" TYPE JSONB USING to_jsonb("interests");
ALTER TABLE "Profile" ALTER COLUMN "tags" TYPE JSONB USING to_jsonb("tags");
ALTER TABLE "Profile" ALTER COLUMN "bitIds" TYPE JSONB USING to_jsonb("bitIds");
ALTER TABLE "Profile" ALTER COLUMN "hubs" TYPE JSONB USING to_jsonb("hubs");
ALTER TABLE "ExecutionRun" ALTER COLUMN "callerAppChain" TYPE JSONB USING to_jsonb("callerAppChain");
ALTER TABLE "WasmPackage" ALTER COLUMN "keywords" TYPE JSONB USING to_jsonb("keywords");
ALTER TABLE "WasmPackageVersion" ALTER COLUMN "compiledPlatforms" TYPE JSONB USING to_jsonb("compiledPlatforms");
ALTER TABLE "WasmPackageVersion" ALTER COLUMN "supportedWasmtimeVersions" TYPE JSONB USING to_jsonb("supportedWasmtimeVersions");
