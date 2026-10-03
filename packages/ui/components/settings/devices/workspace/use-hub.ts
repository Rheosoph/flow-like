"use client";

import { skipToken, useQuery } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { getApiOrigin } from "../../../../lib/api-url";
import type { PublicCertificateInventory } from "../../../../lib/device-management/certificates";
import {
	type CertificateNotice,
	type HubError,
	type HubResult,
	toHubError,
} from "../../../../lib/device-management/hub/endpoints";
import {
	HUB_CADENCE,
	type PollState,
	deviceKeys,
	deviceQueryDefaults,
	hubDeviceSupport,
	pollInterval,
	queries,
	releaseConfigOf,
} from "../../../../lib/device-management/hub/queries";
import { classify } from "../../../../lib/device-management/model/freshness";
import type {
	AccountBackupList,
	AppDevicePlacements,
	ArchiveUsage,
	CopyRef,
	DeviceRow,
	DeviceUsageResponse,
	FleetCertificateInventoryRow,
	Freshness,
	FreshnessReason,
	FreshnessSignal,
	HubDeviceSupport,
	HubEnrollment,
	HubErrorCode,
	MyAccess,
	ResourceSummary,
} from "../../../../lib/device-management/model/types";
import type { DeviceSetupReadiness } from "../../../../lib/device-management/readiness";
import { readAccountBackupStatus } from "../../../../lib/device-management/recovery";
import type {
	ManagementPolicy,
	PolicyView,
} from "../../../../lib/device-management/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import type { VerifiedRelease } from "../../../../lib/device-package";
import type { DeviceResources } from "../../../../lib/device-resources";
import { useDeviceWorkspace } from "./device-workspace-provider";
import { type PolicyVerification, useAttentionState } from "./use-attention";

/** One hub read as screens consume it: data, where and when it was read, and the older-hub interim. */
export interface HubRead<T> {
	data: T | undefined;
	freshness: Freshness;
	/** The hub is older than this route (404/405): render the interim, never an error. */
	missingOnHub: boolean;
	error?: HubError;
	/** No data yet and a first read is running. */
	loading: boolean;
	refetch(): Promise<void>;
}

interface QueryView {
	data: unknown;
	error: unknown;
	dataUpdatedAt: number;
	errorUpdatedAt: number;
	errorUpdateCount: number;
	isLoading: boolean;
	refetch(): Promise<unknown>;
}

const ERROR_REASON: Partial<Record<HubErrorCode, FreshnessReason>> = {
	network: "network",
	timeout: "timeout",
	server_error: "server_error",
	rate_limited: "rate_limited",
};

/** Local milliseconds to hub-corrected unix seconds. */
function hubSeconds(workspace: DeviceWorkspace, localMs: number): number {
	return Math.floor(
		(localMs - (workspace.clock.hubOffsetS ?? 0) * 1000) / 1000,
	);
}

function refusal(error: HubError): CopyRef<FreshnessReason> | undefined {
	if (error.code !== "forbidden") return undefined;
	return { code: error.gate === "G12" ? "role_missing" : "access_ended" };
}

function retryAt(
	workspace: DeviceWorkspace,
	query: QueryView,
	intervalMs: number | false,
): number | undefined {
	if (intervalMs === false || !query.errorUpdatedAt) return undefined;
	const state: PollState = {
		data: query.data,
		error: query.error,
		dataUpdateCount: query.data === undefined ? 0 : 1,
		errorUpdateCount: query.errorUpdateCount,
		dataUpdatedAt: query.dataUpdatedAt,
		errorUpdatedAt: query.errorUpdatedAt,
	};
	return hubSeconds(
		workspace,
		query.errorUpdatedAt + pollInterval(intervalMs, state),
	);
}

interface Cadence {
	intervalMs: number | false;
}

/** IA §2.2: a failed refresh keeps the data, says why and when it retries; a hub without the route is "not supported". */
function hubFreshness(
	workspace: DeviceWorkspace,
	query: QueryView,
	signal: FreshnessSignal,
	cadence: Cadence,
	missing: boolean,
): Freshness {
	const loaded = query.data !== undefined && !missing;
	const at = query.dataUpdatedAt
		? hubSeconds(workspace, query.dataUpdatedAt)
		: undefined;
	const error = query.error ? toHubError(query.error) : undefined;
	const noAccess = error ? refusal(error) : undefined;
	const next = retryAt(workspace, query, cadence.intervalMs);
	const freshness = classify(signal, {
		now: at ?? Math.floor(workspace.clock.now() / 1000),
		at,
		loaded,
		...(missing ? { unsupported: { code: "hub_update_needed" as const } } : {}),
		...(noAccess ? { noAccess } : {}),
		...(error && !noAccess
			? {
					error: {
						code: ERROR_REASON[error.code] ?? "refresh_failed",
						...(next === undefined ? {} : { retryAt: next }),
					},
				}
			: {}),
	});
	return cadence.intervalMs === false || freshness.cadenceS !== undefined
		? freshness
		: { ...freshness, cadenceS: cadence.intervalMs / 1000 };
}

function useRead<T>(
	query: QueryView,
	signal: FreshnessSignal,
	cadence: Cadence,
	unwrap: (data: unknown) => { data: T | undefined; missing: boolean },
): HubRead<T> {
	const workspace = useDeviceWorkspace();
	const { data, error, dataUpdatedAt, errorUpdatedAt, errorUpdateCount } =
		query;
	const { isLoading, refetch } = query;
	const run = useCallback(async () => {
		await refetch();
	}, [refetch]);
	return useMemo(() => {
		const view: QueryView = {
			data,
			error,
			dataUpdatedAt,
			errorUpdatedAt,
			errorUpdateCount,
			isLoading,
			refetch,
		};
		const read = unwrap(data);
		return {
			data: read.data,
			freshness: hubFreshness(workspace, view, signal, cadence, read.missing),
			missingOnHub: read.missing,
			...(error ? { error: toHubError(error) } : {}),
			loading: isLoading,
			refetch: run,
		};
	}, [
		workspace,
		data,
		error,
		dataUpdatedAt,
		errorUpdatedAt,
		errorUpdateCount,
		isLoading,
		refetch,
		run,
		signal,
		cadence,
		unwrap,
	]);
}

const plain = <T>(data: unknown) => ({
	data: data as T | undefined,
	missing: false,
});
const result = <T>(data: unknown) => {
	const read = data as HubResult<T> | undefined;
	return {
		data: read?.kind === "ok" ? read.data : undefined,
		missing: read?.kind === "missing_on_hub",
	};
};

/* G2 and the hub pill. */

export interface HubSupportRead {
	support: HubDeviceSupport;
	/** The hub's host, for "Checking api.flow-like.com…". */
	host: string;
	freshness: Freshness;
	/** Epoch milliseconds of the last failed attempt ("Still unreachable at 14:02:10"). */
	failedAt?: number;
	checking: boolean;
	retry(): Promise<void>;
}

function hostOf(origin: string): string {
	try {
		return new URL(origin).host;
	} catch {
		return origin;
	}
}

export function useHubSupport(): HubSupportRead {
	const workspace = useDeviceWorkspace();
	const { input } = useAttentionState();
	const query = useQuery(queries.hub(workspace.hub));
	const read = useRead<unknown>(query, "hub_support", HUB_CADENCE.hub, plain);
	const origin = getApiOrigin(workspace.deps.profile);
	return useMemo(
		() => ({
			support: input.hub,
			host: hostOf(origin),
			freshness: read.freshness,
			...(query.errorUpdatedAt ? { failedAt: query.errorUpdatedAt } : {}),
			checking: query.isFetching,
			retry: read.refetch,
		}),
		[
			input.hub,
			origin,
			read.freshness,
			read.refetch,
			query.errorUpdatedAt,
			query.isFetching,
		],
	);
}

/* The device list: one `GET /devices` every 30 s (plan §2.9). */

export interface DeviceRowsRead {
	rows: DeviceRow[] | undefined;
	freshness: Freshness;
	error?: HubError;
	loading: boolean;
	refetch(): Promise<void>;
}

export function useDeviceRows(): DeviceRowsRead {
	const { rows } = useAttentionState();
	const read = useRead<DeviceRow[]>(
		{ ...rows, isLoading: rows.data === undefined && !rows.error },
		"device_row",
		HUB_CADENCE.list,
		plain,
	);
	return useMemo(
		() => ({
			rows: read.data,
			freshness: read.freshness,
			...(read.error ? { error: read.error } : {}),
			loading: read.loading,
			refetch: read.refetch,
		}),
		[read],
	);
}

export function useDeviceRow(
	deviceId: string | undefined,
): DeviceRow | undefined {
	const { input } = useAttentionState();
	return useMemo(
		() => input.devices.find((row) => row.device_id === deviceId),
		[input.devices, deviceId],
	);
}

/* Area gates (SPEC §3.12) that need the hub; the provider handles the ones before it. */

export type AreaGate =
	| { kind: "ready" }
	| { kind: "token_restricted" }
	| { kind: "session_expired" }
	| { kind: "hub_checking"; host: string }
	| { kind: "hub_off"; host: string }
	| {
			kind: "hub_unreachable";
			host: string;
			error?: CopyRef<HubErrorCode>;
			failedAt?: number;
			checking: boolean;
			retry(): Promise<void>;
	  };

export function useAreaGate(): AreaGate {
	const hub = useHubSupport();
	const { rows } = useAttentionState();
	const listCode = rows.error ? toHubError(rows.error).code : undefined;
	return useMemo<AreaGate>(() => {
		const { state, error } = hub.support;
		if (listCode === "token_restricted") return { kind: "token_restricted" };
		if (listCode === "unauthorized") return { kind: "session_expired" };
		if (state === "checking") return { kind: "hub_checking", host: hub.host };
		if (state === "off") return { kind: "hub_off", host: hub.host };
		if (state === "unreachable")
			return {
				kind: "hub_unreachable",
				host: hub.host,
				...(error ? { error } : {}),
				...(hub.failedAt ? { failedAt: hub.failedAt } : {}),
				checking: hub.checking,
				retry: hub.retry,
			};
		return { kind: "ready" };
	}, [hub, listCode]);
}

/* Hub reads per route. Per-device hooks idle while the id is undefined. */

export function useReadiness(): HubRead<DeviceSetupReadiness> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery(queries.readiness(hub));
	return useRead<DeviceSetupReadiness>(
		query,
		"readiness",
		HUB_CADENCE.readiness,
		plain,
	);
}

export function useEnrollments(
	state: "open" | "recent" = "open",
): HubRead<HubEnrollment[]> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery(queries.enrollments(hub, state));
	return useRead<HubEnrollment[]>(
		query,
		"hub_support",
		HUB_CADENCE.enrollments,
		result,
	);
}

export function useDeviceUsage(): HubRead<DeviceUsageResponse> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery(queries.usage(hub));
	return useRead<DeviceUsageResponse>(
		query,
		"hub_support",
		HUB_CADENCE.usage,
		result,
	);
}

export function useMyAccess(deviceId: string | undefined): HubRead<MyAccess> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery({
		...queries.myAccess(hub, deviceId ?? ""),
		enabled: !!deviceId,
	});
	return useRead<MyAccess>(query, "policy", HUB_CADENCE.myAccess, result);
}

export function useAccountBackups(): HubRead<AccountBackupList> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery(queries.accountBackups(hub));
	return useRead<AccountBackupList>(
		query,
		"hub_support",
		HUB_CADENCE.accountBackups,
		result,
	);
}

/** Older hubs without the backup list: the revision of one device's account backup (0 = none). */
export function useAccountBackupStatus(
	deviceId: string | undefined,
): HubRead<{ revision: number }> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery({
		queryKey: deviceKeys.accountBackup(hub.scopeKey, deviceId ?? ""),
		queryFn: deviceId
			? async ({ signal }) =>
					(await readAccountBackupStatus(
						hub.api,
						hub.profile,
						deviceId,
						signal,
					)) ?? { revision: 0 }
			: skipToken,
		...deviceQueryDefaults(HUB_CADENCE.accountBackups),
	});
	return useRead<{ revision: number }>(
		query,
		"hub_support",
		HUB_CADENCE.accountBackups,
		plain,
	);
}

export function useFleetCertificateInventory(): HubRead<
	FleetCertificateInventoryRow[]
> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery(queries.certInventoryAll(hub));
	return useRead<FleetCertificateInventoryRow[]>(
		query,
		"hub_support",
		HUB_CADENCE.certInventoryAll,
		result,
	);
}

export function useCertificateInventory(
	deviceId: string | undefined,
): HubRead<PublicCertificateInventory> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery({
		...queries.certInventory(hub, deviceId ?? ""),
		enabled: !!deviceId,
	});
	return useRead<PublicCertificateInventory>(
		query,
		"hub_support",
		HUB_CADENCE.certInventory,
		plain,
	);
}

export function useCertificateNotices(
	deviceId: string | undefined,
	certificateId?: string,
): HubRead<CertificateNotice[]> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery({
		...queries.certNotices(hub, deviceId ?? "", certificateId),
		enabled: !!deviceId,
	});
	return useRead<CertificateNotice[]>(
		query,
		"hub_support",
		HUB_CADENCE.certNotices,
		result,
	);
}

export function useArchiveUsage(): HubRead<ArchiveUsage> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery(queries.archiveUsage(hub));
	return useRead<ArchiveUsage>(
		query,
		"archive_list",
		HUB_CADENCE.archiveUsage,
		result,
	);
}

export function useResourceSummary(): HubRead<ResourceSummary> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery(queries.resourceSummary(hub));
	return useRead<ResourceSummary>(
		query,
		"resources",
		HUB_CADENCE.resourceSummary,
		result,
	);
}

export function useDeviceResources(
	deviceId: string | undefined,
): HubRead<DeviceResources> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery({
		...queries.resources(hub, deviceId ?? ""),
		enabled: !!deviceId,
	});
	return useRead<DeviceResources>(
		query,
		"resources",
		HUB_CADENCE.resources,
		plain,
	);
}

export function useAppPlacements(
	appId: string | undefined,
): HubRead<AppDevicePlacements> {
	const { hub } = useDeviceWorkspace();
	const query = useQuery({
		...queries.appPlacements(hub, appId ?? ""),
		enabled: !!appId,
	});
	return useRead<AppDevicePlacements>(
		query,
		"resources",
		HUB_CADENCE.appPlacements,
		result,
	);
}

export interface ReleaseTrustRead extends HubRead<VerifiedRelease> {
	/** The hub states which release keys to trust; without it agents can't be set up or updated. */
	configured: boolean;
}

/** The verified release manifest (N10, agent updates); idle while the hub has no release trust. */
export function useReleaseTrust(): ReleaseTrustRead {
	const { hub } = useDeviceWorkspace();
	const support = useQuery(queries.hub(hub));
	const config = releaseConfigOf(support.data);
	const query = useQuery(queries.release(hub, config));
	const read = useRead<VerifiedRelease>(
		query,
		"hub_support",
		HUB_CADENCE.release,
		plain,
	);
	return useMemo(() => ({ ...read, configured: !!config }), [read, config]);
}

export interface PolicyRead extends HubRead<PolicyView> {
	/** The owner-signed rules, verified with this device's open keys; absent while locked. */
	policy?: ManagementPolicy;
	/** Why `policy` is there or not, once the hub copy is read: `rejected` rules are an error, `pending` ones a wait. */
	verification?: PolicyVerification;
}

/** `awaitingApply` polls every 10 s while an access-rules tray item waits for the device. */
export function usePolicy(
	deviceId: string | undefined,
	options: { awaitingApply?: boolean } = {},
): PolicyRead {
	const { hub } = useDeviceWorkspace();
	const { verifyPolicy, policyState } = useAttentionState();
	const awaitingApply = options.awaitingApply === true;
	const query = useQuery({
		...queries.policy(hub, deviceId ?? "", { awaitingApply }),
		enabled: !!deviceId,
	});
	const read = useRead<PolicyView>(
		query,
		"policy",
		awaitingApply ? HUB_CADENCE.policyAwaitingApply : HUB_CADENCE.policy,
		plain,
	);
	return useMemo(() => {
		if (!deviceId || !read.data) return read;
		const policy = verifyPolicy(deviceId, read.data);
		const verification = policyState(deviceId, read.data);
		return policy
			? { ...read, policy, verification }
			: { ...read, verification };
	}, [read, deviceId, verifyPolicy, policyState]);
}

/** `hubDeviceSupport` for callers that hold their own hub and usage reads (N10). */
export { hubDeviceSupport };
