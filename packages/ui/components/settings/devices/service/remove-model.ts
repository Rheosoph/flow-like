import type {
	PlacementStatusPlus,
	ServiceView,
} from "../../../../lib/device-management/model/types";

/* Removing a service (SPEC §5.3 Danger zone): when the device accepts it, and which cloud access can go with it. No React, no I/O. */

const ENDED = new Set(["stopped", "failed"]);

/**
 * The device removes a service only once it is requested stopped, observed
 * stopped (or failed) and none of its processes is left. A row that is gone
 * has nothing left to wait for.
 */
export function readyToRemove(row: PlacementStatusPlus | undefined): boolean {
	if (!row) return true;
	return (
		row.desired_state === "stopped" &&
		ENDED.has(row.observed_state) &&
		row.process_id == null &&
		!(row.replicas ?? []).some((replica) => replica.process_id != null)
	);
}

/**
 * Buffered changes of an online service that haven't reached the cloud and
 * would be lost with it: their count, or `unknown` while buffering may be on
 * and the queues weren't read.
 */
export function waitingChanges(
	online: boolean,
	buffering: boolean,
	writes: ServiceView["offlineWrites"],
): number | "unknown" {
	if (!online) return 0;
	if (typeof writes === "object") return writes.pending;
	return buffering && writes === "not_loaded" ? "unknown" : 0;
}

/** What the hub lists as cloud access of one service, and what of it the viewer may end: one function for every screen. */
export {
	type ServiceCloudAccess,
	serviceCloudAccess,
} from "../../../../lib/device-resources";
