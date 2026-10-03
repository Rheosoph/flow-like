"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo, useState } from "react";
import type {
	DeviceLimits,
	DeviceUsage,
} from "../../../../lib/device-management/model/types";
import type { VerifiedRelease } from "../../../../lib/device-management/package";
import { useDeviceRows, useHubSupport, usePendingSetups } from "../workspace";
import type { SetupLimits } from "./setup-context";
import {
	type SetupDraft,
	type TargetOption,
	targetOptions,
} from "./setup-state";
import type { ReleaseCheck, SetupChecks } from "./use-setup-checks";

const DAY_S = 86_400;

/** The stepper's labels: Check · Name · Platform · Password · Create · Save · Start · Waiting. */
export function useStepLabels(): string[] {
	const { t } = useTranslation("devices");
	return useMemo(
		() => [
			t("setup.steps.check", "Check"),
			t("setup.steps.name", "Name"),
			t("setup.steps.platform", "Platform"),
			t("setup.steps.password", "Password"),
			t("setup.steps.create", "Create"),
			t("setup.steps.save", "Save"),
			t("setup.steps.start", "Start"),
			t("setup.steps.waiting", "Waiting"),
		],
		[t],
	);
}

/** What the hub states about its limits and this account's use of them; older hubs state less (BG3 interim). */
export function useSetupLimits(): SetupLimits {
	const { support } = useHubSupport();
	return useMemo(() => {
		const stated: Partial<DeviceLimits> = {
			...support.configuredLimits,
			...support.limits,
		};
		const usage: Partial<DeviceUsage> = support.usage ?? {};
		return {
			lifetimeS: stated.enrollment_ttl_seconds ?? DAY_S,
			maxDevices: stated.max_devices,
			maxPending: stated.max_pending_enrollments,
			maxPerDay: stated.max_enrollments_per_day,
			activeDevices: usage.active_devices,
			pending: usage.pending_enrollments,
			lastDay: usage.enrollments_last_24h,
		};
	}, [support]);
}

/** Names already in use, without this setup's own: the waiting setups first (the newest one suggests the next name), then the devices. */
export function useTakenNames(draft: SetupDraft): string[] {
	const { rows } = useDeviceRows();
	const pendingSetups = usePendingSetups();
	const ownDevice = draft.created?.deviceId;
	const ownSetup = draft.created?.enrollmentId;
	return useMemo(
		() => [
			...pendingSetups
				.filter(
					(setup) =>
						setup.state === "pending" && setup.enrollmentId !== ownSetup,
				)
				.map((setup) => setup.name),
			// A renamed device still carries its permanent name; a new one may clash with either.
			...(rows ?? [])
				.filter((row) => row.device_id !== ownDevice)
				.flatMap((row) =>
					row.display_name && row.display_name !== row.name
						? [row.display_name, row.name]
						: [row.name],
				),
		],
		[rows, pendingSetups, ownDevice, ownSetup],
	);
}

export interface KnownRelease {
	release: VerifiedRelease | undefined;
	options: TargetOption[];
}

/** A release in one of these states is not shown any more, not even the last verified one. */
const GONE = new Set<ReleaseCheck["state"]>(["rejected", "missing"]);

/** The verified release for display: it stays while a re-check runs, and goes once the release is rejected or missing. */
export function useKnownRelease(checks: SetupChecks): KnownRelease {
	const [last, setLast] = useState<VerifiedRelease>();
	const kept = GONE.has(checks.release.state) ? undefined : last;
	const release = checks.verified ?? kept;
	if (release !== last) setLast(release);
	const options = useMemo(() => targetOptions(release?.manifest), [release]);
	return { release, options };
}
