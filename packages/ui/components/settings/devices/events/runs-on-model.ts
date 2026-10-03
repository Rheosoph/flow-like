import type { EventKind } from "../../../../lib/device-management/deployment";
import type {
	AppServiceRow,
	AppUnknown,
	AppView,
	MatrixCell,
	MatrixRow,
} from "../../../../lib/device-management/model/app-plan";
import {
	type ScheduleWhere,
	runsSchedule,
	scheduleRun,
} from "../../../../lib/device-management/model/schedule-where";
import type { GateFailure } from "../../../../lib/device-management/model/types";
import type {
	DeviceSchedule,
	OnceSchedule,
} from "../../../../lib/device-management/schedule";

/* APP §4.4: one event's Devices cell, read from the same matrix as App › Devices By event. */

export interface RunsOnServed {
	deviceId: string;
	device: string;
	cell: MatrixCell;
	/** The first service of the app on this device that serves the event. */
	service: AppServiceRow | undefined;
}

export interface RunsOnUnknown {
	deviceId: string;
	device: string;
	unknown: AppUnknown | { kind: "snapshot" };
	/** `snapshot`: the service whose status doesn't list its events. */
	serviceId?: string;
}

export interface RunsOnElsewhere {
	deviceId: string;
	device: string;
	state: "not_served" | "cant_here" | "no_access";
	/** `cant_here`: the device refused it, its agent lacks a flag the event needs, or another service runs the schedule or bot. */
	why?: MatrixCell["why"];
	/** `cant_here` · `refuse`: the device's own words. */
	reason?: string;
	/** `cant_here` · `agent`: the first flag the device's agent lacks. */
	feature?: MatrixCell["feature"];
	/** `cant_here` · `runs_elsewhere`: the event is a bot. */
	bot?: true;
	/** Why a deploy to this device is gated. */
	gate: GateFailure | null;
	/** It never checked in: nothing runs there, and unlocking it changes nothing. */
	never?: true;
}

export interface RunsOnRow {
	eventId: string;
	pin: MatrixRow["pin"];
	newIn: string | null;
	served: RunsOnServed[];
	unknown: RunsOnUnknown[];
	elsewhere: RunsOnElsewhere[];
	/** Served devices whose pin is older than the event's newest one. */
	older: number;
	/** Served devices that run the flow from before its current edits (an event that follows Latest): not older, not newest. */
	edits: number;
	/** Every unknown device is locked on this computer: "2 locked" instead of "2 unknown". */
	lockedOnly: boolean;
	/** The event follows Latest: a device runs the flow as it was at its last deploy or update. */
	followsLatest: boolean;
	/** How a device runs it. */
	kind: EventKind | null;
	/** A repeating schedule a device can run: when it runs. */
	schedule?: DeviceSchedule;
	/** A one-time schedule a device can run: when, as its event record saves it. */
	once?: OnceSchedule;
	/** A schedule or bot of an online app: where it runs; absent while the hub's list is not known. */
	where?: ScheduleWhere;
	/** A schedule or bot saved for devices only: the hub and this computer never run it. */
	deviceOnly?: true;
}

const serviceKey = (deviceId: string, serviceId: string) =>
	`${deviceId}/${serviceId}`;

/** Device id → name for every device the app view mentions. */
export function runsOnDeviceNames(view: AppView): Map<string, string> {
	const names = new Map<string, string>();
	for (const group of view.groups) names.set(group.deviceId, group.name);
	for (const rows of Object.values(view.everywhereElse))
		for (const row of rows) names.set(row.deviceId, row.name);
	return names;
}

interface RowSources {
	names: Map<string, string>;
	services: Map<string, AppServiceRow>;
	snapshotService: Map<string, string>;
	accessGates: Map<string, GateFailure | null>;
	never: Set<string>;
}

function sourcesOf(view: AppView): RowSources {
	return {
		names: runsOnDeviceNames(view),
		services: new Map(
			view.services.map((row) => [
				serviceKey(row.deviceId, row.serviceId),
				row,
			]),
		),
		snapshotService: new Map(
			view.groups.flatMap((group) => {
				const service = group.services.find((row) => row.events === null);
				return service ? [[group.deviceId, service.serviceId] as const] : [];
			}),
		),
		accessGates: new Map(
			view.everywhereElse.noAccess.map((row) => [row.deviceId, row.gate]),
		),
		never: new Set(view.everywhereElse.never.map((row) => row.deviceId)),
	};
}

function rowOf(row: MatrixRow, cols: string[], sources: RowSources): RunsOnRow {
	const { schedule, once, kind } = row.eligibility;
	const result: RunsOnRow = {
		eventId: row.eventId,
		pin: row.pin,
		newIn: row.newIn,
		served: [],
		unknown: [],
		elsewhere: [],
		older: 0,
		edits: 0,
		lockedOnly: true,
		followsLatest: row.eligibility.followsLatest,
		kind,
		...(schedule ? { schedule } : {}),
		...(once ? { once } : {}),
		...(row.where ? { where: row.where } : {}),
		...(row.deviceOnly ? { deviceOnly: true as const } : {}),
	};
	for (const deviceId of cols) {
		const cell = row.cells[deviceId];
		if (!cell) continue;
		const device = sources.names.get(deviceId) ?? deviceId;
		if (cell.state === "served" || cell.state === "staged") {
			const [serviceId] = cell.serviceIds;
			result.served.push({
				deviceId,
				device,
				cell,
				service: serviceId
					? sources.services.get(serviceKey(deviceId, serviceId))
					: undefined,
			});
			if (cell.drift?.state === "edits") result.edits++;
			else if (cell.behind) result.older++;
		} else if (cell.state === "unknown") {
			const unknown = cell.unknown ?? { kind: "notloaded" as const };
			const serviceId = sources.snapshotService.get(deviceId);
			result.unknown.push({
				deviceId,
				device,
				unknown,
				...(unknown.kind === "snapshot" && serviceId ? { serviceId } : {}),
			});
			if (unknown.kind !== "locked") result.lockedOnly = false;
		} else
			result.elsewhere.push({
				deviceId,
				device,
				state: cell.state,
				...(cell.why ? { why: cell.why } : {}),
				...(cell.reason ? { reason: cell.reason } : {}),
				...(cell.feature ? { feature: cell.feature } : {}),
				...(cell.bot ? { bot: true as const } : {}),
				gate:
					cell.state === "no_access"
						? (sources.accessGates.get(deviceId) ?? null)
						: (cell.gate ?? null),
				...(sources.never.has(deviceId) ? { never: true as const } : {}),
			});
	}
	return result;
}

/** A claimed event the rule can run: a schedule of either kind, or a bot. */
export function isClaimedRow(row: RunsOnRow | undefined): row is RunsOnRow {
	return !!row && (!!row.schedule || !!row.once || row.kind === "bot");
}

/** A one-time schedule ran on one of the services that serve it (local-only apps, where no hub says so). */
function ranOnServed(row: RunsOnRow): boolean {
	return row.served.some((served) => {
		const view = served.service?.view;
		// Whether it finished doesn't depend on the clock.
		const run = view ? scheduleRun(view, row.eventId, 0) : null;
		return run?.state === "finished" && run.finished.state === "ran";
	});
}

export type OnDevice = "runs" | "assigned" | "ran" | null;

/** An online app: the hub says who has it, and the device whether it runs or ran. */
const onDeviceOnline = (where: ScheduleWhere): OnDevice => {
	if (where.fact === "device_unconfirmed") return "assigned";
	if (where.fact !== "device") return null;
	if (!where.finished) return "runs";
	return where.finished.state === "ran" ? "ran" : null;
};

/** A local-only app: only the services that serve it can say. */
function onDeviceLocal(row: RunsOnRow): OnDevice {
	const runs = row.served.some(
		(served) =>
			served.service && runsSchedule(served.service.view, row.eventId),
	);
	if (runs) return "runs";
	return row.once && ranOnServed(row) ? "ran" : null;
}

/**
 * A schedule or bot that a device has, for a place that does not run it
 * itself (the Events list's name chip). `runs`: a running service is known to
 * run it. `assigned`: the hub handed it to a service whose state is not known
 * here, so "not running" can't be said. `ran`: a one-time schedule ran on its
 * device and nothing runs it again. Null: no device runs it, or it is neither.
 */
export function scheduleOnDevice(row: RunsOnRow | undefined): OnDevice {
	if (!isClaimedRow(row)) return null;
	return row.where ? onDeviceOnline(row.where) : onDeviceLocal(row);
}

/**
 * One entry per event that can run on a device, and per event that can't but
 * that a device still serves (a paused or changed event keeps running there
 * until its service is stopped or updated).
 */
export function runsOnRows(view: AppView): Map<string, RunsOnRow> {
	const sources = sourcesOf(view);
	const stillServed = view.events.ineligible.filter(
		(row) => Object.keys(row.cells).length > 0,
	);
	return new Map(
		[...view.events.rows, ...stillServed].map((row) => [
			row.eventId,
			rowOf(row, view.events.cols, sources),
		]),
	);
}
