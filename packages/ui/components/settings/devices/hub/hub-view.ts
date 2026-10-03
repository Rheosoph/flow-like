"use client";

import type { HubStandalone } from "../../../../lib/device-management/hub/endpoints";
import type {
	ArchiveUsage,
	DeviceUsageResponse,
	GateResult,
} from "../../../../lib/device-management/model/types";
import type { DeviceSetupReadiness } from "../../../../lib/device-management/readiness";
import { useAreaTime } from "../primitives/area-context";
import {
	type HubRead,
	type HubSupportRead,
	type ReleaseTrustRead,
	useArchiveUsage,
	useDeviceUsage,
	useGate,
	useHubSupport,
	useReadiness,
	useReleaseTrust,
} from "../workspace";
import { type ReleaseVerdict, releaseVerdictOf } from "./release-verdict";
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

export {
	DAY,
	RELEASE_ENDS_SOON_S,
	type ReleaseAttempt,
	type ReleaseCheckFailure,
	type ReleaseVerdict,
	type ReleaseVerdictKind,
	daysLeft,
	releaseCheckFailure,
	releaseVerdictOf,
	usableRelease,
} from "./release-verdict";

/** Everything the Hub status page reads, gathered once and handed to its blocks. */
export interface HubView {
	hub: HubSupportRead;
	host: string;
	origin: string;
	record?: HubStandalone;
	readiness: HubRead<DeviceSetupReadiness>;
	summary?: CheckSummary;
	release: ReleaseTrustRead;
	archive: HubRead<ArchiveUsage>;
	usage: HubRead<DeviceUsageResponse>;
	limits: HubLimits;
	slots?: DeviceSlots;
	/** Why "Set up a device" can't start now; `ok` when it can. */
	setup: GateResult;
}

/**
 * The page as a whole doesn't follow the clock's tick, so nothing here depends
 * on the time; the parts that show a running time follow the clock themselves.
 */
export function useHubView(): HubView {
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
		archive,
		usage,
		limits: limitsOf(hub.support),
		...(slots ? { slots } : {}),
		setup,
	};
}

/** The release verdict as the area clock has it now: the headline, the summary cell and the features read this one. */
export function useReleaseVerdict(
	view: Pick<HubView, "record" | "release">,
): ReleaseVerdict {
	const { nowS } = useAreaTime();
	return releaseVerdictOf(view.record, view.release, nowS);
}

/** The same verdict for a screen that doesn't hold the Hub status view, such as a device's Settings tab. */
export function useAgentReleaseVerdict(): ReleaseVerdict {
	const { record } = useHubRecord();
	const release = useReleaseTrust();
	return useReleaseVerdict({ ...(record ? { record } : {}), release });
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
