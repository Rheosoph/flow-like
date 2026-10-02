"use client";

import { useQueries } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	type UserLookupResult,
	userLookupQueryOptions,
} from "../../../../hooks/use-user-lookup";
import {
	HubError,
	hubReadWith,
	readPolicyView,
} from "../../../../lib/device-management/hub/endpoints";
import {
	deviceKeys,
	queries,
} from "../../../../lib/device-management/hub/queries";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import {
	presence,
	relationshipOf,
} from "../../../../lib/device-management/model/presence";
import type {
	AttentionInput,
	DeviceRow,
	HostIsolationFacts,
	HostIsolationMode,
	Presence,
} from "../../../../lib/device-management/model/types";
import {
	type AccessChange,
	type AccessLocalStore,
	type AccessRules,
	AccessRulesError,
	type AccessRulesErrorCode,
	type GrantRow,
	MAX_GRANTS,
	accessRulesOf,
	changeEntries,
	createAccessLocalStore,
	grantRows,
	nextAccessRules,
	permissionBlock,
} from "../../../../lib/device-management/sharing";
import type {
	ManagementGrant,
	ManagementPolicy,
	PolicyView,
} from "../../../../lib/device-management/types";
import { OwnerPasswordRequiredError } from "../../../../lib/device-management/workspace/keys";
import type {
	DeviceWorkspace,
	KeySessionSnapshot,
} from "../../../../lib/device-management/workspace/types";
import { userDisplayName } from "../../../../lib/user-display";
import { useBackend } from "../../../../state/backend-state";
import type { DevicesT } from "../primitives/area-context";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { hubErrorCopy } from "../workspace/area-context";
import {
	useDeviceWorkspace,
	useManagerValue,
} from "../workspace/device-workspace-provider";
import {
	type AttentionState,
	useAttentionState,
} from "../workspace/use-attention";
import {
	type DeviceActionOutcome,
	type DeviceActionRequest,
	useDeviceAction,
} from "../workspace/use-device-action";
import { type PolicyRead, usePolicy } from "../workspace/use-hub";

/* What this computer remembers about access (imported requests, saved changes). */

const stores = new WeakMap<DeviceWorkspace, AccessLocalStore>();

export function accessStoreOf(workspace: DeviceWorkspace): AccessLocalStore {
	let store = stores.get(workspace);
	if (!store) {
		store = createAccessLocalStore(workspace.scopeKey);
		stores.set(workspace, store);
	}
	return store;
}

export function useAccessLocal() {
	const workspace = useDeviceWorkspace();
	const store = accessStoreOf(workspace);
	const state = useManagerValue(store.subscribe, store.get);
	return useMemo(() => ({ store, ...state }), [store, state]);
}

/* The devices of the list, by what the viewer is to them. */

export interface FleetAccess {
	me: string;
	/** `GET /devices` answered: empty lists then mean "none". */
	loaded: boolean;
	owned: DeviceRow[];
	shared: DeviceRow[];
	/** Devices the viewer only approved cloud access or pays for. */
	cloudOnly: DeviceRow[];
	revokedOwned: DeviceRow[];
}

export function useFleetAccess(): FleetAccess {
	const { input } = useAttentionState();
	const { devices, me, devicesLoaded } = input;
	return useMemo(() => {
		const fleet: FleetAccess = {
			me,
			loaded: devicesLoaded === true,
			owned: [],
			shared: [],
			cloudOnly: [],
			revokedOwned: [],
		};
		for (const row of devices) {
			const relationship = relationshipOf(row, me);
			if (relationship === "owner")
				(row.status === "revoked" ? fleet.revokedOwned : fleet.owned).push(row);
			else if (relationship === "cloud_approval") fleet.cloudOnly.push(row);
			else if (row.status === "active") fleet.shared.push(row);
		}
		const byName = (a: DeviceRow, b: DeviceRow) =>
			deviceName(a).localeCompare(deviceName(b));
		fleet.owned.sort(byName);
		fleet.shared.sort(byName);
		return fleet;
	}, [devices, me, devicesLoaded]);
}

/* One owned device's access as the screens show it. */

export interface DeviceAccess {
	row: DeviceRow;
	deviceId: string;
	name: string;
	presence: Presence;
	keys: KeySessionSnapshot;
	view?: PolicyView;
	policy?: ManagementPolicy;
	/** `null`: nothing shared yet; `undefined`: not read yet. */
	rules: AccessRules | null | undefined;
	/** Absent while the people can't be read here (locked, no keys). */
	rows?: GrantRow[];
	/** The hub did not answer for this device's rules. */
	readFailed?: boolean;
	/** `null` until the device was read live. */
	isolation: HostIsolationMode | null;
	isolationFacts?: HostIsolationFacts;
	/** Whether the agent can share Manage certificates; `undefined` until read live. */
	certificateSupport?: boolean;
	platform?: string;
}

function deviceAccessOf(
	row: DeviceRow,
	input: AttentionInput,
	workspace: DeviceWorkspace,
	changes: readonly AccessChange[],
): DeviceAccess {
	const deviceId = row.device_id;
	const stored = input.policies[deviceId];
	const policy = stored?.policy;
	const inspection = input.live[deviceId]?.inspection?.value;
	const rules = stored
		? accessRulesOf(stored, policy, row.access_rules_expire_at)
		: row.access_rules_expire_at === null
			? null
			: undefined;
	return {
		row,
		deviceId,
		name: deviceName(row),
		presence: presence(row, input.now),
		keys:
			input.keys.find((session) => session.deviceId === deviceId) ??
			workspace.keys.snapshot(deviceId),
		...(stored ? { view: stored } : {}),
		...(policy ? { policy } : {}),
		rules,
		...(policy && stored
			? {
					rows: grantRows(
						policy,
						stored,
						input.now,
						changes.filter((change) => change.deviceId === deviceId),
					),
				}
			: {}),
		isolation: inspection?.hostIsolation ?? null,
		...(inspection?.isolation ? { isolationFacts: inspection.isolation } : {}),
		...(inspection
			? { certificateSupport: inspection.certificate_management === 1 }
			: {}),
		...(inspection?.isolation?.platform
			? { platform: inspection.isolation.platform }
			: {}),
	};
}

/**
 * Access of every owned device. Rules are read once for the devices the hub
 * says are shared (older hubs: for all of them); the sections on screen keep
 * them fresh with `useRulesRead`.
 */
export function useOwnedAccess(owned: readonly DeviceRow[]): DeviceAccess[] {
	const { workspace, input } = useAttentionState();
	const { changes } = useAccessLocal();
	const ids = useMemo(
		() =>
			owned
				.filter((row) => row.access_rules_expire_at !== null)
				.map((row) => row.device_id),
		[owned],
	);
	const reads = useQueries({
		queries: ids.map((deviceId) => ({
			...queries.policy(workspace.hub, deviceId),
			refetchInterval: false as const,
		})),
	});
	const failed = ids
		.filter((_, index) => reads[index]?.isError && !reads[index]?.data)
		.join("|");
	return useMemo(() => {
		const unreadable = new Set(failed ? failed.split("|") : []);
		return owned.map((row) => {
			const access = deviceAccessOf(row, input, workspace, changes);
			return unreadable.has(access.deviceId)
				? { ...access, readFailed: true }
				: access;
		});
	}, [owned, input, workspace, changes, failed]);
}

/** Reads the rules of these devices again (after a failed read). */
export function useRetryRules(): (deviceIds: readonly string[]) => void {
	const workspace = useDeviceWorkspace();
	const { queryClient } = workspace.deps;
	return useCallback(
		(deviceIds) => {
			for (const deviceId of deviceIds)
				void queryClient.invalidateQueries({
					queryKey: deviceKeys.policy(workspace.scopeKey, deviceId),
				});
		},
		[queryClient, workspace.scopeKey],
	);
}

export function useDeviceAccess(deviceId: string): DeviceAccess | undefined {
	const { workspace, input } = useAttentionState();
	const { changes } = useAccessLocal();
	return useMemo(() => {
		const row = input.devices.find((entry) => entry.device_id === deviceId);
		return row ? deviceAccessOf(row, input, workspace, changes) : undefined;
	}, [deviceId, input, workspace, changes]);
}

/** After the crypto module is there, the verified rules arrive with the next render; past this they don't verify. */
const VERIFY_GRACE_MS = 500;

/**
 * True when the owner keys are open here and the rules the hub returned still
 * don't check out with the owner key: their people can't be shown, and that is
 * an error, never "nobody" and never an endless wait.
 */
export function useRulesUnverified(device: DeviceAccess | undefined): boolean {
	const workspace = useDeviceWorkspace();
	const waiting =
		!!device &&
		device.keys.state === "unlocked" &&
		!!device.view?.policy_jws &&
		!device.policy;
	const [given, setGiven] = useState(false);
	useEffect(() => {
		if (!waiting) {
			setGiven(false);
			return;
		}
		let active = true;
		let timer: ReturnType<typeof setTimeout> | undefined;
		void workspace.deps.crypto().then(
			() => {
				if (active) timer = setTimeout(() => setGiven(true), VERIFY_GRACE_MS);
			},
			() => undefined,
		);
		return () => {
			active = false;
			if (timer) clearTimeout(timer);
		};
	}, [waiting, workspace]);
	return waiting && given;
}

/** The rules of one device on screen: every 30 s, every 10 s while the device has not applied them. */
export function useRulesRead(deviceId: string | undefined): PolicyRead {
	const [awaitingApply, setAwaitingApply] = useState(false);
	const read = usePolicy(deviceId, { awaitingApply });
	const waiting = !!read.data && read.data.applied_version < read.data.version;
	useEffect(() => setAwaitingApply(waiting), [waiting]);
	return read;
}

/* People. */

export interface PersonName {
	/** "Mira Novak", or a neutral phrase when the account can't be looked up. */
	name: string;
	/** "Mira". */
	first: string;
	known: boolean;
}

export type PersonNames = (userId: string) => PersonName;

function personOf(t: DevicesT, user: UserLookupResult | undefined): PersonName {
	if (!user) {
		const name = t("devices:access.person.unknown", "Unknown account");
		return {
			name,
			first: t("devices:access.person.them", "them"),
			known: false,
		};
	}
	const name = userDisplayName(user, user.id);
	return { name, first: name.split(/\s+/u)[0] ?? name, known: true };
}

/** Display names for sentences that mention several people. */
export function usePersonNames(
	t: DevicesT,
	userIds: readonly string[],
): PersonNames {
	const backend = useBackend();
	const key = JSON.stringify([...new Set(userIds)].sort());
	const ids = useMemo(() => JSON.parse(key) as string[], [key]);
	const results = useQueries({
		queries: ids.map((id) => userLookupQueryOptions(backend.userState, id)),
	});
	const users = results.map((result) => result.data);
	const signature = users
		.map((user) => (user ? userDisplayName(user, user.id) : ""))
		.join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: `signature` stands for the looked-up users
	return useMemo(() => {
		const byId = new Map(ids.map((id, index) => [id, users[index]]));
		return (userId: string) => personOf(t, byId.get(userId));
	}, [ids, signature, t]);
}

/* Saving a new version of a device's rules: the one signing path. */

export interface SaveAccessRequest {
	device: { deviceId: string; name: string };
	/** Verb + object: confirm button, tray item and result. */
	label: string;
	upserts?: readonly ManagementGrant[];
	removeIds?: readonly string[];
	/** R8 rows; omit when the screen showed them itself (wizard review, renew sheet). */
	consequence?: ConsequenceRows;
	confirm?: DeviceActionRequest<SavedAccessRules>["confirm"];
	/**
	 * Typed in place when the owner key is not held by the key session; a
	 * function is read at signing time (a field inside the confirm sheet).
	 */
	password?: string | (() => string | undefined);
}

export interface SavedAccessRules {
	view: PolicyView;
	version: number;
	policy: ManagementPolicy;
	/** Unix seconds. */
	savedAt: number;
}

export type SaveAccessOutcome =
	| DeviceActionOutcome<SavedAccessRules>
	/** The owner key is not held: ask for the device password in place and save again. */
	| { status: "password_required" };

export const accessResultKey = (deviceId: string) => `access:${deviceId}`;

/** Manage certificates can be newly given only while the agent says it supports sharing it; checked right before signing. */
function checkCertificateSupport(
	state: AttentionState,
	deviceId: string,
	current: readonly ManagementGrant[],
	upserts: readonly ManagementGrant[],
) {
	const inspection = state.input.live[deviceId]?.inspection?.value;
	const support = inspection
		? inspection.certificate_management === 1
		: undefined;
	for (const grant of upserts) {
		if (!grant.capabilities.includes("manage_certificates")) continue;
		const held = current
			.find((existing) => existing.grant_id === grant.grant_id)
			?.capabilities.includes("manage_certificates");
		if (permissionBlock("manage_certificates", "device", support, held))
			throw new AccessRulesError(
				"certificates_unsupported",
				"The device's agent does not support sharing certificate management.",
			);
	}
}

async function sign(
	workspace: DeviceWorkspace,
	deviceId: string,
	policy: ManagementPolicy,
	password: string | undefined,
): Promise<string> {
	const signer = workspace.keys.signer(deviceId);
	if (!signer)
		throw new AccessRulesError(
			"locked",
			"The owner keys of this device are not unlocked.",
		);
	try {
		return await signer.signPolicy(policy, password);
	} catch (error) {
		if (error instanceof OwnerPasswordRequiredError) throw error;
		throw new AccessRulesError(
			password === undefined ? "signing_failed" : "wrong_password",
			error instanceof Error ? error.message : String(error),
		);
	}
}

export function useSaveAccessRules(): (
	request: SaveAccessRequest,
) => Promise<SaveAccessOutcome> {
	const actions = useDeviceAction();
	const state = useAttentionState();
	const latest = useRef(state);
	latest.current = state;
	const workspace = state.workspace;
	return useCallback(
		async (request) => {
			const { deviceId, name } = request.device;
			const upserts = request.upserts ?? [];
			const removeIds = request.removeIds ?? [];
			if (
				request.password === undefined &&
				workspace.keys.signer(deviceId) &&
				!workspace.keys.snapshot(deviceId).canSign
			)
				return { status: "password_required" };
			const outcome = await actions.run<SavedAccessRules>({
				action: "share_access",
				deviceId,
				label: request.label,
				resultKey: accessResultKey(deviceId),
				...(request.consequence ? { consequence: request.consequence } : {}),
				...(request.confirm ? { confirm: request.confirm } : {}),
				call: async ({ workspace: current }) => {
					const { api, profile } = current.deps;
					const view = await readPolicyView(api, profile, deviceId);
					const policy = view.policy_jws
						? latest.current.verifyPolicy(deviceId, view)
						: undefined;
					const savedAt = Math.floor(current.clock.now() / 1000);
					const next = nextAccessRules({
						deviceId,
						view,
						...(policy ? { policy } : {}),
						now: savedAt,
						upserts,
						removeIds,
					});
					const before = policy?.grants ?? [];
					checkCertificateSupport(latest.current, deviceId, before, upserts);
					const typed =
						typeof request.password === "function"
							? request.password() || undefined
							: request.password;
					const policyJws = await sign(current, deviceId, next, typed);
					const saved = await hubReadWith(
						"device",
						"PUT devices/{id}/management/policy",
						() =>
							api.put<PolicyView>(
								profile,
								`devices/${encodeURIComponent(deviceId)}/management/policy`,
								{ policy_jws: policyJws },
							),
					);
					accessStoreOf(current).recordChange({
						deviceId,
						version: next.policy_version,
						savedAt,
						entries: changeEntries(before, upserts, removeIds),
					});
					return {
						view: saved,
						version: next.policy_version,
						policy: next,
						savedAt,
					};
				},
				activity: {
					kind: "access_rules",
					deviceName: name,
					href: { screen: "access", tab: "people" },
					resume: (result) => ({ type: "policy", version: result.version }),
				},
				invalidate: [deviceKeys.policy(workspace.scopeKey, deviceId)],
			});
			if (
				outcome.status === "failed" &&
				outcome.error instanceof OwnerPasswordRequiredError
			)
				return { status: "password_required" };
			return outcome;
		},
		[actions, workspace],
	);
}

/** Why a save did not happen, in the viewer's words (R3: never the raw error). */
export function saveErrorText(
	t: DevicesT,
	outcome: SaveAccessOutcome,
	device: string,
): string | undefined {
	if (outcome.status === "done" || outcome.status === "cancelled")
		return undefined;
	const copy = STATUS_COPY[outcome.status];
	if (copy) return copy(t, device, MAX_GRANTS);
	return failureText(
		t,
		outcome.status === "failed" ? outcome.error : undefined,
		device,
	);
}

type ErrorCopy = (t: DevicesT, device: string, max: number) => string;

/** Outcomes that are not a failed save: nothing was sent, or the answer is missing. */
const STATUS_COPY: Partial<Record<SaveAccessOutcome["status"], ErrorCopy>> = {
	busy: (t, device) =>
		t(
			"devices:access.error.busy",
			"Another change to {{device}}'s access rules is still being saved.",
			{ device },
		),
	password_required: (t, device) =>
		t(
			"devices:access.error.passwordRequired",
			"Type the device password of {{device}} to sign this change.",
			{ device },
		),
	gated: (t, device) =>
		t(
			"devices:access.error.gated",
			"This change can't be saved for {{device}} right now.",
			{ device },
		),
	unknown: (t, device) =>
		t(
			"devices:access.error.unknown",
			"The hub didn't answer in time. The change may or may not be saved; check the access rules of {{device}} before trying again.",
			{ device },
		),
};

const unverifiedCopy: ErrorCopy = (t, device) =>
	t(
		"devices:access.error.unverified",
		"The hub returned access rules for {{device}} that don't check out with your owner key, so nothing was signed. Try again; if it stays, don't change access from this hub.",
		{ device },
	);

const RULES_ERROR_COPY: Record<AccessRulesErrorCode, ErrorCopy> = {
	locked: (t, device) =>
		t(
			"devices:access.error.locked",
			"{{device}} is locked. Unlock it to sign the change.",
			{ device },
		),
	wrong_password: (t, device) =>
		t(
			"devices:access.error.wrongPassword",
			"That password didn't open the owner key of {{device}}. Nothing was saved.",
			{ device },
		),
	signing_failed: (t, device) =>
		t(
			"devices:access.error.signingFailed",
			"The change couldn't be signed with the owner key of {{device}}. Lock and unlock the device, then try again.",
			{ device },
		),
	unverified: unverifiedCopy,
	incomplete: unverifiedCopy,
	wrong_device: unverifiedCopy,
	wrong_version: unverifiedCopy,
	grant_gone: (t, device) =>
		t(
			"devices:access.error.grantGone",
			"That access isn't in the rules of {{device}} any more. The list was refreshed.",
			{ device },
		),
	grant_conflict: (t, device) =>
		t(
			"devices:access.error.grantConflict",
			"A request ID in this change already belongs to someone else's access on {{device}}. Ask for a new access request.",
			{ device },
		),
	duplicate_grant: (t, device) =>
		t(
			"devices:access.error.duplicateGrant",
			"The same access request is listed twice for {{device}}. Keep one of them.",
			{ device },
		),
	too_many: (t, device, max) =>
		t(
			"devices:access.error.tooMany",
			"{{device}} would have more than {{max, number}} people. Remove access you no longer need first.",
			{ device, max },
		),
	no_permissions: (t) =>
		t("devices:access.error.noPermissions", "Choose at least one permission."),
	certificates_unsupported: (t, device) =>
		t(
			"devices:access.error.certificatesUnsupported",
			"{{device}}'s agent can't share Manage certificates. Update the agent and connect once, or remove that permission.",
			{ device },
		),
};

function failureText(t: DevicesT, error: unknown, device: string): string {
	if (error instanceof AccessRulesError)
		return RULES_ERROR_COPY[error.code](
			t,
			device,
			Number(error.params.max ?? MAX_GRANTS),
		);
	if (error instanceof HubError)
		return t(
			"devices:access.error.hub",
			"Couldn't save: {{why}} Nothing changed, and {{device}} keeps its current access rules.",
			{ why: hubErrorCopy(t, error.code), device },
		);
	return t(
		"devices:access.error.other",
		"The change wasn't saved. Nothing changed, and {{device}} keeps its current access rules.",
		{ device },
	);
}

/* File reads that must not outlive the account they were started under. */

/**
 * Runs an async read and reports its result only while this component is
 * mounted, the account scope is the one it started under and no newer read
 * began: a late file import never lands in another account.
 */
export function useGuardedRead() {
	const workspace = useDeviceWorkspace();
	const generation = useRef(0);
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
			generation.current++;
		};
	}, []);
	const scopeKey = useRef(workspace.scopeKey);
	scopeKey.current = workspace.scopeKey;
	return useCallback(
		async <T>(read: () => Promise<T>, apply: (value: T) => void) => {
			const run = ++generation.current;
			const started = scopeKey.current;
			const value = await read();
			if (
				mounted.current &&
				run === generation.current &&
				started === scopeKey.current
			)
				apply(value);
		},
		[],
	);
}
