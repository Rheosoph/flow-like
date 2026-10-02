"use client";

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { userLookupQueryOptions } from "../../../../hooks/use-user-lookup";
import { ATTENTION_KEY_GROUPS } from "../../../../lib/device-management/model/attention";
import {
	type ServicesUnavailable,
	deviceListKnown,
	deviceName,
	fleetFacts,
	keysLocked,
} from "../../../../lib/device-management/model/device-view";
import {
	type AttentionItem,
	type AttentionKey,
	DEVICE_TABS,
	type DeviceTab,
	type DeviceViewModel,
	type DevicesScope,
	type FixAction,
	type Freshness,
	type GateResult,
	type GrantedCapabilities,
	type InspectionPlus,
	type ServiceView,
} from "../../../../lib/device-management/model/types";
import type { Capability } from "../../../../lib/device-management/types";
import type {
	KeyError,
	ServiceSummary,
} from "../../../../lib/device-management/workspace/types";
import { userDisplayName } from "../../../../lib/user-display";
import { useBackend } from "../../../../state/backend-state";
import { gateCopy } from "../copy/gate-copy";
import type { AreaTime, DevicesT } from "../primitives/area-context";
import type { Gate } from "../primitives/gate-notice";
import type { KeyChipState } from "../primitives/status-chip";
import type { TabCountTone } from "../primitives/underline-tabs";
import { useAppNames } from "../shell/attention-popover";
import { keyChipOf } from "../shell/keys-popover";
import {
	buildGateContext,
	useAttention,
	useAttentionState,
	useDeviceView,
} from "../workspace";

export interface DeviceAppContext {
	id: string;
	name: string;
}

export type IdentityMismatch = Extract<KeyError, { code: "identity_mismatch" }>;

/** One device as the N2 page reads it: the view model plus what only this page derives. */
export interface DevicePage {
	deviceId: string;
	scope: DevicesScope;
	view: DeviceViewModel;
	name: string;
	owner: boolean;
	revoked: boolean;
	/** The viewer only approves or pays for cloud access: no access to the device itself. */
	consentOnly: boolean;
	/** Set when the page is opened from an app's settings. */
	app: DeviceAppContext | null;
	/** The last live read; gone when the keys lock. */
	inspection: InspectionPlus | undefined;
	inspectionSource: Freshness | undefined;
	/** A live session is open: live facts are current. */
	liveOpen: boolean;
	/** Readable rows of every app; `null` while they can't be read. */
	services: ServiceView[] | null;
	unavailable: ServicesUnavailable | null;
	/** What was read before the keys were locked, shown greyed. */
	lockedRows: { readAt: number; services: ServiceSummary[] } | null;
	/** Undefined while the viewer's permissions aren't known (older hub, locked). */
	capabilities: readonly GrantedCapabilities[] | undefined;
	/** The hub reports other keys than the ones trusted on this computer: no commands, deploys or access changes. */
	identity: IdentityMismatch | null;
	tabs: readonly DeviceTab[];
	keyChip: {
		state: KeyChipState;
		transport?: "direct" | "relayed";
		renewsAt?: number;
	};
	/** Open items of the whole device, most urgent first. */
	attention: AttentionItem[];
	/** In an app: only the items about that app. Otherwise the same as `attention`. */
	appAttention: AttentionItem[];
}

export type DevicePageRead =
	| { state: "ready"; page: DevicePage }
	| { state: "loading" }
	| { state: "unknown" };

const CONSENT_TABS: readonly DeviceTab[] = ["overview", "access"];
const REVOKED_TABS: readonly DeviceTab[] = ["overview", "access", "keys"];

/** SPEC §5.2 tab visibility: a revoked device isn't read, a consent-only viewer sees cloud approvals only. */
export function allowedTabs(
	view: Pick<DeviceViewModel, "relationship" | "row">,
): readonly DeviceTab[] {
	if (view.relationship === "cloud_approval") return CONSENT_TABS;
	if (view.row.status === "revoked") return REVOKED_TABS;
	return DEVICE_TABS;
}

const SETTINGS_KEYS = new Set<AttentionKey>([
	"agent_update_available",
	"status_subscription_expiring",
	"rebooted_unexpectedly",
]);

/** The tab that resolves an open item (its badge). */
export function tabOfAttention(key: AttentionKey): DeviceTab {
	if (SETTINGS_KEYS.has(key)) return "settings";
	if (key === "upload_paused") return "services";
	const tabs = {
		device: "overview",
		setup: "overview",
		keys: "keys",
		access: "access",
		cloud: "access",
		services: "services",
		offline_writes: "services",
		certificates: "certificates",
		history: "activity",
		operations: "activity",
	} as const;
	return tabs[ATTENTION_KEY_GROUPS[key]];
}

export function tabCount(
	items: readonly AttentionItem[],
): { count: number; tone: TabCountTone } | undefined {
	const counted = items.filter((item) => item.severity !== "info");
	if (!counted.length) return undefined;
	const tone = counted.some((item) => item.severity === "critical")
		? "critical"
		: counted.some((item) => item.severity === "warning")
			? "warning"
			: "info";
	return { count: counted.length, tone };
}

/** The capabilities that cover `appId` (or the whole device when no app is named). */
export function capabilitiesFor(
	page: Pick<DevicePage, "owner" | "capabilities">,
	appId?: string,
): readonly Capability[] | undefined {
	if (!page.capabilities) return undefined;
	const caps = new Set<Capability>();
	for (const grant of page.capabilities) {
		const covers =
			grant.scope.kind === "device" ||
			(appId !== undefined && grant.scope.project_id === appId);
		if (covers) for (const cap of grant.caps) caps.add(cap);
	}
	return [...caps];
}

/** The app a scoped viewer is limited to; `null` for whole-device access or unknown permissions. */
export function scopedApp(
	page: Pick<DevicePage, "owner" | "capabilities">,
): string | null {
	if (page.owner || !page.capabilities?.length) return null;
	if (page.capabilities.some((grant) => grant.scope.kind === "device"))
		return null;
	const apps = new Set(
		page.capabilities.flatMap((grant) =>
			grant.scope.kind === "device" ? [] : [grant.scope.project_id],
		),
	);
	return apps.size === 1 ? ([...apps][0] ?? null) : null;
}

/** "Linux", "macOS": the system the agent reports; never the wire value (R3). */
export function platformLabel(t: DevicesT, platform: string): string {
	const labels: Record<string, string> = {
		linux: t("devices:device.platform.linux", "Linux"),
		macos: t("devices:device.platform.macos", "macOS"),
		windows: t("devices:device.platform.windows", "Windows"),
	};
	return (
		labels[platform.toLowerCase()] ??
		t("devices:device.platform.other", "Another system")
	);
}

/** How many other people the signed access rules name; undefined while the rules can't be read here. */
export function sharedPeople(
	policy: { grants: readonly { user_id: string }[] } | undefined,
	me: string | null | undefined,
): number | undefined {
	if (!policy) return undefined;
	const others = policy.grants
		.map((grant) => grant.user_id)
		.filter((user) => user !== me);
	return new Set(others).size;
}

/** Services of one app; every service when no app is given. */
export function servicesOfApp<T extends { projectId: string }>(
	services: readonly T[],
	appId: string | null | undefined,
): T[] {
	return appId
		? services.filter((service) => service.projectId === appId)
		: [...services];
}

/**
 * Everything the device page reads in one place, so header, verdict, tabs and
 * panels agree. `unknown`: the hub list is loaded and doesn't have the device.
 */
export function useDevicePage(
	deviceId: string,
	scope: DevicesScope,
): DevicePageRead {
	const view = useDeviceView(deviceId);
	const state = useAttentionState();
	const appName = useAppNames();
	const appId = scope.kind === "app" ? scope.appId : undefined;
	const attention = useAttention({ deviceId });
	const scoped = useAttention({ deviceId, appId });
	const { input } = state;

	return useMemo<DevicePageRead>(() => {
		if (!view) return { state: deviceListKnown(input) ? "unknown" : "loading" };
		const facts = fleetFacts(input).byId.get(deviceId);
		const services = Array.isArray(view.services) ? view.services : null;
		const chip = keyChipOf(view.keys, view.live);
		const renewsAt =
			view.live.kind === "live" && view.live.transport === "webrtc"
				? view.live.expiresAt
				: undefined;
		const lastError = view.keys.lastError;
		const locked = keysLocked(view.keys) ? view.keys.lockedSummary : undefined;
		const page: DevicePage = {
			deviceId,
			scope,
			view,
			name: deviceName(view.row),
			owner: view.relationship === "owner",
			revoked: view.row.status === "revoked",
			consentOnly: view.relationship === "cloud_approval",
			app: appId ? { id: appId, name: appName(appId) ?? appId } : null,
			inspection: facts?.inspection,
			inspectionSource: facts?.inspectionSource,
			liveOpen: facts?.liveOpen ?? false,
			services,
			unavailable: Array.isArray(view.services) ? null : view.services,
			lockedRows: locked ?? null,
			capabilities: buildGateContext(state, deviceId).capabilities,
			identity:
				lastError?.code === "identity_mismatch" && view.keys.state === "blocked"
					? lastError
					: null,
			tabs: allowedTabs(view),
			keyChip: {
				...chip,
				...(renewsAt === undefined ? {} : { renewsAt }),
			},
			attention,
			appAttention: appId ? scoped : attention,
		};
		return { state: "ready", page };
	}, [view, input, state, deviceId, scope, appId, appName, attention, scoped]);
}

/** A failed gate as the `GatedAction` primitive takes it, with its fix when there is one. */
export interface GateView {
	gate: Gate;
	fix?: { label: string; action: FixAction };
}

export function gateView(
	t: DevicesT,
	time: Pick<AreaTime, "at" | "locale">,
	result: GateResult,
): GateView | null {
	if (result.ok) return null;
	const copy = gateCopy(t, result, { at: time.at, locale: time.locale });
	return {
		gate: { kind: result.kind, reason: copy.inline },
		...(result.fix && copy.fix
			? { fix: { label: copy.fix, action: result.fix } }
			: {}),
	};
}

/**
 * A person's display name; undefined while it isn't known (the caller falls
 * back to a neutral phrase). Never the account id or a sign-in provider's
 * handle: `userDisplayName` decides what counts as a name.
 */
export function usePersonName(
	userId: string | undefined,
	enabled = true,
): string | undefined {
	const backend = useBackend();
	const query = useQuery({
		...userLookupQueryOptions(backend.userState, enabled ? userId : undefined),
		retry: false,
	});
	return query.data ? userDisplayName(query.data, "") || undefined : undefined;
}
