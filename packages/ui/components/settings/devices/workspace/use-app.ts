"use client";

import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import { queries } from "../../../../lib/device-management/hub/queries";
import {
	type AppDeviceInput,
	type AppEventInput,
	type AppInput,
	type AppUploadInput,
	type AppVersionInput,
	type AppView,
	type AppVisibility,
	type LocalServiceChange,
	buildAppView,
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
import type { AppDevicePlacements } from "../../../../lib/device-management/model/types";
import type {
	ActivityItem,
	ActivityState,
} from "../../../../lib/device-management/workspace/types";
import type { IApp } from "../../../../lib/schema/app/app";
import { useBackend } from "../../../../state/backend-state";
import { useActivity } from "./use-activity";
import { buildGateContext, useAttentionState } from "./use-attention";
import { type HubRead, useAppPlacements } from "./use-hub";

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

	const rows = useMemo<AppEventInput[] | undefined>(
		() =>
			events.data?.map((event) => ({
				id: event.id,
				name: event.name,
				active: event.active,
				event_type: event.event_type,
				canary: event.canary,
				variants: event.variants,
				default_page_id: event.default_page_id,
				event_version: event.event_version,
				board_version: event.board_version,
				ineligibleReason: ineligible?.[event.id] ?? null,
			})),
		[events.data, ineligible],
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
			services: appServices(input, appId),
			sentAt: revisionsSent(activity, appId),
		});
	}, [appId, rows, label, changedAt, input, activity]);

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
		}),
		[view, appInput, enabled, error, placements],
	);
}
