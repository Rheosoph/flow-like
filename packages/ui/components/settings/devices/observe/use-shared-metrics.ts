"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import {
	GroupMetricsReader,
	applyTelemetryPolicy,
	digestText,
	readTelemetryRoster,
} from "../../../../lib/device-management/telemetry";
import type {
	Ed25519PublicKey,
	ManagementPolicy,
	TelemetryMember,
	TelemetryRoster,
} from "../../../../lib/device-management/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { deviceCall, useDeviceAction, useDeviceWorkspace } from "../workspace";
import { livePhase } from "./live-state";
import {
	type DeviceTrust,
	SaveReadersError,
	type UnsavedOutcome,
	deviceTrust,
	policyAppliedOf,
	unsavedOutcome,
	verifiedPolicy,
} from "./use-history";
import type { ObserveTarget } from "./use-observe-target";

/** The request a person sends the owner to read shared live metrics on their computer. */
export const METRICS_REQUEST_KIND = "flow-like.metrics-reader-request";
const DAY_S = 86_400;
const MAX_REQUESTS = 30;
const EVERY_MS = 60_000;

export interface MetricsReaderRequest {
	member: TelemetryMember;
	key_package: string;
}

export interface MetricsRequestFile {
	kind: typeof METRICS_REQUEST_KIND;
	version: 1;
	device_id: string;
	scope: string;
	request: MetricsReaderRequest;
}

export type MetricsRequestProblem = "invalid" | "other_device" | "too_many";

const ENDPOINT_ID = /^[A-Za-z0-9_:.-]{1,128}$/u;
const MAX_KEY_PACKAGE_CHARS = 16_384;

const isKeyPackage = (value: unknown): value is string =>
	typeof value === "string" &&
	value !== "" &&
	value.length <= MAX_KEY_PACKAGE_CHARS;

const isMember = (value: unknown): value is TelemetryMember => {
	const member = value as Partial<TelemetryMember> | null;
	return (
		typeof member?.endpoint_id === "string" &&
		ENDPOINT_ID.test(member.endpoint_id) &&
		typeof member.signing_key?.x === "string"
	);
};

function requestOf(value: unknown): MetricsReaderRequest | undefined {
	const row = value as Partial<MetricsReaderRequest> | null;
	if (!row || !isKeyPackage(row.key_package) || !isMember(row.member))
		return undefined;
	return { member: row.member, key_package: row.key_package };
}

type ParsedMetricsRequest =
	| { ok: true; requests: MetricsReaderRequest[] }
	| { ok: false; problem: MetricsRequestProblem };

/** The requests a parsed value names: the file's one request, or what it is itself. */
function requestedRows(
	value: unknown,
	deviceId: string,
	scope: string,
): { rows: unknown[] } | { problem: MetricsRequestProblem } {
	const file = value as Partial<MetricsRequestFile> | null;
	const isFile =
		!!file && typeof file === "object" && file.kind === METRICS_REQUEST_KIND;
	if (isFile && (file.device_id !== deviceId || file.scope !== scope))
		return { problem: "other_device" };
	const named = isFile ? file.request : value;
	return { rows: Array.isArray(named) ? named : [named] };
}

/** Reads the request file, one bare request, or the list older versions of the app showed as text. */
export function parseMetricsRequest(
	text: string,
	deviceId: string,
	scope: string,
): ParsedMetricsRequest {
	let value: unknown;
	try {
		value = JSON.parse(text);
	} catch {
		return { ok: false, problem: "invalid" };
	}
	const named = requestedRows(value, deviceId, scope);
	if ("problem" in named) return { ok: false, problem: named.problem };
	if (named.rows.length > MAX_REQUESTS)
		return { ok: false, problem: "too_many" };
	const requests = named.rows.flatMap((row) => {
		const request = requestOf(row);
		return request ? [request] : [];
	});
	return requests.length && requests.length === named.rows.length
		? { ok: true, requests }
		: { ok: false, problem: "invalid" };
}

/** RFC 7638 thumbprint of an Ed25519 key, as the device reports it for a reader's controller (BG24). */
export function keyThumbprint(key: Ed25519PublicKey): Promise<string> {
	return digestText(`{"crv":"Ed25519","kty":"OKP","x":"${key.x}"}`);
}

export interface SharedReader {
	endpointId: string;
	/** The person behind the endpoint; known only to the owner on a newer agent (BG24). */
	userId?: string;
	/** This computer's own reader. */
	you: boolean;
	/** Whether it confirmed the latest sample; undefined when the device doesn't say. */
	confirmed: boolean | undefined;
}

export interface SharedRoster {
	roster: TelemetryRoster | null;
	rosterText: string | null;
	readers: SharedReader[];
	/** Readers are shown as people (BG24) and not as endpoint IDs. */
	named: boolean;
	/** Unix seconds. */
	readAt: number;
}

async function ownersOf(
	trust: DeviceTrust,
	workspace: DeviceWorkspace,
	deviceId: string,
	policy: ManagementPolicy | undefined,
): Promise<Map<string, string>> {
	const vault = workspace.keys.vault(deviceId);
	const ownerKey =
		vault?.ownerControllerKey ?? trust.controller.publicBundle().controller_key;
	const byThumbprint = new Map<string, string>();
	byThumbprint.set(await keyThumbprint(ownerKey), trust.manifest.owner_id);
	for (const grant of policy?.grants ?? [])
		byThumbprint.set(await keyThumbprint(grant.controller_key), grant.user_id);
	return byThumbprint;
}

/** Everyone in the group but the device itself, which publishes. */
function readersOf(
	roster: TelemetryRoster,
	facts: {
		mine: string;
		confirmed: readonly string[] | undefined;
		userOf(endpointId: string): string | undefined;
	},
): SharedReader[] {
	return roster.members
		.filter((member) => member.endpoint_id !== roster.publisher.endpoint_id)
		.map((member) => {
			const userId = facts.userOf(member.endpoint_id);
			return {
				endpointId: member.endpoint_id,
				...(userId ? { userId } : {}),
				you: member.endpoint_id === facts.mine,
				confirmed: facts.confirmed?.includes(member.endpoint_id),
			};
		});
}

/** The readers list as the owner signed it for this group; null when the device has none. */
function verifiedRoster(
	trust: DeviceTrust,
	text: string | null | undefined,
	scope: string,
): TelemetryRoster | null {
	if (!text) return null;
	const roster = trust.crypto.verifyHistoricalTelemetryRoster(
		text,
		trust.manifest.owner_invitation_key,
	);
	if (roster.device_id !== trust.manifest.device_id || roster.scope !== scope)
		throw new Error("The device returned the readers list of another group.");
	return roster;
}

/** Tells the attention engine when this group's list expires; other groups keep theirs. */
function recordReaders(
	workspace: DeviceWorkspace,
	deviceId: string,
	scope: string,
	roster: TelemetryRoster | null,
): void {
	const known = workspace.facts.get(deviceId)?.metricReaders ?? [];
	workspace.facts.record(deviceId, {
		metricReaders: [
			...known.filter((entry) => entry.scope !== scope),
			...(roster ? [{ scope, expiresAt: roster.expires_at }] : []),
		],
	});
}

async function readRoster(
	workspace: DeviceWorkspace,
	deviceId: string,
	scope: string,
	policy: ManagementPolicy | undefined,
): Promise<SharedRoster> {
	const trust = await deviceTrust(workspace, deviceId);
	const read = await readTelemetryRoster(
		deviceCall(workspace, deviceId, "poll"),
		scope,
	);
	const roster = verifiedRoster(trust, read.text, scope);
	const readAt = Math.floor(workspace.clock.now() / 1000);
	recordReaders(workspace, deviceId, scope, roster);
	if (!roster)
		return { roster, rosterText: null, readers: [], named: false, readAt };
	const bindings = read.reader_bindings;
	const people = bindings
		? await ownersOf(trust, workspace, deviceId, policy)
		: new Map<string, string>();
	return {
		roster,
		rosterText: read.text,
		readers: readersOf(roster, {
			mine: trust.controller.publicBundle().endpoint_id,
			confirmed: read.confirmed_readers,
			userOf: (endpointId) => {
				const binding = bindings?.find((row) => row.endpoint_id === endpointId);
				return binding
					? people.get(binding.controller_key_thumbprint)
					: undefined;
			},
		}),
		named: !!bindings,
		readAt,
	};
}

const sharedKey = (scopeKey: string, deviceId: string, scope: string) =>
	["devices", scopeKey, "observe-shared-metrics", deviceId, scope] as const;

export interface SharedRosterRead {
	data: SharedRoster | undefined;
	reading: boolean;
	failed: boolean;
	refresh(): Promise<void>;
}

/** The readers of the shared live metrics; read only while the block is open and a session is up. */
export function useSharedRoster(
	target: ObserveTarget,
	scope: string,
	open: boolean,
): SharedRosterRead {
	const workspace = useDeviceWorkspace();
	const { deviceId, policy } = target;
	const enabled = open && livePhase(target) === "open";
	const query = useQuery({
		queryKey: sharedKey(workspace.scopeKey, deviceId, scope),
		enabled,
		retry: false,
		gcTime: 0,
		staleTime: EVERY_MS / 2,
		refetchInterval: EVERY_MS,
		queryFn: () => readRoster(workspace, deviceId, scope, policy),
	});
	const { data, isFetching, isError, refetch } = query;
	const refresh = useCallback(async () => {
		await refetch();
	}, [refetch]);
	return useMemo(
		() => ({
			data: enabled ? data : undefined,
			reading: enabled && isFetching,
			failed: enabled && isError && !data,
			refresh,
		}),
		[enabled, data, isFetching, isError, refresh],
	);
}

/** This computer's request to become a reader: a public key package, no secret. */
export async function createMetricsRequest(
	workspace: DeviceWorkspace,
	deviceId: string,
	scope: string,
): Promise<MetricsRequestFile> {
	const trust = await deviceTrust(workspace, deviceId);
	const reader = await GroupMetricsReader.open(
		trust.controller,
		workspace.deps.scope,
		trust.manifest,
		trust.receipt,
		scope,
	);
	try {
		const request = await reader.keyPackage(
			trust.controller.publicBundle().telemetry_member,
		);
		return {
			kind: METRICS_REQUEST_KIND,
			version: 1,
			device_id: deviceId,
			scope,
			request,
		};
	} finally {
		reader.close();
	}
}

export interface SaveMetricReadersRequest {
	/** Reader requests to admit; none = renew the list as it is. */
	add: readonly MetricsReaderRequest[];
	/** Readers (endpoint IDs) the new list leaves out: they stop receiving samples. */
	remove?: readonly string[];
	/** Verb + object: tray item and result. */
	label: string;
	/** R8 rows for the confirm step; omit when the sheet showed them itself. */
	consequence?: ConsequenceRows;
	/** The confirm step takes something away. */
	danger?: boolean;
	password?: string;
}

export type SaveMetricReadersOutcome =
	| { status: "done"; expiresAt: number; readers: number }
	| UnsavedOutcome;

/** Where the outcome of a change to the shared readers shows (R9). */
export const sharedResultKey = (deviceId: string, scope: string) =>
	`observe-shared:${deviceId}:${scope}`;

/**
 * Signs and applies the shared-metrics readers list: admits new readers and
 * moves the expiry (at most a day ahead, never past the access rules).
 */
export function useSaveMetricReaders(target: ObserveTarget, scope: string) {
	const workspace = useDeviceWorkspace();
	const actions = useDeviceAction();
	const client = useQueryClient();
	const { deviceId, me, name } = target;
	const applied = policyAppliedOf(target);

	return useCallback(
		async (
			request: SaveMetricReadersRequest,
		): Promise<SaveMetricReadersOutcome> => {
			const outcome = await actions.run({
				action: "approve_metric_readers",
				deviceId,
				target: { extra: { policyApplied: applied } },
				label: request.label,
				...(request.consequence
					? { consequence: request.consequence, strength: "none" as const }
					: {}),
				...(request.danger ? { confirm: { tone: "danger" as const } } : {}),
				resultKey: sharedResultKey(deviceId, scope),
				activity: { kind: "metric_readers", deviceName: name },
				call: async (context) => {
					const trust = await deviceTrust(workspace, deviceId);
					const signer = workspace.keys.signer(deviceId);
					if (!signer || me !== trust.manifest.owner_id)
						throw new SaveReadersError("other");
					const now = Math.floor(workspace.clock.now() / 1000);
					const current = await readTelemetryRoster(context.call, scope);
					const previous = current.text
						? trust.crypto.verifyHistoricalTelemetryRoster(
								current.text,
								trust.manifest.owner_invitation_key,
							)
						: undefined;
					if (
						previous &&
						(previous.device_id !== trust.manifest.device_id ||
							previous.scope !== scope)
					)
						throw new SaveReadersError("other");
					const { view: saved, policy } = await verifiedPolicy(
						workspace,
						trust,
						deviceId,
					);
					const reader = await GroupMetricsReader.open(
						trust.controller,
						workspace.deps.scope,
						trust.manifest,
						trust.receipt,
						scope,
					);
					try {
						const keyPackages = [...request.add];
						if (!reader.position().joined)
							keyPackages.push(
								await reader.keyPackage(
									trust.controller.publicBundle().telemetry_member,
								),
							);
						const publisher: TelemetryMember = {
							endpoint_id: trust.manifest.device_id,
							signing_key: trust.receipt.identity.telemetry_key,
						};
						const left = new Set(request.remove ?? []);
						const members = [...(previous?.members ?? [publisher])].filter(
							(member) =>
								member.endpoint_id === publisher.endpoint_id ||
								!left.has(member.endpoint_id),
						);
						for (const entry of keyPackages) {
							if (
								members.some(
									(member) => member.endpoint_id === entry.member.endpoint_id,
								)
							)
								continue;
							members.push(entry.member);
						}
						const expiresAt = Math.min(
							now + DAY_S,
							policy?.expires_at ?? Number.POSITIVE_INFINITY,
						);
						const roster: TelemetryRoster = {
							version: 1,
							device_id: trust.manifest.device_id,
							scope,
							policy_version: (previous?.policy_version ?? 0) + 1,
							previous_policy_digest: current.text
								? await digestText(current.text)
								: null,
							management_policy_digest: saved.digest,
							publisher,
							members,
							issued_at: now,
							expires_at: expiresAt,
						};
						const signed = await signer.signTelemetryRoster(
							roster,
							request.password,
						);
						await applyTelemetryPolicy(context.call, {
							scope,
							policy_jws: signed,
							key_packages: keyPackages.filter(
								(entry) =>
									!previous?.members.some(
										(member) => member.endpoint_id === entry.member.endpoint_id,
									),
							),
						});
						return { expiresAt, readers: members.length - 1 };
					} finally {
						reader.close();
					}
				},
			});
			if (outcome.status !== "done")
				return unsavedOutcome(outcome, request.password !== undefined);
			await client.invalidateQueries({
				queryKey: sharedKey(workspace.scopeKey, deviceId, scope),
			});
			return { status: "done", ...outcome.result };
		},
		[actions, workspace, client, deviceId, scope, me, name, applied],
	);
}
