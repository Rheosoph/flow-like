"use client";

import { useQueries, useQuery } from "@tanstack/react-query";
import { useEffect, useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import {
	type DeploymentCatalog,
	type DeploymentVariable,
	type PlacementConfiguration,
	discoverOfflineVariables,
	discoverPreviousOnlineVariables,
	readExistingDeployment,
} from "../../../../lib/device-management/deployment";
import { queries } from "../../../../lib/device-management/hub/queries";
import type { AttentionInput } from "../../../../lib/device-management/model/types";
import { prepareOnlineMetadata } from "../../../../lib/device-management/online-metadata";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { RolePermissions } from "../../../../lib/permission/role-permission";
import {
	type IBackendState,
	useBackend,
} from "../../../../state/backend-state";
import type { IOwnRole } from "../../../../state/backend-state/types";
import { deviceCall } from "../workspace/use-live";
import type { DeployDevice } from "./deploy-facts";

/* What the wizard reads besides the hub list: the picked devices' configurations and the variable definitions a plan needs before it is prepared. */

export type EventVariables = Readonly<
	Record<string, readonly DeploymentVariable[]>
>;
export type Configurations = Readonly<
	Record<string, readonly PlacementConfiguration[]>
>;

/** Variable definitions read outside the preparation: from the hub (settings-only updates) or from the device (offline updates). */
export interface DefinitionsRead {
	variables?: EventVariables;
	loading: boolean;
	error?: string;
}

/** The viewer's role on an app, as far as the hub answers. */
export interface DeployRole {
	/** False only when the role is known and lacks Read boards. */
	canReadFlows: boolean;
	/** Undefined without a role answer (local-only app, signed out, older hub). */
	isOwner?: boolean;
}

type OwnRoleReader = (appId: string) => Promise<IOwnRole>;

async function noRole(): Promise<IOwnRole | null> {
	return null;
}

function ownRoleReader(backend: IBackendState): {
	roleState: unknown;
	getOwnRole: OwnRoleReader | null;
} {
	const roleState = backend.roleState as
		| { getOwnRole?: OwnRoleReader }
		| undefined;
	return {
		roleState,
		getOwnRole:
			typeof roleState?.getOwnRole === "function" ? roleState.getOwnRole : null,
	};
}

export function roleFacts(role: IOwnRole | null | undefined): DeployRole {
	if (!role || !Number.isInteger(role.permissions))
		return { canReadFlows: true };
	return {
		canReadFlows: new RolePermissions(BigInt(role.permissions)).hasPermission(
			RolePermissions.ReadBoards,
		),
		isOwner: role.is_owner,
	};
}

/** The chosen app's role: Read boards opens the deploy, and only the app's owner can approve its online files. */
export function useDeployRole(appId: string | null | undefined): DeployRole {
	const backend = useBackend();
	const { roleState, getOwnRole } = ownRoleReader(backend);
	const role = useInvoke<IOwnRole | null, [string]>(
		getOwnRole ?? noRole,
		roleState,
		[appId ?? ""],
		!!getOwnRole && !!appId,
	);
	return useMemo(() => roleFacts(role.data), [role.data]);
}

/** The app picker: which of the listed apps the viewer's role can't read. An app without a role answer stays open. */
export function useUnreadableApps(
	appIds: readonly string[],
): ReadonlySet<string> {
	const backend = useBackend();
	const { roleState, getOwnRole } = ownRoleReader(backend);
	const results = useQueries({
		queries: appIds.map((appId) => ({
			queryKey: ["getOwnRole", appId],
			queryFn: () =>
				getOwnRole ? getOwnRole.call(roleState, appId) : noRole(),
			enabled: !!getOwnRole,
		})),
	});
	const stamp = stampOf(results);
	// biome-ignore lint/correctness/useExhaustiveDependencies: `stamp` changes whenever a result does
	return useMemo(
		() =>
			new Set(
				appIds.filter(
					(_, index) => !roleFacts(results[index]?.data).canReadFlows,
				),
			),
		[appIds, stamp],
	);
}

/** `${deviceId}/${serviceId}`: one service on one device. */
export const serviceKey = (deviceId: string, serviceId: string) =>
	`${deviceId}/${serviceId}`;

const stampOf = (
	results: readonly { dataUpdatedAt: number; errorUpdatedAt: number }[],
) =>
	results
		.map((result) => `${result.dataUpdatedAt}:${result.errorUpdatedAt}`)
		.join("|");

/** A selected device that is unlocked gets a live session while the wizard is open. */
export function useLiveDemand(
	workspace: DeviceWorkspace,
	devices: readonly DeployDevice[],
	selected: readonly string[],
) {
	const wanted = devices
		.filter(
			(device) =>
				selected.includes(device.id) &&
				device.keyState === "unlocked" &&
				device.gate === null,
		)
		.map((device) => device.id)
		.sort()
		.join("|");
	useEffect(() => {
		if (!wanted) return;
		const releases = wanted
			.split("|")
			.map((deviceId) => workspace.live.acquire(deviceId, "view"));
		return () => {
			for (const release of releases) release();
		};
	}, [workspace, wanted]);
}

/**
 * What each shared device lets this account do (BG22), so a device without
 * Deploy for this app is gated before it is unlocked. An older hub leaves it
 * unknown: the device can be picked and says so once it is unlocked.
 */
export function useSharedAccess(
	workspace: DeviceWorkspace,
	input: AttentionInput,
): AttentionInput {
	const me = workspace.deps.scope.account;
	const shared = useMemo(
		() =>
			input.devices
				.filter((row) => row.status === "active" && row.owner_id !== me)
				.map((row) => row.device_id),
		[input.devices, me],
	);
	const results = useQueries({
		queries: shared.map((deviceId) =>
			queries.myAccess(workspace.hub, deviceId),
		),
	});
	const stamp = stampOf(results);
	// biome-ignore lint/correctness/useExhaustiveDependencies: `stamp` stands for the results' content
	return useMemo(() => {
		const known = { ...input.myAccess };
		shared.forEach((deviceId, index) => {
			const read = results[index]?.data;
			if (read?.kind === "ok") known[deviceId] = read.data;
		});
		return { ...input, myAccess: known };
	}, [input, shared, stamp]);
}

interface ServiceRef {
	deviceId: string;
	serviceId: string;
	projectId: string;
}

/** Configurations of every service on the selected live devices: ports in use and what an update starts from. */
export function useDeviceConfigurations(
	workspace: DeviceWorkspace,
	devices: readonly DeployDevice[],
	selected: readonly string[],
): Record<string, PlacementConfiguration[]> {
	const refs = useMemo<ServiceRef[]>(
		() =>
			devices
				.filter((device) => selected.includes(device.id) && device.isLive)
				.flatMap((device) =>
					(device.services ?? []).map((service) => ({
						deviceId: device.id,
						serviceId: service.serviceId,
						projectId: service.projectId,
					})),
				),
		[devices, selected],
	);
	const results = useQueries({
		queries: refs.map((ref) => ({
			queryKey: [
				"devices-deploy-configuration",
				workspace.scopeKey,
				ref.deviceId,
				ref.serviceId,
			],
			queryFn: () =>
				readExistingDeployment(
					deviceCall(workspace, ref.deviceId, "poll"),
					ref.serviceId,
					ref.projectId,
				),
			staleTime: 30_000,
			retry: false,
		})),
	});
	const stamp = stampOf(results);
	// biome-ignore lint/correctness/useExhaustiveDependencies: `stamp` stands for the results' content
	return useMemo(() => {
		const byDevice: Record<string, PlacementConfiguration[]> = {};
		refs.forEach((ref, index) => {
			const data = results[index]?.data;
			if (data)
				byDevice[ref.deviceId] = [...(byDevice[ref.deviceId] ?? []), data];
		});
		return byDevice;
	}, [refs, stamp]);
}

/** The app's services among the configurations read, each with its device. */
function appConfigurations(
	configurations: Configurations,
	appId: string | null,
	source: "offline" | "online",
) {
	return Object.entries(configurations).flatMap(([deviceId, list]) =>
		list
			.filter((row) => row.project_id === appId && row.config.source === source)
			.map((configuration) => ({ deviceId, configuration })),
	);
}

async function discoverInstalled(
	workspace: DeviceWorkspace,
	deviceId: string,
	configuration: PlacementConfiguration,
): Promise<Record<string, DeploymentVariable[]>> {
	const call = deviceCall(workspace, deviceId, "poll");
	const installed = {
		project_id: configuration.project_id,
		project_path: configuration.config.project_path,
		revision: configuration.config.revision,
		source: "offline" as const,
	};
	const variables: Record<string, DeploymentVariable[]> = {};
	for (const event of configuration.config.events)
		variables[event.event_id] = await discoverOfflineVariables(
			call,
			installed,
			event.event_id,
		);
	return variables;
}

export interface InstalledRead extends DefinitionsRead {
	/** Definitions of the version each service runs now, by `serviceKey`. */
	byService: Readonly<Record<string, readonly DeploymentVariable[]>>;
}

/** Offline copies: the settings of the services an update starts from, as their devices describe them. */
export function useInstalledVariables(
	workspace: DeviceWorkspace,
	enabled: boolean,
	appId: string | null,
	configurations: Configurations,
): InstalledRead {
	const own = useMemo(
		() => (enabled ? appConfigurations(configurations, appId, "offline") : []),
		[enabled, appId, configurations],
	);
	const results = useQueries({
		queries: own.map(({ deviceId, configuration }) => ({
			queryKey: [
				"devices-deploy-installed-variables",
				workspace.scopeKey,
				deviceId,
				configuration.placement_id,
				configuration.config.revision,
			],
			queryFn: () => discoverInstalled(workspace, deviceId, configuration),
			staleTime: Number.POSITIVE_INFINITY,
			retry: false,
		})),
	});
	const stamp = stampOf(results);
	const loading = results.some((result) => result.isLoading);
	// biome-ignore lint/correctness/useExhaustiveDependencies: `stamp` stands for the results' content
	return useMemo(() => {
		const merged: Record<string, readonly DeploymentVariable[]> = {};
		const byService: Record<string, DeploymentVariable[]> = {};
		own.forEach(({ deviceId, configuration }, index) => {
			const data = results[index]?.data;
			if (!data) return;
			Object.assign(merged, data);
			byService[serviceKey(deviceId, configuration.placement_id)] =
				Object.values(data).flat();
		});
		const failed = results.find((result) => result.error);
		return {
			...(Object.keys(merged).length ? { variables: merged } : {}),
			byService,
			loading,
			...(failed?.error ? { error: failed.error.message } : {}),
		};
	}, [own, stamp, loading]);
}

/** Online apps whose update keeps each service's version are never prepared; their settings still need the definitions. */
export function useKeepCatalog(
	workspace: DeviceWorkspace,
	appId: string | null,
	enabled: boolean,
): { catalog?: DeploymentCatalog; loading: boolean; error?: string } {
	const backend = useBackend();
	const query = useQuery({
		queryKey: ["devices-deploy-catalog", workspace.scopeKey, appId],
		queryFn: async () =>
			(
				await prepareOnlineMetadata(
					appId ?? "",
					backend,
					workspace.deps.profile,
				)
			).catalog,
		enabled: enabled && !!appId,
		staleTime: 60_000,
		retry: false,
	});
	const { data, isLoading, error } = query;
	return useMemo(
		() => ({
			...(data ? { catalog: data } : {}),
			loading: isLoading,
			...(error ? { error: error.message } : {}),
		}),
		[data, isLoading, error],
	);
}

/**
 * Online updates: the definitions each stored secret was written for, so a
 * secret whose type changed in the newest version is caught before Review.
 * A read that fails leaves the service out; the device still refuses a mismatch.
 */
export function usePreviousSecrets(
	appId: string | null,
	catalog: DeploymentCatalog | null,
	configurations: Configurations,
): Record<string, readonly DeploymentVariable[]> {
	const backend = useBackend();
	const own = useMemo(
		() =>
			catalog
				? appConfigurations(configurations, appId, "online").filter(
						({ configuration }) =>
							Object.keys(configuration.config.secret_overrides).length > 0,
					)
				: [],
		[catalog, appId, configurations],
	);
	const pins = (catalog?.events ?? [])
		.map((event) => `${event.id}@${event.board_version?.join(".")}`)
		.join(",");
	const results = useQueries({
		queries: own.map(({ deviceId, configuration }) => ({
			queryKey: [
				"devices-deploy-previous-secrets",
				deviceId,
				configuration.placement_id,
				configuration.config_revision,
				pins,
			],
			queryFn: () =>
				discoverPreviousOnlineVariables(
					backend.eventState,
					backend.boardState,
					{
						project_id: configuration.project_id,
						project_path: configuration.config.project_path,
						source: "online",
						...(catalog ? { online_catalog: catalog } : {}),
					},
					configuration,
				),
			staleTime: Number.POSITIVE_INFINITY,
			retry: false,
		})),
	});
	const stamp = stampOf(results);
	// biome-ignore lint/correctness/useExhaustiveDependencies: `stamp` stands for the results' content
	return useMemo(() => {
		const byService: Record<string, readonly DeploymentVariable[]> = {};
		own.forEach(({ deviceId, configuration }, index) => {
			const data = results[index]?.data;
			if (data)
				byService[serviceKey(deviceId, configuration.placement_id)] = data;
		});
		return byService;
	}, [own, stamp]);
}
