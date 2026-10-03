"use client";

import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import { queries } from "../../../../lib/device-management/hub/queries";
import {
	type AppDeviceInput,
	type AppEventInput,
	type AppHubFacts,
	type AppInput,
	type AppUploadInput,
	type AppVersionInput,
	type AppView,
	type AppVisibility,
	type LocalServiceChange,
	appEventRule,
	appHubFacts,
	appMode,
	buildAppView,
	endpointOwnToken,
	eventFormFacts,
} from "../../../../lib/device-management/model/app-plan";
import {
	appServices,
	refineView,
	revisionsSent,
	versionInputs,
} from "../../../../lib/device-management/model/app-versions";
import { buildDeviceView } from "../../../../lib/device-management/model/attention";
import { coverage } from "../../../../lib/device-management/model/coverage";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import { evaluateGate } from "../../../../lib/device-management/model/gates";
import type {
	AppDevicePlacements,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type {
	ActivityItem,
	ActivityState,
} from "../../../../lib/device-management/workspace/types";
import {
	CRON_EVENT_TYPE,
	projectScheduleConfig,
} from "../../../../lib/schedule-config";
import type { IApp } from "../../../../lib/schema/app/app";
import type { IEvent } from "../../../../lib/schema/flow/event";
import { parseUint8ArrayToJson } from "../../../../lib/uint8";
import { useBackend } from "../../../../state/backend-state";
import { useActivity } from "./use-activity";
import { buildGateContext, useAttentionState } from "./use-attention";
import { type HubRead, useAppPlacements } from "./use-hub";
import { useFlowStates } from "./use-latest-flows";

/** Facts the app screen computes itself and adds to the base view. */
export interface AppViewExtras {
	/** Newest first; `null` when the viewer's role can't read flows. Defaults to what the app pins now and what readable services still run. */
	versions?: readonly AppVersionInput[] | null;
	/** What this computer recorded per `${deviceId}/${serviceId}` (BG12 interim). */
	changes?: Readonly<Record<string, LocalServiceChange>>;
	focusDeviceIds?: readonly string[];
	/** Device-side refusals: device id → event id → reason. */
	refusals?: Readonly<Record<string, Readonly<Record<string, string>>>>;
	/** Why the approved bundle could not prepare an event: event id → reason. */
	ineligible?: Readonly<Record<string, string>>;
}

const NO_UPLOADS: readonly AppUploadInput[] = [];
const NO_DEVICES: readonly string[] = [];
const NO_VERSIONS: readonly AppVersionInput[] = [];
const NO_SERVICES: readonly ServiceView[] = [];

/** The app record's last change as unix seconds. */
const changedSeconds = (
	time: IApp["updated_at"] | null | undefined,
): number | null =>
	typeof time?.secs_since_epoch === "number" && time.secs_since_epoch > 0
		? time.secs_since_epoch
		: null;
const UPLOADING = new Set<ActivityState>(["active", "paused", "waiting"]);

/** Uploads of this app that run or can be resumed, newest first (APP §2.8). */
function uploadsOf(
	items: readonly ActivityItem[],
	appId: string,
): readonly AppUploadInput[] {
	const uploads = items
		.filter((item) => UPLOADING.has(item.state))
		.sort((a, b) => b.updatedAt - a.updatedAt)
		.flatMap((item): AppUploadInput[] => {
			const { resume, target } = item;
			if (resume?.type !== "transfer" || resume.projectId !== appId) return [];
			const progress =
				typeof item.progress === "object" ? item.progress : undefined;
			return [
				{
					id: item.id,
					deviceId: target.deviceId,
					...(target.serviceId ? { serviceId: target.serviceId } : {}),
					state: item.state === "paused" ? "paused" : "active",
					done: progress?.done ?? 0,
					total: progress?.total ?? 0,
					expiresAt: resume.expiresAt,
				},
			];
		});
	return uploads.length ? uploads : NO_UPLOADS;
}

export interface AppViewRead {
	/** `undefined` until the app and its events are loaded. */
	view: AppView | undefined;
	app: AppInput | undefined;
	loading: boolean;
	error?: Error;
	/** E20, or the older-hub interim (`missingOnHub`): counts then come from readable devices only. */
	placements: HubRead<AppDevicePlacements>;
	/** What the app's hub can do for its events; nothing is known while the placement list loads. */
	hub: AppHubFacts;
}

/** The record's config as an object; null when it can't be read. */
function configOf(
	event: Pick<IEvent, "config">,
): Record<string, unknown> | null {
	try {
		const config: unknown = event.config?.length
			? parseUint8ArrayToJson(event.config)
			: null;
		return config && typeof config === "object" && !Array.isArray(config)
			? (config as Record<string, unknown>)
			: null;
	} catch {
		return null;
	}
}

/**
 * The fields of an event record the app view reads. The rule reads routes,
 * schedules and bot settings from the config; whether an Endpoint has a
 * token of its own is read here, never the token.
 */
export function eventInput(
	event: IEvent,
	ineligibleReason: string | null,
): AppEventInput {
	const config = configOf(event);
	const schedule =
		event.event_type === CRON_EVENT_TYPE
			? projectScheduleConfig(config)
			: undefined;
	const form = eventFormFacts(
		event.event_type,
		(event.inputs ?? []).map((input) => input.data_type),
	);
	const ownToken = endpointOwnToken(event.event_type, config);
	return {
		id: event.id,
		name: event.name,
		active: event.active,
		event_type: event.event_type,
		canary: event.canary,
		variants: event.variants,
		default_page_id: event.default_page_id,
		event_version: event.event_version,
		board_version: event.board_version,
		boardId: event.board_id,
		config: event.config,
		...(schedule ? { schedule } : {}),
		...(ownToken === undefined ? {} : { ownToken }),
		...(form ? { form } : {}),
		ineligibleReason,
	};
}

/**
 * The flows whose state as a version is read: one per event that follows
 * Latest and that either can be deployed or is served somewhere. Sorted, so
 * the same events give the same list.
 */
export function latestBoards(
	rows: readonly AppEventInput[] | undefined,
	served: ReadonlySet<string>,
): string[] {
	const boards = new Set<string>();
	for (const row of rows ?? []) {
		const rule = appEventRule(row);
		if (!row.boardId || !rule.followsLatest) continue;
		if (rule.eligible || served.has(row.id)) boards.add(row.boardId);
	}
	return [...boards].sort();
}

/**
 * App › Devices: where and how one app runs (APP §2), from the app's metadata
 * and events, the hub's placement list and what each device's planes can say.
 */
export function useAppView(
	appId: string | undefined,
	extras: AppViewExtras = {},
): AppViewRead {
	const backend = useBackend();
	const state = useAttentionState();
	const { input, items } = state;
	const enabled = !!appId;
	const id = appId ?? "";
	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[id],
		enabled,
	);
	const meta = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[id],
		enabled,
	);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[id],
		enabled,
	);
	const placements = useAppPlacements(appId);
	const { versions, changes, focusDeviceIds, refusals, ineligible } = extras;
	const activity = useActivity().items;
	const uploads = useMemo(
		() => (appId ? uploadsOf(activity, appId) : NO_UPLOADS),
		[activity, appId],
	);
	// What a shared device's access covers decides "no access" against "unknown until unlocked": the hub says it per device.
	const sharedIds = useMemo(
		() =>
			enabled
				? input.devices
						.filter(
							(row) => row.status === "active" && row.relationship === "shared",
						)
						.map((row) => row.device_id)
				: NO_DEVICES,
		[enabled, input.devices],
	);
	useQueries({
		queries: sharedIds.map((deviceId) =>
			queries.myAccess(state.workspace.hub, deviceId),
		),
	});
	const noAccess = useMemo(
		() => (appId ? coverage(input, appId).noAccess : NO_DEVICES),
		[input, appId],
	);

	const records = useMemo<AppEventInput[] | undefined>(
		() =>
			events.data?.map((event) =>
				eventInput(event, ineligible?.[event.id] ?? null),
			),
		[events.data, ineligible],
	);
	const mode = app.data
		? appMode(app.data.visibility as AppVisibility)
		: undefined;
	const services = useMemo(
		() => (appId ? appServices(input, appId) : NO_SERVICES),
		[input, appId],
	);
	// One string, so the list keeps its identity while the same flows are read.
	const boardKey = useMemo(() => {
		const served = new Set(
			services.flatMap((service) =>
				(service.events ?? []).map((event) => event.event_id),
			),
		);
		return latestBoards(records, served).join("\n");
	}, [records, services]);
	const boards = useMemo(
		() => (boardKey ? boardKey.split("\n") : NO_DEVICES),
		[boardKey],
	);
	const flows = useFlowStates(appId, mode, boards);
	const rows = useMemo<AppEventInput[] | undefined>(
		() =>
			records?.map((row) => {
				const flow = row.boardId ? flows[row.boardId] : undefined;
				return flow !== undefined && row.board_version == null
					? { ...row, flow }
					: row;
			}),
		[records, flows],
	);
	const placementsMissing = placements.missingOnHub;
	const hub = useMemo<AppHubFacts>(
		() =>
			mode
				? appHubFacts(mode, {
						placements: placements.data,
						...(placementsMissing ? { placementsOnHub: false } : {}),
					})
				: {},
		[mode, placements.data, placementsMissing],
	);
	const label = app.data?.version ?? null;
	const changedAt = changedSeconds(app.data?.updated_at);
	// The hub keeps no version history: what the app pins now, plus what readable services still run.
	const known = useMemo(() => {
		if (!appId || !rows) return NO_VERSIONS;
		return versionInputs({
			label,
			changedAt,
			events: rows,
			services,
			sentAt: revisionsSent(activity, appId),
			hub,
		});
	}, [appId, rows, label, changedAt, services, activity, hub]);

	const appInput = useMemo<AppInput | undefined>(() => {
		if (!appId || !app.data || !rows) return undefined;
		return {
			id: appId,
			name: meta.data?.name ?? appId,
			visibility: app.data.visibility as AppVisibility,
			versions: versions === undefined ? known : versions,
			events: rows,
		};
	}, [appId, app.data, rows, meta.data?.name, versions, known]);

	const devices = useMemo<AppDeviceInput[]>(() => {
		if (!appId) return [];
		return input.devices.flatMap((row) => {
			const view = buildDeviceView(row.device_id, input, items);
			if (!view) return [];
			const gate = evaluateGate(
				"create_service",
				buildGateContext(state, row.device_id, { projectId: appId }),
			);
			const refused = refusals?.[row.device_id];
			return [
				{
					id: row.device_id,
					name: deviceName(row),
					presence: view.presence,
					relationship: view.relationship,
					keyState: view.keys.state,
					services: view.services,
					...(refused ? { refusals: refused } : {}),
					deployGate: gate.ok ? null : gate,
					...(view.features ? { features: view.features } : {}),
				},
			];
		});
	}, [appId, input, items, state, refusals]);

	const view = useMemo(
		() =>
			appInput
				? refineView(
						buildAppView({
							app: appInput,
							devices,
							placements: placements.data ?? null,
							now: input.now,
							...(placementsMissing ? { placementsOnHub: false } : {}),
							...(changes ? { changes } : {}),
							...(focusDeviceIds ? { focusDeviceIds } : {}),
							...(uploads.length ? { uploads } : {}),
							...(noAccess.length ? { noAccess } : {}),
						}),
					)
				: undefined,
		[
			appInput,
			devices,
			placements.data,
			placementsMissing,
			input.now,
			changes,
			focusDeviceIds,
			uploads,
			noAccess,
		],
	);

	const error = app.error ?? events.error ?? undefined;
	return useMemo(
		() => ({
			view,
			app: appInput,
			loading: enabled && !appInput && !error,
			...(error ? { error } : {}),
			placements,
			hub,
		}),
		[view, appInput, enabled, error, placements, hub],
	);
}
