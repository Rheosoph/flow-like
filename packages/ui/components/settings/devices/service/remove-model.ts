import type {
	PlacementStatusPlus,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { DeviceResources } from "../../../../lib/device-resources";

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

export interface ServiceCloudAccess {
	/**
	 * `listed`: the hub lists cloud access that still runs for this service.
	 * `none`: the viewer owns the device, so the list is complete, and it has none.
	 * `hidden`: nothing is listed on someone else's device, where approvals of other people aren't shown.
	 * `unknown`: the list wasn't read.
	 */
	state: "listed" | "none" | "hidden" | "unknown";
	/** Approvals the viewer may revoke: every one on a device they own, else the ones they gave. */
	approvals: string[];
	/** Spending limits of those approvals that the viewer pays. */
	limits: string[];
}

const without = (state: ServiceCloudAccess["state"]): ServiceCloudAccess => ({
	state,
	approvals: [],
	limits: [],
});

/** What the hub lists as cloud access of one service, and what of it the viewer may end. */
export function serviceCloudAccess(
	resources: DeviceResources | undefined,
	serviceId: string,
	viewer: { id: string; owner: boolean },
	nowS: number,
): ServiceCloudAccess {
	if (!resources) return without("unknown");
	const running = resources.grants.filter(
		(grant) =>
			grant.placement_id === serviceId &&
			grant.status === "active" &&
			(grant.effective_expires_at ?? grant.expires_at) > nowS,
	);
	if (!running.length) return without(viewer.owner ? "none" : "hidden");
	const ids = new Set(running.map((grant) => grant.grant_id));
	return {
		state: "listed",
		approvals: running
			.filter((grant) => viewer.owner || grant.delegating_user_id === viewer.id)
			.map((grant) => grant.grant_id),
		limits: resources.billing
			.filter(
				(limit) =>
					limit.status === "active" &&
					limit.payer_id === viewer.id &&
					ids.has(limit.grant_id),
			)
			.map((limit) => limit.billing_grant_id),
	};
}
