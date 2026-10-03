import type {
	AgeState,
	AgentFeature,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { ServiceApp } from "./service-header";

/* The events of one service by kind, and what its process reports about each: shared by the blocks of Service › Status. No React. */

export type AppEventRow = NonNullable<
	ServiceApp["view"]
>["events"]["rows"][number];

/** The app's events by id, the ones that can't run on devices included; empty while they can't be read. */
export function appEventRows(
	app: Pick<ServiceApp, "view">,
): ReadonlyMap<string, AppEventRow> {
	return new Map(
		app.view
			? [...app.view.events.rows, ...app.view.events.ineligible].map((row) => [
					row.eventId,
					row,
				])
			: [],
	);
}

/**
 * What "Add an event…" can offer a service: the app's events that can run on
 * a device and that it doesn't serve (`open`), how many of them this device
 * can take, and, when it can take none of them for that reason, the first
 * agent flag they wait for.
 */
export function addableEvents(
	rows: readonly AppEventRow[],
	deviceId: string,
	served: ReadonlySet<string>,
) {
	const open = rows.filter(
		(row) => row.eligibility.eligible && !served.has(row.eventId),
	);
	const here = open.filter((row) => row.cells[deviceId]?.state !== "cant_here");
	const waiting = open
		.map((row) => row.cells[deviceId])
		.find((cell) => cell?.why === "agent" && cell.feature);
	const needs: AgentFeature | undefined = here.length
		? undefined
		: waiting?.feature;
	return { open, addable: here.length, needs };
}

/**
 * The events of one kind a service has: those it serves that the app lists
 * with that kind, and those its process reports. A service that is stopped,
 * or whose agent can't say, still lists the ones its events name.
 */
export function serviceEventIds(
	service: Pick<ServiceView, "events">,
	reported: readonly { event_id: string }[],
	isKind: (eventId: string) => boolean,
): string[] {
	return [
		...new Set([
			...(service.events ?? []).map((event) => event.event_id).filter(isKind),
			...reported.map((entry) => entry.event_id),
		]),
	];
}

/** A service asked to stay stopped, or stopped by failing, runs nothing, whatever its row still carries. */
export function serviceIdle(view: Pick<ServiceView, "desired" | "conv">) {
	return (
		view.desired === "stopped" ||
		view.conv === "stopped_by_user" ||
		view.conv === "crash_looping" ||
		view.conv === "failed_stopped"
	);
}

type ReportedList<T> = T[] | "not_reported" | "needs_agent" | "not_loaded";

/**
 * What a service's process says about one of its bots, actions or forms.
 * `stopped`: the service runs nothing. `not_reported`: it runs and has not
 * said anything about it yet. `needs_agent`: its agent is too old to say.
 * `unknown`: this plane does not say (locked, or a status without flags).
 */
export type ListRun<T> =
	| { state: "reported"; entry: T }
	| { state: "stopped" }
	| { state: "not_reported" }
	| { state: "needs_agent" }
	| { state: "unknown" };

export function listRun<T extends { event_id: string }>(
	view: Pick<ServiceView, "desired" | "conv">,
	list: ReportedList<T> | undefined,
	eventId: string,
): ListRun<T> {
	if (list === undefined || list === "not_loaded") return { state: "unknown" };
	if (list === "needs_agent") return { state: "needs_agent" };
	if (serviceIdle(view)) return { state: "stopped" };
	const entry = Array.isArray(list)
		? list.find((value) => value.event_id === eventId)
		: undefined;
	return entry ? { state: "reported", entry } : { state: "not_reported" };
}

export const reportedEntries = <T>(list: ReportedList<T> | undefined): T[] =>
	Array.isArray(list) ? list : [];

/** The device reports a file field in this form: only the service page can send one (design R2 §4.8). */
export const formTakesFile = (
	view: Pick<ServiceView, "actions">,
	eventId: string,
): boolean =>
	reportedEntries(view.actions).some(
		(entry) => entry.event_id === eventId && entry.file_fields > 0,
	);

const FRESH_AGES: readonly AgeState[] = ["live", "current", "delayed"];

/** Counters belong to the process that was read: a status that old no longer speaks for it. */
export const countersFresh = (view: Pick<ServiceView, "freshness">) =>
	FRESH_AGES.includes(view.freshness.age);
