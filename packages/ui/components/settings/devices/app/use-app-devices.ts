"use client";

import { useCallback, useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import type {
	AppHubFacts,
	AppInput,
	AppView,
	AppVisibility,
} from "../../../../lib/device-management/model/app-plan";
import { evaluateGate } from "../../../../lib/device-management/model/gates";
import {
	type Headline,
	headline,
} from "../../../../lib/device-management/model/headline";
import type {
	AppDevicePlacements,
	DeviceViewModel,
	GateFailure,
} from "../../../../lib/device-management/model/types";
import type { ActivityItem } from "../../../../lib/device-management/workspace/types";
import { RolePermissions } from "../../../../lib/permission/role-permission";
import { useBackend } from "../../../../state/backend-state";
import type { IOwnRole } from "../../../../state/backend-state/types";
import {
	type Coverage,
	type HubRead,
	buildGateContext,
	useActivity,
	useAppView,
	useAttention,
	useAttentionState,
	useCoverage,
	useDeviceViews,
	useDeviceWorkspace,
	useManagerValue,
} from "../workspace";
import {
	type AppRun,
	type ServiceUpload,
	appRuns,
	appUploads,
	changesOf,
	headlineApp,
	openRunIds,
} from "./app-view-local";

export interface AppRole {
	/** The role request is still running: hold device reads until it answers. */
	loading: boolean;
	/** The hub answered with a role; local-only apps have none. */
	known: boolean;
	/** Read boards: without it the page can't show where the app runs (APP §1.6). */
	canReadFlows: boolean;
	isOwner: boolean;
	roleName?: string;
}

async function noRole(): Promise<IOwnRole | null> {
	return null;
}

/** The viewer's role on the app; an app without a role table (local-only, signed out) is open. */
export function useAppRole(appId: string): AppRole {
	const backend = useBackend();
	const roleState = backend.roleState as
		| { getOwnRole?: (appId: string) => Promise<IOwnRole> }
		| undefined;
	const getOwnRole =
		typeof roleState?.getOwnRole === "function" ? roleState.getOwnRole : null;
	const role = useInvoke<IOwnRole | null, [string]>(
		getOwnRole ?? noRole,
		roleState,
		[appId],
		!!getOwnRole && !!appId,
	);
	return useMemo(() => {
		const data = Number.isInteger(role.data?.permissions)
			? role.data
			: undefined;
		const permissions = data
			? new RolePermissions(BigInt(data.permissions))
			: undefined;
		return {
			loading: !!getOwnRole && role.isLoading,
			known: !!data,
			canReadFlows:
				permissions?.hasPermission(RolePermissions.ReadBoards) ?? true,
			// `is_owner` also holds for an Admin; only the Owner permission names the app's owner.
			isOwner: permissions?.contains(RolePermissions.Owner) ?? false,
			...(data?.role_name ? { roleName: data.role_name } : {}),
		};
	}, [role.data, role.isLoading, getOwnRole]);
}

export interface AppDevicesData {
	appId: string;
	/** The signed-in account. */
	me: string;
	/** The app with its events, once loaded. */
	app: AppInput | undefined;
	/** What the app's hub can do for its events (which types it hands to devices, Latest). */
	hub: AppHubFacts;
	role: AppRole;
	/** The app's name once known, else its id. */
	appName: string;
	visibility: AppVisibility | undefined;
	loading: boolean;
	error?: Error;
	view: AppView | undefined;
	headline: Headline | undefined;
	placements: HubRead<AppDevicePlacements>;
	devices: ReadonlyMap<string, DeviceViewModel>;
	/** Whose status the page can vouch for (the same numbers as the headline). */
	coverage: Coverage;
	/** Why a device can't take a deploy of this app now, by device id; absent = it can. */
	deployGates: ReadonlyMap<string, GateFailure>;
	/** Event id → name, for every event of the app. */
	eventNames: ReadonlyMap<string, string>;
	uploads: ServiceUpload[];
	runs: AppRun[];
	/** Ids of this app's runs that still have work open, a run on one device included. */
	openRuns: ReadonlySet<string>;
	activity: ActivityItem[];
	/** Unix seconds the app's events and settings were read from the backend. */
	readAt?: number;
	/** Re-reads the hub lists and every unlocked device's status; `failed` counts the device reads that didn't answer. */
	refresh(): Promise<{ failed: number }>;
}

/**
 * Everything App › Devices shows, from the app's metadata, the hub, each
 * device's planes and what this computer tracked. Without Read boards
 * nothing about devices is read.
 */
export function useAppDevices(
	appId: string,
	focusDeviceId?: string,
): AppDevicesData {
	const backend = useBackend();
	const workspace = useDeviceWorkspace();
	const state = useAttentionState();
	const { input, items } = state;
	const role = useAppRole(appId);
	const app = useInvoke(backend.appState.getApp, backend.appState, [appId]);
	// A local-only app exists only in the desktop app that created it: the web shows a gate and reads nothing.
	const webLocal =
		app.data?.visibility === "Offline" && workspace.deps.platform === "web";
	const allowed = !role.loading && role.canReadFlows && !!app.data && !webLocal;
	const readId = allowed ? appId : undefined;

	const meta = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId],
		!!appId,
	);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[appId],
		allowed,
	);

	const activity = useActivity({ projectId: appId }).items;
	const runList = useManagerValue(
		useCallback(
			(listener: () => void) => workspace.activity.subscribe(listener),
			[workspace],
		),
		useCallback(() => workspace.activity.runs(), [workspace]),
		(previous, next) =>
			previous.length === next.length &&
			previous.every((run, index) => run === next[index]),
	);

	const views = useDeviceViews({ watch: allowed });
	const devices = useMemo(
		() => new Map(views.map((view) => [view.row.device_id, view])),
		[views],
	);
	const coverage = useCoverage(appId);
	const deployGates = useMemo(() => {
		const gates = new Map<string, GateFailure>();
		for (const row of input.devices) {
			if (row.status !== "active") continue;
			const gate = evaluateGate(
				"create_service",
				buildGateContext(state, row.device_id, { projectId: appId }),
			);
			if (!gate.ok) gates.set(row.device_id, gate);
		}
		return gates;
	}, [input.devices, state, appId]);

	const changes = useMemo(
		() => changesOf(activity, appId, input.me),
		[activity, appId, input.me],
	);
	const extras = useMemo(
		() => ({
			changes,
			...(focusDeviceId ? { focusDeviceIds: [focusDeviceId] } : {}),
		}),
		[changes, focusDeviceId],
	);
	const read = useAppView(readId, extras);
	const { view } = read;
	const appHeadline = useMemo(
		() =>
			view ? headline(input, { items, app: headlineApp(view) }) : undefined,
		[view, input, items],
	);
	const eventNames = useMemo(
		() => new Map((events.data ?? []).map((event) => [event.id, event.name])),
		[events.data],
	);
	const uploads = useMemo(() => appUploads(activity, appId), [activity, appId]);
	const runs = useMemo(
		() => appRuns(runList, activity, appId),
		[runList, activity, appId],
	);
	const openRuns = useMemo(
		() => openRunIds(runList, activity, appId),
		[runList, activity, appId],
	);
	const { refetch: refetchApp } = app;
	const { refetch: refetchEvents } = events;
	const refresh = useCallback(async () => {
		const unlocked = input.keys
			.filter((session) => session.state === "unlocked")
			.map((session) => session.deviceId);
		// A connected device answers for itself: its live status is newer than its snapshot.
		const connected = unlocked.filter(
			(deviceId) => workspace.live.state(deviceId).kind === "live",
		);
		// The app and its events too: "Try again" after a failed load has to read them again.
		const [, devices] = await Promise.all([
			Promise.allSettled([
				state.rows.refetch(),
				read.placements.refetch(),
				refetchApp(),
				refetchEvents(),
			]),
			Promise.allSettled([
				...unlocked.map((deviceId) => workspace.fleet.refresh(deviceId)),
				...connected.map((deviceId) =>
					workspace.live.refreshInspection(deviceId),
				),
			]),
		]);
		return {
			failed: devices.filter((result) => result.status === "rejected").length,
		};
	}, [
		input.keys,
		state.rows,
		read.placements,
		workspace,
		refetchApp,
		refetchEvents,
	]);

	const error = read.error ?? app.error ?? undefined;
	const readMs = Math.min(
		app.dataUpdatedAt || Number.POSITIVE_INFINITY,
		events.dataUpdatedAt || Number.POSITIVE_INFINITY,
	);
	const readAt = Number.isFinite(readMs)
		? Math.floor(readMs / 1000)
		: undefined;
	const appName = meta.data?.name ?? view?.app.name ?? appId;
	const visibility = app.data?.visibility as AppVisibility | undefined;
	const loading = role.loading || (allowed && read.loading);
	const { app: appInput, placements, hub } = read;
	return useMemo(
		() => ({
			appId,
			me: input.me,
			app: appInput,
			hub,
			role,
			appName,
			visibility,
			loading,
			...(error ? { error } : {}),
			...(readAt === undefined ? {} : { readAt }),
			view,
			headline: appHeadline,
			placements,
			devices,
			coverage,
			deployGates,
			eventNames,
			uploads,
			runs,
			openRuns,
			activity,
			refresh,
		}),
		[
			appId,
			input.me,
			appInput,
			hub,
			role,
			appName,
			visibility,
			loading,
			error,
			readAt,
			view,
			appHeadline,
			placements,
			devices,
			coverage,
			deployGates,
			eventNames,
			uploads,
			runs,
			openRuns,
			activity,
			refresh,
		],
	);
}

/** Attention items of this app above info, most severe first. */
export function useAppAttention(appId: string) {
	return useAttention({ appId, minSeverity: "notice" });
}
