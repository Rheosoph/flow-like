import type { DeviceRow, Presence, Relationship } from "./types";

/** IA §2.1: Online ≤ 2 min, Late 2–10 min, Offline since … > 10 min. */
export const PRESENCE_ONLINE_S = 120;
export const PRESENCE_LATE_S = 600;

export type PresenceRow = Pick<DeviceRow, "status" | "last_seen_at"> &
	Partial<Pick<DeviceRow, "revoked_at">>;

/** `now` is hub-corrected unix seconds. A heartbeat is not a live connection. */
export function presence(row: PresenceRow, now: number): Presence {
	if (row.status === "revoked")
		return row.revoked_at == null
			? { kind: "revoked" }
			: { kind: "revoked", since: row.revoked_at };
	if (row.last_seen_at === null) return { kind: "never" };
	const since = row.last_seen_at;
	const age = now - since;
	if (age <= PRESENCE_ONLINE_S) return { kind: "online", since };
	if (age <= PRESENCE_LATE_S) return { kind: "late", since };
	return { kind: "offline", since };
}

/** BG1 `relationship` when the hub sends it; otherwise only ownership is known (interim "Shared or cloud approvals"). */
export function relationshipOf(
	row: Pick<DeviceRow, "owner_id" | "relationship">,
	me: string,
): Relationship {
	if (row.relationship) return row.relationship;
	return row.owner_id === me ? "owner" : "unknown";
}
