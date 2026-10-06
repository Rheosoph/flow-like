"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueries, useQuery } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import { readExistingDeployment } from "../../../../lib/device-management/deployment";
import type {
	BillingEligibility,
	BillingUsage,
	HubError,
	HubResult,
} from "../../../../lib/device-management/hub/endpoints";
import { queries } from "../../../../lib/device-management/hub/queries";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	DeviceRow,
	Freshness,
	GateContext,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { DeviceResources } from "../../../../lib/device-resources";
import { RolePermissions } from "../../../../lib/permission/role-permission";
import type { IBit } from "../../../../lib/schema/bit/bit";
import {
	type IBackendState,
	useBackend,
} from "../../../../state/backend-state";
import type { IOwnRole } from "../../../../state/backend-state/types";
import { useAppNames } from "../shell/attention-popover";
import {
	deviceCall,
	useAttentionState,
	useDeviceResources,
	useDeviceRows,
	useDeviceWorkspace,
	useResourceSummary,
} from "../workspace";
import {
	type CloudApproval,
	approvalsOfDevice,
	approvalsOfSummary,
	byState,
	mergeApprovals,
} from "./cloud-model";
import { type ModelAccess, modelAccess } from "./model-access";

/* Devices and apps by name. */

export interface DeviceRef {
	id: string;
	name: string;
	/** Absent when the hub list doesn't carry the device. */
	row?: DeviceRow;
	revoked: boolean;
	mine: boolean;
}

export function useDeviceRefs(): (deviceId: string) => DeviceRef {
	const { input } = useAttentionState();
	const rows = useMemo(
		() => new Map(input.devices.map((row) => [row.device_id, row])),
		[input.devices],
	);
	return useCallback(
		(deviceId) => {
			const row = rows.get(deviceId);
			return row
				? {
						id: deviceId,
						name: deviceName(row),
						row,
						revoked: row.status === "revoked",
						mine: row.owner_id === input.me,
					}
				: {
						id: deviceId,
						name: deviceId.slice(0, 8),
						revoked: false,
						mine: false,
					};
		},
		[rows, input.me],
	);
}

export { useAppNames };

export interface AppRole {
	/** False while the role isn't read, and for local-only apps (no role table). */
	known: boolean;
	/** The app's owner: the only one who can allow access to its project files. */
	owner: boolean;
	projectRole?: NonNullable<GateContext["projectRole"]>;
}

type RoleReader = Pick<IBackendState["roleState"], "getOwnRole">;
const noRole = async (_appId: string): Promise<IOwnRole | undefined> => undefined;

/** The viewer's role on an app, as the approval gates need it (Admin or Owner with Execute boards). */
export function useAppRole(appId: string | null | undefined): AppRole {
	const backend = useBackend();
	const state = backend.roleState as RoleReader | undefined;
	const readable = typeof state?.getOwnRole === "function";
	const role = useInvoke(
		readable ? (state as RoleReader).getOwnRole : noRole,
		state,
		[appId ?? ""],
		readable && !!appId,
	);
	const data = role.data as IOwnRole | undefined;
	return useMemo(() => {
		if (!data || !Number.isInteger(data.permissions))
			return { known: false, owner: false };
		const bits = new RolePermissions(BigInt(data.permissions));
		// `is_owner` also holds for an Admin; only the Owner permission itself names the app's owner.
		const owner = bits.contains(RolePermissions.Owner);
		return {
			known: true,
			owner,
			projectRole: {
				readBoards: bits.hasPermission(RolePermissions.ReadBoards),
				// The hub accepts the Admin or the Owner permission for an approval.
				admin: data.is_owner || owner || bits.contains(RolePermissions.Admin),
				executeBoards: bits.hasPermission(RolePermissions.ExecuteBoards),
				owner,
			},
		};
	}, [data]);
}

/* Models by name (the approval lists Bit ids). */

const MODEL_TYPES: ReadonlySet<string> = new Set([
	"Llm",
	"Vlm",
	"Embedding",
	"ImageEmbedding",
]);
const NAME_STALE_MS = 10 * 60_000;
const MAX_APP_MODELS = 64;

type BitReader = Pick<IBackendState["bitState"], "getBit">;

function bitReader(backend: IBackendState): BitReader | undefined {
	const state = backend.bitState as BitReader | undefined;
	return typeof state?.getBit === "function" ? state : undefined;
}

function bitName(bit: IBit, language: string): string {
	const meta = bit.meta ?? {};
	const known = meta[language] ?? meta[language.split("-")[0] ?? ""] ?? meta.en;
	return known?.name || Object.values(meta)[0]?.name || bit.id;
}

function bitQuery(reader: BitReader | undefined, id: string, hub?: string) {
	return {
		queryKey: ["devices", "model-bit", id, hub ?? null] as const,
		queryFn: () => (reader as BitReader).getBit(id, hub),
		enabled: !!reader && !!id,
		staleTime: NAME_STALE_MS,
		retry: false,
		meta: { persist: false },
	};
}

interface BitResult {
	data: IBit | undefined;
	isLoading: boolean;
}

/*
 * `combine` is passed as a new function on every render on purpose: with a
 * stable one, the queries observer answers the first render after the list of
 * queries changed with the result of the previous list. The result keeps its
 * identity anyway while no query changed (`combine` shares structure).
 */
function readBits(results: readonly BitResult[]) {
	return {
		bits: results.map((entry) => entry.data),
		loading: results.some((entry) => entry.isLoading),
	};
}

/** Display names of model ids; an id the catalogue can't resolve stays as it is. */
export function useModelNames(ids: readonly string[]): (id: string) => string {
	const backend = useBackend();
	const { i18n } = useTranslation("devices");
	const language = i18n?.language ?? "en";
	const reader = bitReader(backend);
	const key = [...new Set(ids)].sort().join("|");
	const unique = useMemo(() => (key ? key.split("|") : []), [key]);
	const { bits } = useQueries({
		queries: unique.map((id) => bitQuery(reader, id)),
		combine: (results) => readBits(results),
	});
	const names = useMemo(() => {
		const found = new Map<string, string>();
		unique.forEach((id, index) => {
			const bit = bits[index];
			if (bit) found.set(id, bitName(bit, language));
		});
		return found;
	}, [unique, bits, language]);
	return useCallback((id) => names.get(id) ?? id, [names]);
}

export interface AppModel {
	id: string;
	name: string;
	access: ModelAccess;
}

export interface AppModelsRead {
	models: AppModel[];
	localModels: AppModel[];
	loading: boolean;
	/** False when the app or its model list can't be read here. */
	known: boolean;
}

/** The models an app uses (its pinned model Bits), for the approval form. */
export function useAppModels(appId: string | null | undefined): AppModelsRead {
	const backend = useBackend();
	const { i18n } = useTranslation("devices");
	const language = i18n?.language ?? "en";
	const reader = bitReader(backend);
	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[appId ?? ""],
		!!appId,
	);
	const refs = useMemo(
		() =>
			[...new Set(app.data?.bits ?? [])].slice(0, MAX_APP_MODELS).map((ref) => {
				const split = ref.lastIndexOf(":");
				return {
					id: ref.slice(split + 1),
					...(split >= 0 ? { hub: ref.slice(0, split) } : {}),
				};
			}),
		[app.data?.bits],
	);
	const { bits, loading } = useQueries({
		queries: refs.map((ref) => bitQuery(reader, ref.id, ref.hub)),
		combine: (results) => readBits(results),
	});
	return useMemo(() => {
		const models = bits.flatMap((bit) =>
			bit && MODEL_TYPES.has(bit.type)
				? [
						{
							id: bit.id,
							name: bitName(bit, language),
							access: modelAccess(bit),
						},
					]
				: [],
		);
		return {
			models: models.filter((model) => model.access !== "local"),
			localModels: models.filter((model) => model.access === "local"),
			loading: !!appId && (app.isLoading || loading),
			known: !!appId && !!reader && !!app.data && !app.error,
		};
	}, [
		bits,
		loading,
		language,
		appId,
		app.isLoading,
		app.data,
		app.error,
		reader,
	]);
}

/* Approvals. */

export type DetailState = "loaded" | "loading" | "failed" | "unread";

export interface FleetApprovals {
	rows: CloudApproval[];
	freshness: Freshness;
	/** Older hub: there is no fleet-wide list, so rows come from devices read one by one. */
	perDevice: boolean;
	loading: boolean;
	error?: HubError;
	/** Whether the device's own list (models, instances, leases) is read. */
	detail(deviceId: string): DetailState;
	/** Older hub: devices of the fleet list whose approvals weren't read yet. */
	unread: string[];
	refetch(): Promise<void>;
}

const MAX_DETAIL_DEVICES = 40;

interface DetailEntry {
	data: DeviceResources | undefined;
	state: Exclude<DetailState, "unread">;
	refetch(): Promise<unknown>;
}

function readDetails(
	results: readonly {
		data: DeviceResources | undefined;
		error: unknown;
		refetch(): Promise<unknown>;
	}[],
): DetailEntry[] {
	return results.map((entry) => ({
		data: entry.data,
		state: entry.data ? "loaded" : entry.error ? "failed" : "loading",
		refetch: entry.refetch,
	}));
}

/**
 * Every approval the viewer gave, owns the device of, or pays for (FG4).
 * `cap` rows get their device's own list read for the details; `extra` names
 * further devices to read (older hubs, expanded rows).
 */
export function useFleetApprovals(options: {
	cap: number;
	extra?: readonly string[];
	appId?: string;
}): FleetApprovals {
	const { cap, extra, appId } = options;
	const summary = useResourceSummary();
	const list = useDeviceRows();
	const { input, workspace } = useAttentionState();
	const { now, me } = input;
	const perDevice = summary.missingOnHub;

	const listed = useMemo(() => {
		const rows = summary.data ? approvalsOfSummary(summary.data, now) : [];
		return byState(appId ? rows.filter((row) => row.appId === appId) : rows);
	}, [summary.data, now, appId]);

	const opened = useMemo(
		() => Object.keys(input.resources).sort().join("|"),
		[input.resources],
	);
	const detailIds = useMemo(() => {
		const ids = new Set<string>(extra ?? []);
		if (perDevice)
			for (const id of opened ? opened.split("|") : []) ids.add(id);
		for (const row of listed.slice(0, cap)) ids.add(row.deviceId);
		return [...ids].slice(0, MAX_DETAIL_DEVICES);
	}, [extra, perDevice, opened, listed, cap]);

	const details = useQueries({
		queries: detailIds.map((id) => queries.resources(workspace.hub, id)),
		combine: (results) => readDetails(results),
	});

	const own = useMemo(
		() =>
			details.flatMap((entry) =>
				entry.data ? approvalsOfDevice(entry.data, me, now) : [],
			),
		[details, me, now],
	);
	const rows = useMemo(() => {
		const all = byState(mergeApprovals(listed, own));
		return appId ? all.filter((row) => row.appId === appId) : all;
	}, [listed, own, appId]);

	const detail = useCallback(
		(deviceId: string): DetailState => {
			return details[detailIds.indexOf(deviceId)]?.state ?? "unread";
		},
		[details, detailIds],
	);
	const unread = useMemo(
		() =>
			perDevice
				? input.devices
						.map((row) => row.device_id)
						.filter((id) => !detailIds.includes(id))
				: [],
		[perDevice, input.devices, detailIds],
	);
	const refetchSummary = summary.refetch;
	const refetch = useCallback(async () => {
		await Promise.all([
			refetchSummary(),
			...details.map((entry) => entry.refetch()),
		]);
	}, [refetchSummary, details]);

	return {
		rows,
		freshness: perDevice ? list.freshness : summary.freshness,
		perDevice,
		loading: perDevice
			? details.some((entry) => entry.state === "loading")
			: summary.loading,
		...(summary.error && !perDevice ? { error: summary.error } : {}),
		detail,
		unread,
		refetch,
	};
}

export interface DeviceApprovals {
	rows: CloudApproval[];
	freshness: Freshness;
	loading: boolean;
	error?: HubError;
	/** The device's own list was read at least once. */
	loaded: boolean;
	refetch(): Promise<void>;
}

/** One device's approvals: its own list, plus what the viewer pays for there without having given it. */
export function useDeviceApprovals(deviceId: string): DeviceApprovals {
	const resources = useDeviceResources(deviceId);
	const summary = useResourceSummary();
	const { input } = useAttentionState();
	const { now, me } = input;
	const rows = useMemo(() => {
		const listed = summary.data
			? approvalsOfSummary(summary.data, now).filter(
					(row) => row.deviceId === deviceId,
				)
			: [];
		const own = resources.data
			? approvalsOfDevice(resources.data, me, now)
			: [];
		return byState(mergeApprovals(listed, own));
	}, [summary.data, resources.data, deviceId, me, now]);
	return {
		rows,
		freshness: resources.freshness,
		loading: resources.loading,
		...(resources.error ? { error: resources.error } : {}),
		loaded: resources.data !== undefined,
		refetch: resources.refetch,
	};
}

/* E17 and E18: older hubs answer "missing", which the screens show as their interim. */

export interface OptionalRead<T> {
	data: T | undefined;
	/** The hub has no such route (BG34, BG35 interims). */
	missing: boolean;
	loading: boolean;
	failed: boolean;
}

function useOptionalRead<T>(
	data: HubResult<T> | undefined,
	isLoading: boolean,
	error: unknown,
): OptionalRead<T> {
	return useMemo(
		() => ({
			data: data?.kind === "ok" ? data.data : undefined,
			missing: data?.kind === "missing_on_hub",
			loading: isLoading,
			failed: !!error && data === undefined,
		}),
		[data, isLoading, error],
	);
}

/** BG35: spend per instance of one spending limit. */
export function useBillingUsage(
	deviceId: string,
	billingId: string | undefined,
): OptionalRead<BillingUsage> {
	const { hub } = useDeviceWorkspace();
	const { data, isLoading, error } = useQuery({
		...queries.billingUsage(hub, deviceId, billingId ?? ""),
		enabled: !!billingId,
	});
	return useOptionalRead(data, isLoading, error);
}

/** BG34: whether the viewer's plan covers the approval's models. */
export function useBillingEligibility(
	deviceId: string,
	grantId: string | undefined,
): OptionalRead<BillingEligibility> {
	const { hub } = useDeviceWorkspace();
	const { data, isLoading, error } = useQuery({
		...queries.billingEligibility(hub, deviceId, grantId ?? ""),
		enabled: !!grantId,
	});
	return useOptionalRead(data, isLoading, error);
}

/* Buffered changes that wait under an earlier approval. */

export interface PausedWrites {
	count: number;
	/** False when only the service's total is known, not what is paused. */
	exact: boolean;
}

/** Changes that are paused because cloud access changed; undefined when none are. */
export function usePausedWrites(
	deviceId: string,
	serviceId: string,
	service: ServiceView | undefined,
): PausedWrites | undefined {
	const { input } = useAttentionState();
	const queues = input.live[deviceId]?.offlineQueues?.[serviceId];
	return useMemo(() => {
		if (queues) {
			const count = queues
				.filter((queue) => queue.quarantined)
				.reduce((sum, queue) => sum + queue.pending_count, 0);
			return count > 0 ? { count, exact: true } : undefined;
		}
		const writes = service?.offlineWrites;
		return typeof writes === "object" && writes.quarantined
			? { count: writes.pending, exact: false }
			: undefined;
	}, [queues, service?.offlineWrites]);
}

/* Which approval a service's settings name (read live, needs Deploy & configure). */

export interface ServiceBinding {
	/** `unread`: no live connection, or the settings couldn't be read. */
	state: "bound" | "unbound" | "unread" | "loading";
	grantId?: string;
	version?: number;
	limitId?: string;
	/** The settings can't be read with the viewer's permissions. */
	refused: boolean;
	refresh(): Promise<void>;
}

const BINDING_STALE_MS = 30_000;

export function useServiceBinding(
	deviceId: string,
	serviceId: string,
	projectId: string | undefined,
): ServiceBinding {
	const { workspace, input } = useAttentionState();
	const live = input.live[deviceId];
	const kind = live?.state.kind;
	const connected = kind === "live" || kind === "renewing";
	const query = useQuery({
		queryKey: [
			"devices",
			workspace.scopeKey,
			"cloud-binding",
			deviceId,
			serviceId,
		],
		queryFn: async () => {
			const existing = await readExistingDeployment(
				deviceCall(workspace, deviceId, "poll"),
				serviceId,
				projectId ?? "",
			);
			const grant = existing.config.resource_grant ?? null;
			if (workspace.keys.snapshot(deviceId).state === "unlocked")
				workspace.facts.record(deviceId, {
					placements: {
						[serviceId]: {
							...workspace.facts.get(deviceId)?.placements?.[serviceId],
							resourceGrantId: grant?.grant_id ?? null,
						},
					},
				});
			return grant
				? {
						grantId: grant.grant_id,
						version: grant.authz_version,
						limitId: grant.billing_grant_id ?? undefined,
					}
				: null;
		},
		enabled: connected && !!projectId,
		staleTime: BINDING_STALE_MS,
		retry: false,
		meta: { persist: false },
	});
	const known = live?.placements?.[serviceId]?.resourceGrantId;
	const { data, error, isLoading, refetch } = query;
	return useMemo(() => {
		const refresh = async () => {
			await refetch();
		};
		const refused = !!error && data === undefined;
		if (data)
			return {
				state: "bound" as const,
				grantId: data.grantId,
				version: data.version,
				...(data.limitId ? { limitId: data.limitId } : {}),
				refused: false,
				refresh,
			};
		if (data === null)
			return { state: "unbound" as const, refused: false, refresh };
		if (known)
			return { state: "bound" as const, grantId: known, refused, refresh };
		if (known === null) return { state: "unbound" as const, refused, refresh };
		return {
			state: isLoading ? ("loading" as const) : ("unread" as const),
			refused,
			refresh,
		};
	}, [data, error, isLoading, refetch, known]);
}
