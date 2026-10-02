"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { agentSupports } from "../../../../lib/device-management/agent-reads";
import { readPolicyView } from "../../../../lib/device-management/hub/endpoints";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import type {
	ArchiveRecordingStatus,
	LiveDeviceInput,
} from "../../../../lib/device-management/model/types";
import {
	type ManagementCall,
	digestText,
	readArchiveRoster,
} from "../../../../lib/device-management/telemetry";
import {
	type ArchiveRecipient,
	type ArchiveRoster,
	type BrowserController,
	type DeviceCrypto,
	type DeviceReceipt,
	type EncryptedArchive,
	type ManagementPolicy,
	type OnboardingManifest,
	type PolicyView,
	managementRejection,
} from "../../../../lib/device-management/types";
import { LiveCallError } from "../../../../lib/device-management/workspace/errors";
import { OwnerPasswordRequiredError } from "../../../../lib/device-management/workspace/keys";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import {
	type DeviceActionOutcome,
	deviceCall,
	useDeviceAction,
	useDeviceWorkspace,
} from "../workspace";
import { livePhase } from "./live-state";
import type { HistoryPauseReason } from "./timeline-model";
import type { ObserveTarget } from "./use-observe-target";

export type HistoryKind = "logs" | "metrics";
export const HISTORY_KINDS: readonly HistoryKind[] = ["logs", "metrics"];
/** The readers list of the whole device. */
export const DEVICE_SCOPE = "device";
const EVERY_MS = 60_000;
const MAX_ROSTER_CHARS = 13_000;
export const MAX_READERS = 32;
const DAY_S = 86_400;
/** The protocol's ceiling for a readers list. */
export const MAX_RECORDING_S = 31 * DAY_S;

/** One retained stream: a scope (the device or a service) and what is kept (logs or metrics). */
export interface HistoryStream {
	scope: string;
	kind: HistoryKind;
	/** The verified readers list; `null` while history isn't set up for this stream. */
	roster: ArchiveRoster | null;
	rosterText: string | null;
	/** What the device says about recording (BG30); absent on older agents. */
	status: ArchiveRecordingStatus | undefined;
	/** The device refused the read: no Read logs / Read metrics on this scope. */
	denied: boolean;
}

export interface DeviceTrust {
	crypto: DeviceCrypto;
	controller: BrowserController;
	manifest: OnboardingManifest;
	receipt: DeviceReceipt;
}

/** The verified setup facts of an unlocked device; rejects while its keys are locked. */
export async function deviceTrust(
	workspace: DeviceWorkspace,
	deviceId: string,
): Promise<DeviceTrust> {
	const controller = workspace.keys.controller(deviceId);
	const vault = workspace.keys.vault(deviceId);
	const receipt = workspace.keys.receipt(deviceId);
	if (!controller || !vault || !receipt)
		throw new LiveCallError("keys_locked", "Unlock this device first.");
	const crypto = await workspace.deps.crypto();
	const manifest = crypto.verifyDeviceReceipt(
		receipt,
		vault.manifestJws,
		vault.ownerControllerKey ?? controller.publicBundle().controller_key,
	);
	return { crypto, controller, manifest, receipt };
}

/** This computer's reader identity: what an owner adds to a readers list. */
export function ownRecipient(
	controller: BrowserController,
	userId: string,
): ArchiveRecipient {
	const bundle = controller.publicBundle();
	return {
		recipient_id: bundle.controller_key.x,
		user_id: userId,
		public_key: bundle.archive_key,
	};
}

export type Recording =
	| { state: "none" }
	| { state: "recording"; until: number }
	| {
			state: "paused";
			reason: HistoryPauseReason | null;
			since?: number;
			until: number;
	  };

/**
 * Whether a stream records. A newer agent says so (BG30); for an older one the
 * state follows from the list's expiry and the access rules it was signed for.
 */
export function recordingOf(
	stream: Pick<HistoryStream, "roster" | "status">,
	now: number,
	appliedDigest: string | null | undefined,
): Recording {
	const { roster, status } = stream;
	if (!roster) return { state: "none" };
	const until = roster.expires_at;
	if (status)
		return status.state === "paused"
			? { state: "paused", reason: status.reason, since: status.since, until }
			: { state: "recording", until };
	if (until <= now)
		return { state: "paused", reason: "roster_expired", since: until, until };
	if (
		appliedDigest !== undefined &&
		roster.management_policy_digest !== appliedDigest
	)
		return { state: "paused", reason: "rules_changed", until };
	return { state: "recording", until };
}

async function readStream(
	trust: DeviceTrust,
	call: ManagementCall,
	scope: string,
	kind: HistoryKind,
): Promise<HistoryStream> {
	let refused: string | undefined;
	const tapped: ManagementCall = async (command, operationId) => {
		const response = await call(command, operationId);
		if (response.state === "rejected")
			refused = managementRejection(response)?.code ?? "";
		return response;
	};
	try {
		const read = await readArchiveRoster(tapped, scope, kind);
		const roster = read.text
			? trust.crypto.verifyArchiveRosterHead(
					read.text,
					trust.manifest.owner_invitation_key,
				)
			: null;
		if (
			roster &&
			(roster.device_id !== trust.manifest.device_id ||
				roster.scope !== scope ||
				roster.kind !== kind)
		)
			throw new Error(
				`The device returned the readers list of another stream for ${scope} ${kind}.`,
			);
		return {
			scope,
			kind,
			roster,
			rosterText: read.text,
			status: read.status,
			denied: false,
		};
	} catch (error) {
		if (refused !== "unauthorized") throw error;
		return {
			scope,
			kind,
			roster: null,
			rosterText: null,
			status: undefined,
			denied: true,
		};
	}
}

type HistoryFact = NonNullable<LiveDeviceInput["history"]>[number];

/** Tells the attention engine what was read, keeping the entries of scopes this read didn't cover. */
function recordHistory(
	workspace: DeviceWorkspace,
	deviceId: string,
	streams: readonly HistoryStream[],
) {
	const read = new Set(streams.map((row) => `${row.scope}|${row.kind}`));
	const kept = (workspace.facts.get(deviceId)?.history ?? []).filter(
		(entry) => !read.has(`${entry.scope}|${entry.kind}`),
	);
	const fresh = streams.flatMap((row): HistoryFact[] =>
		row.roster
			? [
					{
						scope: row.scope,
						kind: row.kind,
						expiresAt: row.roster.expires_at,
						policyVersion: row.roster.policy_version,
						...(row.status ? { status: row.status } : {}),
					},
				]
			: [],
	);
	workspace.facts.record(deviceId, { history: [...kept, ...fresh] });
}

export interface HistoryRead {
	/** Undefined until the first read finished. */
	streams: HistoryStream[] | undefined;
	/** Unix seconds of the last read. */
	readAt: number | undefined;
	reading: boolean;
	failed: boolean;
	refresh(): Promise<void>;
}

const historyKey = (scopeKey: string, deviceId: string) =>
	["devices", scopeKey, "observe-history", deviceId] as const;

/**
 * The readers lists of the given scopes, read over the live session. The
 * command is as old as retained history, so older agents answer it too; only
 * the recording state (BG30) is new.
 */
export function useHistoryStreams(
	target: ObserveTarget,
	scopes: readonly string[],
): HistoryRead {
	const workspace = useDeviceWorkspace();
	const { deviceId } = target;
	const enabled = livePhase(target) === "open" && scopes.length > 0;
	const scopesKey = scopes.join("|");
	const query = useQuery({
		queryKey: [...historyKey(workspace.scopeKey, deviceId), scopesKey],
		enabled,
		retry: false,
		gcTime: 0,
		staleTime: EVERY_MS / 2,
		refetchInterval: EVERY_MS,
		queryFn: async () => {
			const trust = await deviceTrust(workspace, deviceId);
			const call = deviceCall(workspace, deviceId, "poll");
			const streams: HistoryStream[] = [];
			for (const scope of scopesKey.split("|"))
				for (const kind of HISTORY_KINDS)
					streams.push(await readStream(trust, call, scope, kind));
			recordHistory(workspace, deviceId, streams);
			return { streams, readAt: Math.floor(workspace.clock.now() / 1000) };
		},
	});
	const { data, isFetching, isError, refetch } = query;
	const refresh = useCallback(async () => {
		await refetch();
	}, [refetch]);
	return useMemo(
		() => ({
			streams: enabled ? data?.streams : undefined,
			readAt: enabled ? data?.readAt : undefined,
			reading: enabled && isFetching,
			failed: enabled && isError && !data,
			refresh,
		}),
		[enabled, data, isFetching, isError, refresh],
	);
}

/* The hub's list of sealed chunks. */

export interface ArchiveRow {
	archive_id: string;
	sequence: number;
	created_at: number;
	expires_at: number;
}

interface ArchivePage {
	archives: ArchiveRow[];
	next: number;
}

const isCount = (value: unknown): value is number =>
	Number.isSafeInteger(value) && Number(value) >= 0;

function parseArchivePage(value: unknown, after: number): ArchivePage {
	const page = value as { archives?: unknown; next?: unknown } | null;
	const rows = page?.archives;
	if (
		!Array.isArray(rows) ||
		rows.length > 100 ||
		!isCount(page?.next) ||
		page.next < after
	)
		throw new Error("The hub returned an invalid history listing.");
	return {
		archives: rows.flatMap((row: Record<string, unknown> | null) =>
			typeof row?.archive_id === "string" &&
			isCount(row.sequence) &&
			isCount(row.created_at) &&
			isCount(row.expires_at)
				? [
						{
							archive_id: row.archive_id,
							sequence: row.sequence,
							created_at: row.created_at,
							expires_at: row.expires_at,
						},
					]
				: [],
		),
		next: page.next,
	};
}

const archivesPath = (deviceId: string) =>
	`devices/${encodeURIComponent(deviceId)}/archives`;

export interface ArchiveList {
	/** Newest first; undefined until the first answer. */
	rows: ArchiveRow[] | undefined;
	/** Epoch milliseconds of the last answer. */
	checkedAt: number;
	loading: boolean;
	error: unknown;
	/** More chunks exist than the newest ones listed. */
	more: boolean;
	refresh(): Promise<void>;
}

const LIST_PAGES = 5;

/** The newest sealed chunks of one stream as the hub lists them (it never sees their content). */
export function useArchiveList(
	deviceId: string,
	scope: string,
	kind: HistoryKind,
	enabled: boolean,
): ArchiveList {
	const { hub } = useDeviceWorkspace();
	const query = useQuery({
		queryKey: deviceKeys.archives(hub.scopeKey, deviceId, scope, kind),
		enabled,
		staleTime: 15_000,
		refetchInterval: 30_000,
		queryFn: async () => {
			let after = 0;
			let rows: ArchiveRow[] = [];
			let more = false;
			for (let page = 0; page < LIST_PAGES; page++) {
				const search = new URLSearchParams({
					scope,
					kind,
					after: String(after),
				});
				const result = parseArchivePage(
					await hub.api.get(
						hub.profile,
						`${archivesPath(deviceId)}?${search.toString()}`,
					),
					after,
				);
				rows = [...rows, ...result.archives];
				more = result.archives.length >= 100;
				if (!more) break;
				after = result.next;
			}
			return {
				rows: rows.sort((a, b) => b.sequence - a.sequence),
				more,
			};
		},
	});
	const { data, dataUpdatedAt, isLoading, error, refetch } = query;
	const refresh = useCallback(async () => {
		await refetch();
	}, [refetch]);
	return useMemo(
		() => ({
			rows: data?.rows,
			checkedAt: dataUpdatedAt,
			loading: isLoading,
			error,
			more: data?.more ?? false,
			refresh,
		}),
		[data, dataUpdatedAt, isLoading, error, refresh],
	);
}

export interface ArchiveContent {
	records: unknown[];
	/** Records the device dropped before this chunk. */
	dropped: number;
}

/** Downloads one sealed chunk from the hub and opens it with this computer's reader key. */
export async function readArchive(
	workspace: DeviceWorkspace,
	input: {
		deviceId: string;
		scope: string;
		kind: HistoryKind;
		archiveId: string;
	},
): Promise<ArchiveContent> {
	const trust = await deviceTrust(workspace, input.deviceId);
	const bundle = (await workspace.hub.api.get(
		workspace.hub.profile,
		`${archivesPath(input.deviceId)}/${encodeURIComponent(input.archiveId)}`,
	)) as EncryptedArchive;
	const bytes = trust.controller.openArchive(
		{
			device_id: trust.manifest.device_id,
			scope: input.scope,
			kind: input.kind,
			owner_invitation_key: trust.manifest.owner_invitation_key,
			device_signing_key: trust.receipt.identity.telemetry_key,
		},
		bundle,
		trust.controller.publicBundle().controller_key.x,
	);
	try {
		const parsed = JSON.parse(
			new TextDecoder("utf-8", { fatal: true }).decode(bytes),
		) as { records?: unknown; gap?: { dropped?: unknown } } | null;
		return {
			records: Array.isArray(parsed?.records) ? parsed.records : [],
			dropped: isCount(parsed?.gap?.dropped) ? parsed.gap.dropped : 0,
		};
	} finally {
		bytes.fill(0);
	}
}

/* Owner: signing a readers list. */

const grantCovers = (
	grant: ManagementPolicy["grants"][number],
	scope: string,
	projectId: string | null,
) =>
	grant.scope.kind === "device" ||
	(grant.scope.kind === "placement" &&
		grant.scope.project_id === projectId &&
		grant.scope.placement_id === scope) ||
	(grant.scope.kind === "project" && grant.scope.project_id === projectId);

/** Until when `userId` may read `kind` on the scope under the given rules; undefined when not at all. */
export function readerGrantExpiry(
	policy: ManagementPolicy | undefined,
	userId: string,
	stream: { scope: string; kind: HistoryKind; projectId: string | null },
	now: number,
): number | undefined {
	const expiries = (policy?.grants ?? [])
		.filter(
			(grant) =>
				grant.user_id === userId &&
				grant.capabilities.includes(stream.kind) &&
				grant.expires_at > now &&
				grantCovers(grant, stream.scope, stream.projectId),
		)
		.map((grant) => grant.expires_at);
	return expiries.length ? Math.max(...expiries) : undefined;
}

export interface SaveReadersRequest {
	stream: { scope: string; kind: HistoryKind; projectId: string | null };
	/** Readers besides this computer's own key, which is always included. */
	others: readonly ArchiveRecipient[];
	/** Seconds the list stays valid; capped by the access rules and each reader's access. */
	durationS: number;
	/** Verb + object ("Resume recording metrics"): tray item and result. */
	label: string;
	password?: string;
}

export type SaveReadersOutcome =
	| { status: "done"; expiresAt: number; version: number }
	| { status: "password_required" }
	| { status: "stopped" }
	| { status: "failed"; reason: SaveFailure; detail?: string };

export type SaveFailure =
	| "rules_not_applied"
	| "rules_unverified"
	| "reader_without_access"
	| "too_many_readers"
	| "wrong_password"
	| "rejected"
	| "other";

export class SaveReadersError extends Error {
	constructor(
		readonly reason: SaveFailure,
		readonly detail?: string,
	) {
		super(reason);
		this.name = "SaveReadersError";
	}
}

const appliedBy = (view: PolicyView) =>
	view.version === view.applied_version && view.digest === view.applied_digest;

/** The signed rules of a hub record, checked against this device, its version and its digest. */
async function signedPolicy(
	trust: DeviceTrust,
	view: PolicyView,
	signed: string,
): Promise<ManagementPolicy> {
	const policy = trust.crypto.verifyManagementPolicy(
		signed,
		trust.manifest.owner_invitation_key,
	);
	const matches =
		policy.device_id === trust.manifest.device_id &&
		policy.policy_version === view.version &&
		(await digestText(signed)) === view.digest;
	if (!matches) throw new SaveReadersError("rules_unverified");
	return policy;
}

/** The device's access rules as the hub holds them, verified; rejects until the device applied the newest ones. */
export async function verifiedPolicy(
	workspace: DeviceWorkspace,
	trust: DeviceTrust,
	deviceId: string,
): Promise<{ view: PolicyView; policy: ManagementPolicy | undefined }> {
	const view = await readPolicyView(
		workspace.hub.api,
		workspace.hub.profile,
		deviceId,
	);
	if (!appliedBy(view)) throw new SaveReadersError("rules_not_applied");
	if (view.policy_jws)
		return { view, policy: await signedPolicy(trust, view, view.policy_jws) };
	// No rules were ever saved: only then is an unsigned record genuine.
	if (view.version !== 0 || view.digest !== null)
		throw new SaveReadersError("rules_unverified");
	return { view, policy: undefined };
}

/**
 * Signs and applies a readers list (the one path for Set up, Change readers
 * and Resume recording). The sheet shows the consequences itself, so the
 * action layer only gates, tracks and reports.
 */
export function useSaveReaders(target: ObserveTarget) {
	const workspace = useDeviceWorkspace();
	const actions = useDeviceAction();
	const client = useQueryClient();
	const { deviceId, me, name } = target;
	const applied = policyAppliedOf(target);

	return useCallback(
		async (request: SaveReadersRequest): Promise<SaveReadersOutcome> => {
			const { stream } = request;
			const outcome = await actions.run({
				action: "approve_history_readers",
				deviceId,
				target: {
					...(stream.scope === DEVICE_SCOPE
						? {}
						: { placementId: stream.scope }),
					...(stream.projectId ? { projectId: stream.projectId } : {}),
					extra: { policyApplied: applied },
				},
				label: request.label,
				resultKey: historyResultKey(deviceId),
				activity: {
					kind: "history_readers",
					deviceName: name,
					...(stream.scope === DEVICE_SCOPE ? {} : { serviceId: stream.scope }),
				},
				call: async (context) => {
					const trust = await deviceTrust(workspace, deviceId);
					const signer = workspace.keys.signer(deviceId);
					if (!signer || me !== trust.manifest.owner_id)
						throw new SaveReadersError("other");
					const now = Math.floor(workspace.clock.now() / 1000);
					const previous = await readStream(
						trust,
						context.call,
						stream.scope,
						stream.kind,
					);
					const { view, policy } = await verifiedPolicy(
						workspace,
						trust,
						deviceId,
					);
					const mine = ownRecipient(trust.controller, me);
					const recipients = [
						mine,
						...request.others.filter(
							(row) => row.recipient_id !== mine.recipient_id,
						),
					];
					if (recipients.length > MAX_READERS)
						throw new SaveReadersError("too_many_readers");
					let expiresAt = Math.min(
						now + Math.min(request.durationS, MAX_RECORDING_S),
						policy?.expires_at ?? Number.POSITIVE_INFINITY,
					);
					for (const reader of recipients) {
						if (reader.user_id === trust.manifest.owner_id) continue;
						const until = readerGrantExpiry(
							policy,
							reader.user_id,
							stream,
							now,
						);
						if (until === undefined)
							throw new SaveReadersError(
								"reader_without_access",
								reader.user_id,
							);
						expiresAt = Math.min(expiresAt, until);
					}
					const roster: ArchiveRoster = {
						version: 1,
						device_id: trust.manifest.device_id,
						scope: stream.scope,
						project_id: stream.scope === DEVICE_SCOPE ? null : stream.projectId,
						kind: stream.kind,
						policy_version: (previous.roster?.policy_version ?? 0) + 1,
						previous_policy_digest: previous.rosterText
							? await digestText(previous.rosterText)
							: null,
						management_policy_digest: view.digest,
						recipients,
						issued_at: now,
						expires_at: expiresAt,
					};
					const signed = await signer.signArchiveRoster(
						roster,
						request.password,
					);
					if (signed.length > MAX_ROSTER_CHARS)
						throw new SaveReadersError("too_many_readers");
					await context.request({ type: "archive_policy", policy_jws: signed });
					return { expiresAt, version: roster.policy_version };
				},
			});
			if (outcome.status !== "done")
				return unsavedOutcome(outcome, request.password !== undefined);
			await client.invalidateQueries({
				queryKey: historyKey(workspace.scopeKey, deviceId),
			});
			return { status: "done", ...outcome.result };
		},
		[actions, workspace, client, deviceId, me, name, applied],
	);
}

export type UnsavedOutcome = Exclude<SaveReadersOutcome, { status: "done" }>;

/** Why an owner signature or its command didn't go through, as the sheets report it. */
export function unsavedOutcome(
	outcome: Exclude<DeviceActionOutcome<unknown>, { status: "done" }>,
	typedPassword: boolean,
): UnsavedOutcome {
	if (outcome.status === "failed") {
		const error = outcome.error;
		if (error instanceof OwnerPasswordRequiredError)
			return { status: "password_required" };
		if (error instanceof SaveReadersError)
			return {
				status: "failed",
				reason: error.reason,
				...(error.detail ? { detail: error.detail } : {}),
			};
		if (
			typedPassword &&
			error instanceof Error &&
			/incorrect password/i.test(error.message)
		)
			return { status: "failed", reason: "wrong_password" };
		return { status: "failed", reason: "other" };
	}
	if (outcome.status === "rejected")
		return {
			status: "failed",
			reason: "rejected",
			...(outcome.rejection.error ? { detail: outcome.rejection.error } : {}),
		};
	return outcome.status === "unknown"
		? { status: "failed", reason: "other" }
		: { status: "stopped" };
}

/** Where the outcome of a readers change shows (R9). */
export const historyResultKey = (deviceId: string) =>
	`observe-history:${deviceId}`;

/** Whether the device applied the newest access rules; a readers list can only be signed for applied rules. */
export function policyAppliedOf(target: ObserveTarget): boolean {
	return !target.policyView || appliedBy(target.policyView);
}

/**
 * The rules digest a readers list must match to keep recording, for
 * `recordingOf`. Only an agent that doesn't report the recording state (BG30)
 * needs the comparison; a newer one says "paused" itself.
 */
export function interimDigest(
	target: ObserveTarget,
): string | null | undefined {
	if (agentSupports(target.features, "archive_status")) return undefined;
	return target.policyView ? target.policyView.applied_digest : undefined;
}
