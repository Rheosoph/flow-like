"use client";

import { useCallback, useEffect, useRef } from "react";
import type {
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import { AddModelSheet } from "../models/add-model/add-model-sheet";
import { ModelSettingsSheet } from "../models/settings/model-settings-sheet";
import { devicesHref } from "../routing/devices-href";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { useDevicesRoute } from "../routing/use-devices-route";
import { RunNowSheet } from "../run/run-now-sheet";
import {
	useOptionalDeviceWorkspace,
	useWorkspacePassive,
} from "../workspace/device-workspace-provider";
import {
	type OverlayState,
	type RunNowRequest,
	useOverlayStore,
} from "../workspace/overlay-store";
import { DiagnoseSheet } from "./diagnose-sheet";
import { PlaneSheet } from "./plane-sheet";
import { UnlockSeveralSheet } from "./unlock-several-sheet";
import { UnlockSheet } from "./unlock-sheet";

/** What the host hands every sheet: links are built for `scope`, places open through `onNavigate`. */
export interface OverlaySheetProps {
	scope: DevicesScope;
	onNavigate: (route: DevicesRoute) => void;
	onClose: () => void;
}

export interface AreaOverlaysProps {
	/** Outside the Devices area: the scope links are built for. The account area by default. */
	scope?: DevicesScope;
	/** Outside the Devices area: the page's router, so a link opens without a page load. The browser loads it by default. */
	onNavigate?: (href: string) => void;
}

interface HostProps {
	scopeKey: string;
	scope: DevicesScope;
	onNavigate: (route: DevicesRoute) => void;
}

/** One sheet per run request: another event, service or run starts a fresh one. */
const runNowKey = (request: RunNowRequest) =>
	[
		request.deviceId,
		request.serviceId,
		request.eventId,
		request.operationId ?? "",
	].join("/");

let mountedHosts = 0;

type OverlayStoreState = ReturnType<typeof useOverlayStore.getState>;
const selectClose = (state: OverlayStoreState) => state.close;
const selectOverlay = (state: OverlayStoreState) => state.overlay;

/** A request never outlives its account or the last host: the next page must not reopen it. */
function useOverlayLifetime(scopeKey: string) {
	const close = useOverlayStore(selectClose);
	const shown = useRef(scopeKey);
	useEffect(() => {
		if (shown.current !== scopeKey) close();
		shown.current = scopeKey;
	}, [scopeKey, close]);
	useEffect(() => {
		mountedHosts += 1;
		return () => {
			mountedHosts -= 1;
			queueMicrotask(() => {
				if (mountedHosts === 0) useOverlayStore.getState().close();
			});
		};
	}, []);
	return close;
}

function OverlayHost({ scopeKey, scope, onNavigate }: Readonly<HostProps>) {
	const overlay = useOverlayStore(selectOverlay);
	const onClose = useOverlayLifetime(scopeKey);
	return sheetFor(overlay, { scope, onNavigate, onClose });
}

type ModelOverlay = Extract<
	OverlayState,
	{ kind: "model_add" | "model_settings" }
>;

const isModelOverlay = (overlay: OverlayState): overlay is ModelOverlay =>
	overlay.kind === "model_add" || overlay.kind === "model_settings";

/** Models tab › Add model and a model's settings. */
function modelSheetFor(overlay: ModelOverlay, sheet: OverlaySheetProps) {
	if (overlay.kind === "model_add")
		return (
			<AddModelSheet
				key={overlay.deviceId}
				deviceId={overlay.deviceId}
				{...sheet}
			/>
		);
	return (
		<ModelSettingsSheet
			key={`${overlay.deviceId}/${overlay.modelId}`}
			deviceId={overlay.deviceId}
			modelId={overlay.modelId}
			{...sheet}
		/>
	);
}

/** The sheet the store asks for, keyed so another request starts it fresh. */
function sheetFor(overlay: OverlayState, sheet: OverlaySheetProps) {
	if (isModelOverlay(overlay)) return modelSheetFor(overlay, sheet);
	switch (overlay.kind) {
		case "unlock":
			return (
				<UnlockSheet
					key={overlay.deviceId}
					deviceId={overlay.deviceId}
					connectLive={overlay.connectLive}
					returnTo={overlay.returnTo}
					forModels={overlay.forModels}
					{...sheet}
				/>
			);
		case "unlock_several":
			return <UnlockSeveralSheet {...sheet} />;
		case "diagnose":
			return (
				<DiagnoseSheet
					key={`${overlay.deviceId}/${overlay.serviceId ?? ""}`}
					deviceId={overlay.deviceId}
					serviceId={overlay.serviceId}
					{...sheet}
				/>
			);
		case "plane":
			return (
				<PlaneSheet key={overlay.plane} plane={overlay.plane} {...sheet} />
			);
		case "run_now":
			return <RunNowSheet key={runNowKey(overlay)} {...overlay} {...sheet} />;
		case "none":
			return null;
	}
}

function AreaHost({ scopeKey }: Readonly<{ scopeKey: string }>) {
	const { scope, navigate } = useDevicesRoute();
	return (
		<OverlayHost scopeKey={scopeKey} scope={scope} onNavigate={navigate} />
	);
}

/** A page that only mounts the workspace (Events): no area route, so places open as links. */
function PageHost({
	scopeKey,
	scope = ACCOUNT_SCOPE,
	onNavigate,
}: Readonly<AreaOverlaysProps & { scopeKey: string }>) {
	const navigate = useCallback(
		(route: DevicesRoute) => {
			const href = devicesHref(route, scope);
			if (onNavigate) onNavigate(href);
			else globalThis.location?.assign(href);
		},
		[scope, onNavigate],
	);
	return (
		<OverlayHost scopeKey={scopeKey} scope={scope} onNavigate={navigate} />
	);
}

/**
 * Renders the sheet the overlay store asks for (IA §6.4): Unlock, Unlock
 * several, Diagnose, the data sources sheet and Run now. Mounted once by `DevicesArea`;
 * a page outside the area mounts it under a passive `DeviceWorkspaceProvider`.
 * Without a workspace (signed out, no profile) it renders nothing and the
 * request waits.
 */
export function AreaOverlays(props: Readonly<AreaOverlaysProps>) {
	const workspace = useOptionalDeviceWorkspace();
	const passive = useWorkspacePassive();
	if (!workspace) return null;
	if (passive) return <PageHost scopeKey={workspace.scopeKey} {...props} />;
	return <AreaHost scopeKey={workspace.scopeKey} />;
}
