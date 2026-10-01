import type { IAppVisibility } from "../../schema/app/app";
import {
	type EligibilityEvent,
	type EventEligibility,
	eventEligibility,
} from "../deployment";
import type { KeyState } from "../workspace/types";
import type {
	AgeState,
	AppDevicePlacements,
	Convergence,
	CopyRef,
	FreshnessReason,
	GateFailure,
	PlacementEvent,
	Presence,
	Relationship,
	ServiceView,
} from "./types";

/* App › Devices (APP §2): where and how one app runs, from what each plane can say. */

export type AppVisibility = `${IAppVisibility}`;
export type AppMode = "online" | "offline";
export type VersionTriple = readonly [number, number, number];

/** A1: the mode follows the app's visibility; it is never a choice. */
export function appMode(visibility: AppVisibility): AppMode {
	return visibility === "Offline" ? "offline" : "online";
}

export interface AppEventInput extends EligibilityEvent {
	id: string;
	name: string;
	/** Why the approved bundle could not prepare this event's flow. */
	ineligibleReason?: string | null;
}

export interface AppVersionPin {
	eventId: string;
	eventVersion: VersionTriple;
	boardVersion: VersionTriple;
}

export interface AppVersionInput {
	/** Copy revision (local-only) or approved-definitions hash (online). */
	hash: string;
	label?: string | null;
	builtAt?: number | null;
	by?: string | null;
	pins: readonly AppVersionPin[];
}

export interface AppInput {
	id: string;
	name: string;
	visibility: AppVisibility;
	/** Newest first; null when the viewer's role can't read flows. */
	versions: readonly AppVersionInput[] | null;
	/** Every event of the app, in the Events list order. */
	events: readonly AppEventInput[];
}

export interface AppDeviceInput {
	id: string;
	name: string;
	presence: Presence;
	relationship: Relationship;
	keyState?: KeyState;
	/** Never `[]` for "not loaded": a state object says why nothing is readable. */
	services:
		| readonly ServiceView[]
		| { state: AgeState; reason?: CopyRef<FreshnessReason> };
	/** Device-side refusals by event id (offline discovery after upload). */
	refusals?: Readonly<Record<string, string>>;
	/** Deploy gate for this app on this device, evaluated by the caller. */
	deployGate?: GateFailure | null;
}

/** What this computer recorded when it deployed a service (BG12 interim, APP §5.4). */
export interface LocalServiceChange {
	at: number;
	kind: "deploy" | "update" | "settings" | "rollback";
	hash?: string;
	settings?: number;
	by?: string | null;
	/** Offline copies: when this service got its data. */
	seededAt?: number;
}

export interface AppViewInput {
	app: AppInput;
	devices: readonly AppDeviceInput[];
	/** E20; undefined when not loaded or the hub is older. */
	placements?: AppDevicePlacements | null;
	/** Keyed `${deviceId}/${serviceId}`. */
	changes?: Readonly<Record<string, LocalServiceChange>>;
	/** `d-<device>` deep-link targets, always shown as By event columns. */
	focusDeviceIds?: readonly string[];
}

export type AppUnknownKind =
	| "locked"
	| "nokeys"
	| "never"
	| "offline"
	| "noaccess"
	| "notloaded"
	| "error";

export interface AppUnknown {
	kind: AppUnknownKind;
	since?: number;
	reason?: CopyRef<FreshnessReason>;
}

export interface VersionDiffRow {
	eventId: string;
	kind: "added" | "changed" | "removed";
	from?: Omit<AppVersionPin, "eventId">;
	to?: Omit<AppVersionPin, "eventId">;
}

export interface VersionRunning {
	deviceId: string;
	serviceId: string;
	conv: Convergence;
	lastKnown: boolean;
}

export interface AppVersionView {
	hash: string;
	short: string;
	label: string | null;
	builtAt: number | null;
	by: string | null;
	/** 0 = newest. */
	index: number;
	pins: readonly AppVersionPin[];
	/** Against the next older version; null for the oldest. */
	diff: VersionDiffRow[] | null;
	runningOn: VersionRunning[];
	/** Devices or services whose version can't be read now (unknown ≠ not running). */
	unknownOn: string[];
}

export type AppCloudCell =
	| {
			state: "approved";
			placement: AppDevicePlacements["placements"][number];
	  }
	| { state: "hidden" }
	| { state: "none" }
	| { state: "unknown" };

export interface AppServiceRow {
	deviceId: string;
	serviceId: string;
	view: ServiceView;
	mode: AppMode;
	events: PlacementEvent[] | null;
	/** Why `events` is null: the status snapshot carries no event list (BG-A1). */
	eventsWhy: "snapshot" | null;
	version: AppVersionView | null;
	/** 0 = newest, n = behind, null = unknown. */
	behind: number | null;
	staged: boolean;
	data: { where: "cloud" | "device"; since?: number };
	writes: ServiceView["offlineWrites"] | null;
	cloud: AppCloudCell;
	lastChange: LocalServiceChange | null;
	lastKnown: boolean;
	/** 0 crash · 1 writes need you · 2 updating · 3 unknown · 4 as requested. */
	rank: number;
}

export interface AppDeviceGroup {
	deviceId: string;
	name: string;
	presence: Presence;
	relationship: Relationship;
	services: AppServiceRow[];
	/** Set when this device's services for the app can't be read. */
	unknown: AppUnknown | null;
	rank: number;
}

export type MatrixCellState =
	| "served"
	| "staged"
	| "not_served"
	| "cant_here"
	| "unknown"
	| "no_access";

export interface MatrixCell {
	state: MatrixCellState;
	deviceId: string;
	/** Every service of the app on this device that serves the event. */
	serviceIds: string[];
	conv?: Convergence;
	pin?: Omit<AppVersionPin, "eventId">;
	/** The served pin differs from the newest version's pin. */
	behind?: boolean;
	unknown?: AppUnknown | { kind: "snapshot" };
	/** `cant_here`: the device's own words. */
	reason?: string;
	/** `not_served`: why Deploy here is gated. */
	gate?: GateFailure | null;
}

export interface MatrixRow {
	eventId: string;
	name: string;
	eventType: string;
	eligibility: EventEligibility;
	/** Pin in the newest version, else the event's own pins. */
	pin: Omit<AppVersionPin, "eventId"> | null;
	/** Label of the newest version when the event is new in it. */
	newIn: string | null;
	cells: Record<string, MatrixCell>;
}

export const MATRIX_MAX_COLUMNS = 4;

export interface AppEventMatrix {
	cols: string[];
	rows: MatrixRow[];
	/** "Can't run on devices": no device cells. */
	ineligible: Omit<MatrixRow, "cells">[];
	listMode: boolean;
}

export interface EverywhereRow {
	deviceId: string;
	name: string;
	presence: Presence;
	/** Other services the device runs (readable rows only). */
	runs: Pick<ServiceView, "serviceId" | "projectId">[];
	gate: GateFailure | null;
	unknown?: AppUnknown;
}

export interface AppCoverage {
	total: number;
	readable: number;
	unknown: number;
	locked: string[];
}

export type AppLayout = "normal" | "never" | "all_unknown" | "no_devices";

export interface AppView {
	app: {
		id: string;
		name: string;
		visibility: AppVisibility;
		mode: AppMode;
		localOnly: boolean;
		canReadFlows: boolean;
	};
	howRuns: {
		visibility: AppVisibility;
		mode: AppMode;
		newest: AppVersionView | null;
	};
	layout: AppLayout;
	groups: AppDeviceGroup[];
	services: AppServiceRow[];
	events: AppEventMatrix;
	versions: AppVersionView[];
	everywhereElse: {
		notDeployed: EverywhereRow[];
		unknown: EverywhereRow[];
		noAccess: EverywhereRow[];
	};
	coverage: AppCoverage;
	/** Version foot (APP §2.9): newest version and how many services run it. */
	newestRuns: { version: AppVersionView; services: number; of: number } | null;
}

/** Same rule as the attention items (`isLastKnown`): anything but live or current is last known. */
const CURRENT_AGES: readonly AgeState[] = ["live", "current"];

export function shortHash(hash: string) {
	return hash.slice(0, 8);
}

function samePin(
	left: readonly number[] | null | undefined,
	right: readonly number[] | null | undefined,
) {
	return (
		!!left &&
		!!right &&
		left.length === right.length &&
		left.every((value, index) => value === right[index])
	);
}

function sameHash(left: string | undefined, right: string) {
	if (!left) return false;
	const a = left.toLowerCase();
	const b = right.toLowerCase();
	return a === b || (a.length >= 8 && (b.startsWith(a) || a.startsWith(b)));
}

function versionDiff(
	newer: AppVersionInput,
	older: AppVersionInput | undefined,
): VersionDiffRow[] | null {
	if (!older) return null;
	const rows: VersionDiffRow[] = [];
	for (const pin of newer.pins) {
		const before = older.pins.find((value) => value.eventId === pin.eventId);
		const to = {
			eventVersion: pin.eventVersion,
			boardVersion: pin.boardVersion,
		};
		if (!before) rows.push({ eventId: pin.eventId, kind: "added", to });
		else if (
			!samePin(before.eventVersion, pin.eventVersion) ||
			!samePin(before.boardVersion, pin.boardVersion)
		)
			rows.push({
				eventId: pin.eventId,
				kind: "changed",
				from: {
					eventVersion: before.eventVersion,
					boardVersion: before.boardVersion,
				},
				to,
			});
	}
	for (const pin of older.pins)
		if (!newer.pins.some((value) => value.eventId === pin.eventId))
			rows.push({
				eventId: pin.eventId,
				kind: "removed",
				from: {
					eventVersion: pin.eventVersion,
					boardVersion: pin.boardVersion,
				},
			});
	return rows;
}

const baseVersions = (app: AppInput): AppVersionView[] => {
	const versions = app.versions ?? [];
	return versions.map((version, index) => ({
		hash: version.hash,
		short: shortHash(version.hash),
		label: version.label ?? null,
		builtAt: version.builtAt ?? null,
		by: version.by ?? null,
		index,
		pins: version.pins,
		diff: versionDiff(version, versions[index + 1]),
		runningOn: [],
		unknownOn: [],
	}));
};

/** By hash first; by event pins only when exactly one version matches them. */
function matchVersion(
	view: ServiceView,
	versions: AppVersionView[],
): AppVersionView | null {
	const hash = view.appVersion?.hash;
	if (hash)
		return versions.find((version) => sameHash(hash, version.hash)) ?? null;
	const events = view.events;
	if (!events?.length) return null;
	const matches = versions.filter((version) =>
		events.every((event) => {
			const pin = version.pins.find(
				(value) => value.eventId === event.event_id,
			);
			return (
				pin &&
				samePin(pin.eventVersion, event.event_version) &&
				samePin(pin.boardVersion, event.board_version)
			);
		}),
	);
	return matches.length === 1 ? matches[0] : null;
}

type DeviceServices = AppDeviceInput["services"];
type UnreadableServices = Exclude<DeviceServices, readonly ServiceView[]>;

function isUnreadable(
	services: DeviceServices,
): services is UnreadableServices {
	return !Array.isArray(services);
}

function readable(device: AppDeviceInput): readonly ServiceView[] | null {
	return isUnreadable(device.services) ? null : device.services;
}

export function appUnknownOf(device: AppDeviceInput): AppUnknown {
	const services = isUnreadable(device.services) ? device.services : undefined;
	const reason = services?.reason;
	if (device.presence.kind === "never") return { kind: "never" };
	if (reason?.code === "no_keys_here" || device.keyState === "none")
		return { kind: "nokeys" };
	if (services?.state === "noaccess")
		return { kind: "noaccess", ...(reason ? { reason } : {}) };
	if (
		services?.state === "locked" ||
		device.keyState === "locked" ||
		device.keyState === "stale" ||
		device.keyState === "held_elsewhere"
	)
		return { kind: "locked", ...(reason ? { reason } : {}) };
	if (device.presence.kind === "offline")
		return {
			kind: "offline",
			...(device.presence.since ? { since: device.presence.since } : {}),
		};
	if (services?.state === "error")
		return { kind: "error", ...(reason ? { reason } : {}) };
	return { kind: "notloaded", ...(reason ? { reason } : {}) };
}

const CONV_RANK: Record<Convergence, number> = {
	crash_looping: 0,
	failed_stopped: 0,
	update_in_progress: 2,
	converging: 2,
	unknown: 3,
	converged: 4,
	stopped_by_user: 4,
};

const serviceRank = (view: ServiceView, staged: boolean): number => {
	const rank = CONV_RANK[view.conv];
	const writesNeedYou =
		typeof view.offlineWrites === "object" && view.offlineWrites.quarantined;
	if (rank === 0) return 0;
	if (writesNeedYou) return 1;
	return staged ? Math.min(rank, 2) : rank;
};

const cloudCell = (
	view: ServiceView,
	mode: AppMode,
	placements: AppDevicePlacements | null | undefined,
): AppCloudCell => {
	if (!placements) return { state: "unknown" };
	const placement = placements.placements.find(
		(value) =>
			value.device_id === view.deviceId &&
			value.placement_id === view.serviceId,
	);
	if (placement) return { state: "approved", placement };
	return mode === "online" ? { state: "hidden" } : { state: "none" };
};

function serviceRow(
	device: AppDeviceInput,
	view: ServiceView,
	input: AppViewInput,
	appModeValue: AppMode,
	versions: AppVersionView[],
): AppServiceRow {
	const mode = view.source ?? appModeValue;
	const version = matchVersion(view, versions);
	const staged = view.rollout?.state === "staged";
	const lastChange = input.changes?.[`${device.id}/${view.serviceId}`] ?? null;
	return {
		deviceId: device.id,
		serviceId: view.serviceId,
		view,
		mode,
		events: view.events,
		eventsWhy: view.events ? null : "snapshot",
		version,
		behind: version ? version.index : null,
		staged,
		data:
			mode === "online"
				? { where: "cloud" }
				: {
						where: "device",
						...(lastChange?.seededAt ? { since: lastChange.seededAt } : {}),
					},
		writes: view.offlineWrites ?? null,
		cloud: cloudCell(view, mode, input.placements),
		lastChange,
		lastKnown: !CURRENT_AGES.includes(view.freshness.age),
		rank: serviceRank(view, staged),
	};
}

function byRankThenName<T extends { rank: number }>(
	name: (value: T) => string,
): (a: T, b: T) => number {
	return (a, b) => a.rank - b.rank || name(a).localeCompare(name(b));
}

function eventPin(
	event: AppEventInput,
	newest: AppVersionView | undefined,
): Omit<AppVersionPin, "eventId"> | null {
	const pin = newest?.pins.find((value) => value.eventId === event.id);
	if (pin)
		return { eventVersion: pin.eventVersion, boardVersion: pin.boardVersion };
	const rule = eventEligibility(event);
	return rule.eventVersion && rule.boardVersion
		? { eventVersion: rule.eventVersion, boardVersion: rule.boardVersion }
		: null;
}

function newInLabel(
	eventId: string,
	versions: AppVersionView[],
): string | null {
	const [newest, older] = versions;
	if (!newest || !older) return null;
	const added = newest.diff?.some(
		(row) => row.eventId === eventId && row.kind === "added",
	);
	return added ? (newest.label ?? newest.short) : null;
}

function matrixCell(
	event: AppEventInput,
	device: AppDeviceInput,
	group: AppDeviceGroup | undefined,
	newestPin: Omit<AppVersionPin, "eventId"> | null,
): MatrixCell {
	const base = { deviceId: device.id, serviceIds: [] as string[] };
	if (!readable(device)) {
		const unknown = group?.unknown ?? appUnknownOf(device);
		return unknown.kind === "noaccess"
			? { ...base, state: "no_access" }
			: { ...base, state: "unknown", unknown };
	}
	const serving = (group?.services ?? []).filter((row) =>
		row.events?.some((value) => value.event_id === event.id),
	);
	if (serving.length) {
		const primary = serving[0];
		const pin = primary.events?.find((value) => value.event_id === event.id);
		return {
			...base,
			state: serving.some((row) => row.staged) ? "staged" : "served",
			serviceIds: serving.map((row) => row.serviceId),
			conv: primary.view.conv,
			...(pin
				? {
						pin: {
							eventVersion: pin.event_version,
							boardVersion: pin.board_version,
						},
						behind: newestPin
							? !samePin(pin.event_version, newestPin.eventVersion) ||
								!samePin(pin.board_version, newestPin.boardVersion)
							: false,
					}
				: {}),
		};
	}
	if (group?.services.some((row) => row.events === null))
		return { ...base, state: "unknown", unknown: { kind: "snapshot" } };
	const refusal = device.refusals?.[event.id];
	if (refusal) return { ...base, state: "cant_here", reason: refusal };
	return { ...base, state: "not_served", gate: device.deployGate ?? null };
}

function buildMatrix(
	input: AppViewInput,
	devices: readonly AppDeviceInput[],
	groups: AppDeviceGroup[],
	noAccess: EverywhereRow[],
	versions: AppVersionView[],
): AppEventMatrix {
	const cols = [
		...new Set([
			...groups.map((group) => group.deviceId),
			...noAccess.map((row) => row.deviceId),
			...(input.focusDeviceIds ?? []).filter((id) =>
				devices.some((device) => device.id === id),
			),
		]),
	];
	const rows: MatrixRow[] = [];
	const ineligible: Omit<MatrixRow, "cells">[] = [];
	for (const event of input.app.events) {
		const eligibility = eventEligibility(event, {
			ineligibleReason: event.ineligibleReason,
		});
		const pin = eventPin(event, versions[0]);
		const row = {
			eventId: event.id,
			name: event.name,
			eventType: event.event_type,
			eligibility,
			pin,
			newIn: newInLabel(event.id, versions),
		};
		if (!eligibility.eligible) {
			ineligible.push(row);
			continue;
		}
		const cells: Record<string, MatrixCell> = {};
		for (const id of cols) {
			const device = devices.find((value) => value.id === id);
			if (!device) continue;
			cells[id] = matrixCell(
				event,
				device,
				groups.find((group) => group.deviceId === id),
				pin,
			);
		}
		rows.push({ ...row, cells });
	}
	return { cols, rows, ineligible, listMode: cols.length > MATRIX_MAX_COLUMNS };
}

function everywhereRow(
	device: AppDeviceInput,
	appId: string,
	unknown?: AppUnknown,
): EverywhereRow {
	return {
		deviceId: device.id,
		name: device.name,
		presence: device.presence,
		runs: (readable(device) ?? [])
			.filter((view) => view.projectId !== appId)
			.map((view) => ({
				serviceId: view.serviceId,
				projectId: view.projectId,
			})),
		gate: device.deployGate ?? null,
		...(unknown ? { unknown } : {}),
	};
}

export function buildAppView(input: AppViewInput): AppView {
	const { app } = input;
	const mode = appMode(app.visibility);
	const versions = baseVersions(app);
	const devices = input.devices.filter(
		(device) => device.presence.kind !== "revoked",
	);
	const groups: AppDeviceGroup[] = [];
	const everywhereElse: AppView["everywhereElse"] = {
		notDeployed: [],
		unknown: [],
		noAccess: [],
	};
	const coverage: AppCoverage = {
		total: devices.length,
		readable: 0,
		unknown: 0,
		locked: [],
	};
	for (const device of devices) {
		const views = readable(device);
		const group = {
			deviceId: device.id,
			name: device.name,
			presence: device.presence,
			relationship: device.relationship,
		};
		if (!views) {
			const unknown = appUnknownOf(device);
			coverage.unknown++;
			if (unknown.kind === "locked") coverage.locked.push(device.id);
			if (unknown.kind === "noaccess")
				everywhereElse.noAccess.push(everywhereRow(device, app.id, unknown));
			else {
				everywhereElse.unknown.push(everywhereRow(device, app.id, unknown));
				groups.push({ ...group, services: [], unknown, rank: 3 });
			}
			continue;
		}
		coverage.readable++;
		const services = views
			.filter((view) => view.projectId === app.id)
			.map((view) => serviceRow(device, view, input, mode, versions))
			.sort(byRankThenName((row) => row.serviceId));
		if (!services.length) {
			everywhereElse.notDeployed.push(everywhereRow(device, app.id));
			continue;
		}
		groups.push({
			...group,
			services,
			unknown: null,
			rank: Math.min(...services.map((row) => row.rank)),
		});
	}
	groups.sort(byRankThenName((group) => group.name));
	const services = groups.flatMap((group) => group.services);
	const unknownOn = [
		...groups.filter((group) => group.unknown).map((group) => group.deviceId),
		...services
			.filter((row) => !row.version)
			.map((row) => `${row.deviceId}/${row.serviceId}`),
	];
	for (const version of versions) {
		version.runningOn = services
			.filter((row) => row.version === version)
			.map((row) => ({
				deviceId: row.deviceId,
				serviceId: row.serviceId,
				conv: row.view.conv,
				lastKnown: row.lastKnown,
			}));
		version.unknownOn = unknownOn;
	}
	const newest = versions[0] ?? null;
	return {
		app: {
			id: app.id,
			name: app.name,
			visibility: app.visibility,
			mode,
			localOnly: mode === "offline",
			canReadFlows: app.versions !== null,
		},
		howRuns: { visibility: app.visibility, mode, newest },
		layout: !devices.length
			? "no_devices"
			: services.length
				? "normal"
				: coverage.readable
					? "never"
					: "all_unknown",
		groups,
		services,
		events: buildMatrix(
			input,
			devices,
			groups,
			everywhereElse.noAccess,
			versions,
		),
		versions,
		everywhereElse,
		coverage,
		newestRuns: newest
			? {
					version: newest,
					services: newest.runningOn.length,
					of: services.length,
				}
			: null,
	};
}
