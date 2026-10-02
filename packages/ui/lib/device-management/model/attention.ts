import { ACCESS_RULES } from "./attention-rules/access";
import { CERTIFICATE_RULES } from "./attention-rules/certificates";
import { CLOUD_RULES, revokedApprovalServices } from "./attention-rules/cloud";
import { DEVICE_RULES } from "./attention-rules/device";
import { HISTORY_RULES } from "./attention-rules/history";
import { KEY_RULES } from "./attention-rules/keys";
import { OFFLINE_WRITE_RULES } from "./attention-rules/offline-writes";
import { OPERATION_RULES } from "./attention-rules/operations";
import { SERVICE_RULES } from "./attention-rules/services";
import { SETUP_RULES } from "./attention-rules/setup";
import {
	DAY_S,
	type DeviceFacts,
	type FleetFacts,
	deviceListKnown,
	fleetFacts,
	subjectDevice,
} from "./device-view";
import type {
	AttentionCandidate,
	AttentionInput,
	AttentionItem,
	AttentionKey,
	AttentionMemory,
	AttentionRule,
	AttentionSubject,
	LiveDeviceInput,
	Severity,
} from "./types";

export {
	buildDeviceView,
	buildServiceViews,
	rollupHealth,
} from "./device-view";

/* CA11 carries every input W1-ATTN first declared here; the names stay for the files that import them. */

export type {
	AccessRequestRecord,
	AgentLastRead,
	PlacementConfigFacts,
} from "./types";

export type LiveDeviceInputExt = LiveDeviceInput;
export type AttentionInputExt = AttentionInput;

export type AttentionCandidateExt = AttentionCandidate & {
	/** When the condition started, if the data says so (earlier than the first sighting). */
	since?: number;
};

export interface AttentionRuleExt extends AttentionRule {
	evaluate(input: AttentionInputExt): AttentionCandidateExt[];
}

export type AttentionGroup =
	| "device"
	| "setup"
	| "keys"
	| "access"
	| "services"
	| "offline_writes"
	| "cloud"
	| "certificates"
	| "history"
	| "operations";

/** Every IA §6.5 condition key, by group. Exhaustive: a missing key fails `tsc`. */
export const ATTENTION_KEY_GROUPS: Readonly<
	Record<AttentionKey, AttentionGroup>
> = {
	offline_since: "device",
	late: "device",
	no_heartbeat_since_enrollment: "device",
	revoked: "device",
	you_still_pay_for_a_revoked_device: "device",
	identity_mismatch: "device",
	snapshot_integrity_error: "device",
	clock_skew: "device",
	access_denied: "device",
	status_stale_while_online: "device",
	background_task_failing: "device",
	agent_update_available: "device",
	rebooted_unexpectedly: "device",
	status_subscription_expiring: "device",
	device_slots_nearly_full: "device",
	pending_setup_waiting: "setup",
	pending_setup_expired: "setup",
	hub_not_ready: "setup",
	release_trust_missing: "setup",
	keys_missing_here: "keys",
	keys_not_backed_up_to_account: "keys",
	account_backup_upload_pending: "keys",
	account_backup_old_password: "keys",
	account_backup_hub_newer: "keys",
	storage_not_persistent: "keys",
	request_keys_unbacked: "keys",
	backup_slots_nearly_full: "keys",
	stale_local_keys: "keys",
	shared_access_expiring: "access",
	shared_access_ended: "access",
	grant_expiring: "access",
	sharing_policy_waiting_for_device: "access",
	sharing_policy_expiring: "access",
	sharing_policy_expired: "access",
	access_slots_nearly_full: "access",
	access_request_pending: "access",
	code_running_access_without_sandbox: "access",
	service_crash_looping: "services",
	service_not_as_requested: "services",
	service_settings_not_applied: "services",
	service_degraded: "services",
	rollout_in_progress: "services",
	rollout_failed_service_stopped: "services",
	rollout_rolled_back: "services",
	rollout_not_applied: "services",
	rollout_staged_waiting: "services",
	secret_write_pending: "services",
	secret_write_failed: "services",
	endpoint_unencrypted_exposed: "services",
	event_tokens_after_revoke: "services",
	offline_writes_conflict: "offline_writes",
	offline_writes_blocked: "offline_writes",
	offline_writes_outcome_unknown: "offline_writes",
	offline_writes_quarantined: "offline_writes",
	offline_writes_backlog: "offline_writes",
	offline_mirror_error: "offline_writes",
	cloud_access_invalid: "cloud",
	cloud_access_ending: "cloud",
	spending_limit_low: "cloud",
	spending_limit_ending: "cloud",
	online_files_read_only: "cloud",
	certificate_expired: "certificates",
	certificate_expiring: "certificates",
	certificate_not_yet_valid: "certificates",
	renewal_delegation_error: "certificates",
	renewal_authority_expiring: "certificates",
	acme_error: "certificates",
	acme_staging_in_use: "certificates",
	signing_request_attention: "certificates",
	certificate_inventory_stale: "certificates",
	certificate_slots_nearly_full: "certificates",
	org_ca_signing_key_expiring: "certificates",
	org_ca_root_expiring: "certificates",
	history_paused_readers_expired: "history",
	history_paused_access_changed: "history",
	history_not_stored_by_plan: "history",
	history_storage_nearly_full: "history",
	metric_readers_expiring: "history",
	unconfirmed_command: "operations",
	device_operation_failed: "operations",
	device_operation_unknown: "operations",
	upload_paused: "operations",
};

export const ATTENTION_KEYS = Object.keys(
	ATTENTION_KEY_GROUPS,
) as readonly AttentionKey[];

/** Keys without a rule yet, and the missing input. */
export const DEFERRED_KEYS: Readonly<Partial<Record<AttentionKey, string>>> = {
	event_tokens_after_revoke:
		"local deploy record of hosted event tokens (IA §6.5, audit #41, not re-verified)",
};

export const ATTENTION_RULES: readonly AttentionRuleExt[] = [
	...SERVICE_RULES,
	...OFFLINE_WRITE_RULES,
	...CLOUD_RULES,
	...DEVICE_RULES,
	...CERTIFICATE_RULES,
	...ACCESS_RULES,
	...HISTORY_RULES,
	...KEY_RULES,
	...SETUP_RULES,
	...OPERATION_RULES,
];

export const SEVERITY_RANK: Readonly<Record<Severity, number>> = {
	critical: 0,
	warning: 1,
	notice: 2,
	info: 3,
};

/** IA §6.5: service > device > certificate > access > keys > setup. */
export const SUBJECT_RANK: Readonly<Record<AttentionSubject["kind"], number>> =
	{
		service: 0,
		device: 1,
		certificate: 2,
		authority: 2,
		access: 3,
		keys: 4,
		setup: 5,
		hub: 5,
	};

export const SNOOZE_S = 7 * DAY_S;

const KEPT_WHEN_REVOKED = new Set<AttentionKey>([
	"revoked",
	"you_still_pay_for_a_revoked_device",
	"stale_local_keys",
]);

function compareCandidates(a: AttentionCandidateExt, b: AttentionCandidateExt) {
	return SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity];
}

interface DedupeContext {
	input: AttentionInputExt;
	facts: FleetFacts;
	revokedApprovals: ReadonlySet<string>;
}

/** (d) a revoked device keeps only consent and local-key items. */
const droppedAsRevoked = (
	candidate: AttentionCandidateExt,
	device: DeviceFacts | undefined,
) =>
	!!device &&
	!device.active &&
	!KEPT_WHEN_REVOKED.has(candidate.key) &&
	candidate.subject.kind !== "keys";

/** (a) offline: service items become last known, not-as-requested is dropped. */
const offlineAdjusted = (
	item: AttentionCandidateExt,
	device: DeviceFacts | undefined,
) => {
	if (device?.presence.kind !== "offline" || item.subject.kind !== "service")
		return item;
	return item.key === "service_not_as_requested"
		? undefined
		: { ...item, lastKnown: true };
};

/** (b) expired access rules suppress history-paused items. */
const droppedByExpiredRules = (
	item: AttentionCandidateExt,
	input: AttentionInputExt,
) => {
	const deviceId = subjectDevice(item.subject);
	if (!deviceId || !item.key.startsWith("history_paused_")) return false;
	const expiresAt = input.policies[deviceId]?.policy?.expires_at;
	return expiresAt !== undefined && expiresAt <= input.now;
};

/** (c) a revoked approval suppresses quarantined offline writes. */
const droppedByRevokedApproval = (
	item: AttentionCandidateExt,
	revokedApprovals: ReadonlySet<string>,
) =>
	item.key === "offline_writes_quarantined" &&
	item.subject.kind === "service" &&
	revokedApprovals.has(`${item.subject.deviceId}/${item.subject.serviceId}`);

const dedupeOne = (candidate: AttentionCandidateExt, ctx: DedupeContext) => {
	const deviceId = subjectDevice(candidate.subject);
	const device = deviceId ? ctx.facts.byId.get(deviceId) : undefined;
	if (droppedAsRevoked(candidate, device)) return undefined;
	const item = offlineAdjusted(candidate, device);
	if (
		!item ||
		droppedByExpiredRules(item, ctx.input) ||
		droppedByRevokedApproval(item, ctx.revokedApprovals)
	)
		return undefined;
	return item;
};

/** M-DATA §3.8 dedupe rules (a)–(e); (e) is one item per id. */
function dedupe(
	candidates: AttentionCandidateExt[],
	input: AttentionInputExt,
): AttentionCandidateExt[] {
	const ctx: DedupeContext = {
		input,
		facts: fleetFacts(input),
		revokedApprovals: revokedApprovalServices(input),
	};
	const seen = new Set<string>();
	return [...candidates].sort(compareCandidates).flatMap((candidate) => {
		const item = dedupeOne(candidate, ctx);
		if (!item || seen.has(item.id)) return [];
		seen.add(item.id);
		return [item];
	});
}

export function compareAttention(a: AttentionItem, b: AttentionItem): number {
	return (
		SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity] ||
		SUBJECT_RANK[a.subject.kind] - SUBJECT_RANK[b.subject.kind] ||
		a.firstSeenAt - b.firstSeenAt ||
		a.id.localeCompare(b.id)
	);
}

function toItem(
	candidate: AttentionCandidateExt,
	firstSeenAt: number,
): AttentionItem {
	const { since: _since, dwellS: _dwell, ...item } = candidate;
	return { ...item, firstSeenAt };
}

/** Conditions read from a device's service rows: unknown, not cleared, while those rows can't be read. */
const SERVICE_ROW_GROUPS = new Set<AttentionGroup>([
	"services",
	"offline_writes",
]);
const KEEP_UNREAD_S = 30 * DAY_S;

/** `id` is `${key}:${deviceId}/…` for these groups. */
const stillUnknown = (
	id: string,
	input: AttentionInputExt,
	memory: AttentionMemory,
) => {
	const split = id.indexOf(":");
	const group = ATTENTION_KEY_GROUPS[id.slice(0, split) as AttentionKey];
	if (!SERVICE_ROW_GROUPS.has(group)) return false;
	if (input.now - (memory.firstSeen[id] ?? 0) > KEEP_UNREAD_S) return false;
	const deviceId = id.slice(split + 1).split("/")[0];
	const device = fleetFacts(input).byId.get(deviceId);
	return !!device?.active && !Array.isArray(device.services);
};

/**
 * Forgets cleared conditions and ended snoozes; true when memory changed.
 * After a reload every device is locked and lists are still loading: a
 * condition that can't be evaluated keeps its first sighting (dwell, "since").
 */
const pruneMemory = (
	memory: AttentionMemory,
	current: ReadonlySet<string>,
	input: AttentionInputExt,
) => {
	const { now } = input;
	const cleared = !deviceListKnown(input)
		? []
		: Object.keys(memory.firstSeen).filter(
				(id) => !current.has(id) && !stillUnknown(id, input, memory),
			);
	const woken = Object.keys(memory.snoozed).filter(
		(id) => (memory.snoozed[id] ?? 0) <= now,
	);
	for (const id of cleared) delete memory.firstSeen[id];
	for (const id of woken) delete memory.snoozed[id];
	return cleared.length > 0 || woken.length > 0;
};

/** First sighting, or the data-known start when earlier. */
const recordFirstSeen = (
	memory: AttentionMemory,
	candidate: AttentionCandidateExt,
	now: number,
) => {
	const known = memory.firstSeen[candidate.id];
	const firstSeenAt = Math.min(known ?? now, candidate.since ?? now, now);
	memory.firstSeen[candidate.id] = firstSeenAt;
	return { firstSeenAt, changed: known !== firstSeenAt };
};

/** Dwell conditions wait their time; snoozed notices stay hidden. */
const shown = (
	memory: AttentionMemory,
	candidate: AttentionCandidateExt,
	firstSeenAt: number,
	now: number,
) => {
	const dwelling =
		candidate.dwellS !== undefined && now - firstSeenAt < candidate.dwellS;
	const snoozed =
		candidate.severity === "notice" &&
		memory.snoozed[candidate.id] !== undefined;
	return !dwelling && !snoozed;
};

/** Evaluate → dedupe → dwell/snooze via memory → order. Updates and saves `memory` when it changes. */
export function computeAttention(
	input: AttentionInputExt,
	memory: AttentionMemory,
): AttentionItem[] {
	const { now } = input;
	const candidates = dedupe(
		ATTENTION_RULES.flatMap((rule) => rule.evaluate(input)),
		input,
	);
	const ids = new Set(candidates.map((candidate) => candidate.id));
	let changed = pruneMemory(memory, ids, input);
	const items: AttentionItem[] = [];
	for (const candidate of candidates) {
		const seen = recordFirstSeen(memory, candidate, now);
		changed ||= seen.changed;
		if (shown(memory, candidate, seen.firstSeenAt, now))
			items.push(toItem(candidate, seen.firstSeenAt));
	}
	if (changed) memory.save();
	return items.sort(compareAttention);
}

/** Notices only (IA §6.5): hidden for 7 days on this computer. */
export function snoozeAttention(
	memory: AttentionMemory,
	item: Pick<AttentionItem, "id" | "severity">,
	now: number,
): boolean {
	if (item.severity !== "notice") return false;
	memory.snoozed[item.id] = now + SNOOZE_S;
	memory.save();
	return true;
}

export interface AttentionFilter {
	deviceId?: string;
	serviceId?: string;
	/** App scope: the app's services, plus device items of devices that run it (APP A10). */
	appId?: string;
	minSeverity?: Severity;
}

function runsApp(input: AttentionInputExt, deviceId: string, appId: string) {
	const services = fleetFacts(input).byId.get(deviceId)?.services;
	return (
		Array.isArray(services) &&
		services.some((service) => service.projectId === appId)
	);
}

const matchesSubject = (subject: AttentionSubject, filter: AttentionFilter) =>
	(!filter.deviceId || subjectDevice(subject) === filter.deviceId) &&
	(!filter.serviceId ||
		(subject.kind === "service" && subject.serviceId === filter.serviceId));

/** App scope (APP A10): items naming the app, its services, and device items of devices that run it. */
const matchesApp = (
	item: AttentionItem,
	appId: string,
	input: AttentionInputExt,
) => {
	const itemApp = item.copy.params?.app;
	if (typeof itemApp === "string") return itemApp === appId;
	const { subject } = item;
	if (subject.kind === "service") return subject.projectId === appId;
	return subject.kind === "device" && runsApp(input, subject.deviceId, appId);
};

export function filterAttention(
	items: readonly AttentionItem[],
	filter: AttentionFilter,
	input: AttentionInputExt,
): AttentionItem[] {
	const maxRank = SEVERITY_RANK[filter.minSeverity ?? "info"];
	return items.filter(
		(item) =>
			SEVERITY_RANK[item.severity] <= maxRank &&
			matchesSubject(item.subject, filter) &&
			(!filter.appId || matchesApp(item, filter.appId, input)),
	);
}

export interface AttentionCounts {
	critical: number;
	warning: number;
	notice: number;
	info: number;
	/** Counted items: Info never counts. */
	total: number;
}

export function countAttention(
	items: readonly AttentionItem[],
): AttentionCounts {
	const counts: AttentionCounts = {
		critical: 0,
		warning: 0,
		notice: 0,
		info: 0,
		total: 0,
	};
	for (const item of items) {
		counts[item.severity]++;
		if (item.severity !== "info") counts.total++;
	}
	return counts;
}

const MEMORY_PREFIX = "fl.devices.attention.";

type MemoryStorage = Pick<Storage, "getItem" | "setItem">;

function browserStorage(): MemoryStorage | undefined {
	try {
		return globalThis.localStorage ?? undefined;
	} catch {
		return undefined;
	}
}

function numberRecord(value: unknown): Record<string, number> {
	if (!value || typeof value !== "object" || Array.isArray(value)) return {};
	return Object.fromEntries(
		Object.entries(value).filter(
			(entry): entry is [string, number] =>
				typeof entry[1] === "number" && Number.isFinite(entry[1]),
		),
	);
}

/** Dwell and snooze memory per account scope; storage failures degrade to memory only. */
export function createAttentionMemory(
	scopeKey: string,
	storage: MemoryStorage | undefined = browserStorage(),
): AttentionMemory {
	const key = `${MEMORY_PREFIX}${scopeKey}`;
	let stored: { firstSeen?: unknown; snoozed?: unknown } = {};
	try {
		stored = JSON.parse(storage?.getItem(key) ?? "{}") ?? {};
	} catch {
		stored = {};
	}
	const memory: AttentionMemory = {
		firstSeen: numberRecord(stored.firstSeen),
		snoozed: numberRecord(stored.snoozed),
		save() {
			try {
				storage?.setItem(
					key,
					JSON.stringify({
						firstSeen: memory.firstSeen,
						snoozed: memory.snoozed,
					}),
				);
			} catch {
				/* private window or quota: memory stays in this tab */
			}
		},
	};
	return memory;
}
