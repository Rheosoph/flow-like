"use client";

import { useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import {
	type AppDeviceInput,
	type AppEventInput,
	type AppInput,
	type AppVersionInput,
	type AppView,
	type AppVisibility,
	type LocalServiceChange,
	buildAppView,
} from "../../../../lib/device-management/model/app-plan";
import { buildDeviceView } from "../../../../lib/device-management/model/attention";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import { evaluateGate } from "../../../../lib/device-management/model/gates";
import type { AppDevicePlacements } from "../../../../lib/device-management/model/types";
import { useBackend } from "../../../../state/backend-state";
import { buildGateContext, useAttentionState } from "./use-attention";
import { type HubRead, useAppPlacements } from "./use-hub";

/** Facts the app screen computes itself and adds to the base view. */
export interface AppViewExtras {
	/** Newest first; `null` when the viewer's role can't read flows. Defaults to none known. */
	versions?: readonly AppVersionInput[] | null;
	/** What this computer recorded per `${deviceId}/${serviceId}` (BG12 interim). */
	changes?: Readonly<Record<string, LocalServiceChange>>;
	focusDeviceIds?: readonly string[];
	/** Device-side refusals: device id → event id → reason. */
	refusals?: Readonly<Record<string, Readonly<Record<string, string>>>>;
	/** Why the approved bundle could not prepare an event: event id → reason. */
	ineligible?: Readonly<Record<string, string>>;
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

	const appInput = useMemo<AppInput | undefined>(() => {
		if (!appId || !app.data || !events.data) return undefined;
		const rows: AppEventInput[] = events.data.map((event) => ({
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
		}));
		return {
			id: appId,
			name: meta.data?.name ?? appId,
			visibility: app.data.visibility as AppVisibility,
			versions: versions === undefined ? [] : versions,
			events: rows,
		};
	}, [appId, app.data, events.data, meta.data?.name, versions, ineligible]);

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
				? buildAppView({
						app: appInput,
						devices,
						placements: placements.data ?? null,
						...(changes ? { changes } : {}),
						...(focusDeviceIds ? { focusDeviceIds } : {}),
					})
				: undefined,
		[appInput, devices, placements.data, changes, focusDeviceIds],
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
