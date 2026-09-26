"use client";

import { useQuery } from "@tanstack/react-query";
import { isRecord } from "../../../../lib/response-shape";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import type { IChainStatusResponse } from "./types";

/** Default of `retention.pending_alert_seconds`, used when the server omits it. */
export const DEFAULT_PENDING_ALERT_SECONDS = 900;
/** Default of `retention.epoch_interval_seconds`, used when the server omits it. */
export const DEFAULT_EPOCH_INTERVAL_SECONDS = 300;
/** Slack for one worker tick before a waiting seal counts as not anchored in time. */
const ANCHOR_GRACE_SECONDS = 60;

/** Shared by the dashboard widget and the explorer so both read one cache entry. */
export function useChainStatus(profile: IProfile | undefined) {
	const backend = useBackend();
	return useQuery({
		queryKey: ["admin", "logs", "chain-status", profile?.hub, profile?.id],
		queryFn: async (): Promise<IChainStatusResponse> => {
			if (!profile) throw new Error("Profile not loaded");
			const status = await backend.apiState.get<IChainStatusResponse>(
				profile,
				"admin/logs/chain-status",
			);
			// Hubs before 2026-09-21 answer this path with the retired AuditEntry summary.
			if (
				isRecord(status) &&
				isRecord(status.epochs) &&
				isRecord(status.platform) &&
				Array.isArray(status.recent_chains) &&
				Array.isArray(status.verifying_kids)
			) {
				return status;
			}
			throw new Error(
				"admin/logs/chain-status returned an unrecognized shape (expected epochs, platform, recent_chains, verifying_kids); the hub may predate sealed audit chains",
			);
		},
		enabled: !!profile,
		staleTime: 60_000,
		refetchInterval: 60_000,
		meta: { adminDashboard: true, persist: false },
	});
}

export function pendingAlertSeconds(status: IChainStatusResponse): number {
	return status.pending_alert_seconds ?? DEFAULT_PENDING_ALERT_SECONDS;
}

export function epochIntervalSeconds(status: IChainStatusResponse): number {
	return status.epoch_interval_seconds ?? DEFAULT_EPOCH_INTERVAL_SECONDS;
}

/** Age after which the oldest unanchored seal means the worker is not anchoring. */
export function anchorAlertSeconds(status: IChainStatusResponse): number {
	return epochIntervalSeconds(status) + ANCHOR_GRACE_SECONDS;
}
