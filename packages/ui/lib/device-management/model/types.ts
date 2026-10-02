import type { DeviceResources } from "../../device-resources";
import type { DeviceStatus } from "../../devices";
import type { AcmeCertificate } from "../certificate-acme";
import type { LocalCertificateAuthority } from "../certificate-authority";
import type {
	CertificateIssuer,
	CertificateRequest,
} from "../certificate-issuance";
import type {
	CertificateInventory,
	PublicCertificateInventory,
} from "../certificates";
import type { DeploymentRolloutStatus } from "../deployment";
import type { OfflineQueueStatus } from "../offline-queue";
import type { DeviceSetupReadiness } from "../readiness";
import type {
	Capability,
	DeviceReceipt,
	Inspection,
	InventoryScope,
	ManagementPolicy,
	PlacementStatus,
	PolicyView,
} from "../types";
import type {
	ActivityItem,
	FleetDeviceState,
	KeySessionSnapshot,
	LiveInspection,
	LiveState,
	LocalSummary,
} from "../workspace/types";

/* Copy references (P7): the model returns codes, DV/copy maps them to t(). */

export type CopyParams = Record<string, string | number>;
export interface CopyRef<C extends string> {
	code: C;
	params?: CopyParams;
}

/* Freshness (M-DATA §3.6, IA §2). CA9: byte-identical to the FreshnessStamp props. */

export const SOURCE_PLANES = [
	"hub",
	"snap",
	"saved",
	"live",
	"local",
	"device",
] as const;
export type SourcePlane = (typeof SOURCE_PLANES)[number];

export const AGE_STATES = [
	"live",
	"current",
	"delayed",
	"lastknown",
	"snapshot",
	"locked",
	"notloaded",
	"noaccess",
	"unsupported",
	"error",
] as const;
export type AgeState = (typeof AGE_STATES)[number];

export type FreshnessReason =
	| "refresh_failed"
	| "network"
	| "timeout"
	| "server_error"
	| "rate_limited"
	| "integrity"
	| "access_ended"
	| "session_closed"
	| "device_unreachable"
	| "rejected"
	| "needs_capability"
	| "needs_device_scope"
	| "owner_only"
	| "not_a_reader"
	| "role_missing"
	| "agent_update_needed"
	| "needs_linux"
	| "hub_update_needed"
	| "plan_no_history"
	| "browser_unsupported"
	| "never_reported"
	| "only_on_device"
	| "unlock_required"
	| "no_keys_here"
	| "connect_required";

export interface Freshness {
	src: SourcePlane;
	age: AgeState;
	/** Unix seconds, hub-corrected. */
	at?: number;
	cadenceS?: number;
	/** With `age === "error"`: when the data shown was read. */
	dataFrom?: number;
	error?: CopyRef<FreshnessReason> & { retryAt?: number };
	/** Set when |skew| > 120 s or the age is negative. */
	skewS?: number;
	/** Why `noaccess` / `unsupported` / `notloaded`. */
	reason?: CopyRef<FreshnessReason>;
}

export type FreshnessSignal =
	| "device_row"
	| "presence"
	| "cert_inventory"
	| "hub_support"
	| "readiness"
	| "policy"
	| "resources"
	| "archive_list"
	| "fleet_status"
	| "fleet_metrics"
	| "saved_inventory"
	| "live_inspection"
	| "live_metrics"
	| "project_metrics"
	| "logs"
	| "certificates_live"
	| "offline_queues"
	| "rollout"
	| "host_operation"
	| "session"
	| "local_vault"
	| "agent_local";

/* Hub rows (CA3) and hub mirrors (CA6, CA12; plan §3.2). */

export type DeviceIdentity = DeviceReceipt["identity"];
export type DeviceRelationship = "owner" | "shared" | "cloud_approval";
export type Relationship = DeviceRelationship | "none" | "unknown";

export interface DeviceAuthRejection {
	code: "clock_skew" | "revoked_credential";
	skew_seconds: number | null;
	count: number;
	first_at: number;
	last_at: number;
}

/** Client view of a hub device row. Every field beyond `DeviceStatus` is absent on older hubs. */
export interface DeviceRow extends DeviceStatus {
	identity: DeviceIdentity;
	display_name?: string | null;
	relationship?: DeviceRelationship;
	access_expires_at?: number | null;
	access_rules_expire_at?: number | null;
	revoked_at?: number | null;
	cloud_approvals?: {
		resource_grants: number;
		billing_grants: number;
		expires_at: number;
	} | null;
	auth_rejection?: DeviceAuthRejection | null;
}

export type HubErrorCode =
	| "network"
	| "timeout"
	| "unauthorized"
	| "forbidden"
	| "token_restricted"
	| "not_found"
	| "rate_limited"
	| "server_error"
	| "invalid_response";

export interface DeviceLimits {
	max_devices: number;
	max_pending_enrollments: number;
	enrollment_ttl_seconds: number;
	max_enrollments_per_day: number;
	max_account_backups: number;
}
export interface DeviceUsage {
	active_devices: number;
	revoked_devices: number;
	pending_enrollments: number;
	enrollments_last_24h: number;
	account_backups: number;
}
/** E6 `GET /devices/usage`. */
export interface DeviceUsageResponse {
	server_time: number;
	limits: DeviceLimits;
	usage: DeviceUsage;
}

export interface HubDeviceSupport {
	state: "checking" | "on" | "off" | "unreachable";
	error?: CopyRef<HubErrorCode>;
	limits?: DeviceLimits;
	usage?: DeviceUsage;
	/** What the hub JSON states (BG3 interim "limits without usage" on hubs without `GET /devices/usage`). */
	configuredLimits?: Partial<
		Pick<
			DeviceLimits,
			"max_devices" | "max_pending_enrollments" | "enrollment_ttl_seconds"
		>
	>;
	serverTime?: number;
	releaseTrust?: unknown;
	telemetryTiers?: unknown;
}

/** E5 `GET /devices/enrollments`. */
export interface HubEnrollment {
	enrollment_id: string;
	device_id: string;
	name: string;
	state: "pending" | "expired" | "cancelled";
	created_at: number;
	expires_at: number;
	controller_key_thumbprint: string;
}

/** E7 `GET /devices/controller-vaults` (no ciphertext). */
export interface AccountBackupList {
	vaults: {
		key_id: string;
		revision: number;
		updated_at: number;
		public_key_thumbprint: string;
	}[];
	used: number;
	max: number;
}

/** E8 `GET /devices/{id}/management/my-access`. */
export interface MyAccess {
	device_id: string;
	role: "owner" | "grantee";
	owner_id: string;
	policy_version: number;
	policy_expires_at: number | null;
	applied_version: number;
	applied: boolean;
	grants: {
		grant_id: string;
		scope: InventoryScope;
		capabilities: Capability[];
		expires_at: number;
		controller_key_thumbprint: string;
		group_id: string | null;
	}[];
}

/** E13 `GET /devices/certificate-inventory` row. */
export interface FleetCertificateInventoryRow
	extends PublicCertificateInventory {
	device_id: string;
}

/** E14 `GET /devices/archive-usage`. */
export interface ArchiveUsage {
	tier: string;
	max_bytes: number;
	retention_seconds: number;
	used_bytes: number;
	devices: {
		device_id: string;
		used_bytes: number;
		segments: number;
		oldest_created_at: number | null;
		newest_expires_at: number | null;
	}[];
}

export type EffectiveLimit = "approval" | "sharing_grant" | "access_rules";
export type OnlineAccess = "read_only" | "read_write";
export type GrantStatus = "active" | "revoked";

/** E19 `GET /devices/resource-summary`. */
export interface ResourceSummary {
	server_time: number;
	devices: {
		device_id: string;
		approvals: {
			grant_id: string;
			placement_id: string;
			app_id: string | null;
			status: GrantStatus;
			expires_at: number;
			effective_expires_at: number;
			effective_limit: EffectiveLimit;
			online_access: OnlineAccess | null;
			online_write_blocked: "storage_full" | null;
			payer_is_me: boolean;
			approver_is_me: boolean;
		}[];
		billing: {
			billing_grant_id: string;
			grant_id: string;
			limit_micros: number;
			used_micros: number;
			reserved_micros: number;
			expires_at: number;
			payer_is_me: boolean;
		}[];
	}[];
}

/** E20 `GET /apps/{app_id}/device-placements`. */
export interface AppDevicePlacements {
	server_time: number;
	placements: {
		device_id: string;
		placement_id: string;
		deployment_id: string;
		relationship: DeviceRelationship;
		grant: {
			grant_id: string;
			status: GrantStatus;
			expires_at: number;
			effective_expires_at: number;
			effective_limit: EffectiveLimit;
			online_access: OnlineAccess | null;
			model_ids: string[];
			max_instances: number;
			approved_by_user_id: string | null;
			created_at: number | null;
		};
		billing: {
			billing_grant_id: string;
			limit_micros: number;
			used_micros: number;
			reserved_micros: number;
			expires_at: number;
			payer_is_me: boolean;
		} | null;
		instances: { active: number; newest_lease_expires_at: number | null };
	}[];
}

/* Agent facts (CA4, plan §3.4). Every field is absent on older agents. */

export const AGENT_FEATURES = [
	"placement_diagnostics",
	"task_health",
	"placement_events",
	"offline_summary",
	"host_operation",
	"network_interfaces",
	"rollout_history",
	"operations",
	"metrics_history",
	"offline_lookup",
	"reader_bindings",
	"acme_failure_detail",
	"archive_status",
	"artifact_capacity",
] as const;
export type AgentFeature = (typeof AGENT_FEATURES)[number];
export type AgentFeatures = Partial<Record<AgentFeature, 1>>;

export interface ReplicaRestarts {
	failures: number;
	max_restarts: number;
	crash_looping: boolean;
	retry_in_seconds: number | null;
	last_started_at: number | null;
}

export interface PlacementEvent {
	event_id: string;
	event_version: [number, number, number];
	board_version: [number, number, number];
}

export interface OfflineWritesSummary {
	scopes: number;
	pending_count: number;
	pending_bytes: number;
	oldest_at: number | null;
	quarantined_scopes: number;
	needs_attention: number;
	mirror_error: boolean;
}

export type ReplicaStatus = NonNullable<PlacementStatus["replicas"]>[number];
export interface ReplicaStatusPlus extends ReplicaStatus {
	process_id?: number | null;
	last_error?: string | null;
	/** Sent instead of `last_error` to Status-only readers and in status snapshots. */
	has_error?: boolean;
	restarts?: ReplicaRestarts;
}

export interface PlacementStatusPlus extends Omit<PlacementStatus, "replicas"> {
	replicas?: ReplicaStatusPlus[];
	process_id?: number | null;
	last_error?: string | null;
	has_error?: boolean;
	restarts?: ReplicaRestarts;
	offline_writes?: OfflineWritesSummary;
	source?: "offline" | "online";
	events?: PlacementEvent[];
	events_truncated?: boolean;
	online_metadata_sha256?: string | null;
}

export interface TaskHealth {
	/** snake_case and open-ended (`fleet_publisher`, `management_transport`, …). */
	name: string;
	/** `stopped`: the task ended, failed for good or panicked. */
	state: "ok" | "failing" | "stopped";
	since: number;
	/** Absent in status snapshots (it would change on every failed pass). */
	consecutive_failures?: number;
	category?:
		| "hub_unreachable"
		| "hub_refused"
		| "storage"
		| "policy"
		| "internal"
		| null;
}

export type HostOperationState =
	| "pending"
	| "requesting"
	| "requested"
	| "draining"
	| "staging"
	| "completed"
	| "rolled_back"
	| "failed"
	| "unknown"
	| (string & {});

export interface HostOperationView {
	operation_id: string;
	kind: "reboot" | "update_agent";
	state: HostOperationState;
	created_at: number;
	issued_by: "you" | "owner" | "another_person";
}

export interface NetworkInterface {
	name: string;
	addresses: string[];
	loopback: boolean;
}

export type HostIsolationMode = "required" | "optional" | "none";
export interface HostIsolationFacts {
	platform: string;
	sandbox_available: boolean;
	require_isolation: boolean;
	placement_preflight_required?: boolean;
	network_boundary?: string;
	disk_requirement?: string;
	landlock_abi?: number | null;
	reason?: string | null;
}

export interface AgentRelease {
	version: string;
	release_version: string | null;
	release_sequence: number | null;
}

export interface InspectionPlus extends Omit<Inspection, "placements"> {
	placements: PlacementStatusPlus[];
	features: AgentFeatures;
	agentVersion?: string;
	hostOperations?: { reboot: boolean; update_agent: boolean };
	hostIsolation?: HostIsolationMode | null;
	isolation?: HostIsolationFacts | null;
	agent?: AgentRelease;
	host?: { booted_at: number | null; agent_started_at: number };
	tasks?: TaskHealth[];
	hostOperation?: HostOperationView | null;
	network?: { interfaces: NetworkInterface[] };
}

/** BG30 recording state per (scope, kind). */
export interface ArchiveRecordingStatus {
	state: "recording" | "paused";
	reason:
		| "rules_expired"
		| "rules_changed"
		| "roster_expired"
		| "outbox_full"
		| "quota_reached"
		| "tier_without_history"
		| null;
	since: number;
}

/* Routing (CA7, plan §2.6). Wire values equal the union members. */

export type DevicesScope = { kind: "account" } | { kind: "app"; appId: string };

export const FLEET_VIEWS = ["devices", "services"] as const;
export type FleetView = (typeof FLEET_VIEWS)[number];
export const FLEET_FILTERS = [
	"critical",
	"attention",
	"unknown",
	"healthy",
	"offline",
	"revoked",
	"locked",
] as const;
export type FleetFilter = (typeof FLEET_FILTERS)[number];
export const DEVICE_TABS = [
	"overview",
	"services",
	"activity",
	"metrics",
	"certificates",
	"access",
	"keys",
	"settings",
] as const;
export type DeviceTab = (typeof DEVICE_TABS)[number];
export const SERVICE_TABS = [
	"status",
	"activity",
	"metrics",
	"configuration",
	"endpoint",
	"cloud",
	"offline",
] as const;
export type ServiceTab = (typeof SERVICE_TABS)[number];
export const SETUP_STEPS = [0, 1, 2, 3, 4, 5, 6, 7] as const;
export type SetupStep = (typeof SETUP_STEPS)[number];
export const DEPLOY_STEP_IDS = [
	"what",
	"how",
	"where",
	"settings",
	"endpoint",
	"access_cost",
	"copy_upload",
	"review",
	"rollout",
] as const;
export type DeployStepId = (typeof DEPLOY_STEP_IDS)[number];
export const ACCESS_TABS = ["people", "shared", "cloud"] as const;
export type AccessTab = (typeof ACCESS_TABS)[number];
export const CERTIFICATES_TABS = [
	"expiry",
	"authorities",
	"reminders",
] as const;
export type CertificatesTab = (typeof CERTIFICATES_TABS)[number];

export interface FleetRoute {
	screen: "fleet";
	view: FleetView;
	filter?: FleetFilter;
	q?: string;
	focus?: "attention";
}
export interface DeviceRoute {
	screen: "device";
	deviceId: string;
	/** Absent = legacy link: resolved to Certificates or Overview after the first fleet load. */
	tab?: DeviceTab;
	certificateId?: string;
	action?: "revoke" | "rename";
}
export interface ServiceRoute {
	screen: "service";
	deviceId: string;
	serviceId: string;
	tab?: ServiceTab;
	stream?: "errors";
}
export interface SetupRoute {
	screen: "setup";
	enrollmentId?: string;
	step?: SetupStep;
}
/** Account scope: one `device`, optional `appId`. App scope: `mode`, `eventId`, repeatable devices, `from`. */
export interface DeployRoute {
	screen: "deploy";
	deviceIds: string[];
	appId?: string;
	serviceId?: string;
	mode?: "new" | "update";
	eventId?: string;
	from?: "events";
	step?: DeployStepId;
}
export interface AccessRoute {
	screen: "access";
	tab?: AccessTab;
	import?: "request" | "connection";
	action?: "add-people" | "request";
}
export interface CertificatesRoute {
	screen: "certificates";
	tab?: CertificatesTab;
	action?: "create-authority";
}
export interface KeysRoute {
	screen: "keys";
	guide?: "new-computer" | "forgot-password";
	focusDeviceId?: string;
}
export interface HubRoute {
	screen: "hub";
}
export interface AppDevicesRoute {
	screen: "app-devices";
	by: "device" | "event";
	focusDeviceId?: string;
	eventId?: string;
	action?: "update-all";
}

export type AccountRoute =
	| FleetRoute
	| DeviceRoute
	| ServiceRoute
	| SetupRoute
	| DeployRoute
	| AccessRoute
	| CertificatesRoute
	| KeysRoute
	| HubRoute;
export type AppRoute =
	| AppDevicesRoute
	| DeviceRoute
	| ServiceRoute
	| DeployRoute;
export type DevicesRoute = AccountRoute | AppRoute;
export type NavTarget = DevicesRoute;

/* Presence and convergence (M-DATA §3.6, §3.12). */

export type PresenceKind = "online" | "late" | "offline" | "never" | "revoked";
export interface Presence {
	kind: PresenceKind;
	/** Unix seconds: last check-in, or revocation when known. */
	since?: number;
}

export type Convergence =
	| "converged"
	| "converging"
	| "crash_looping"
	| "stopped_by_user"
	| "update_in_progress"
	| "failed_stopped"
	| "unknown";

/* Gates (M-DATA §3.7, IA §3.1–§3.3, SPEC §6.3). */

export const GATE_IDS = [
	"G0",
	"G1",
	"G2",
	"G3",
	"G4",
	"G5",
	"G6",
	"G7",
	"G8",
	"G9",
	"G10",
	"G11",
	"G12",
	"G13",
] as const;
export type GateId = (typeof GATE_IDS)[number];

/** SPEC §4.5 / §6.3 gate notice kinds (12). */
export type GateNoticeKind =
	| "locked"
	| "nokeys"
	| "noaccess"
	| "owner"
	| "unsupported"
	| "policy"
	| "plan"
	| "role"
	| "platform"
	| "hub"
	| "live"
	| "busy";

export type ActionId =
	| "view_list"
	| "view_cert_summary"
	| "revoke_device"
	| "rename_device"
	| "setup_device"
	| "cancel_setup"
	| "request_access"
	| "fleet_monitor"
	| "saved_inventory"
	| "connect_live"
	| "refresh_status"
	| "start"
	| "stop"
	| "restart"
	| "scale"
	| "remove_service"
	| "upload_revision"
	| "create_service"
	| "update_service"
	| "update_with_checks"
	| "activate_staged"
	| "discard_staged"
	| "set_secret"
	| "change_tls"
	| "advanced_config"
	| "live_metrics"
	| "project_metrics"
	| "logs"
	| "messages"
	| "check_unconfirmed"
	| "group_metrics_read"
	| "approve_metric_readers"
	| "reset_metric_reader"
	| "approve_history_readers"
	| "history_read_device"
	| "history_read_cloud"
	| "share_access"
	| "agent_update"
	| "reboot"
	| "certificates_list"
	| "certificate_import"
	| "certificate_delete"
	| "csr_create"
	| "csr_install"
	| "sign_with_org_ca"
	| "renewal_delegation"
	| "acme_configure"
	| "org_ca_manage"
	| "certificate_reminders"
	| "offline_queue_read"
	| "offline_queue_retry"
	| "offline_queue_skip"
	| "cloud_access_create"
	| "cloud_access_revoke"
	| "spending_limit_create"
	| "spending_limit_revoke"
	| "account_backup_save"
	| "account_backup_restore"
	| "import_key_file"
	| "download_key_file"
	| "change_device_password"
	| "delete_local_keys"
	| "forget_identity";

/** One code per IA §3.3 reason; `DV/copy/gate-copy.ts` maps each to a literal t() call. */
export type GateReason =
	// G0 platform
	| "desktop_only"
	| "desktop_only_offline_app"
	| "browser_size_limit"
	// G1–G3 sign-in, hub, token
	| "sign_in"
	| "hub_checking"
	| "hub_devices_off"
	| "hub_unreachable"
	| "token_restricted"
	// G4 relationship
	| "not_shared_with_you"
	| "owner_only_device"
	| "device_revoked"
	| "device_not_active"
	| "access_ended"
	// G5 capability × scope
	| "needs_capability"
	| "needs_device_scope"
	| "unlock_to_check_permissions"
	// G6 keys
	| "no_keys_here"
	| "keys_unusable"
	// G7 unlocked
	| "locked_status"
	| "locked_logs"
	| "locked_metrics"
	| "locked_change"
	| "locked_access"
	| "locked_keys"
	| "locked_certificates"
	| "unlocking"
	| "held_elsewhere"
	| "password_required"
	| "lock_unsupported"
	| "crypto_unavailable"
	| "identity_blocked"
	// G8 session
	| "offline_needs_live"
	| "never_connected_needs_live"
	| "connect_first"
	| "connecting"
	| "connection_failed"
	| "connection_not_configured"
	// G9 agent features
	| "agent_update_needed"
	| "agent_features_unknown"
	| "needs_linux_systemd"
	| "rollout_source_unsupported"
	// G10 host policy
	| "sandbox_required"
	| "sandbox_unavailable"
	// G11 plan
	| "plan_no_history"
	// G12 app role
	| "role_read_boards"
	| "role_admin_execute"
	| "role_app_owner"
	// G13 owner powers
	| "owner_only"
	| "owner_keys_elsewhere"
	// action preconditions (busy)
	| "rollout_in_progress"
	| "wait_for_rollout"
	| "secret_pending"
	| "host_operation_running"
	| "service_must_be_stopped"
	| "service_must_be_running"
	| "single_instance_only"
	| "no_staged_update"
	| "different_source"
	| "release_trust_missing"
	| "certificate_in_use"
	| "certificate_not_valid_now"
	| "config_too_large"
	| "queue_quarantined"
	| "cloud_approval_required"
	| "approval_exists"
	| "no_models_in_approval"
	| "payer_only"
	| "delegator_or_owner_only"
	| "operation_too_old"
	| "policy_not_applied"
	| "not_a_reader"
	| "access_slots_full"
	| "readiness_failing"
	| "device_limit_reached"
	| "pending_setup_limit_reached"
	| "no_placement_in_app"
	| "authority_missing";

export type FixAction =
	| { kind: "sign_in" }
	| { kind: "use_full_token" }
	| { kind: "open_hub_status" }
	| { kind: "unlock"; deviceId: string; connectLive?: boolean }
	| { kind: "take_over"; deviceId: string }
	| { kind: "restore_keys"; deviceId: string }
	| { kind: "import_key_file"; deviceId: string }
	| { kind: "request_access"; deviceId: string }
	| { kind: "ask_to_renew"; deviceId: string }
	| { kind: "connect"; deviceId: string }
	| { kind: "diagnose"; deviceId: string; serviceId?: string }
	| { kind: "update_agent"; deviceId: string }
	| {
			kind: "ask_owner";
			deviceId: string;
			need: Capability[];
			scope?: InventoryScope;
	  }
	| { kind: "use_desktop" }
	| { kind: "see_plans" }
	| { kind: "keep_keys_safely" }
	| { kind: "review_identity"; deviceId: string }
	| { kind: "forget_identity"; deviceId: string }
	| { kind: "fix_clock"; deviceId?: string };

export interface GateFeatures {
	flags: AgentFeatures;
	hostOperations?: { reboot: boolean; update_agent: boolean };
	certificateManagement?: boolean;
	certificateIssuance?: boolean;
	certificateAcme?: boolean;
	rolloutSources?: readonly ("offline" | "online")[];
	/** Copy parameter only: G9 never compares versions (`agent_version` is a constant). */
	agentVersion?: string;
	/** `isolation.platform`, e.g. "linux". */
	os?: string;
	source: Freshness;
}

export interface GrantedCapabilities {
	scope: InventoryScope;
	caps: readonly Capability[];
	expiresAt?: number;
}

/** Action-specific preconditions (IA §3.3 "Other"). Absent = not known, never "fails". */
export interface GateExtra {
	activeRollout?: boolean;
	rolloutDeadlineAt?: number;
	rolloutState?: DeploymentRolloutStatus["state"];
	/** The revision's source; G9 checks it against `GateFeatures.rolloutSources`. */
	source?: "offline" | "online";
	sameSource?: boolean;
	/** The isolation profile the form currently holds (G10). */
	isolationProfile?: "trusted_process" | "linux_sandbox";
	pendingSecret?: boolean;
	hostOperationActive?: boolean;
	desiredState?: string;
	observedState?: string;
	maxReplicas?: number;
	offlineLocalApp?: boolean;
	browserUploadBytes?: { largestFile: number; total: number };
	configBytes?: number;
	bindingCount?: number;
	certificateValidNow?: boolean;
	queueQuarantined?: boolean;
	releaseTrust?: boolean;
	operationIssuedAt?: number;
	policyApplied?: boolean;
	rosterMember?: boolean;
	grantCount?: number;
	readinessOk?: boolean;
	hadPlacementInApp?: boolean;
	authorityPresent?: boolean;
	approvalExists?: boolean;
	approvalHasModels?: boolean;
	payerIsMe?: boolean;
	delegatorIsMe?: boolean;
}

export interface GateContext {
	/** Hub-corrected unix seconds. */
	now: number;
	platform: "desktop" | "web";
	auth: { signedIn: boolean; tokenScopeAll: boolean };
	hub: HubDeviceSupport;
	device?: DeviceRow;
	relationship: Relationship;
	/** Owner ⇒ all; otherwise BG22 `my-access`, else `policy.myGrant` after unlock; undefined = unknown. */
	capabilities?: readonly GrantedCapabilities[];
	target?: { projectId?: string; placementId?: string };
	keys?: KeySessionSnapshot;
	live?: LiveState;
	features?: GateFeatures;
	hostIsolation?: HostIsolationMode;
	planTier?: { storesHistory: boolean; tierName?: string };
	projectRole?: {
		readBoards: boolean;
		admin: boolean;
		executeBoards: boolean;
		owner: boolean;
	};
	/** G13: invitation vault on this computer and `owner_id === me`. */
	ownerPowers: boolean;
	labels?: { owner?: string; service?: string };
	extra?: GateExtra;
}

export type GateFailure = {
	ok: false;
	gate: GateId;
	kind: GateNoticeKind;
	/** Only when the control cannot apply at all (R7), e.g. reboot on macOS. */
	hide: boolean;
	copy: CopyRef<GateReason>;
	have?: Capability[];
	need?: Capability[];
	fix?: FixAction;
};
export type GateResult = { ok: true } | GateFailure;

export interface GateCheck {
	reason: GateReason;
	params?: CopyParams;
	hide?: boolean;
	fix?: FixAction;
}

export interface GateRequirement {
	plane: SourcePlane | readonly SourcePlane[];
	platform?: "desktop";
	relationship?: readonly Relationship[];
	deviceActive?: true;
	caps?: readonly Capability[] | "owner";
	deviceScopeOnly?: boolean;
	keys?: true | "creates";
	unlocked?: true | "password";
	session?: true;
	features?: (features: GateFeatures, ctx: GateContext) => GateCheck | null;
	plan?: "history";
	role?: "read_boards" | "admin_execute" | "app_owner";
	ownerPowers?: true;
	extra?: (
		ctx: GateContext,
	) => (GateCheck & { gate: GateId; kind: GateNoticeKind }) | null;
}

/* Pre-flight (IA §6.4.3, CA2). */

export const PREFLIGHT_IDS = [
	"D1",
	"D2",
	"D3",
	"D4",
	"D5",
	"D6",
	"D7",
	"D8",
	"D9",
	"D10",
	"D11",
	"D12",
	"D13",
] as const;
export type PreflightId = (typeof PREFLIGHT_IDS)[number];

export type PreflightCode =
	| "checking"
	// D1 hub
	| "hub_ready"
	| "hub_devices_off"
	| "hub_unreachable"
	// D2 sign-in
	| "signed_in"
	| "sign_in_required"
	| "token_restricted"
	// D3 access
	| "access_owner"
	| "access_shared"
	| "access_cloud_approval"
	| "access_unknown"
	| "access_ended"
	| "device_revoked"
	// D4 keys
	| "keys_owner_here"
	| "keys_shared_here"
	| "keys_missing"
	| "keys_unusable"
	// D5 browser
	| "browser_ready"
	| "browser_cannot_protect_keys"
	| "browser_may_delete_keys"
	| "crypto_unavailable"
	// D6 lock holder
	| "lock_free"
	| "lock_held_here"
	| "lock_held_elsewhere"
	| "lock_unsupported"
	// D7 identity
	| "identity_trusted"
	| "identity_first_use"
	| "identity_mismatch"
	// D8 check-in
	| "checkin_online"
	| "checkin_late"
	| "checkin_offline"
	| "checkin_never"
	// D9 clocks
	| "clock_ok"
	| "clock_computer_off"
	| "clock_device_off"
	| "clock_unknown"
	// D10 connection service
	| "connection_ready"
	| "connection_not_configured"
	| "connection_needs_wss"
	| "connection_access_expired"
	| "connection_invalid_admission"
	| "relay_unreachable"
	| "connection_failed"
	// D11 access rules applied
	| "policy_applied"
	| "policy_waiting"
	| "policy_unknown"
	// D12 connection slots
	| "slots_available"
	| "slots_full"
	// D13 agent features
	| "agent_features_ok"
	| "agent_features_missing"
	| "agent_features_unknown";

/* Attention (M-DATA §3.8, IA §6.5, CA1, CA11). */

export const SEVERITIES = ["critical", "warning", "notice", "info"] as const;
export type Severity = (typeof SEVERITIES)[number];

export type AttentionKey =
	// device & presence
	| "offline_since"
	| "late"
	| "no_heartbeat_since_enrollment"
	| "revoked"
	| "you_still_pay_for_a_revoked_device"
	| "identity_mismatch"
	| "snapshot_integrity_error"
	| "clock_skew"
	| "access_denied"
	| "status_stale_while_online"
	| "background_task_failing"
	| "agent_update_available"
	| "rebooted_unexpectedly"
	| "status_subscription_expiring"
	| "device_slots_nearly_full"
	// setup & hub
	| "pending_setup_waiting"
	| "pending_setup_expired"
	| "hub_not_ready"
	| "release_trust_missing"
	// keys
	| "keys_missing_here"
	| "keys_not_backed_up_to_account"
	| "account_backup_upload_pending"
	| "account_backup_old_password"
	| "account_backup_hub_newer"
	| "storage_not_persistent"
	| "request_keys_unbacked"
	| "backup_slots_nearly_full"
	| "stale_local_keys"
	// access
	| "shared_access_expiring"
	| "shared_access_ended"
	| "grant_expiring"
	| "sharing_policy_waiting_for_device"
	| "sharing_policy_expiring"
	| "sharing_policy_expired"
	| "access_slots_nearly_full"
	| "access_request_pending"
	| "code_running_access_without_sandbox"
	// services
	| "service_crash_looping"
	| "service_not_as_requested"
	| "service_settings_not_applied"
	| "service_degraded"
	| "rollout_in_progress"
	| "rollout_failed_service_stopped"
	| "rollout_rolled_back"
	| "rollout_not_applied"
	| "rollout_staged_waiting"
	| "secret_write_pending"
	| "secret_write_failed"
	| "endpoint_unencrypted_exposed"
	| "event_tokens_after_revoke"
	// offline writes
	| "offline_writes_conflict"
	| "offline_writes_blocked"
	| "offline_writes_outcome_unknown"
	| "offline_writes_quarantined"
	| "offline_writes_backlog"
	| "offline_mirror_error"
	// cloud
	| "cloud_access_invalid"
	| "cloud_access_ending"
	| "spending_limit_low"
	| "spending_limit_ending"
	| "online_files_read_only"
	// certificates
	| "certificate_expired"
	| "certificate_expiring"
	| "certificate_not_yet_valid"
	| "renewal_delegation_error"
	| "renewal_authority_expiring"
	| "acme_error"
	| "acme_staging_in_use"
	| "signing_request_attention"
	| "certificate_inventory_stale"
	| "certificate_slots_nearly_full"
	| "org_ca_signing_key_expiring"
	| "org_ca_root_expiring"
	// history & shared metrics
	| "history_paused_readers_expired"
	| "history_paused_access_changed"
	| "history_not_stored_by_plan"
	| "history_storage_nearly_full"
	| "metric_readers_expiring"
	// operations
	| "unconfirmed_command"
	| "device_operation_failed"
	| "device_operation_unknown"
	| "upload_paused";

/** Primary/secondary action labels of IA §6.5. */
export type AttentionActionCode =
	| "diagnose"
	| "show_start_instructions"
	| "delete_keys"
	| "remove_from_computer"
	| "revoke"
	| "revoke_spending_limit"
	| "review_identity"
	| "review_diagnostics"
	| "fix_clock"
	| "show_recovery_steps"
	| "connect_live"
	| "update_agent"
	| "how_to_update"
	| "view_activity"
	| "renew"
	| "review_pending_setups"
	| "set_up_again"
	| "view_hub_status"
	| "restore_keys"
	| "back_up_to_account"
	| "retry_upload"
	| "update_account_backup"
	| "review_backups"
	| "keep_keys_safely"
	| "download_request_again"
	| "ask_to_renew"
	| "view_access"
	| "renew_access_rules"
	| "review_access"
	| "view_status"
	| "view_instances"
	| "follow"
	| "view_update"
	| "activate"
	| "check_again"
	| "try_again"
	| "assign_certificate"
	| "open_app_events"
	| "review_change"
	| "fix_cloud_access"
	| "view_queue"
	| "replace_approval"
	| "replace_limit"
	| "open_app_storage"
	| "view_certificate"
	| "fix_renewal"
	| "install_renewal_authority"
	| "review"
	| "switch_to_production"
	| "finish_or_discard"
	| "review_certificates"
	| "renew_signing_key"
	| "plan_replacement"
	| "resume_recording"
	| "see_plans"
	| "renew_readers"
	| "check_result"
	| "view_details"
	| "check_status"
	| "resume";

export type AttentionSubject =
	| { kind: "service"; deviceId: string; serviceId: string; projectId?: string }
	| { kind: "device"; deviceId: string }
	| { kind: "certificate"; deviceId: string; certificateId?: string }
	| { kind: "access"; deviceId: string; personId?: string }
	| { kind: "keys"; deviceId?: string }
	| { kind: "setup"; enrollmentId?: string }
	| { kind: "authority"; authorityId: string }
	| { kind: "hub" };

export interface AttentionAction {
	code: AttentionActionCode;
	target: FixAction | DevicesRoute;
	gate?: GateResult;
}

export interface AttentionItem {
	/** Stable: `${key}:${subjectId}`. */
	id: string;
	/** Shown only in diagnostics and with technical keys on (R3). */
	key: AttentionKey;
	severity: Severity;
	subject: AttentionSubject;
	copy: CopyRef<AttentionKey>;
	action?: AttentionAction;
	secondary?: Omit<AttentionAction, "gate">;
	source: Freshness;
	lastKnown: boolean;
	firstSeenAt: number;
	snoozedUntil?: number;
}

export type AttentionCandidate = Omit<
	AttentionItem,
	"firstSeenAt" | "snoozedUntil"
> & {
	/** Emitted only once the condition has held this long (IA §6.5 "for > 2 min"). */
	dwellS?: number;
};

export interface PendingSetup {
	enrollmentId: string;
	deviceId?: string;
	name: string;
	state: HubEnrollment["state"];
	createdAt?: number;
	expiresAt: number;
	/** True when only the local setup record knows it (hub without BG2). */
	local: boolean;
}

/** The agent's installed release (`agent.release_version`, never its constant crate version) from the last live read, kept on this computer (BG8 replaces it). */
export interface AgentLastRead {
	version: string;
	/** `agent.release_sequence`, when the agent reported it. */
	sequence?: number | null;
	/** Unix seconds. */
	at: number;
}

/** An access request made from this computer (real state: BG23). */
export interface AccessRequestRecord {
	deviceId: string;
	deviceName?: string;
	ownerId?: string;
	/** Unix seconds. */
	createdAt: number;
	approved: boolean;
}

/** Placement configuration facts from the last live configuration read. */
export interface PlacementConfigFacts {
	host?: string;
	port?: number;
	tlsCertificateId?: string | null;
	offlineWrites?: { maxAgeS: number; maxBytes: number };
	resourceGrantId?: string | null;
}

export interface LiveDeviceInput {
	state: LiveState;
	inspection?: LiveInspection;
	/** Keyed by placement id. */
	placements?: Record<string, PlacementConfigFacts>;
	/** Keyed by placement id. */
	offlineQueues?: Record<string, OfflineQueueStatus[]>;
	certificates?: CertificateInventory;
	certificateRequests?: CertificateRequest[];
	certificateIssuers?: CertificateIssuer[];
	acme?: AcmeCertificate[];
	rollouts?: DeploymentRolloutStatus[];
	history?: {
		scope: string;
		kind: "logs" | "metrics";
		expiresAt: number;
		policyVersion: number;
		status?: ArchiveRecordingStatus;
	}[];
	metricReaders?: { scope: string; expiresAt: number }[];
}

/** Every BG-backed field is optional; a rule whose input is absent emits nothing. */
export interface AttentionInput {
	/** Hub-corrected unix seconds. */
	now: number;
	me: string;
	hub: HubDeviceSupport;
	readiness?: DeviceSetupReadiness;
	releaseTrust?: unknown;
	latestRelease?: { version: string; sequence: number | null };
	usage?: { limits: DeviceLimits; usage: DeviceUsage };
	clock?: {
		hubOffsetS?: number;
		deviceSkewS: Record<string, number | undefined>;
	};
	devices: DeviceRow[];
	pendingSetups?: PendingSetup[];
	accountBackups: Record<
		string,
		{ revision: number; updatedAt?: number } | undefined
	>;
	accountBackupSlots?: { used: number; max: number };
	myAccess?: Record<string, MyAccess | undefined>;
	certInventory: Record<string, PublicCertificateInventory | undefined>;
	resources: Record<string, DeviceResources | undefined>;
	resourceSummary?: ResourceSummary;
	archiveUsage?: ArchiveUsage;
	keys: KeySessionSnapshot[];
	local: LocalSummary;
	fleet: Record<string, FleetDeviceState>;
	live: Record<string, LiveDeviceInput>;
	policies: Record<string, PolicyView & { policy?: ManagementPolicy }>;
	authorities: LocalCertificateAuthority[];
	activity: ActivityItem[];
	agentLastRead?: Record<string, AgentLastRead | undefined>;
	accessRequests?: AccessRequestRecord[];
	/** `GET /devices` answered: an empty `devices` then means "none", not "not known yet". */
	devicesLoaded?: boolean;
}

export interface AttentionRule {
	key: AttentionKey;
	evaluate(input: AttentionInput): AttentionCandidate[];
}

/** Persisted per scope (localStorage, try/catch). */
export interface AttentionMemory {
	firstSeen: Record<string, number>;
	snoozed: Record<string, number>;
	save(): void;
}

export type HealthLevel =
	| "critical"
	| "attention"
	| "healthy"
	| "revoked"
	| "unknown";

/* View models (M-DATA §3.12, CA8). */

export interface ServiceView {
	deviceId: string;
	serviceId: string;
	projectId: string;
	deploymentId: string;
	desired: "running" | "stopped" | (string & {});
	observed: string;
	conv: Convergence;
	since?: number;
	settings: { applied: number | null; latest: number };
	instances: { requested: number; ready: number; running: number; max: number };
	/** Plane and age of this row: live > snap > saved. */
	freshness: Freshness;
	/** How the app runs (BG-A1); null when this plane doesn't say. */
	source: "offline" | "online" | null;
	events: PlacementEvent[] | null;
	appVersion: { label?: string; hash?: string } | null;
	diagnostics?: {
		hasError: boolean;
		lastError?: string;
		restarts?: ReplicaRestarts;
	};
	offlineWrites?:
		| {
				pending: number;
				head?: OfflineQueueStatus["head"];
				quarantined: boolean;
		  }
		| "not_loaded";
	rollout?: DeploymentRolloutStatus;
}

export interface DeviceViewModel {
	row: DeviceRow;
	presence: Presence;
	relationship: Relationship;
	keys: KeySessionSnapshot;
	live: LiveState;
	agent?: { version: string; source: Freshness };
	/** Never an empty array for "not loaded". */
	services:
		| ServiceView[]
		| { state: AgeState; reason?: CopyRef<FreshnessReason> };
	certificates?: PublicCertificateInventory;
	resources?: DeviceResources;
	health: HealthLevel;
	attention: AttentionItem[];
}
