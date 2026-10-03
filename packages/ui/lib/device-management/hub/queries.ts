import { queryOptions, skipToken } from "@tanstack/react-query";
import type { IApiState } from "../../../state/backend-state/api-state";
import type { IProfile } from "../../../types";
import { getApiOrigin } from "../../api-url";
import { type ReleaseConfig, fetchVerifiedRelease } from "../../device-package";
import { loadDeviceResources } from "../../device-resources";
import { getDevice, listDevices } from "../../devices";
import type {
	DeviceUsageResponse,
	HubDeviceSupport,
	HubErrorCode,
} from "../model/types";
import { checkDeviceSetup } from "../readiness";
import type { ClockModel } from "../workspace/types";
import {
	type HubResult,
	type HubStandalone,
	getArchiveUsage,
	getBillingEligibility,
	getBillingUsage,
	getDeviceUsage,
	getFleetCertificateInventory,
	getMyAccess,
	getResourceSummary,
	hubReadWith,
	listAccountBackups,
	listAppDevicePlacements,
	listCertificateNoticeMutes,
	listCertificateNotices,
	listEnrollments,
	readCertificateInventory,
	readHubStandalone,
	readPolicyView,
	toHubError,
} from "./endpoints";

/** `s` = `accountStorageKey(scope)`. Per-device keys sit under their kind so one kind can be invalidated fleet-wide. */
export const deviceKeys = {
	root: (s: string) => ["devices", s] as const,
	hub: (s: string) => ["devices", s, "hub"] as const,
	readiness: (s: string) => ["devices", s, "readiness"] as const,
	list: (s: string) => ["devices", s, "list"] as const,
	device: (s: string, id: string) => ["devices", s, "device", id] as const,
	identity: (s: string, id: string, epoch: number) =>
		["devices", s, "identity", id, epoch] as const,
	policy: (s: string, id: string) => ["devices", s, "policy", id] as const,
	certInventory: (s: string, id: string) =>
		["devices", s, "cert-inventory", id] as const,
	certInventoryAll: (s: string) => ["devices", s, "cert-inventory"] as const,
	certNotices: (s: string, id: string, certificateId?: string) =>
		["devices", s, "cert-notices", id, certificateId ?? "*"] as const,
	certNoticeMutes: (s: string, id: string) =>
		["devices", s, "cert-notice-mutes", id] as const,
	resources: (s: string, id: string) =>
		["devices", s, "resources", id] as const,
	resourceSummary: (s: string) => ["devices", s, "resource-summary"] as const,
	billingEligibility: (s: string, id: string, grantId: string) =>
		["devices", s, "billing-eligibility", id, grantId] as const,
	billingUsage: (s: string, id: string, billingId: string) =>
		["devices", s, "billing-usage", id, billingId] as const,
	accountBackup: (s: string, id: string) =>
		["devices", s, "account-backup", id] as const,
	accountBackups: (s: string) => ["devices", s, "account-backups"] as const,
	archives: (s: string, id: string, scope: string, kind: "logs" | "metrics") =>
		["devices", s, "archives", id, scope, kind] as const,
	archiveUsage: (s: string) => ["devices", s, "archive-usage"] as const,
	enrollments: (s: string, state: "open" | "recent" = "open") =>
		["devices", s, "enrollments", state] as const,
	usage: (s: string) => ["devices", s, "usage"] as const,
	myAccess: (s: string, id: string) => ["devices", s, "my-access", id] as const,
	appPlacements: (s: string, appId: string) =>
		["devices", s, "app-placements", appId] as const,
	release: (s: string) => ["devices", s, "release"] as const,
};

const SECOND = 1_000;
const MINUTE = 60 * SECOND;
export const MAX_BACKOFF_MS = 5 * MINUTE;
export const DEVICE_QUERY_GC_MS = 5 * MINUTE;

interface Cadence {
	/** Poll interval while visible; `false` = on demand only. */
	intervalMs: number | false;
	staleMs: number;
}
const cadence = (intervalMs: number | false, staleMs: number): Cadence => ({
	intervalMs,
	staleMs,
});

/** M-DATA §3.5 cadences, plus the routes this work adds. */
export const HUB_CADENCE = {
	hub: cadence(5 * MINUTE, MINUTE),
	list: cadence(30 * SECOND, 15 * SECOND),
	device: cadence(false, 15 * SECOND),
	readiness: cadence(5 * MINUTE, MINUTE),
	policy: cadence(30 * SECOND, 5 * SECOND),
	policyAwaitingApply: cadence(10 * SECOND, 5 * SECOND),
	certInventory: cadence(MINUTE, 30 * SECOND),
	certInventoryAll: cadence(MINUTE, 30 * SECOND),
	certNotices: cadence(false, 30 * SECOND),
	resources: cadence(30 * SECOND, 10 * SECOND),
	resourceSummary: cadence(30 * SECOND, 10 * SECOND),
	billingEligibility: cadence(false, MINUTE),
	billingUsage: cadence(MINUTE, 30 * SECOND),
	accountBackups: cadence(5 * MINUTE, MINUTE),
	archiveUsage: cadence(5 * MINUTE, MINUTE),
	enrollments: cadence(MINUTE, 30 * SECOND),
	usage: cadence(5 * MINUTE, MINUTE),
	myAccess: cadence(MINUTE, 30 * SECOND),
	appPlacements: cadence(30 * SECOND, 10 * SECOND),
	release: cadence(60 * MINUTE, 30 * MINUTE),
} satisfies Record<string, Cadence>;

/** The parts of a react-query `QueryState` the poll schedule reads. */
export interface PollState {
	data: unknown;
	error: unknown;
	dataUpdateCount: number;
	errorUpdateCount: number;
	dataUpdatedAt: number;
	errorUpdatedAt: number;
}

/**
 * Failed polls since the last good read. React-query resets its own failure
 * count at every fetch, so a streak after a success is read from the gap to the
 * last good data: polls at base·2^k put n failures base·(2^n − 1) after it.
 */
export function consecutiveFailures(baseMs: number, state: PollState): number {
	if (!state.error) return 0;
	if (state.dataUpdateCount === 0) return Math.max(1, state.errorUpdateCount);
	const sinceGood = Math.max(0, state.errorUpdatedAt - state.dataUpdatedAt);
	return Math.max(1, Math.round(Math.log2(sinceGood / baseMs + 1)));
}

const isMissingResult = (data: unknown) =>
	(data as { kind?: unknown } | undefined)?.kind === "missing_on_hub";

/** IA §2.2: polling never stops on errors; it backs off to `min(base·2^failures, 5 min)` and never below `base`. */
export function pollInterval(baseMs: number, state: PollState): number {
	if (!state.error && isMissingResult(state.data))
		return Math.max(baseMs, MAX_BACKOFF_MS);
	const failures = consecutiveFailures(baseMs, state);
	if (failures === 0) return baseMs;
	return Math.max(baseMs, Math.min(baseMs * 2 ** failures, MAX_BACKOFF_MS));
}

const RETRYABLE: ReadonlySet<HubErrorCode> = new Set([
	"network",
	"timeout",
	"server_error",
]);

/** Up to two retries, for transient failures only: a refusal (401/403/404) or a bad body is a verdict. */
export function retryHubQuery(failureCount: number, error: unknown): boolean {
	return failureCount < 2 && RETRYABLE.has(toHubError(error).code);
}

export const hubRetryDelay = (attempt: number) =>
	Math.min(SECOND * 2 ** attempt, 8 * SECOND);

/**
 * Shared react-query options for every device-area hub query (M-DATA §3.5).
 * A failed refetch keeps the query's data and `gcTime` keeps it across route
 * switches. There is deliberately no `placeholderData: keepPreviousData`: it
 * only acts when the key changes, where it would show another device's, app's
 * or account's rows under the new one while it loads.
 */
export function deviceQueryDefaults({ intervalMs, staleMs }: Cadence) {
	return {
		staleTime: staleMs,
		gcTime: DEVICE_QUERY_GC_MS,
		refetchInterval:
			intervalMs === false
				? (false as const)
				: (query: { state: PollState }) =>
						pollInterval(intervalMs, query.state),
		refetchIntervalInBackground: false,
		retry: retryHubQuery,
		retryDelay: hubRetryDelay,
		meta: { persist: false },
	};
}

export interface HubQueryContext {
	api: IApiState;
	profile: IProfile;
	/** `accountStorageKey(scope)`. */
	scopeKey: string;
	/** Receives the hub's `server_time` from usage, resource summary and app placements. */
	clock?: Pick<ClockModel, "observe">;
	/** This computer's raw clock in milliseconds (never the hub-corrected one); injectable for tests. */
	now?: () => number;
	/** Fetch for the unauthenticated hub JSON; injectable for tests. */
	fetch?: typeof fetch;
}

function observeServerTime<T extends { server_time: number }>(
	ctx: HubQueryContext,
	result: HubResult<T>,
): HubResult<T> {
	if (result.kind === "ok")
		ctx.clock?.observe(
			"server_time",
			result.data.server_time,
			(ctx.now ?? Date.now)(),
		);
	return result;
}

/** Replaces `useHub`'s silent path for the area: errors surface instead of being logged. */
export function hubSupportQuery(ctx: HubQueryContext) {
	return queryOptions({
		queryKey: deviceKeys.hub(ctx.scopeKey),
		queryFn: ({ signal }) =>
			readHubStandalone(getApiOrigin(ctx.profile), ctx.fetch, signal),
		...deviceQueryDefaults(HUB_CADENCE.hub),
		refetchOnWindowFocus: true,
	});
}

export const queries = {
	hub: hubSupportQuery,
	list: (ctx: HubQueryContext) =>
		queryOptions({
			queryKey: deviceKeys.list(ctx.scopeKey),
			queryFn: () =>
				hubReadWith("account", "GET devices", () =>
					listDevices(ctx.api, ctx.profile),
				),
			...deviceQueryDefaults(HUB_CADENCE.list),
		}),
	device: (ctx: HubQueryContext, deviceId: string) =>
		queryOptions({
			queryKey: deviceKeys.device(ctx.scopeKey, deviceId),
			queryFn: () =>
				hubReadWith("device", "GET devices/{id}", () =>
					getDevice(ctx.api, ctx.profile, deviceId),
				),
			...deviceQueryDefaults(HUB_CADENCE.device),
		}),
	readiness: (ctx: HubQueryContext) =>
		queryOptions({
			queryKey: deviceKeys.readiness(ctx.scopeKey),
			queryFn: ({ signal }) =>
				hubReadWith("account", "GET devices/setup", () =>
					checkDeviceSetup(ctx.api, ctx.profile, signal),
				),
			...deviceQueryDefaults(HUB_CADENCE.readiness),
		}),
	/** `awaitingApply` while an access-rules tray item waits for `applied_version`. */
	policy: (
		ctx: HubQueryContext,
		deviceId: string,
		options: { awaitingApply?: boolean } = {},
	) =>
		queryOptions({
			queryKey: deviceKeys.policy(ctx.scopeKey, deviceId),
			queryFn: () => readPolicyView(ctx.api, ctx.profile, deviceId),
			...deviceQueryDefaults(
				options.awaitingApply
					? HUB_CADENCE.policyAwaitingApply
					: HUB_CADENCE.policy,
			),
		}),
	certInventory: (ctx: HubQueryContext, deviceId: string) =>
		queryOptions({
			queryKey: deviceKeys.certInventory(ctx.scopeKey, deviceId),
			queryFn: () => readCertificateInventory(ctx.api, ctx.profile, deviceId),
			...deviceQueryDefaults(HUB_CADENCE.certInventory),
		}),
	certInventoryAll: (ctx: HubQueryContext) =>
		queryOptions({
			queryKey: deviceKeys.certInventoryAll(ctx.scopeKey),
			queryFn: () => getFleetCertificateInventory(ctx.api, ctx.profile),
			...deviceQueryDefaults(HUB_CADENCE.certInventoryAll),
		}),
	certNotices: (
		ctx: HubQueryContext,
		deviceId: string,
		certificateId?: string,
	) =>
		queryOptions({
			queryKey: deviceKeys.certNotices(ctx.scopeKey, deviceId, certificateId),
			queryFn: () =>
				listCertificateNotices(ctx.api, ctx.profile, deviceId, certificateId),
			...deviceQueryDefaults(HUB_CADENCE.certNotices),
		}),
	certNoticeMutes: (ctx: HubQueryContext, deviceId: string) =>
		queryOptions({
			queryKey: deviceKeys.certNoticeMutes(ctx.scopeKey, deviceId),
			queryFn: () => listCertificateNoticeMutes(ctx.api, ctx.profile, deviceId),
			...deviceQueryDefaults(HUB_CADENCE.certNotices),
		}),
	resources: (ctx: HubQueryContext, deviceId: string) =>
		queryOptions({
			queryKey: deviceKeys.resources(ctx.scopeKey, deviceId),
			queryFn: () =>
				hubReadWith("device", "GET devices/{id}/resource-grants", () =>
					loadDeviceResources(ctx.api, ctx.profile, deviceId),
				),
			...deviceQueryDefaults(HUB_CADENCE.resources),
		}),
	resourceSummary: (ctx: HubQueryContext) =>
		queryOptions({
			queryKey: deviceKeys.resourceSummary(ctx.scopeKey),
			queryFn: async () =>
				observeServerTime(ctx, await getResourceSummary(ctx.api, ctx.profile)),
			...deviceQueryDefaults(HUB_CADENCE.resourceSummary),
		}),
	billingEligibility: (
		ctx: HubQueryContext,
		deviceId: string,
		grantId: string,
	) =>
		queryOptions({
			queryKey: deviceKeys.billingEligibility(ctx.scopeKey, deviceId, grantId),
			queryFn: () =>
				getBillingEligibility(ctx.api, ctx.profile, deviceId, grantId),
			...deviceQueryDefaults(HUB_CADENCE.billingEligibility),
		}),
	billingUsage: (ctx: HubQueryContext, deviceId: string, billingId: string) =>
		queryOptions({
			queryKey: deviceKeys.billingUsage(ctx.scopeKey, deviceId, billingId),
			queryFn: () => getBillingUsage(ctx.api, ctx.profile, deviceId, billingId),
			...deviceQueryDefaults(HUB_CADENCE.billingUsage),
		}),
	accountBackups: (ctx: HubQueryContext) =>
		queryOptions({
			queryKey: deviceKeys.accountBackups(ctx.scopeKey),
			queryFn: () => listAccountBackups(ctx.api, ctx.profile),
			...deviceQueryDefaults(HUB_CADENCE.accountBackups),
		}),
	archiveUsage: (ctx: HubQueryContext) =>
		queryOptions({
			queryKey: deviceKeys.archiveUsage(ctx.scopeKey),
			queryFn: () => getArchiveUsage(ctx.api, ctx.profile),
			...deviceQueryDefaults(HUB_CADENCE.archiveUsage),
		}),
	enrollments: (ctx: HubQueryContext, state: "open" | "recent" = "open") =>
		queryOptions({
			queryKey: deviceKeys.enrollments(ctx.scopeKey, state),
			queryFn: () => listEnrollments(ctx.api, ctx.profile, state),
			...deviceQueryDefaults(HUB_CADENCE.enrollments),
		}),
	usage: (ctx: HubQueryContext) =>
		queryOptions({
			queryKey: deviceKeys.usage(ctx.scopeKey),
			queryFn: async () =>
				observeServerTime(ctx, await getDeviceUsage(ctx.api, ctx.profile)),
			...deviceQueryDefaults(HUB_CADENCE.usage),
		}),
	myAccess: (ctx: HubQueryContext, deviceId: string) =>
		queryOptions({
			queryKey: deviceKeys.myAccess(ctx.scopeKey, deviceId),
			queryFn: () => getMyAccess(ctx.api, ctx.profile, deviceId),
			...deviceQueryDefaults(HUB_CADENCE.myAccess),
		}),
	appPlacements: (ctx: HubQueryContext, appId: string) =>
		queryOptions({
			queryKey: deviceKeys.appPlacements(ctx.scopeKey, appId),
			queryFn: async () =>
				observeServerTime(
					ctx,
					await listAppDevicePlacements(ctx.api, ctx.profile, appId),
				),
			...deviceQueryDefaults(HUB_CADENCE.appPlacements),
		}),
	/**
	 * Verified release manifest (N10, `agent_update_available`); idle without
	 * release trust. The trust it is verified with is part of the key, under
	 * `deviceKeys.release`: a manifest checked against old keys never answers
	 * for new ones.
	 */
	release: (ctx: HubQueryContext, config: ReleaseConfig | undefined) =>
		queryOptions({
			queryKey: [
				...deviceKeys.release(ctx.scopeKey),
				config
					? [config.manifestUrl, config.minimumSequence, ...config.publicKeys]
					: null,
			] as const,
			queryFn: config
				? ({ signal }) =>
						hubReadWith("account", "GET release manifest", () =>
							fetchVerifiedRelease(config, signal),
						)
				: skipToken,
			...deviceQueryDefaults(HUB_CADENCE.release),
		}),
};

/** `release_trust` from the hub JSON in the shape `fetchVerifiedRelease` takes. */
export function releaseConfigOf(
	standalone: HubStandalone | undefined,
): ReleaseConfig | undefined {
	const trust = standalone?.release_trust;
	return trust
		? {
				manifestUrl: trust.manifest_url,
				publicKeys: trust.public_keys,
				minimumSequence: trust.minimum_sequence,
			}
		: undefined;
}

function configuredLimits(
	standalone: HubStandalone,
): HubDeviceSupport["configuredLimits"] {
	const {
		max_devices_per_user: devices,
		max_pending_enrollments_per_user: pending,
		enrollment_ttl_seconds: ttl,
	} = standalone;
	const limits = {
		...(devices === undefined ? {} : { max_devices: devices }),
		...(pending === undefined ? {} : { max_pending_enrollments: pending }),
		...(ttl === undefined ? {} : { enrollment_ttl_seconds: ttl }),
	};
	return Object.keys(limits).length ? limits : undefined;
}

/**
 * G2 (CA6) from the hub JSON query and, when present, `GET /devices/usage`.
 * No answer yet is `checking`; no answer ever is `unreachable`; a failing
 * refresh keeps the last known state and adds the error (IA §2.2).
 */
export function hubDeviceSupport(
	hub: { data?: HubStandalone; error?: unknown },
	usage?: { data?: HubResult<DeviceUsageResponse> },
): HubDeviceSupport {
	const error = hub.error ? { code: toHubError(hub.error).code } : undefined;
	if (!hub.data)
		return error ? { state: "unreachable", error } : { state: "checking" };
	const counts = usage?.data?.kind === "ok" ? usage.data.data : undefined;
	const configured = configuredLimits(hub.data);
	return {
		state: hub.data.enabled ? "on" : "off",
		...(error ? { error } : {}),
		...(configured ? { configuredLimits: configured } : {}),
		...(counts
			? {
					limits: counts.limits,
					usage: counts.usage,
					serverTime: counts.server_time,
				}
			: {}),
		...(hub.data.release_trust ? { releaseTrust: hub.data.release_trust } : {}),
		...(hub.data.telemetry_tiers
			? { telemetryTiers: hub.data.telemetry_tiers }
			: {}),
	};
}
