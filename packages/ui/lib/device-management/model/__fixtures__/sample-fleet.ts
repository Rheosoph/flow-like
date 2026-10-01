/*
 * The design sample fleet (scratchpad inventory/_sample.json + prototype enrichments)
 * as an `AttentionInputExt`. Clock: 30 Sept 2026 14:00 CEST. Golden: 2 critical · 15 total.
 */
import type { DeviceCertificate } from "../../certificates";
import type { DeploymentRolloutStatus } from "../../deployment";
import type { RetainedObservation } from "../../inventory";
import type { OfflineQueueStatus } from "../../offline-queue";
import type { DeviceSetupReadiness } from "../../readiness";
import type {
	Ed25519PublicKey,
	ManagementGrant,
	ManagementPolicy,
	PolicyView,
} from "../../types";
import type {
	ActivityItem,
	FleetDeviceState,
	KeySessionSnapshot,
	LocalVaultSummary,
} from "../../workspace/types";
import type { AttentionInputExt, LiveDeviceInputExt } from "../attention";
import { classify } from "../freshness";
import type {
	DeviceRow,
	InspectionPlus,
	PlacementStatusPlus,
	ResourceSummary,
} from "../types";

export const SAMPLE_NOW = 1_790_769_600;
export const SAMPLE_ME = "usr_2Nf8KqLx";
export const SAMPLE_PEOPLE = {
	felix: SAMPLE_ME,
	mira: "usr_9QmT3rVb",
	jonas: "usr_7JkD2wQe",
	partner: "usr_4HbW8sPz",
} as const;

export const SAMPLE_IDS = {
	edge: "5b794764-6afc-4ac9-89c2-c6d9eb91fd42",
	warehouse: "54484ac9-891c-4570-a0e5-51d016e09143",
	studio: "91d32b25-0039-499a-87dd-fadb91b67675",
	lab: "c8ea48cc-8d4c-4e10-b8f8-686a7ebce0a2",
	oldKiosk: "e170e51b-f6c9-46fb-a5a9-3b9a199133ef",
	partner: "0d177f9c-5836-4688-b844-31e35630dbf8",
	cold: "97060b0d-a4b4-452b-b6c2-ce1eba903ad5",
	miraRender: "e3b1c2d4-7a8f-4c1e-9b2d-5f6a7c8d9e01",
} as const;

export const SAMPLE_APPS = {
	supportPortal: "app_support_portal",
	invoiceAi: "app_invoice_ai",
	crmSync: "app_crm_sync",
	warehouseScanner: "app_warehouse_scan",
	fieldNotes: "app_field_notes",
	partnerReports: "app_partner_reports",
} as const;

const NOW = SAMPLE_NOW;
const ME = SAMPLE_ME;
const { mira: MIRA, jonas: JONAS, partner: PARTNER } = SAMPLE_PEOPLE;
const ID = SAMPLE_IDS;
const APP = SAMPLE_APPS;
const ALL_FEATURES = {
	placement_diagnostics: 1,
	task_health: 1,
	placement_events: 1,
	offline_summary: 1,
	host_operation: 1,
	network_interfaces: 1,
	rollout_history: 1,
	operations: 1,
	metrics_history: 1,
	offline_lookup: 1,
	reader_bindings: 1,
	acme_failure_detail: 1,
	archive_status: 1,
	artifact_capacity: 1,
} as const;

export function ed25519(x: string): Ed25519PublicKey {
	return { kty: "OKP", crv: "Ed25519", x };
}

export function identityOf(seed: string): DeviceRow["identity"] {
	return {
		auth_key: ed25519(`auth-${seed}`),
		telemetry_key: ed25519(`telemetry-${seed}`),
		management_key: [...seed].map((char) => char.charCodeAt(0) % 256),
	};
}

export function placement(
	row: Partial<PlacementStatusPlus> &
		Pick<
			PlacementStatusPlus,
			"id" | "project_id" | "desired_state" | "observed_state"
		>,
): PlacementStatusPlus {
	return {
		deployment_id: `${row.id}-deployment`,
		revision: `${row.id}-revision`,
		config_revision: 1,
		intent_revision: 1,
		applied_revision: row.config_revision ?? 1,
		desired_replicas: 1,
		running_replicas: row.observed_state === "running" ? 1 : 0,
		ready_replicas: row.observed_state === "running" ? 1 : 0,
		max_replicas: 1,
		...row,
	};
}

export function observation(
	deviceId: string,
	placements: PlacementStatusPlus[],
	observedAtSeconds: number,
	bootId: string | null,
): RetainedObservation {
	return {
		scope: { kind: "device" },
		device_id: deviceId,
		boot_id: bootId,
		observed_at: observedAtSeconds * 1000,
		placements,
	};
}

export function fleetState(
	deviceId: string,
	parts: Omit<FleetDeviceState, "deviceId" | "freshness">,
	now = NOW,
): FleetDeviceState {
	return {
		deviceId,
		...parts,
		freshness: {
			status: classify("fleet_status", {
				now,
				at: parts.status?.observedAt,
				loaded: !!parts.status,
			}),
			metrics: classify("fleet_metrics", { now, loaded: false }),
			saved: classify("saved_inventory", {
				now,
				at: parts.saved?.observedAt,
				loaded: !!parts.saved,
			}),
		},
	};
}

export function keySession(
	deviceId: string,
	state: KeySessionSnapshot["state"],
	role: KeySessionSnapshot["role"] = "owner",
	grantId = `owner-${deviceId.slice(0, 8)}`,
): KeySessionSnapshot {
	const unlocked = state === "unlocked";
	return {
		deviceId,
		state,
		role,
		grantId,
		canSign: unlocked && role === "owner",
		keepUnlocked: false,
		restoredNeedsFreshEndpoint: false,
		...(unlocked
			? { unlockedAt: NOW - 600, lastUsedAt: NOW - 5, idleLocksAt: NOW + 1_200 }
			: {}),
	};
}

export function vault(
	deviceId: string,
	role: LocalVaultSummary["role"] = "owner",
	grantId = `owner-${deviceId.slice(0, 8)}`,
): LocalVaultSummary {
	return { deviceId, role, grantId, requiresFreshEndpoint: false };
}

export function deviceRow(
	row: Omit<DeviceRow, "identity" | "auth_epoch"> &
		Partial<Pick<DeviceRow, "auth_epoch">>,
): DeviceRow {
	return {
		auth_epoch: 1,
		display_name: null,
		access_expires_at: null,
		access_rules_expire_at: null,
		revoked_at: null,
		cloud_approvals: null,
		auth_rejection: null,
		...row,
		identity: identityOf(row.device_id),
	};
}

const DEVICES: DeviceRow[] = [
	deviceRow({
		device_id: ID.edge,
		owner_id: ME,
		name: "edge-berlin-01",
		status: "active",
		registered_at: 1_773_489_600,
		last_seen_at: 1_790_769_560,
		relationship: "owner",
		access_rules_expire_at: 1_793_412_000,
		cloud_approvals: {
			resource_grants: 1,
			billing_grants: 1,
			expires_at: 1_793_275_200,
		},
	}),
	deviceRow({
		device_id: ID.warehouse,
		owner_id: ME,
		name: "warehouse-pi",
		status: "active",
		registered_at: 1_780_401_600,
		last_seen_at: 1_790_758_800,
		relationship: "owner",
	}),
	deviceRow({
		device_id: ID.studio,
		owner_id: ME,
		name: "studio-mac-mini",
		status: "active",
		registered_at: 1_786_881_600,
		last_seen_at: 1_790_769_505,
		relationship: "owner",
		access_rules_expire_at: 1_792_447_920,
		cloud_approvals: {
			resource_grants: 1,
			billing_grants: 0,
			expires_at: 1_793_275_200,
		},
	}),
	deviceRow({
		device_id: ID.lab,
		owner_id: MIRA,
		name: "lab-gpu-02",
		status: "active",
		registered_at: 1_773_489_600,
		last_seen_at: 1_790_769_560,
		relationship: "shared",
		access_expires_at: 1_790_820_000,
	}),
	deviceRow({
		device_id: ID.oldKiosk,
		owner_id: ME,
		name: "old-kiosk",
		status: "revoked",
		registered_at: 1_756_209_600,
		last_seen_at: 1_780_401_600,
		auth_epoch: 2,
		relationship: "owner",
	}),
	deviceRow({
		device_id: ID.partner,
		owner_id: PARTNER,
		name: "partner-edge",
		status: "revoked",
		registered_at: 1_773_489_600,
		last_seen_at: 1_789_992_000,
		auth_epoch: 2,
		relationship: "cloud_approval",
		cloud_approvals: {
			resource_grants: 1,
			billing_grants: 1,
			expires_at: 1_792_929_600,
		},
	}),
	deviceRow({
		device_id: ID.cold,
		owner_id: ME,
		name: "cold-storage-nas",
		status: "active",
		registered_at: 1_790_768_880,
		last_seen_at: null,
		relationship: "owner",
	}),
];

const EDGE_PLACEMENTS: PlacementStatusPlus[] = [
	placement({
		id: "support-bot",
		project_id: APP.supportPortal,
		deployment_id: "5d2ee3ca-1b6c-4e42-a152-bafa2370440f",
		revision:
			"71c6216b4fd24fcdc7a0130015a710bdb9568eeb9010187ef2e18402a3dae929",
		desired_state: "running",
		observed_state: "running",
		config_revision: 7,
		intent_revision: 4,
		applied_revision: 7,
		desired_replicas: 2,
		running_replicas: 2,
		ready_replicas: 2,
		max_replicas: 4,
		source: "offline",
		events: [
			{
				event_id: "evt_support_chat",
				event_version: [2, 3, 0],
				board_version: [5, 1, 2],
			},
			{
				event_id: "evt_support_http",
				event_version: [1, 0, 4],
				board_version: [5, 1, 2],
			},
		],
	}),
	placement({
		id: "invoice-extractor",
		project_id: APP.invoiceAi,
		deployment_id: "ec74014e-300d-452d-86ff-033569da8e40",
		revision:
			"491e8acfcbe8a07759ae759cd5c46a6717d3fd67a18a56c68d1f70212e332f91",
		desired_state: "running",
		observed_state: "starting",
		config_revision: 12,
		intent_revision: 6,
		applied_revision: 11,
		running_replicas: 1,
		ready_replicas: 0,
		source: "online",
		events: [
			{
				event_id: "evt_extract_http",
				event_version: [1, 4, 0],
				board_version: [2, 1, 0],
			},
		],
	}),
	placement({
		id: "nightly-sync",
		project_id: APP.crmSync,
		deployment_id: "5bcf1278-e24f-49a7-918d-2ae49c07cd00",
		revision:
			"a589985403501c44bacdf9f1dea8e62c6cf0cef2d613f8c6d6687e3631190b61",
		desired_state: "stopped",
		observed_state: "stopped",
		config_revision: 3,
		intent_revision: 2,
		applied_revision: 3,
		source: "offline",
		events: [
			{
				event_id: "evt_crm_nightly",
				event_version: [1, 1, 0],
				board_version: [4, 0, 0],
			},
		],
	}),
];

const STUDIO_PLACEMENTS: PlacementStatusPlus[] = [
	placement({
		id: "field-notes",
		project_id: APP.fieldNotes,
		deployment_id: "496e5d88-3147-4ec4-84bc-dfce6cc635fc",
		revision:
			"ef87af1b693b495e6445e96db85fc4dc719a250e42e0de78432e312e05e29845",
		desired_state: "running",
		observed_state: "running",
		config_revision: 9,
		intent_revision: 2,
		applied_revision: 9,
		source: "online",
		events: [
			{
				event_id: "evt_notes_http",
				event_version: [1, 2, 0],
				board_version: [3, 0, 1],
			},
		],
	}),
];

const SCANNER = {
	id: "scanner-ingest",
	project_id: APP.warehouseScanner,
	deployment_id: "6246a63d-9f9c-414c-8b43-e1f97b2ac13c",
	revision: "a417baa4fe9fd55c794745dc48594cc3311531acdb4a1158f6faf3cac5bf995f",
	desired_state: "running",
	config_revision: 5,
	intent_revision: 3,
	applied_revision: 5,
} as const;

const EDGE_INSPECTION: InspectionPlus = {
	device_id: ID.edge,
	boot_id: "f30e158c6645af7e",
	observed_at: 1_790_769_590_000,
	certificate_management: 1,
	certificate_issuance: 1,
	certificate_acme: 1,
	can_manage_certificates: true,
	can_delegate_certificate_renewal: true,
	placements: EDGE_PLACEMENTS,
	features: ALL_FEATURES,
	agentVersion: "0.1.0",
	agent: { version: "0.1.0", release_version: "0.9.4", release_sequence: 44 },
	hostOperations: { reboot: true, update_agent: true },
	hostIsolation: "required",
	isolation: {
		platform: "linux",
		sandbox_available: true,
		require_isolation: true,
		placement_preflight_required: true,
		landlock_abi: 6,
		reason: null,
	},
	host: { booted_at: 1_790_596_860, agent_started_at: 1_790_596_860 },
	tasks: [
		{
			name: "heartbeat",
			state: "ok",
			since: 1_790_596_860,
			consecutive_failures: 0,
		},
	],
	hostOperation: {
		operation_id: "0ffef803-aeb7-430d-9f8f-930bf00a6ea9",
		kind: "update_agent",
		state: "completed",
		created_at: 1_790_596_800,
		issued_by: "you",
	},
};

const STUDIO_INSPECTION: InspectionPlus = {
	device_id: ID.studio,
	boot_id: "275db9c05832d9c7",
	observed_at: 1_790_769_540_000,
	certificate_management: 1,
	certificate_issuance: 1,
	certificate_acme: 1,
	can_manage_certificates: true,
	can_delegate_certificate_renewal: true,
	placements: STUDIO_PLACEMENTS,
	features: ALL_FEATURES,
	agentVersion: "0.1.0",
	agent: { version: "0.1.0", release_version: "0.9.4", release_sequence: 44 },
	hostOperations: { reboot: false, update_agent: false },
	hostIsolation: "none",
	isolation: {
		platform: "macos",
		sandbox_available: false,
		require_isolation: false,
		placement_preflight_required: false,
		landlock_abi: null,
		reason:
			"linux_sandbox requires Linux; use trusted_process only for a dedicated trusted account",
	},
	host: { booted_at: 1_790_510_400, agent_started_at: 1_790_510_430 },
	tasks: [],
	hostOperation: null,
};

const INVOICE_ROLLOUT: DeploymentRolloutStatus = {
	rollout_id: "a0e5259e-a9bf-4eef-99df-f9666dffbab4",
	placement_id: "invoice-extractor",
	project_id: APP.invoiceAi,
	state: "activating",
	failure_code: null,
	active_revision: 12,
	base_revision: 11,
	previous_replicas: 1,
	candidate_replicas: 1,
	stabilization_seconds: 10,
	deadline_seconds: 120,
	created_at: 1_790_769_360,
	updated_at: 1_790_769_570,
	deadline_at: 1_790_769_690,
	stable_since: null,
};

function certificate(
	row: Pick<
		DeviceCertificate,
		"certificate_id" | "label" | "revision" | "not_before" | "not_after"
	> &
		Partial<DeviceCertificate>,
): DeviceCertificate {
	return {
		subject: "",
		issuer: "",
		dns_names: [],
		ip_addresses: [],
		sha256_fingerprint: "0".repeat(64),
		bindings: [],
		binding_count: row.bindings?.length ?? 0,
		...row,
	};
}

const EDGE_API = "24f6fe22-c6c2-4e15-9d37-7a41a379afb9";
const INTERNAL_MQTT = "93bcc1ef-5f49-4bb3-b90c-7b052822bf02";
const WAREHOUSE_CERT = "82ac7195-73a6-48a1-816f-7e37f7f67602";

function queue(
	row: Omit<OfflineQueueStatus, "mirror_error"> &
		Partial<Pick<OfflineQueueStatus, "mirror_error">>,
): OfflineQueueStatus {
	return { mirror_error: null, ...row };
}

const STUDIO_QUEUES: OfflineQueueStatus[] = [
	queue({
		scope: "25db08116711b6e982332459d70bb8657fedb1e468cde18ac13a1761e256f7f6",
		quarantined: false,
		pending_count: 14,
		pending_bytes: 3_145_728,
		oldest_at: 1_790_758_200,
		head: {
			sequence: 881,
			operation_id: "f40615e0-b484-42d1-acd5-408b2c9e9938",
			resource:
				'{"kind":"table","purpose":"storage","database":"db","table":"notes"}',
			payload: null,
			state: "conflict",
			attempts: 2,
			created_at: 1_790_758_200,
			error: "The cloud table changed since this write was queued.",
			local_version: 412,
		},
	}),
	queue({
		scope: "f7c74b735d7a202f3106f07d16858366f04a6d80855fd3aeefe3f3bc9ed40308",
		quarantined: true,
		pending_count: 3,
		pending_bytes: 48_128,
		oldest_at: 1_789_992_000,
		head: {
			sequence: 12,
			operation_id: "f366cea9-cc31-466b-804f-1f5dd490aa2f",
			resource:
				'{"kind":"file","purpose":"user","path":"exports/2026-09-21-summary.pdf"}',
			payload: null,
			state: "blocked",
			attempts: 1,
			created_at: 1_789_992_000,
			error: "Delegated authorization for this queue is no longer valid.",
			local_version: null,
		},
	}),
];

function grant(
	row: Pick<
		ManagementGrant,
		"grant_id" | "user_id" | "scope" | "capabilities" | "expires_at"
	>,
): ManagementGrant {
	return {
		controller_key: ed25519(`controller-${row.user_id}`),
		group_id: null,
		group_version: null,
		...row,
	};
}

function policyView(
	version: number,
	appliedVersion: number,
	policy?: ManagementPolicy,
): PolicyView & { policy?: ManagementPolicy } {
	return {
		policy_jws: null,
		version,
		digest: `digest-v${version}`,
		applied_version: appliedVersion,
		applied_digest: `digest-v${appliedVersion}`,
		...(policy ? { policy } : {}),
	};
}

const POLICIES: AttentionInputExt["policies"] = {
	[ID.edge]: policyView(5, 5, {
		version: 1,
		device_id: ID.edge,
		policy_version: 5,
		previous_policy_digest: "digest-v4",
		issued_at: 1_790_733_600,
		expires_at: 1_793_412_000,
		grants: [
			grant({
				grant_id: "b7a0e44c-72e4-4b87-9b5d-3a98b09cdd6f",
				user_id: MIRA,
				scope: { kind: "device" },
				capabilities: ["status", "metrics", "logs"],
				expires_at: 1_790_791_200,
			}),
			grant({
				grant_id: "b352ae65-b4b0-4783-a0ad-cae4dd27d6fa",
				user_id: JONAS,
				scope: { kind: "project", project_id: APP.invoiceAi },
				capabilities: [
					"status",
					"logs",
					"deploy",
					"start",
					"stop",
					"restart",
					"scale",
				],
				expires_at: 1_790_820_000,
			}),
		],
	}),
	[ID.studio]: policyView(2, 1, {
		version: 1,
		device_id: ID.studio,
		policy_version: 2,
		previous_policy_digest: "digest-v1",
		issued_at: 1_790_769_120,
		expires_at: 1_792_447_920,
		grants: [
			grant({
				grant_id: "0b5c8e21-7d4a-4f93-b6e2-1a9c3d5f7e08",
				user_id: MIRA,
				scope: { kind: "project", project_id: APP.fieldNotes },
				capabilities: ["status", "logs", "metrics"],
				expires_at: 1_790_856_000,
			}),
		],
	}),
};

const RESOURCES: AttentionInputExt["resources"] = {
	[ID.edge]: {
		grants: [
			{
				grant_id: "d99ba88b-717b-445e-a719-a2084df3aec0",
				device_id: ID.edge,
				placement_id: "invoice-extractor",
				deployment_id: "ec74014e-300d-452d-86ff-033569da8e40",
				project_id: APP.invoiceAi,
				app_id: APP.invoiceAi,
				delegating_user_id: ME,
				authz_version: 1,
				model_ids: ["bge-m3", "gpt-4.1-mini"],
				online_access: "read_write",
				max_instances: 2,
				expires_at: 1_793_275_200,
				status: "active",
			},
		],
		billing: [
			{
				billing_grant_id: "b936e936-d443-4252-a8e6-9420d361c037",
				grant_id: "d99ba88b-717b-445e-a719-a2084df3aec0",
				payer_id: ME,
				authz_version: 1,
				limit_micros: 25_000_000,
				used_micros: 7_412_300,
				reserved_micros: 120_000,
				expires_at: 1_793_275_200,
				status: "active",
			},
		],
		instances: [
			{
				instance_id: "edd47b6d-a53a-4eae-9ade-ccab311c47e8",
				purpose: "workload",
				device_id: ID.edge,
				grant_id: "d99ba88b-717b-445e-a719-a2084df3aec0",
				billing_grant_id: "b936e936-d443-4252-a8e6-9420d361c037",
				registered_at: 1_790_769_574,
				lease_expires_at: 1_790_770_174,
			},
		],
	},
	[ID.studio]: {
		grants: [
			{
				grant_id: "6e2f1a9c-3b7d-4e05-8a1c-9d4b2f6e0a73",
				device_id: ID.studio,
				placement_id: "field-notes",
				deployment_id: "496e5d88-3147-4ec4-84bc-dfce6cc635fc",
				project_id: APP.fieldNotes,
				app_id: APP.fieldNotes,
				delegating_user_id: ME,
				authz_version: 2,
				model_ids: ["gpt-4.1-mini"],
				online_access: "read_write",
				max_instances: 1,
				expires_at: 1_793_275_200,
				status: "active",
			},
		],
		billing: [],
		instances: [],
	},
	[ID.partner]: {
		grants: [
			{
				grant_id: "22c49df5-abd8-4df5-9508-0de6a80418ae",
				device_id: ID.partner,
				placement_id: "report-renderer",
				deployment_id: "b9c8c658-66ef-4f1b-a462-a7fd674bf139",
				project_id: APP.partnerReports,
				app_id: APP.partnerReports,
				delegating_user_id: PARTNER,
				authz_version: 1,
				model_ids: ["mistral-small-3"],
				online_access: null,
				max_instances: 1,
				expires_at: 1_796_040_000,
				status: "active",
			},
		],
		billing: [
			{
				billing_grant_id: "cc3e21cf-49b2-4ed9-9eae-a337d7912be3",
				grant_id: "22c49df5-abd8-4df5-9508-0de6a80418ae",
				payer_id: ME,
				authz_version: 1,
				limit_micros: 50_000_000,
				used_micros: 12_500_000,
				reserved_micros: 0,
				expires_at: 1_792_929_600,
				status: "active",
			},
		],
		instances: [],
	},
};

type ResourceLists = NonNullable<AttentionInputExt["resources"][string]>;
type SummaryRow = ResourceSummary["devices"][number];

const summaryApproval = (
	grantRow: ResourceLists["grants"][number],
	entry: ResourceLists,
	me: string,
): SummaryRow["approvals"][number] => ({
	grant_id: grantRow.grant_id,
	placement_id: grantRow.placement_id,
	app_id: grantRow.app_id,
	status: grantRow.status,
	expires_at: grantRow.expires_at,
	effective_expires_at: grantRow.expires_at,
	effective_limit: "approval",
	online_access: grantRow.online_access ?? null,
	online_write_blocked: null,
	payer_is_me: entry.billing.some(
		(billing) =>
			billing.grant_id === grantRow.grant_id && billing.payer_id === me,
	),
	approver_is_me: grantRow.delegating_user_id === me,
});

const summaryBilling = (
	billing: ResourceLists["billing"][number],
	me: string,
): SummaryRow["billing"][number] => ({
	billing_grant_id: billing.billing_grant_id,
	grant_id: billing.grant_id,
	limit_micros: billing.limit_micros,
	used_micros: billing.used_micros,
	reserved_micros: billing.reserved_micros,
	expires_at: billing.expires_at,
	payer_is_me: billing.payer_id === me,
});

const summaryRow = (
	deviceId: string,
	entry: ResourceLists,
	me: string,
): SummaryRow => ({
	device_id: deviceId,
	approvals: entry.grants.map((grantRow) =>
		summaryApproval(grantRow, entry, me),
	),
	billing: entry.billing
		.filter((billing) => billing.status === "active")
		.map((billing) => summaryBilling(billing, me)),
});

/** FG4 `GET /devices/resource-summary` as the hub would derive it from the lists above. */
export function resourceSummaryOf(
	resources: AttentionInputExt["resources"],
	me: string,
	now: number,
): ResourceSummary {
	return {
		server_time: now,
		devices: Object.entries(resources).flatMap(([deviceId, entry]) =>
			entry ? [summaryRow(deviceId, entry, me)] : [],
		),
	};
}

type ActivityRow = Omit<
	ActivityItem,
	"startedBy" | "actions" | "label" | "updatedAt"
> &
	Partial<Pick<ActivityItem, "actions" | "updatedAt">>;

/** Rows are written in unix seconds like the rest of the sample; the tray keeps milliseconds. */
function activity(row: ActivityRow): ActivityItem {
	const { deadlineAt, finishedAt, updatedAt, ...rest } = row;
	return {
		startedBy: "you",
		actions: ["open"],
		label: { code: row.kind },
		...rest,
		startedAt: row.startedAt * 1_000,
		updatedAt: (updatedAt ?? row.startedAt) * 1_000,
		...(deadlineAt === undefined ? {} : { deadlineAt: deadlineAt * 1_000 }),
		...(finishedAt === undefined ? {} : { finishedAt: finishedAt * 1_000 }),
	};
}

const ACTIVITY: ActivityItem[] = [
	activity({
		id: "a0e5259e-a9bf-4eef-99df-f9666dffbab4",
		kind: "safe_update",
		target: {
			deviceId: ID.edge,
			deviceName: "edge-berlin-01",
			serviceId: "invoice-extractor",
			projectId: APP.invoiceAi,
		},
		state: "active",
		deadlineAt: 1_790_769_690,
		startedAt: 1_790_769_360,
		resume: {
			type: "rollout",
			rolloutId: "a0e5259e-a9bf-4eef-99df-f9666dffbab4",
			placementId: "invoice-extractor",
			projectId: APP.invoiceAi,
		},
	}),
	activity({
		id: "65fe54b5-c164-4731-8c72-36c8bc0bce2c",
		kind: "upload",
		target: {
			deviceId: ID.edge,
			deviceName: "edge-berlin-01",
			projectId: APP.crmSync,
		},
		state: "paused",
		progress: { done: 12, total: 38, unit: "files" },
		deadlineAt: 1_790_841_600,
		startedAt: 1_790_766_000,
		resume: {
			type: "transfer",
			transferId: "65fe54b5-c164-4731-8c72-36c8bc0bce2c",
			projectId: APP.crmSync,
			manifestSha256:
				"dbb662b1a3aafbb8ddb89f1a949334914b40d75cd5d07d38481e337177d30c03",
			expiresAt: 1_790_841_600,
		},
		actions: ["resume", "cancel"],
	}),
	activity({
		id: "3c7e19a0-5d2b-4f86-9e41-b0a2c8d6f174",
		kind: "access_rules",
		target: { deviceId: ID.studio, deviceName: "studio-mac-mini" },
		state: "waiting",
		startedAt: 1_790_769_120,
		resume: { type: "policy", version: 2 },
	}),
	activity({
		id: "d81f4b27-0e6a-4c39-a5b8-2f71c9e0d364",
		kind: "account_backup",
		target: { deviceId: ID.studio, deviceName: "studio-mac-mini" },
		state: "waiting",
		startedAt: 1_790_768_700,
		resume: { type: "account_backup", revision: 2 },
	}),
	activity({
		id: "0ffef803-aeb7-430d-9f8f-930bf00a6ea9",
		kind: "agent_update",
		target: { deviceId: ID.edge, deviceName: "edge-berlin-01" },
		state: "done",
		startedAt: 1_790_596_800,
		finishedAt: 1_790_596_860,
		actions: ["dismiss"],
	}),
];

/** The golden sample (2 critical · 8 warning · 5 notice = 15, plus info). A fresh deep copy per call. */
export function sampleFleet(): AttentionInputExt {
	return structuredClone(goldenInput());
}

type ReadinessCheck = DeviceSetupReadiness["checks"][number];

const READY_IDS: ReadinessCheck["id"][] = [
	"policy",
	"signing",
	"api",
	"signaling",
	"release",
	"database",
];

const READY_CHECKS: ReadinessCheck[] = READY_IDS.map((id) => ({
	id,
	ready: true,
	message: `${id} ready`,
}));

type HubUsage = NonNullable<AttentionInputExt["usage"]>;

const HUB_LIMITS: HubUsage["limits"] = {
	max_devices: 100,
	max_pending_enrollments: 10,
	enrollment_ttl_seconds: 86_400,
	max_enrollments_per_day: 220,
	max_account_backups: 256,
};

const HUB_USAGE: HubUsage["usage"] = {
	active_devices: 5,
	revoked_devices: 2,
	pending_enrollments: 1,
	enrollments_last_24h: 1,
	account_backups: 4,
};

const PENDING_SETUPS: NonNullable<AttentionInputExt["pendingSetups"]> = [
	{
		enrollmentId: "c382dd52-1611-4477-b983-4e1e1417b669",
		deviceId: "fcbf72ca-b336-47a6-8b51-55dff38351d1",
		name: "factory-line-3",
		state: "pending",
		createdAt: 1_790_755_200,
		expiresAt: 1_790_841_600,
		local: true,
	},
	{
		enrollmentId: "bd3b57bb-a630-4470-b978-4c6e1ecb5c4f",
		deviceId: "b9c8c658-66ef-4f1b-a462-a7fd674bf139",
		name: "test-vm",
		state: "pending",
		createdAt: 1_790_575_200,
		expiresAt: 1_790_661_600,
		local: true,
	},
];

const ACCOUNT_BACKUPS: AttentionInputExt["accountBackups"] = {
	[ID.edge]: { revision: 3, updatedAt: 1_786_881_900 },
	[ID.warehouse]: { revision: 1, updatedAt: 1_780_401_900 },
	[ID.studio]: { revision: 1, updatedAt: 1_786_882_000 },
	[ID.lab]: { revision: 1, updatedAt: 1_773_490_000 },
	[ID.cold]: { revision: 0 },
};

const LAB_GRANT = "1229c956-36d6-4fec-8e80-01409a530896";

const MY_ACCESS: NonNullable<AttentionInputExt["myAccess"]> = {
	[ID.lab]: {
		device_id: ID.lab,
		role: "grantee",
		owner_id: MIRA,
		policy_version: 3,
		policy_expires_at: 1_792_000_000,
		applied_version: 3,
		applied: true,
		grants: [
			{
				grant_id: LAB_GRANT,
				scope: { kind: "project", project_id: APP.invoiceAi },
				capabilities: ["status", "metrics", "logs", "deploy", "start", "stop"],
				expires_at: 1_790_820_000,
				controller_key_thumbprint: "lab-shared-thumbprint",
				group_id: null,
			},
		],
	},
};

const CERT_INVENTORY: AttentionInputExt["certInventory"] = {
	[ID.edge]: {
		revision: 17,
		updated_at: 1_790_767_800,
		certificates: [
			{
				certificate_id: EDGE_API,
				revision: 4,
				fingerprint_sha256: "4".repeat(64),
				not_after: 1_796_040_000,
			},
			{
				certificate_id: INTERNAL_MQTT,
				revision: 2,
				fingerprint_sha256: "c".repeat(64),
				not_after: 1_791_201_600,
			},
		],
	},
	[ID.warehouse]: {
		revision: 6,
		updated_at: 1_790_758_800,
		certificates: [
			{
				certificate_id: WAREHOUSE_CERT,
				revision: 1,
				fingerprint_sha256: "8".repeat(64),
				not_after: 1_790_596_800,
			},
		],
	},
	[ID.studio]: { revision: 0, updated_at: null, certificates: [] },
	[ID.cold]: { revision: 0, updated_at: null, certificates: [] },
};

const ARCHIVE_USAGE: NonNullable<AttentionInputExt["archiveUsage"]> = {
	tier: "PRO",
	max_bytes: 268_435_456,
	retention_seconds: 604_800,
	used_bytes: 91_226_112,
	devices: [
		{
			device_id: ID.edge,
			used_bytes: 91_226_112,
			segments: 1_183,
			oldest_created_at: 1_790_164_800,
			newest_expires_at: 1_791_374_100,
		},
	],
};

const KEYS: KeySessionSnapshot[] = [
	keySession(ID.edge, "unlocked"),
	keySession(ID.warehouse, "unlocked"),
	keySession(ID.studio, "unlocked"),
	keySession(ID.lab, "locked", "shared", LAB_GRANT),
	keySession(ID.oldKiosk, "stale"),
	keySession(ID.cold, "locked"),
];

const LOCAL: AttentionInputExt["local"] = {
	platform: "desktop",
	persistence: "persisted",
	webLocks: true,
	indexedDb: true,
	cryptoLoaded: true,
	vaults: [
		{ ...vault(ID.edge), identityPinnedAt: 1_773_489_900 },
		vault(ID.warehouse),
		vault(ID.studio),
		vault(ID.lab, "shared", LAB_GRANT),
		vault(ID.oldKiosk),
		vault(ID.cold),
		vault(ID.miraRender, "shared", "mira-render-request"),
	],
	backups: {
		[ID.edge]: { localRevision: 3, pending: false },
		[ID.warehouse]: { localRevision: 1, pending: false },
		[ID.studio]: { localRevision: 2, pending: true },
		[ID.lab]: { localRevision: 1, pending: false },
		[ID.cold]: { localRevision: 0, pending: false },
	},
	authorities: [
		{
			authorityId: "7e2d9c41-58af-4b3e-9d61-2c8f0a7b5e14",
			label: "Rheosoph Internal",
			issuingNotAfter: 1_805_025_600,
			rootNotAfter: 1_868_097_600,
		},
	],
};

const FLEET: AttentionInputExt["fleet"] = {
	[ID.edge]: fleetState(ID.edge, {
		status: {
			observations: [
				observation(
					ID.edge,
					EDGE_PLACEMENTS,
					1_790_769_560,
					"f30e158c6645af7e",
				),
			],
			observedAt: 1_790_769_560,
			bootId: "f30e158c6645af7e",
			sequence: 5_512,
		},
		reader: { revision: 2, expiresAt: 1_818_417_600 },
		policy: { version: 5 },
	}),
	[ID.warehouse]: fleetState(ID.warehouse, {
		status: {
			observations: [
				observation(
					ID.warehouse,
					[
						placement({
							...SCANNER,
							observed_state: "backoff",
							running_replicas: 0,
							ready_replicas: 0,
						}),
					],
					1_790_758_790,
					"a676b4383ab59238",
				),
			],
			observedAt: 1_790_758_790,
			bootId: "a676b4383ab59238",
			sequence: 2_210,
		},
		saved: {
			observations: [
				observation(
					ID.warehouse,
					[placement({ ...SCANNER, observed_state: "running" })],
					1_790_676_000,
					"a676b4383ab59238",
				),
			],
			observedAt: 1_790_676_000,
		},
		reader: { revision: 1, expiresAt: 1_811_937_900 },
	}),
	[ID.lab]: {
		deviceId: ID.lab,
		freshness: {
			status: classify("fleet_status", {
				now: NOW,
				loaded: true,
				locked: true,
			}),
			metrics: classify("fleet_metrics", {
				now: NOW,
				loaded: true,
				locked: true,
			}),
			saved: classify("saved_inventory", { now: NOW, loaded: false }),
		},
	},
};

const EDGE_LIVE: LiveDeviceInputExt = {
	state: {
		kind: "live",
		transport: "webrtc",
		expiresAt: 1_790_769_845,
		bootId: "f30e158c6645af7e",
		connectedAt: 1_790_769_300,
	},
	inspection: { value: EDGE_INSPECTION, readAt: 1_790_769_590 },
	rollouts: [INVOICE_ROLLOUT],
	certificates: {
		inventory_revision: 17,
		certificates: [
			certificate({
				certificate_id: EDGE_API,
				label: "edge-api",
				revision: 4,
				issuer: "CN=R11, O=Let's Encrypt, C=US",
				dns_names: ["edge-berlin.rheosoph.example"],
				not_before: 1_788_264_000,
				not_after: 1_796_040_000,
				bindings: [
					{
						placement_id: "support-bot",
						project_id: APP.supportPortal,
						service: "https",
					},
				],
			}),
			certificate({
				certificate_id: INTERNAL_MQTT,
				label: "internal-mqtt",
				revision: 2,
				subject: "CN=mqtt.lab.internal",
				issuer: "CN=Rheosoph Internal service issuer",
				dns_names: ["mqtt.lab.internal"],
				ip_addresses: ["10.0.4.20"],
				not_before: 1_788_609_600,
				not_after: 1_791_201_600,
			}),
		],
	},
	certificateRequests: [
		{
			request_id: "966dddf3-b745-483e-8283-0f33254a480d",
			certificate_id: "766b2c58-b339-441f-89a7-5992853a6671",
			label: "billing-api",
			expected_revision: 0,
			dns_names: ["billing.lab.internal"],
			ip_addresses: [],
			csr_pem:
				"-----BEGIN CERTIFICATE REQUEST-----\n…\n-----END CERTIFICATE REQUEST-----\n",
			created_at: 1_789_992_000,
			expires_at: 1_792_584_000,
			purpose: "service",
		},
	],
	certificateIssuers: [
		{
			certificate_id: INTERNAL_MQTT,
			revision: 1,
			dns_names: ["mqtt.lab.internal"],
			ip_addresses: ["10.0.4.20"],
			leaf_lifetime_days: 30,
			not_after: 1_790_766_000,
			last_renewed_at: 1_788_609_600,
			next_renewal_at: 1_790_762_400,
			last_error:
				"Issuing authority has expired or expires within one hour. Install a new delegation.",
		},
	],
	acme: [
		{
			certificate_id: EDGE_API,
			label: "edge-api",
			revision: 3,
			dns_names: ["edge-berlin.rheosoph.example"],
			environment: "lets_encrypt_production",
			http_bind: "0.0.0.0:80",
			next_attempt_at: 1_793_448_000,
			last_renewed_at: 1_788_264_000,
			last_error: null,
		},
	],
	history: [
		{
			scope: "device",
			kind: "logs",
			expiresAt: 1_790_791_200,
			policyVersion: 5,
		},
		{
			scope: "device",
			kind: "metrics",
			expiresAt: 1_790_683_200,
			policyVersion: 3,
		},
	],
	placements: {
		"support-bot": {
			host: "0.0.0.0",
			port: 8_443,
			tlsCertificateId: EDGE_API,
		},
		"invoice-extractor": {
			host: "127.0.0.1",
			port: 8_081,
			tlsCertificateId: null,
			offlineWrites: { maxAgeS: 604_800, maxBytes: 268_435_456 },
			resourceGrantId: "d99ba88b-717b-445e-a719-a2084df3aec0",
		},
	},
};

const STUDIO_LIVE: LiveDeviceInputExt = {
	state: {
		kind: "live",
		transport: "websocket",
		expiresAt: 1_790_769_790,
		bootId: "275db9c05832d9c7",
		connectedAt: 1_790_769_400,
	},
	inspection: { value: STUDIO_INSPECTION, readAt: 1_790_769_599 },
	offlineQueues: { "field-notes": STUDIO_QUEUES },
	placements: {
		"field-notes": {
			host: "127.0.0.1",
			port: 8_090,
			tlsCertificateId: null,
			offlineWrites: { maxAgeS: 604_800, maxBytes: 268_435_456 },
			resourceGrantId: "6e2f1a9c-3b7d-4e05-8a1c-9d4b2f6e0a73",
		},
	},
};

const ACCESS_REQUESTS: NonNullable<AttentionInputExt["accessRequests"]> = [
	{
		deviceId: ID.miraRender,
		deviceName: "mira-render-01",
		ownerId: MIRA,
		createdAt: 1_790_740_800,
		approved: false,
	},
];

function goldenInput(): AttentionInputExt {
	return {
		now: NOW,
		me: ME,
		hub: {
			state: "on",
			serverTime: NOW,
			limits: HUB_LIMITS,
			usage: HUB_USAGE,
		},
		readiness: { version: 1, ready: true, checks: READY_CHECKS },
		releaseTrust: {
			manifest_url:
				"https://releases.flow-like.com/standalone/stable/release.jws",
			minimum_sequence: 41,
		},
		latestRelease: { version: "0.9.4", sequence: 44 },
		usage: { limits: { ...HUB_LIMITS }, usage: { ...HUB_USAGE } },
		clock: { hubOffsetS: 0, deviceSkewS: {} },
		devices: DEVICES,
		pendingSetups: PENDING_SETUPS,
		accountBackups: ACCOUNT_BACKUPS,
		accountBackupSlots: { used: 4, max: 256 },
		myAccess: MY_ACCESS,
		certInventory: CERT_INVENTORY,
		resources: RESOURCES,
		resourceSummary: resourceSummaryOf(RESOURCES, ME, NOW),
		archiveUsage: ARCHIVE_USAGE,
		keys: KEYS,
		local: LOCAL,
		fleet: FLEET,
		live: { [ID.edge]: EDGE_LIVE, [ID.studio]: STUDIO_LIVE },
		policies: POLICIES,
		authorities: [],
		activity: ACTIVITY,
		agentLastRead: { [ID.warehouse]: { version: "0.9.2", at: 1_790_676_000 } },
		accessRequests: ACCESS_REQUESTS,
	};
}

/** An input with no devices and nothing loaded, for rule tests. */
export function emptyInput(now = NOW): AttentionInputExt {
	return {
		now,
		me: ME,
		hub: { state: "on" },
		devices: [],
		accountBackups: {},
		certInventory: {},
		resources: {},
		keys: [],
		local: {
			platform: "desktop",
			persistence: "persisted",
			webLocks: true,
			indexedDb: true,
			cryptoLoaded: true,
			vaults: [],
			backups: {},
			authorities: [],
		},
		fleet: {},
		live: {},
		policies: {},
		authorities: [],
		activity: [],
	};
}

const HUB_ONLY_ROW_FIELDS = [
	"display_name",
	"relationship",
	"access_expires_at",
	"access_rules_expire_at",
	"revoked_at",
	"cloud_approvals",
	"auth_rejection",
] as const;

/** A hub without BG1–BG6, BG22, BG25, BG28, BG30, FG4: rows carry only `DeviceStatus` + identity. */
export function sampleFleetOlderHub(): AttentionInputExt {
	const input = sampleFleet();
	const devices = input.devices.map((row) => {
		const copy: DeviceRow = { ...row };
		for (const field of HUB_ONLY_ROW_FIELDS) delete copy[field];
		return copy;
	});
	const {
		usage: _usage,
		accountBackupSlots: _slots,
		myAccess: _myAccess,
		resourceSummary: _summary,
		archiveUsage: _archive,
		...rest
	} = input;
	return {
		...rest,
		hub: { state: "on" },
		devices,
		pendingSetups: input.pendingSetups?.filter((setup) => setup.local),
	};
}

function olderAgentPlacement(row: PlacementStatusPlus): PlacementStatusPlus {
	const {
		process_id: _process,
		last_error: _error,
		has_error: _hasError,
		restarts: _restarts,
		offline_writes: _writes,
		source: _source,
		events: _events,
		events_truncated: _truncated,
		online_metadata_sha256: _metadata,
		...rest
	} = row;
	return rest;
}

function olderAgentInspection(value: InspectionPlus): InspectionPlus {
	const {
		agent: _agent,
		host: _host,
		tasks: _tasks,
		hostOperation: _operation,
		network: _network,
		...rest
	} = value;
	return {
		...rest,
		features: {},
		placements: value.placements.map(olderAgentPlacement),
	};
}

/** Agents before `features` (§3.4.1): no task health, host operation, events, diagnostics or summaries. */
export function sampleFleetOlderAgent(): AttentionInputExt {
	const input = sampleFleet();
	const live = Object.fromEntries(
		Object.entries(input.live).map(([deviceId, entry]) => [
			deviceId,
			entry.inspection
				? {
						...entry,
						inspection: {
							...entry.inspection,
							value: olderAgentInspection(entry.inspection.value),
						},
					}
				: entry,
		]),
	);
	const fleet = Object.fromEntries(
		Object.entries(input.fleet).map(([deviceId, state]) => [
			deviceId,
			state.status
				? {
						...state,
						status: {
							...state.status,
							observations: state.status.observations.map((entry) => ({
								...entry,
								placements: entry.placements.map(olderAgentPlacement),
							})),
						},
					}
				: state,
		]),
	);
	return { ...input, live, fleet };
}
