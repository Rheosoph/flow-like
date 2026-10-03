import { eventEligibility } from "../../../../lib/device-management/deployment";
import type { FlowVersionState } from "../../../../lib/device-management/hub/endpoints";
import type { AppInput } from "../../../../lib/device-management/model/app-plan";
import type { AppScheduleRow } from "../../../../lib/device-management/model/types";

/*
 * The fake hub's part of "schedules on devices" and "deploy Latest": one row
 * per schedule or bot that a person released to a service (keyed by event,
 * never by type), the claim of that service's device, the hand-back grace
 * periods, and each flow as a version.
 */

/** The hub runs a schedule again this long after a hand-back the device confirmed. */
export const CONFIRMED_GRACE_S = 300;
/** … and this long when a process of the service may still be running it. */
export const UNCONFIRMED_GRACE_S = 3_900;

type Triple = [number, number, number];

interface HubSchedule {
	released?: { deviceId: string; placementId: string; at: number };
	claim?: { grantId: string; claimedAt: number; seenAt: number };
	/** A hand-back's grace period: the hub runs it again from then. */
	resumesAt?: number;
}

export interface ScheduleClaimAnswer {
	server_time: number;
	claimed: { event_id: string; since: number }[];
	held: { event_id: string; reason: "not_released" | "runs_elsewhere" }[];
}

export interface FakeSchedulesDeps {
	now(): number;
	/** A hub refusal with a status and a code. */
	refuse(status: number, code: string, message: string): Error;
	/** The approval of a service that still works, with its app. */
	approval(
		deviceId: string,
		placementId: string,
	): { grantId: string; appId: string | null } | undefined;
	/** A process of the service is running (it holds a workload instance). */
	running(deviceId: string, placementId: string): boolean;
	/** The signed-in account can see that device. */
	visible(deviceId: string): boolean;
}

export class FakeSchedules {
	/** App id → event id → row. A schedule the hub runs and nobody released has none. */
	readonly rows = new Map<string, Map<string, HubSchedule>>();
	/** False: the signed-in person may not edit the app's events (no `WriteEvents`). */
	canEditEvents = true;

	constructor(private readonly deps: FakeSchedulesDeps) {}

	private rowsOf(appId: string): Map<string, HubSchedule> {
		const rows = this.rows.get(appId) ?? new Map<string, HubSchedule>();
		this.rows.set(appId, rows);
		return rows;
	}

	private requireEditor() {
		if (!this.canEditEvents)
			throw this.deps.refuse(
				403,
				"FORBIDDEN",
				"Moving a schedule needs the right to edit this app's events",
			);
	}

	private returning(appId: string, eventId: string): Error {
		return this.deps.refuse(
			409,
			"SCHEDULE_RETURNING",
			`Schedule ${eventId} of ${appId} is returning to the hub`,
		);
	}

	/** A person lets one service take the schedule off the hub once it runs. */
	release(
		appId: string,
		eventId: string,
		deviceId: string,
		placementId: string,
	): { state: "released" | "device"; since: number } {
		this.requireEditor();
		const now = this.deps.now();
		const rows = this.rowsOf(appId);
		const row = rows.get(eventId) ?? {};
		const { released, claim } = row;
		if (released?.deviceId === deviceId && released.placementId === placementId)
			return claim
				? { state: "device", since: claim.claimedAt }
				: { state: "released", since: released.at };
		if (claim && released) {
			if (
				this.deps.approval(released.deviceId, released.placementId)?.grantId ===
				claim.grantId
			)
				throw this.deps.refuse(
					409,
					"SCHEDULE_RUNS_ELSEWHERE",
					`Schedule ${eventId} runs on another service`,
				);
			// The claiming approval stopped working: the hub takes it back first.
			rows.set(eventId, { resumesAt: now + UNCONFIRMED_GRACE_S });
			throw this.returning(appId, eventId);
		}
		if ((row.resumesAt ?? 0) > now) throw this.returning(appId, eventId);
		rows.set(eventId, { released: { deviceId, placementId, at: now } });
		return { state: "released", since: now };
	}

	/** A person hands the schedule back to the hub, whatever state the device is in. */
	giveBack(appId: string, eventId: string): { hub_resumes_at: number | null } {
		this.requireEditor();
		const rows = this.rowsOf(appId);
		const row = rows.get(eventId);
		if (!row?.claim || !row.released) {
			// The hub was running it all along.
			if (row?.resumesAt) rows.set(eventId, { resumesAt: row.resumesAt });
			else rows.delete(eventId);
			return { hub_resumes_at: null };
		}
		const { deviceId, placementId } = row.released;
		const resumesAt =
			this.deps.now() +
			(this.deps.running(deviceId, placementId)
				? UNCONFIRMED_GRACE_S
				: CONFIRMED_GRACE_S);
		rows.set(eventId, { resumesAt });
		return { hub_resumes_at: resumesAt };
	}

	/**
	 * A service's process states the complete set of schedules it runs now (the
	 * device's `POST instances/project/schedules`). Throws when no approval of
	 * that service works.
	 */
	claim(
		deviceId: string,
		placementId: string,
		eventIds: readonly string[],
	): ScheduleClaimAnswer {
		const approval = this.deps.approval(deviceId, placementId);
		if (!approval?.appId)
			throw this.deps.refuse(
				403,
				"FORBIDDEN",
				"No working cloud approval for this service",
			);
		const now = this.deps.now();
		const rows = this.rowsOf(approval.appId);
		const answer: ScheduleClaimAnswer = {
			server_time: now,
			claimed: [],
			held: [],
		};
		// What the service no longer lists goes back to the hub, confirmed by the device.
		for (const [eventId, row] of rows)
			if (
				row.claim &&
				row.released?.deviceId === deviceId &&
				row.released.placementId === placementId &&
				!eventIds.includes(eventId)
			)
				rows.set(eventId, { resumesAt: now + CONFIRMED_GRACE_S });
		for (const eventId of eventIds) {
			const row = rows.get(eventId);
			const released = row?.released;
			if (!row || !released) {
				answer.held.push({ event_id: eventId, reason: "not_released" });
				continue;
			}
			if (
				released.deviceId !== deviceId ||
				released.placementId !== placementId
			) {
				answer.held.push({ event_id: eventId, reason: "runs_elsewhere" });
				continue;
			}
			if (row.claim?.grantId === approval.grantId) row.claim.seenAt = now;
			else {
				row.claim = { grantId: approval.grantId, claimedAt: now, seenAt: now };
				row.resumesAt = undefined;
			}
			answer.claimed.push({ event_id: eventId, since: row.claim.claimedAt });
		}
		return answer;
	}

	/** Revoking an approval hands back every schedule it runs; the release stays. */
	revoke(appId: string | null, grantId: string, running: boolean) {
		if (!appId) return;
		const now = this.deps.now();
		for (const row of this.rowsOf(appId).values())
			if (row.claim?.grantId === grantId) {
				row.claim = undefined;
				row.resumesAt =
					now + (running ? UNCONFIRMED_GRACE_S : CONFIRMED_GRACE_S);
			}
	}

	/** The `schedules` array of the app's placement list (`GET apps/{id}/device-placements`). */
	listing(appId: string): AppScheduleRow[] {
		const now = this.deps.now();
		const listed: AppScheduleRow[] = [];
		for (const [eventId, row] of this.rows.get(appId) ?? []) {
			const { released, claim } = row;
			const resuming = (row.resumesAt ?? 0) > now ? row.resumesAt : undefined;
			const service =
				released && this.deps.visible(released.deviceId)
					? { device_id: released.deviceId, placement_id: released.placementId }
					: {};
			if (released && claim)
				listed.push({
					event_id: eventId,
					state: "device",
					since: claim.claimedAt,
					seen_at: claim.seenAt,
					...("device_id" in service ? { grant_id: claim.grantId } : {}),
					...service,
				});
			else if (released)
				listed.push({
					event_id: eventId,
					state: "released",
					since: released.at,
					...(resuming === undefined ? {} : { hub_resumes_at: resuming }),
					...service,
				});
			else if (resuming !== undefined)
				listed.push({
					event_id: eventId,
					state: "returning",
					hub_resumes_at: resuming,
				});
		}
		return listed;
	}
}

/* Flows as versions. */

const compare = (a: readonly number[], b: readonly number[]) =>
	a[0] - b[0] || a[1] - b[1] || a[2] - b[2];

export class FakeFlows {
	private readonly states = new Map<string, FlowVersionState>();
	/** False: the signed-in person may not create a flow version (no `WriteBoards`). */
	canPublish = true;
	/** Flows someone is editing right now (their edit lock is held), by `app/board`. */
	readonly locked = new Set<string>();
	/** Flows that never compare equal to their published version, by `app/board`. */
	readonly incomparable = new Set<string>();

	constructor(
		private readonly apps: () => Readonly<Record<string, AppInput>>,
		private readonly refuse: FakeSchedulesDeps["refuse"],
	) {}

	/** What the app's events say about the flow before anything was published here. */
	private seeded(appId: string, boardId: string): FlowVersionState {
		const events = (this.apps()[appId]?.events ?? []).filter(
			(event) => event.boardId === boardId,
		);
		const stated = events.find((event) => typeof event.flow === "object");
		if (stated && typeof stated.flow === "object") return { ...stated.flow };
		const pinned = events
			.map((event) => eventEligibility(event).boardVersion)
			.filter((version): version is Triple => version !== null)
			.sort(compare);
		const newest = pinned.at(-1) ?? null;
		return { current: newest, newest };
	}

	state(appId: string, boardId: string): FlowVersionState {
		const key = `${appId}/${boardId}`;
		const known = this.states.get(key) ?? this.seeded(appId, boardId);
		this.states.set(key, known);
		return known;
	}

	/** Someone saves edits: no published version equals the flow any more. */
	edit(appId: string, boardId: string) {
		this.state(appId, boardId).current = null;
	}

	/** Publish-if-changed: a Patch version only when no version equals the flow. */
	publish(
		appId: string,
		boardId: string,
	): { version: Triple; created: boolean } {
		const key = `${appId}/${boardId}`;
		if (!this.canPublish)
			throw this.refuse(
				403,
				"FORBIDDEN",
				"Creating a flow version needs the right to edit flows",
			);
		if (this.locked.has(key))
			throw this.refuse(423, "BOARD_LOCKED", "This flow is being edited");
		const state = this.state(appId, boardId);
		if (state.current) return { version: state.current, created: false };
		if (this.incomparable.has(key))
			throw this.refuse(
				422,
				"UNPROCESSABLE_ENTITY",
				"This flow can't be compared with its published version. Pin a flow version in Events to deploy this event.",
			);
		const [major, minor, patch] = state.newest ?? [0, 0, 0];
		const version: Triple = [major, minor, patch + 1];
		state.current = version;
		state.newest = version;
		return { version, created: true };
	}
}
