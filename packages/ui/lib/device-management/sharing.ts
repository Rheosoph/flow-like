import { sha256 } from "@noble/hashes/sha2";
import { z } from "zod";
import { base64url } from "./crypto";
import { MAX_GRANTS } from "./model/gates";
import {
	type PermissionPreset,
	isDeviceOnly,
	orderCapabilities,
	presetOf,
	runsCode,
} from "./model/permissions";
import type { HostIsolationMode } from "./model/types";
import type {
	Capability,
	DeviceReceipt,
	Ed25519PublicKey,
	InventoryScope,
	ManagementGrant,
	ManagementPolicy,
	PolicyView,
} from "./types";

const DAY_S = 86_400;

/** Connection files and access request files are refused above this size. */
export const ACCESS_FILE_MAX_BYTES = 128 * 1024;
/** Every saved change re-signs the access rules for this long. */
export const ACCESS_RULES_LIFETIME_S = 31 * DAY_S;
export const DEFAULT_ACCESS_S = DAY_S;
export const ACCESS_DURATIONS_S = [
	3600,
	8 * 3600,
	DAY_S,
	3 * DAY_S,
	7 * DAY_S,
	14 * DAY_S,
	31 * DAY_S,
] as const;
const RENEWAL_STEPS_S = [DAY_S, 3 * DAY_S, 7 * DAY_S, 14 * DAY_S] as const;
/** Access that ends within this window is called out. */
export const ACCESS_ENDING_SOON_S = 12 * 3600;
export const ACCESS_RULES_ENDING_SOON_S = 7 * DAY_S;

export { MAX_GRANTS };

/* Host isolation as the device reports it. */

export interface HostIsolation {
	platform: string;
	sandboxAvailable: boolean;
	requireIsolation: boolean;
}

export function parseHostIsolation(value: unknown): HostIsolation | undefined {
	if (!value || typeof value !== "object") return undefined;
	const row = value as Record<string, unknown>;
	if (
		typeof row.platform !== "string" ||
		typeof row.sandbox_available !== "boolean" ||
		typeof row.require_isolation !== "boolean"
	)
		return undefined;
	return {
		platform: row.platform.slice(0, 64),
		sandboxAvailable: row.sandbox_available,
		requireIsolation: row.require_isolation,
	};
}

/** Unknown counts as "no sandbox": the trust confirmation is asked unless the device requires one. */
export function isolationModeOf(
	isolation: HostIsolation | undefined,
): HostIsolationMode | null {
	if (!isolation) return null;
	if (isolation.requireIsolation) return "required";
	return isolation.sandboxAvailable ? "optional" : "none";
}

/* Errors the screens translate by code. */

export type AccessRulesErrorCode =
	| "locked"
	| "wrong_password"
	| "signing_failed"
	| "unverified"
	| "incomplete"
	| "wrong_device"
	| "wrong_version"
	| "grant_gone"
	| "grant_conflict"
	| "duplicate_grant"
	| "too_many"
	| "no_permissions"
	| "certificates_unsupported";

export class AccessRulesError extends Error {
	constructor(
		readonly code: AccessRulesErrorCode,
		message: string,
		readonly params: Readonly<Record<string, string | number>> = {},
	) {
		super(message);
		this.name = "AccessRulesError";
	}
}

/* Files exchanged between owner and recipient (public, no secrets). */

const GRANT_ID = /^[A-Za-z0-9_:.\-]{1,128}$/u;
const KEY_X = /^[A-Za-z0-9_-]{43}$/u;

const keySchema = z.object({
	kty: z.literal("OKP"),
	crv: z.literal("Ed25519"),
	x: z.string().regex(KEY_X),
});

const recipientSchema = z.object({
	user_id: z.string().min(1).max(128),
	grant_id: z.string().regex(GRANT_ID).optional(),
	controller_key: keySchema,
});

export type AccessRecipient = z.infer<typeof recipientSchema>;

export type AccessFileError =
	| "too_large"
	| "not_request_file"
	| "too_many_people"
	| "incomplete_entry"
	| "not_connection_file";

export type ParsedRequestFile =
	| { ok: true; recipients: AccessRecipient[] }
	| { ok: false; error: AccessFileError; count?: number };

function parseJson(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		return undefined;
	}
}

/** An access request file: the people asking, each with their account, public key and request id. */
export function parseAccessRequestFile(
	text: string,
	bytes: number,
): ParsedRequestFile {
	if (bytes > ACCESS_FILE_MAX_BYTES) return { ok: false, error: "too_large" };
	const rows = parseJson(text);
	if (!Array.isArray(rows) || rows.length === 0)
		return { ok: false, error: "not_request_file" };
	if (rows.length > MAX_GRANTS)
		return { ok: false, error: "too_many_people", count: rows.length };
	const recipients: AccessRecipient[] = [];
	for (const row of rows) {
		const parsed = recipientSchema.safeParse(row);
		if (!parsed.success) return { ok: false, error: "incomplete_entry" };
		recipients.push({
			user_id: parsed.data.user_id,
			controller_key: {
				kty: "OKP",
				crv: "Ed25519",
				x: parsed.data.controller_key.x,
			},
			...(parsed.data.grant_id ? { grant_id: parsed.data.grant_id } : {}),
		});
	}
	return { ok: true, recipients };
}

export function accessRequestFileText(
	account: string,
	controllerKey: Ed25519PublicKey,
	grantId: string,
): string {
	return JSON.stringify(
		[{ user_id: account, controller_key: controllerKey, grant_id: grantId }],
		null,
		2,
	);
}

export const accessRequestFileName = (deviceId: string) =>
	`device-access-${deviceId}.json`;
export const connectionFileName = (deviceId: string) =>
	`flow-like-connection-${deviceId}.json`;

const REQUEST_FILE_DEVICE =
	/device-access-([0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}|[0-9a-f]{8})/iu;

/** The device a request file was made for, read from its name; a request file itself does not name one. */
export function requestFileDevice(
	fileName: string,
	deviceIds: readonly string[],
): string | undefined {
	const named = REQUEST_FILE_DEVICE.exec(fileName)?.[1]?.toLowerCase();
	if (!named) return undefined;
	const matches = deviceIds.filter((id) => id.toLowerCase().startsWith(named));
	return matches.length === 1 ? matches[0] : undefined;
}

export interface ConnectionFile {
	version: 1;
	receipt: DeviceReceipt;
	owner_controller_key: Ed25519PublicKey;
}

export type ParsedConnectionFile =
	| { ok: true; file: ConnectionFile }
	| { ok: false; error: AccessFileError };

const connectionSchema = z.object({
	version: z.literal(1),
	receipt: z
		.object({ manifest_jws: z.string().min(1), device_id: z.string().min(1) })
		.passthrough(),
	owner_controller_key: keySchema,
});

/** Shape only; the signature is checked with the device crypto afterwards. */
export function parseConnectionFile(
	text: string,
	bytes: number,
): ParsedConnectionFile {
	if (bytes > ACCESS_FILE_MAX_BYTES) return { ok: false, error: "too_large" };
	const parsed = connectionSchema.safeParse(parseJson(text));
	if (!parsed.success) return { ok: false, error: "not_connection_file" };
	return { ok: true, file: parsed.data as unknown as ConnectionFile };
}

export function connectionFileText(
	receipt: DeviceReceipt,
	ownerControllerKey: Ed25519PublicKey,
): string {
	return JSON.stringify({
		version: 1,
		receipt,
		owner_controller_key: ownerControllerKey,
	});
}

/** Keys this computer already holds for the device of a connection file. */
export type RequestKeysState = "none" | "reusable" | "conflict";

/** Keys made earlier for the same connection file can hand out their request again; any other keys block a new request. */
export function requestKeysState(
	existing:
		| {
				grantId: string;
				ownerControllerKey?: Ed25519PublicKey;
				manifestJws: string;
		  }
		| undefined,
	file: ConnectionFile,
): RequestKeysState {
	if (!existing) return "none";
	return existing.grantId !== "owner" &&
		existing.ownerControllerKey?.x === file.owner_controller_key.x &&
		existing.manifestJws === file.receipt.manifest_jws
		? "reusable"
		: "conflict";
}

/* Grants. */

const heldByAnother = (
	grant: ManagementGrant,
	userId: string,
	key: Ed25519PublicKey,
) => grant.user_id !== userId || grant.controller_key.x !== key.x;

function checkRecipientIds(
	active: readonly ManagementGrant[],
	recipients: readonly {
		grant_id?: string;
		user_id: string;
		controller_key: Ed25519PublicKey;
	}[],
): Set<string> {
	const replaced = new Set<string>();
	for (const recipient of recipients) {
		const grantId = recipient.grant_id;
		if (!grantId) continue;
		if (replaced.has(grantId))
			throw new AccessRulesError(
				"duplicate_grant",
				`Grant ${grantId} is listed more than once. List each access request once.`,
				{ grantId },
			);
		const existing = active.find((grant) => grant.grant_id === grantId);
		if (
			existing &&
			heldByAnother(existing, recipient.user_id, recipient.controller_key)
		)
			throw new AccessRulesError(
				"grant_conflict",
				`Grant ${grantId} already belongs to another account or controller. Ask ${recipient.user_id} for a new access request.`,
				{ grantId, userId: recipient.user_id },
			);
		replaced.add(grantId);
	}
	return replaced;
}

/** Re-approving a recipient's own grant replaces it; a grant id held by anyone else is refused. */
export function mergeRecipientGrants(
	active: ManagementGrant[],
	recipients: AccessRecipient[],
	template: Omit<ManagementGrant, "grant_id" | "user_id" | "controller_key">,
	newGrantId: () => string = () => crypto.randomUUID(),
): ManagementGrant[] {
	const replaced = checkRecipientIds(active, recipients);
	return [
		...active.filter((grant) => !replaced.has(grant.grant_id)),
		...recipients.map((recipient) => ({
			...template,
			grant_id: recipient.grant_id ?? newGrantId(),
			user_id: recipient.user_id,
			controller_key: recipient.controller_key,
		})),
	];
}

export function sameScope(a: InventoryScope, b: InventoryScope): boolean {
	if (a.kind !== b.kind) return false;
	if (a.kind === "device") return true;
	if (a.project_id !== (b as typeof a).project_id) return false;
	return a.kind === "project"
		? true
		: a.placement_id === (b as typeof a).placement_id;
}

export function sameCapabilities(
	a: readonly Capability[],
	b: readonly Capability[],
): boolean {
	const left = orderCapabilities(a);
	const right = orderCapabilities(b);
	return (
		left.length === right.length &&
		left.every((capability, index) => capability === right[index])
	);
}

export type GrantChangeKind = "new" | "changed" | "renewed" | "same";

export function grantChangeKind(
	before:
		| Pick<ManagementGrant, "scope" | "capabilities" | "expires_at">
		| undefined,
	after: Pick<ManagementGrant, "scope" | "capabilities" | "expires_at">,
): GrantChangeKind {
	if (!before) return "new";
	if (
		!sameCapabilities(before.capabilities, after.capabilities) ||
		!sameScope(before.scope, after.scope)
	)
		return "changed";
	return before.expires_at === after.expires_at ? "same" : "renewed";
}

export interface CapabilityChange {
	capability: Capability;
	kind: "added" | "removed" | "kept";
}

/** Every permission either side holds, in display order, with what happens to it. */
export function capabilityDiff(
	before: readonly Capability[],
	after: readonly Capability[],
): CapabilityChange[] {
	return orderCapabilities([...before, ...after]).map((capability) => ({
		capability,
		kind: !before.includes(capability)
			? "added"
			: after.includes(capability)
				? "kept"
				: "removed",
	}));
}

/* Which permissions can be chosen. */

export type PermissionBlock =
	| "device_only"
	| "certificates_unsupported"
	| "certificates_unknown";

/**
 * Why a permission cannot be given: device-wide permissions need whole-device
 * access, and Manage certificates needs an agent that supports sharing it
 * (`undefined` = the device was not read live yet). A permission the person
 * already holds is never blocked by agent support.
 */
export function permissionBlock(
	capability: Capability,
	scopeKind: InventoryScope["kind"],
	certificateSupport: boolean | undefined,
	alreadyHeld = false,
): PermissionBlock | null {
	if (scopeKind !== "device" && isDeviceOnly(capability)) return "device_only";
	if (
		capability === "manage_certificates" &&
		!alreadyHeld &&
		certificateSupport !== true
	)
		return certificateSupport === false
			? "certificates_unsupported"
			: "certificates_unknown";
	return null;
}

/** The chosen permissions a scope can hold, in display order. */
export function scopedCapabilities(
	capabilities: readonly Capability[],
	scopeKind: InventoryScope["kind"],
): Capability[] {
	return orderCapabilities(capabilities).filter(
		(capability) => scopeKind === "device" || !isDeviceOnly(capability),
	);
}

export function codeCapabilities(
	capabilities: readonly Capability[],
): Capability[] {
	return orderCapabilities(capabilities).filter((capability) =>
		runsCode([capability]),
	);
}

/**
 * The owner confirms full device trust when a change adds a code-running
 * permission on a device that does not require a sandbox (unknown counts as
 * not required).
 */
export function needsTrustConfirmation(
	before: readonly Capability[] | undefined,
	after: readonly Capability[],
	isolation: HostIsolationMode | null | undefined,
): boolean {
	if (isolation === "required") return false;
	return codeCapabilities(after).some(
		(capability) => !before?.includes(capability),
	);
}

/* Durations. */

export const rulesExpiryAfterSave = (now: number) =>
	now + ACCESS_RULES_LIFETIME_S;

/** Access never outlasts the rules that carry it. */
export function grantExpiry(now: number, durationS: number): number {
	return Math.min(now + Math.max(0, durationS), rulesExpiryAfterSave(now));
}

export interface RenewalOption {
	/** `rules` = until the re-signed rules expire. */
	id: string;
	until: number;
	addS?: number;
}

/** "1 more day … 14 more days" from the current end (or from now once it ended), then "until the access rules expire". */
export function renewalOptions(
	expiresAt: number,
	now: number,
): RenewalOption[] {
	const base = Math.max(expiresAt, now);
	const cap = rulesExpiryAfterSave(now);
	const steps = RENEWAL_STEPS_S.filter((step) => base + step <= cap).map(
		(step) => ({ id: String(step), addS: step, until: base + step }),
	);
	return [...steps, { id: "rules", until: cap }];
}

/* Access rules as the screens show them. */

export interface AccessRules {
	saved: number;
	applied: number;
	waiting: boolean;
	/** Unix seconds; from the verified rules, else from the hub's device row. */
	expiresAt?: number;
	issuedAt?: number;
}

/** `null` = nothing was ever shared; `undefined` view = not read yet. */
export function accessRulesOf(
	view: Pick<PolicyView, "version" | "applied_version"> | undefined,
	policy?: Pick<ManagementPolicy, "expires_at" | "issued_at">,
	rowExpiry?: number | null,
): AccessRules | null | undefined {
	if (!view) return undefined;
	if (view.version === 0) return null;
	const expiresAt = policy?.expires_at ?? rowExpiry ?? undefined;
	return {
		saved: view.version,
		applied: view.applied_version,
		waiting: view.applied_version < view.version,
		...(expiresAt === undefined ? {} : { expiresAt }),
		...(policy ? { issuedAt: policy.issued_at } : {}),
	};
}

export type AccessChangeKind = "added" | "changed" | "renewed" | "removed";

export interface AccessChangeEntry {
	grantId: string;
	userId: string;
	kind: AccessChangeKind;
	/** The grant as it stood before a change or removal. */
	before?: ManagementGrant;
	after?: ManagementGrant;
}

/** One saved version of a device's access rules, as this computer made it (BG13 interim). */
export interface AccessChange {
	deviceId: string;
	version: number;
	/** Unix seconds. */
	savedAt: number;
	entries: AccessChangeEntry[];
}

export type GrantStatus = "active" | "waiting" | "removing" | "expired";

export interface GrantRow {
	grant: ManagementGrant;
	status: GrantStatus;
	preset: PermissionPreset | "custom";
	count: number;
	runsCode: boolean;
	/** The rules version that carries this row's pending change. */
	pendingVersion?: number;
	/** Added in a version the device has not applied: the person cannot connect yet. */
	isNew?: boolean;
}

function rowOf(
	grant: ManagementGrant,
	status: GrantStatus,
	extra: Pick<GrantRow, "pendingVersion" | "isNew"> = {},
): GrantRow {
	const { preset, count } = presetOf(grant.capabilities);
	return {
		grant,
		status,
		preset,
		count,
		runsCode: runsCode(grant.capabilities),
		...extra,
	};
}

/**
 * The people of one device's rules. While the device still uses an older
 * version, this computer's change log says which rows wait; versions it did
 * not make leave every row waiting, because any of them may have changed.
 */
export function grantRows(
	policy: Pick<ManagementPolicy, "grants">,
	view: Pick<PolicyView, "version" | "applied_version">,
	now: number,
	changes: readonly AccessChange[] = [],
): GrantRow[] {
	const waiting = view.applied_version < view.version;
	const pending = pendingChanges(view, changes);
	const rows = policy.grants.map((grant) => {
		if (grant.expires_at <= now) return rowOf(grant, "expired");
		return waiting
			? waitingRow(grant, view.version, pending)
			: rowOf(grant, "active");
	});
	const present = new Set(policy.grants.map((grant) => grant.grant_id));
	for (const { entry, version } of pending.latest.values())
		if (entry.kind === "removed" && entry.before && !present.has(entry.grantId))
			rows.push(rowOf(entry.before, "removing", { pendingVersion: version }));
	return rows;
}

interface PendingChanges {
	/** This computer made every version the device has not applied. */
	complete: boolean;
	/** The newest unapplied change per grant. */
	latest: Map<string, { entry: AccessChangeEntry; version: number }>;
}

function pendingChanges(
	view: Pick<PolicyView, "version" | "applied_version">,
	changes: readonly AccessChange[],
): PendingChanges {
	const pending = changes
		.filter(
			(change) =>
				change.version > view.applied_version && change.version <= view.version,
		)
		.sort((a, b) => a.version - b.version);
	const known = new Set(pending.map((change) => change.version));
	let complete = true;
	for (
		let version = view.applied_version + 1;
		version <= view.version;
		version++
	)
		if (!known.has(version)) complete = false;
	const latest: PendingChanges["latest"] = new Map();
	for (const change of pending)
		for (const entry of change.entries)
			latest.set(entry.grantId, { entry, version: change.version });
	return { complete, latest };
}

function waitingRow(
	grant: ManagementGrant,
	savedVersion: number,
	pending: PendingChanges,
): GrantRow {
	const change = pending.latest.get(grant.grant_id);
	if (change && change.entry.kind !== "removed")
		return rowOf(grant, "waiting", {
			pendingVersion: change.version,
			...(change.entry.kind === "added" ? { isNew: true } : {}),
		});
	return pending.complete
		? rowOf(grant, "active")
		: rowOf(grant, "waiting", { pendingVersion: savedVersion });
}

/** People who hold or are about to hold access (expired rows keep their slot until the next save drops them). */
export const usedSlots = (rows: readonly GrantRow[]) =>
	rows.filter((row) => row.status === "active" || row.status === "waiting")
		.length;

/* Saving a new version. */

/** The digest a new version names as its predecessor: SHA-256 of the signed rules, never the hub's word for it. */
export function policyDigest(policyJws: string): string {
	return base64url(sha256(new TextEncoder().encode(policyJws)));
}

export interface AccessRulesDraft {
	deviceId: string;
	view: PolicyView;
	/** The verified current rules; absent only when nothing was saved yet. */
	policy?: ManagementPolicy;
	/** Unix seconds. */
	now: number;
	/** Grants to add or replace (matched by grant id). */
	upserts?: readonly ManagementGrant[];
	removeIds?: readonly string[];
}

function currentGrants(draft: AccessRulesDraft): ManagementGrant[] {
	const { view, policy, deviceId } = draft;
	if (!view.policy_jws) {
		if (view.version !== 0 || view.digest !== null)
			throw new AccessRulesError(
				"incomplete",
				`The access rules of ${deviceId} are incomplete on the hub (version ${view.version} without signed rules).`,
			);
		return [];
	}
	if (!policy)
		throw new AccessRulesError(
			"unverified",
			`The current access rules of ${deviceId} could not be checked with the owner key.`,
		);
	if (policy.device_id !== deviceId)
		throw new AccessRulesError(
			"wrong_device",
			`The hub returned access rules of ${policy.device_id} for ${deviceId}.`,
		);
	if (policy.policy_version !== view.version)
		throw new AccessRulesError(
			"wrong_version",
			`The hub says version ${view.version} but the signed rules are version ${policy.policy_version}.`,
		);
	return policy.grants;
}

/**
 * The next version of a device's rules: grants that ran out are dropped,
 * `upserts` replace their own grant or are added, `removeIds` are taken out,
 * and the rules are signed for another 31 days.
 */
export function nextAccessRules(draft: AccessRulesDraft): ManagementPolicy {
	const { deviceId, view, now } = draft;
	const upserts = draft.upserts ?? [];
	const removeIds = draft.removeIds ?? [];
	const current = currentGrants(draft);
	for (const grantId of removeIds)
		if (!current.some((grant) => grant.grant_id === grantId))
			throw new AccessRulesError(
				"grant_gone",
				`Grant ${grantId} is no longer in the access rules of ${deviceId}.`,
				{ grantId },
			);
	for (const grant of upserts)
		if (grant.capabilities.length === 0)
			throw new AccessRulesError(
				"no_permissions",
				`Grant ${grant.grant_id} has no permissions.`,
				{ grantId: grant.grant_id },
			);
	const replaced = checkRecipientIds(current, upserts);
	const cap = rulesExpiryAfterSave(now);
	const grants = [
		...current.filter(
			(grant) =>
				grant.expires_at > now &&
				!replaced.has(grant.grant_id) &&
				!removeIds.includes(grant.grant_id),
		),
		...upserts.map((grant) => ({
			...grant,
			capabilities: orderCapabilities(grant.capabilities),
			expires_at: Math.min(grant.expires_at, cap),
		})),
	];
	if (grants.length > MAX_GRANTS)
		throw new AccessRulesError(
			"too_many",
			`These rules would hold ${grants.length} people; a device allows ${MAX_GRANTS}.`,
			{ count: grants.length, max: MAX_GRANTS },
		);
	return {
		version: 1,
		device_id: deviceId,
		policy_version: view.version + 1,
		previous_policy_digest: view.policy_jws
			? policyDigest(view.policy_jws)
			: null,
		grants,
		issued_at: now,
		expires_at: cap,
	};
}

/** What a save changes, per grant, for this computer's change log. */
export function changeEntries(
	current: readonly ManagementGrant[],
	upserts: readonly ManagementGrant[],
	removeIds: readonly string[],
): AccessChangeEntry[] {
	const entries: AccessChangeEntry[] = [];
	for (const after of upserts) {
		const before = current.find((grant) => grant.grant_id === after.grant_id);
		const kind = grantChangeKind(before, after);
		if (kind === "same") continue;
		entries.push({
			grantId: after.grant_id,
			userId: after.user_id,
			kind: kind === "new" ? "added" : kind,
			...(before ? { before } : {}),
			after,
		});
	}
	for (const grantId of removeIds) {
		const before = current.find((grant) => grant.grant_id === grantId);
		if (before)
			entries.push({
				grantId,
				userId: before.user_id,
				kind: "removed",
				before,
			});
	}
	return entries;
}

/* The conclusion sentence of the Access screen. */

export interface AccessDeviceSummary {
	deviceId: string;
	name: string;
	rules: AccessRules | null | undefined;
	/** Absent while the people of this device cannot be read (locked, no keys here). */
	rows?: readonly GrantRow[];
}

export interface AccessHeadline {
	/** Distinct people across the devices whose rules can be read. */
	people: number;
	/** The access that ends first within the next 12 hours. */
	soon?: {
		userId: string;
		deviceId: string;
		deviceName: string;
		expiresAt: number;
	};
	/** Devices that have not applied the saved rules yet. */
	waiting: { deviceId: string; name: string }[];
	/** Shared devices whose people cannot be read here. */
	unread: { deviceId: string; name: string }[];
}

export function accessHeadline(
	devices: readonly AccessDeviceSummary[],
	now: number,
): AccessHeadline {
	const people = new Set<string>();
	let soon: AccessHeadline["soon"];
	const waiting: AccessHeadline["waiting"] = [];
	const unread: AccessHeadline["unread"] = [];
	for (const device of devices) {
		if (!device.rules) continue;
		const ref = { deviceId: device.deviceId, name: device.name };
		if (device.rules.waiting) waiting.push(ref);
		if (!device.rows) {
			unread.push(ref);
			continue;
		}
		for (const row of device.rows) {
			if (row.status === "expired") continue;
			people.add(row.grant.user_id);
			const endsAt = row.grant.expires_at;
			if (
				row.status !== "removing" &&
				endsAt - now <= ACCESS_ENDING_SOON_S &&
				(!soon || endsAt < soon.expiresAt)
			)
				soon = {
					userId: row.grant.user_id,
					deviceId: device.deviceId,
					deviceName: device.name,
					expiresAt: endsAt,
				};
		}
	}
	return { people: people.size, ...(soon ? { soon } : {}), waiting, unread };
}

/* What this computer remembers (BG23 and BG13 interims): imported request files and the changes it saved. */

export interface ImportedAccessRequest {
	id: string;
	file: string;
	userId: string;
	controllerKey: Ed25519PublicKey;
	grantId?: string;
	/** The device the request was made for, when the file name or the importer said so. */
	deviceId?: string;
	/** Unix seconds. */
	importedAt: number;
}

export interface AccessLocalState {
	requests: readonly ImportedAccessRequest[];
	changes: readonly AccessChange[];
}

export interface AccessLocalStore {
	get(): AccessLocalState;
	subscribe(listener: () => void): () => void;
	addRequests(requests: readonly ImportedAccessRequest[]): void;
	removeRequests(ids: readonly string[]): void;
	recordChange(change: AccessChange): void;
}

export interface AccessStorage {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
}

const STORAGE_PREFIX = "flow-like/devices/access/";
const MAX_REQUESTS = 64;
const MAX_CHANGES = 48;

const scopeSchema: z.ZodType<InventoryScope, z.ZodTypeDef, unknown> = z.union([
	z.object({ kind: z.literal("device") }),
	z.object({ kind: z.literal("project"), project_id: z.string() }),
	z.object({
		kind: z.literal("placement"),
		project_id: z.string(),
		placement_id: z.string(),
	}),
]);

const grantSchema = z.object({
	grant_id: z.string(),
	user_id: z.string(),
	controller_key: keySchema,
	scope: scopeSchema,
	capabilities: z.array(z.string()),
	expires_at: z.number(),
	group_id: z.string().nullable(),
	group_version: z.number().nullable(),
});

const stateSchema = z.object({
	requests: z
		.array(
			z.object({
				id: z.string(),
				file: z.string(),
				userId: z.string(),
				controllerKey: keySchema,
				grantId: z.string().optional(),
				deviceId: z.string().optional(),
				importedAt: z.number(),
			}),
		)
		.catch([]),
	changes: z
		.array(
			z.object({
				deviceId: z.string(),
				version: z.number(),
				savedAt: z.number(),
				entries: z.array(
					z.object({
						grantId: z.string(),
						userId: z.string(),
						kind: z.enum(["added", "changed", "renewed", "removed"]),
						before: grantSchema.optional(),
						after: grantSchema.optional(),
					}),
				),
			}),
		)
		.catch([]),
});

const EMPTY: AccessLocalState = { requests: [], changes: [] };

function defaultStorage(): AccessStorage | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

function readState(storage: AccessStorage | undefined, key: string) {
	try {
		const raw = storage?.getItem(key);
		if (!raw) return EMPTY;
		const parsed = stateSchema.safeParse(JSON.parse(raw));
		return parsed.success
			? (parsed.data as unknown as AccessLocalState)
			: EMPTY;
	} catch {
		return EMPTY;
	}
}

const sameRequest = (a: ImportedAccessRequest, b: ImportedAccessRequest) =>
	a.userId === b.userId &&
	a.controllerKey.x === b.controllerKey.x &&
	a.deviceId === b.deviceId;

/** Per account scope; survives reloads where the browser allows it and works in memory where it does not. */
export function createAccessLocalStore(
	scopeKey: string,
	storage: AccessStorage | undefined = defaultStorage(),
): AccessLocalStore {
	const key = `${STORAGE_PREFIX}${scopeKey}`;
	let state = readState(storage, key);
	const listeners = new Set<() => void>();
	const commit = (next: AccessLocalState) => {
		state = next;
		try {
			storage?.setItem(key, JSON.stringify(next));
		} catch {
			// Kept in memory when the browser refuses to store it.
		}
		for (const listener of [...listeners]) listener();
	};
	return {
		get: () => state,
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		addRequests(requests) {
			const kept = state.requests.filter(
				(existing) => !requests.some((added) => sameRequest(existing, added)),
			);
			commit({
				...state,
				requests: [...kept, ...requests].slice(-MAX_REQUESTS),
			});
		},
		removeRequests(ids) {
			if (!state.requests.some((request) => ids.includes(request.id))) return;
			commit({
				...state,
				requests: state.requests.filter((request) => !ids.includes(request.id)),
			});
		},
		recordChange(change) {
			commit({
				...state,
				changes: [
					...state.changes.filter(
						(existing) =>
							existing.deviceId !== change.deviceId ||
							existing.version !== change.version,
					),
					change,
				].slice(-MAX_CHANGES),
			});
		},
	};
}
