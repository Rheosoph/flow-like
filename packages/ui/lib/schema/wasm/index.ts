import type { WidgetContract } from "@flow-like/widget-sdk";
import type { INodePermission } from "../flow/board";

export interface PackageAuthor {
	name: string;
	email?: string;
	url?: string;
}

export interface NetworkPermissions {
	httpEnabled: boolean;
	allowedHosts: string[];
	websocketEnabled: boolean;
	tcpEnabled: boolean;
	udpEnabled: boolean;
	dnsEnabled: boolean;
}

export interface FileSystemPermissions {
	nodeStorage: boolean;
	userStorage: boolean;
	uploadDir: boolean;
	cacheDir: boolean;
}

export interface OAuthScopeRequirement {
	provider: string;
	scopes: string[];
	reason: string;
	required: boolean;
}

export enum MemoryTier {
	Minimal = "minimal",
	Light = "light",
	Standard = "standard",
	Heavy = "heavy",
	Intensive = "intensive",
	Large = "large",
	Huge = "huge",
	Extreme = "extreme",
	Maximum = "maximum",
}

export enum TimeoutTier {
	Quick = "quick",
	Standard = "standard",
	Extended = "extended",
	LongRunning = "long_running",
	VeryLong = "very_long",
	Maximum = "maximum",
}

export type WasmPackageCategory =
	| "DOCUMENT_PROCESSING"
	| "DATA_TRANSFORMATION"
	| "WORKFLOW_AUTOMATION"
	| "COMMUNICATION"
	| "ANALYTICS_REPORTING"
	| "FINANCE_BILLING"
	| "COMPLIANCE_REGULATORY"
	| "HR_PEOPLE"
	| "AI_ML"
	| "INTEGRATION_CONNECTORS"
	| "SECURITY_IDENTITY"
	| "DEVOPS"
	| "IOT_INDUSTRIAL"
	| "ROBOTICS_PHYSICAL_AI"
	| "GAMING_SIMULATION"
	| "HEALTHCARE"
	| "VETERINARY"
	| "LEGAL"
	| "MANUFACTURING"
	| "AGRICULTURE"
	| "REAL_ESTATE"
	| "LOGISTICS"
	| "ENERGY"
	| "CONSTRUCTION_TRADES"
	| "EDUCATION"
	| "GOVERNMENT_DEFENSE"
	| "ECOMMERCE"
	| "INSURANCE"
	| "TELECOM"
	| "SCIENTIFIC_ENGINEERING"
	| "GEOSPATIAL"
	| "MEDIA_CONTENT"
	| "OTHER";

export interface PackagePermissions {
	memory: MemoryTier;
	timeout: TimeoutTier;
	network: NetworkPermissions;
	filesystem: FileSystemPermissions;
	oauthScopes: OAuthScopeRequirement[];
	variables: boolean;
	cache: boolean;
	streaming: boolean;
	a2ui: boolean;
	models: boolean;
}

export interface NodeScores {
	privacy: number;
	security: number;
	performance: number;
	governance: number;
	reliability: number;
	cost: number;
}

export interface PinOptions {
	sensitive?: boolean;
	validValues?: string[];
	range?: [number, number];
	step?: number;
	enforceSchema?: boolean;
	enforceGenericValueType?: boolean;
}

export interface Pin {
	id: string;
	name: string;
	friendlyName?: string;
	pinType: "Input" | "Output";
	dataType: string;
	defaultValue?: unknown;
	options?: PinOptions;
	schema?: unknown;
	depends_on: string[];
	connected_to: string[];
}

export interface PackageNodeEntry {
	id: string;
	name: string;
	friendlyName?: string;
	description: string;
	category: string;
	icon?: string;
	scores?: NodeScores;
	pins: Record<string, Pin>;
	start?: boolean;
	longRunning?: boolean;
	docs?: string;
	eventCallback?: boolean;
	oauthProviders: string[];
	requiredOauthScopes?: Record<string, string[]>;
	onlyOffline: boolean;
	version?: number;
	permissions: INodePermission[];
	metadata: Record<string, unknown>;
}

/**
 * Micro-frontend widget entry in a package manifest (manifest v2).
 * Carries the full typed contract so previews and pin generation work from
 * the manifest alone, without opening the widget bundle.
 */
export interface PackageWidgetEntry {
	id: string;
	name: string;
	description: string;
	/** Icon (base64 data URI or URL) */
	icon?: string | null;
	/** Preview thumbnail (base64 data URI or URL) */
	thumbnail?: string | null;
	contract: WidgetContract;
	keywords?: string[];
}

export interface PackageManifest {
	manifestVersion: number;
	id: string;
	name: string;
	version: string;
	description: string;
	authors: PackageAuthor[];
	license?: string;
	repository?: string;
	homepage?: string;
	permissions: PackagePermissions;
	keywords: string[];
	primaryCategory?: WasmPackageCategory;
	secondaryCategory?: WasmPackageCategory;
	minFlowLikeVersion?: string;
	wasmPath?: string;
	wasmHash?: string;
	/** Micro-frontend widgets shipped by this package (manifest v2) */
	widgets?: PackageWidgetEntry[];
	/** Widget bundle (`.flwb`) path relative to the manifest */
	widgetBundlePath?: string;
	/** SHA-256 hash of the widget bundle */
	widgetBundleHash?: string;
	metadata: Record<string, unknown>;
}

export enum PackageStatus {
	Active = "active",
	Deprecated = "deprecated",
	PendingReview = "pending_review",
	Disabled = "disabled",
	Rejected = "rejected",
	Yanked = "yanked",
}

export interface PackageReviewer {
	userId: string;
	username?: string;
	name?: string;
	avatar?: string;
	role?: string;
}

export interface PackageSource {
	type: "local" | "remote";
	path?: string;
	registryUrl?: string;
	downloadUrl?: string;
}

export interface PackageVersion {
	version: string;
	wasmHash: string;
	wasmSize: number;
	status?: PackageStatus;
	downloadUrl?: string;
	publishedAt: string;
	minFlowLikeVersion?: string;
	releaseNotes?: string;
	yanked: boolean;
	/** Widget bundle (`.flwb`) sha256, when this version ships widgets */
	widgetBundleHash?: string | null;
	/** Widget bundle size in bytes, when this version ships widgets */
	widgetBundleSize?: number | null;
}

export interface RegistryEntry {
	id: string;
	manifest: PackageManifest;
	nodes: PackageNodeEntry[];
	versions: PackageVersion[];
	status: PackageStatus;
	downloadCount: number;
	createdAt: string;
	updatedAt: string;
	source: PackageSource;
	verified: boolean;
	price: number;
	visibility: string;
	currentUserPermission?: number;
	avgRating?: number | null;
	ratingCount?: number;
	ratingSum?: number;
}

export interface CachedPackage {
	entry: RegistryEntry;
	wasmData: number[];
	cachedAt: string;
	expiresAt?: string;
}

export interface PackageSummary {
	id: string;
	name: string;
	description: string;
	latestVersion: string;
	downloadCount: number;
	status: PackageStatus;
	keywords: string[];
	verified: boolean;
	price: number;
	visibility: string;
	primaryCategory?: WasmPackageCategory;
	secondaryCategory?: WasmPackageCategory;
	metadata?: MetaSummary;
	avgRating?: number | null;
	ratingCount?: number;
	/**
	 * Capability tags derived from the package's declared permissions, most
	 * sensitive first (`net.http`, `oauth`, `models`, `storage.user`, …).
	 * Absent on registries that predate the field.
	 */
	capabilities?: string[];
}

export interface SearchResults {
	packages: PackageSummary[];
	totalCount: number;
	offset: number;
	limit: number;
}

export interface InstalledPackage {
	id: string;
	version: string;
	source: PackageSource;
	installedAt: string;
	wasmPath: string;
	manifest: PackageManifest;
	metadata?: MetaSummary;
	wasmHash?: string;
}

export interface SearchFilters {
	query?: string;
	category?: string;
	keywords?: string[];
	author?: string;
	verifiedOnly?: boolean;
	includeDeprecated?: boolean;
	includeDisabled?: boolean;
	sortBy?: "relevance" | "name" | "downloads" | "updated_at" | "created_at";
	sortDesc?: boolean;
	offset?: number;
	limit?: number;
	language?: string;
	includeOwn?: boolean;
	ownedOnly?: boolean;
}

export interface PackageUpdate {
	packageId: string;
	packageName: string;
	currentVersion: string;
	latestVersion: string;
	releaseNotes?: string;
}

// Admin types for package management

export type PackageAdminStatus =
	| "pending_review"
	| "active"
	| "rejected"
	| "deprecated"
	| "disabled";

export interface PackageDetails {
	id: string;
	name: string;
	description: string;
	version: string;
	authors: string[];
	license?: string;
	homepage?: string;
	repository?: string;
	keywords: string[];
	status: PackageAdminStatus;
	visibility: PackageVisibility;
	verified: boolean;
	downloadCount: number;
	wasmSize: number;
	nodes: PackageNodeEntry[];
	permissions: PackagePermissions;
	price: number;
	primaryCategory?: WasmPackageCategory;
	secondaryCategory?: WasmPackageCategory;
	createdAt: string;
	updatedAt: string;
	publishedAt?: string;
	readme?: string;
	submitterId?: string;
}

export type ReviewAction =
	| "submitted"
	| "approve"
	| "reject"
	| "request_changes"
	| "comment"
	| "flag";

export interface PackageReview {
	id: string;
	packageId: string;
	reviewerId: string;
	reviewer?: PackageReviewer;
	action: ReviewAction;
	comment?: string;
	securityScore?: number;
	codeQualityScore?: number;
	documentationScore?: number;
	createdAt: string;
}

export interface ReviewRequest {
	action: "approve" | "reject" | "request_changes" | "comment" | "flag";
	comment?: string;
	internalNote?: string;
	securityScore?: number;
	codeQualityScore?: number;
	documentationScore?: number;
}

export interface RegistryStats {
	totalPackages: number;
	totalVersions: number;
	totalDownloads: number;
	pendingReview: number;
	activePackages: number;
	rejectedPackages: number;
	verifiedPackages: number;
}

export interface AdminPackageListResponse {
	packages: PackageDetails[];
	totalCount: number;
	offset: number;
	limit: number;
}

export interface AdminPackageDetailResponse {
	package: PackageDetails;
	reviews: PackageReview[];
}

export interface AdminEnsureWasmArtifactsFailure {
	packageId: string;
	version: string;
	message: string;
}

export interface AdminEnsureWasmArtifactsResponse {
	targetPlatform: string;
	wasmtimeVersion: string;
	activePackages: number;
	checkedVersions: number;
	skippedVersions: number;
	alreadyAvailable: number;
	alreadyPending: number;
	jobsStarted: number;
	failed: number;
	failures: AdminEnsureWasmArtifactsFailure[];
}

// Package visibility and compilation
export type PackageVisibility = "private" | "public" | "public_request_access";

export type CompilationStatus = "compiled" | "local_only" | "pending";

export type InvitationStatus = "pending" | "accepted" | "rejected" | "expired";

// Package user/team management
export type PackagePermissionLevel = "owner" | "maintainer" | "user" | "buyer";

export interface PackageUser {
	id: string;
	userId: string;
	username?: string;
	name?: string;
	avatar?: string;
	permission: number;
	grantedAt: string;
}

export interface PackageInvitation {
	id: string;
	packageId: string;
	inviteeId: string;
	invitedById: string;
	permission: number;
	status: InvitationStatus;
	createdAt: string;
	expiresAt?: string;
}

export interface InviteUserRequest {
	inviteeId: string;
	permission: number;
}

export interface UpdateUserPermissionRequest {
	permission: number;
}

// Two-step publish flow
export interface UploadUrlResponse {
	uploadUrl: string;
	tmpPath: string;
	expiresInSecs: number;
}

export interface TwoStepPublishRequest {
	manifest: PackageManifest;
	tmpPath: string;
}

// Recompile
export interface RecompileRequest {
	packageId: string;
	version: string;
}

export interface RecompileResponse {
	success: boolean;
	message: string;
}

export interface MetaSummary {
	lang: string;
	name: string;
	description: string;
	icon?: string;
	thumbnail?: string;
}

export interface PackageMeta {
	id: string;
	lang: string;
	name: string;
	description?: string;
	longDescription?: string;
	tags?: string[];
	icon?: string;
	thumbnail?: string;
	website?: string;
	supportUrl?: string;
	docsUrl?: string;
	useCase?: string;
	releaseNotes?: string;
	previewMedia?: string[];
	ageRating?: number;
}

export interface UpsertPackageMetaRequest {
	name: string;
	description?: string;
	longDescription?: string;
	tags?: string[];
	website?: string;
	supportUrl?: string;
	docsUrl?: string;
	useCase?: string;
	releaseNotes?: string;
	ageRating?: number;
}

export interface PushMediaResponse {
	signed_url: string;
}

// App package management
export interface AppPackage {
	id: string;
	appId: string;
	packageId: string;
	packageName?: string;
	version: string;
	autoUpdate: boolean;
	addedAt: string;
	stale: boolean;
	metadata?: MetaSummary;
}

export interface AddAppPackageRequest {
	packageId: string;
	version: string;
	autoUpdate: boolean;
}

export interface UpdateAppPackageRequest {
	version?: string;
	autoUpdate?: boolean;
}

// Extended PackageVersion with compilation info
export interface PackageVersionExtended extends PackageVersion {
	compilationStatus: CompilationStatus;
	compiledPlatforms: string[];
	compilationError?: string;
	duplicateOfPackageId?: string;
	duplicateOfVersion?: string;
	duplicateFlagged: boolean;
	nodes: PackageNodeEntry[];
}

// Extended PackageDetails with visibility
export interface PackageDetailsExtended extends PackageDetails {
	visibility: PackageVisibility;
	readme?: string;
	users: PackageUser[];
}

// Purchase and access request types

export interface WasmPurchaseParams {
	successUrl?: string;
	cancelUrl?: string;
}

export interface WasmPurchaseResponse {
	checkoutUrl?: string;
	alreadyHasAccess: boolean;
	packageId: string;
}

export interface RequestAccessParams {
	comment?: string;
}

export interface RequestAccessResponse {
	granted: boolean;
	queued: boolean;
	requiresPurchase: boolean;
	packageId: string;
}

export interface AccessRequest {
	id: string;
	userId: string;
	packageId: string;
	comment?: string;
	createdAt: string;
}

// Package comment / review types

export interface PackageCommentItem {
	id: string;
	text: string;
	rating: number;
	userId: string;
	userName?: string;
	userAvatar?: string;
	createdAt: string;
	updatedAt: string;
}

export interface PackageCommentsResponse {
	comments: PackageCommentItem[];
	total: number;
	offset: number;
	limit: number;
}

export interface UpsertPackageCommentRequest {
	text: string;
	rating: number;
}

export interface UpsertPackageCommentResponse {
	commentId: string;
}
