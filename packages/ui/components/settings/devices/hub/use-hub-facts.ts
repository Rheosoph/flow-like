"use client";

import { useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { getApiOrigin } from "../../../../lib/api-url";
import {
	type HubError,
	type HubStandalone,
	toHubError,
} from "../../../../lib/device-management/hub/endpoints";
import {
	queries,
	releaseConfigOf,
} from "../../../../lib/device-management/hub/queries";
import type { HubDeviceSupport } from "../../../../lib/device-management/model/types";
import type { DeviceSetupReadiness } from "../../../../lib/device-management/readiness";
import type { VerifiedRelease } from "../../../../lib/device-package";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";

export type CheckId = DeviceSetupReadiness["checks"][number]["id"];

export const CHECK_IDS: readonly CheckId[] = [
	"policy",
	"signing",
	"api",
	"signaling",
	"release",
	"database",
];

export interface CheckSummary {
	total: number;
	passed: number;
	failing: CheckId[];
}

export function summarizeChecks(
	readiness: DeviceSetupReadiness | undefined,
): CheckSummary | undefined {
	if (!readiness) return undefined;
	const failing = readiness.checks
		.filter((check) => !check.ready)
		.map((check) => check.id);
	const total = readiness.checks.length;
	return { total, passed: total - failing.length, failing };
}

/** What the hub enforces per account, as far as it says so. */
export interface HubLimits {
	devices?: number;
	pending?: number;
	lifetimeS?: number;
	perDay?: number;
}

/** The hub's own limits when it reports usage, else what its public record states (BG3 interim). */
export function limitsOf(support: HubDeviceSupport): HubLimits {
	const reported = support.limits;
	const stated = reported ?? support.configuredLimits ?? {};
	const devices = stated.max_devices;
	const pending = stated.max_pending_enrollments;
	const perDay =
		reported?.max_enrollments_per_day ??
		(devices === undefined || pending === undefined
			? undefined
			: 2 * (devices + pending));
	return {
		...(devices === undefined ? {} : { devices }),
		...(pending === undefined ? {} : { pending }),
		...(stated.enrollment_ttl_seconds === undefined
			? {}
			: { lifetimeS: stated.enrollment_ttl_seconds }),
		...(perDay === undefined ? {} : { perDay }),
	};
}

/** Device slots: registered devices and setup packages nobody started yet both hold one. */
export interface DeviceSlots {
	used: number;
	max: number;
	pending: number;
	/** More in use than the hub allows now (the limit was lowered). */
	over: boolean;
	/** No room for another device. */
	full: boolean;
	near: boolean;
}

export function slotsOf(support: HubDeviceSupport): DeviceSlots | undefined {
	const { limits, usage } = support;
	if (!limits || !usage) return undefined;
	const used = usage.active_devices + usage.pending_enrollments;
	const max = limits.max_devices;
	return {
		used,
		max,
		pending: usage.pending_enrollments,
		over: used > max,
		full: used >= max,
		near: used >= Math.ceil(max * 0.9) && used < max,
	};
}

/** Meter tone of a count against its limit. */
export function limitTone(
	used: number,
	max: number,
): "critical" | "warning" | "neutral" {
	if (used > max) return "critical";
	return used >= Math.ceil(max * 0.9) ? "warning" : "neutral";
}

const BYTE_UNITS = ["B", "KiB", "MiB", "GiB", "TiB"];

/** "179 MiB", "1 GiB", "87.4 MiB". */
export function bytesText(locale: string, bytes: number): string {
	let value = Math.max(0, bytes);
	let unit = 0;
	while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
		value /= 1024;
		unit += 1;
	}
	const digits = Number.isInteger(value) || value >= 100 ? 0 : 1;
	const amount = new Intl.NumberFormat(locale, {
		maximumFractionDigits: digits,
	}).format(value);
	return `${amount} ${BYTE_UNITS[unit]}`;
}

/** The hub's public record (`GET /api/v1`), typed: limits, API address, release trust, history tiers. */
export function useHubRecord(): {
	record: HubStandalone | undefined;
	origin: string;
} {
	const workspace = useDeviceWorkspace();
	const { data } = useQuery(queries.hub(workspace.hub));
	return { record: data, origin: getApiOrigin(workspace.deps.profile) };
}

interface Refetched<T> {
	isError: boolean;
	data: T | undefined;
	error: unknown;
}

/** What one asked-for read came back with; `at` is unix seconds. */
export type RefetchOutcome<T> =
	| { ok: true; at: number; data: T }
	| { ok: false; at: number; error: HubError };

export interface TrackedRefetch<T> {
	busy: boolean;
	/** The outcome of the last run, until dismissed (R9). */
	outcome?: RefetchOutcome<T>;
	run(): Promise<void>;
	dismiss(): void;
}

/**
 * A read the viewer asked for, with a visible result. A completion that
 * arrives after the component unmounted or the account changed is dropped.
 */
function useTrackedRefetch<T>(
	refetch: () => Promise<Refetched<T>>,
): TrackedRefetch<T> {
	const workspace = useDeviceWorkspace();
	const scope = workspace.scopeKey;
	const [running, setRunning] = useState<string>();
	const [last, setLast] = useState<{
		scope: string;
		outcome: RefetchOutcome<T>;
	}>();
	const live = useRef({ scope, mounted: true });
	useEffect(() => {
		live.current = { scope, mounted: true };
		return () => {
			live.current.mounted = false;
		};
	}, [scope]);

	const run = useCallback(async () => {
		setRunning(scope);
		setLast(undefined);
		const answer = await refetch();
		if (!live.current.mounted || live.current.scope !== scope) return;
		setRunning(undefined);
		const at = Math.floor(workspace.clock.now() / 1000);
		setLast({
			scope,
			outcome:
				answer.isError || answer.data === undefined
					? { ok: false, at, error: toHubError(answer.error) }
					: { ok: true, at, data: answer.data },
		});
	}, [scope, refetch, workspace]);
	const dismiss = useCallback(() => setLast(undefined), []);

	return {
		busy: running === scope,
		...(last?.scope === scope ? { outcome: last.outcome } : {}),
		run,
		dismiss,
	};
}

/** "Check again": the hub's readiness checks, read now. */
export function useRecheck(): TrackedRefetch<DeviceSetupReadiness> {
	const { hub } = useDeviceWorkspace();
	const { refetch } = useQuery(queries.readiness(hub));
	return useTrackedRefetch(refetch);
}

/** "Verify again": the signed release list, fetched and verified now. */
export function useReverify(): TrackedRefetch<VerifiedRelease> {
	const { hub } = useDeviceWorkspace();
	const { data } = useQuery(queries.hub(hub));
	const { refetch } = useQuery(queries.release(hub, releaseConfigOf(data)));
	return useTrackedRefetch(refetch);
}
