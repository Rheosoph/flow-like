"use client";

import type { HubStandalone } from "../../../../lib/device-management/hub/endpoints";
import type {
	ArchiveUsage,
	DeviceUsageResponse,
	GateResult,
} from "../../../../lib/device-management/model/types";
import type { DeviceSetupReadiness } from "../../../../lib/device-management/readiness";
import {
	type HubRead,
	type HubSupportRead,
	type ReleaseTrustRead,
	useArchiveUsage,
	useDeviceUsage,
	useDeviceWorkspace,
	useGate,
	useHubSupport,
	useReadiness,
	useReleaseTrust,
} from "../workspace";
import {
	type CheckId,
	type CheckSummary,
	type DeviceSlots,
	type HubLimits,
	limitsOf,
	slotsOf,
	summarizeChecks,
	useHubRecord,
} from "./use-hub-facts";

export const DAY = 86_400;

export type ReleaseState =
	| "waiting"
	| "missing"
	| "verifying"
	| "verified"
	| "failed";

/** Everything the Hub status page reads, gathered once and handed to its blocks. */
export interface HubView {
	hub: HubSupportRead;
	host: string;
	origin: string;
	record?: HubStandalone;
	readiness: HubRead<DeviceSetupReadiness>;
	summary?: CheckSummary;
	release: ReleaseTrustRead;
	releaseState: ReleaseState;
	archive: HubRead<ArchiveUsage>;
	usage: HubRead<DeviceUsageResponse>;
	limits: HubLimits;
	slots?: DeviceSlots;
	/** Why "Set up a device" can't start now; `ok` when it can. */
	setup: GateResult;
}

const releaseStateOf = (
	record: HubStandalone | undefined,
	release: ReleaseTrustRead,
	nowS: number,
) => {
	if (!record) return "waiting";
	if (!record.release_trust) return "missing";
	if (release.data)
		return release.data.manifest.expires_at > nowS ? "verified" : "failed";
	return release.error ? "failed" : "verifying";
};

/**
 * Reads the clock when the hub's data changes instead of following its tick, so
 * the page as a whole doesn't re-render every second; the parts that show a
 * running time follow the clock themselves.
 */
export function useHubView(): HubView {
	const workspace = useDeviceWorkspace();
	const nowS = Math.floor(workspace.clock.now() / 1000);
	const hub = useHubSupport();
	const { record, origin } = useHubRecord();
	const readiness = useReadiness();
	const release = useReleaseTrust();
	const archive = useArchiveUsage();
	const usage = useDeviceUsage();
	const summary = summarizeChecks(readiness.data);
	const slots = slotsOf(hub.support);
	const setup = useGate("setup_device", undefined, {
		extra: {
			...(readiness.data ? { readinessOk: readiness.data.ready } : {}),
			...(record ? { releaseTrust: !!record.release_trust } : {}),
		},
	});
	return {
		hub,
		host: hub.host,
		origin,
		...(record ? { record } : {}),
		readiness,
		...(summary ? { summary } : {}),
		release,
		releaseState: releaseStateOf(record, release, nowS),
		archive,
		usage,
		limits: limitsOf(hub.support),
		...(slots ? { slots } : {}),
		setup,
	};
}

export const firstFailing = (view: HubView): CheckId | undefined =>
	view.summary?.failing[0];

export const failsCheck = (view: HubView, id: CheckId) =>
	view.summary?.failing.includes(id) ?? false;

/** Scrolls a block of the page into view and moves focus to its heading. */
export function jumpTo(id: string) {
	const block = globalThis.document?.getElementById(id);
	if (!block) return;
	block.scrollIntoView?.({ behavior: "smooth", block: "start" });
	const heading = block.querySelector<HTMLElement>("h2");
	if (!heading) return;
	heading.tabIndex = -1;
	heading.focus({ preventScroll: true });
}

/** Hub plan keys are names the operator chose ("PRO"); shown as written in a sentence ("Pro"). */
export const planName = (key: string) => {
	const lower = key.toLowerCase().replaceAll("_", " ");
	return lower.charAt(0).toUpperCase() + lower.slice(1);
};
