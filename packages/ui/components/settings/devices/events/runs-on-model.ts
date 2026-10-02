import type {
	AppServiceRow,
	AppUnknown,
	AppView,
	MatrixCell,
	MatrixRow,
} from "../../../../lib/device-management/model/app-plan";
import type { GateFailure } from "../../../../lib/device-management/model/types";

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
	/** `cant_here`: the device's own words. */
	reason?: string;
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
	/** Every unknown device is locked on this computer: "2 locked" instead of "2 unknown". */
	lockedOnly: boolean;
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
	const result: RunsOnRow = {
		eventId: row.eventId,
		pin: row.pin,
		newIn: row.newIn,
		served: [],
		unknown: [],
		elsewhere: [],
		older: 0,
		lockedOnly: true,
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
			if (cell.behind) result.older++;
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
				...(cell.reason ? { reason: cell.reason } : {}),
				gate:
					cell.state === "no_access"
						? (sources.accessGates.get(deviceId) ?? null)
						: (cell.gate ?? null),
				...(sources.never.has(deviceId) ? { never: true as const } : {}),
			});
	}
	return result;
}

/** One entry per event that can run on a device; events that can't have no device cells. */
export function runsOnRows(view: AppView): Map<string, RunsOnRow> {
	const sources = sourcesOf(view);
	return new Map(
		view.events.rows.map((row) => [
			row.eventId,
			rowOf(row, view.events.cols, sources),
		]),
	);
}
