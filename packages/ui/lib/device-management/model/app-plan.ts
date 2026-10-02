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

/** What an update to a version sends to a device: the definitions (online) or the copy (local-only). */
export interface AppVersionSends {
	bytes: number;
	/** Local-only copies. */
	files?: number;
}

export interface AppVersionInput {
	/** Copy revision (local-only) or approved-definitions hash (online). */
	hash: string;
	label?: string | null;
	builtAt?: number | null;
	by?: string | null;
	pins: readonly AppVersionPin[];
	/** Known once this computer prepared or sent the version. */
	sends?: AppVersionSends | null;
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

/** An upload of the app to a device that is running or can be resumed (APP §2.8). */
export interface AppUploadInput {
	/** The caller's handle, e.g. the activity item. */
	id: string;
	deviceId: string;
	/** Absent while the upload is not tied to one service. */
	serviceId?: string;
	state: "active" | "paused";
	done: number;
	total: number;
	/** Unix seconds the device keeps the partial upload. */
	expiresAt: number;
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
	/** Newest first. */
	uploads?: readonly AppUploadInput[];
	/**
	 * Shared devices whose access covers other apps only (APP §2.12). Locked or
	 * not, they say nothing about this app: "no access", never "unknown until
	 * unlocked" and never "not deployed".
	 */
	noAccess?: readonly string[];
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
	sends?: AppVersionSends | null;
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
	/**
	 * Set while `staged`: the version the update switches to. The device's
	 * rollout status carries none, so it is known only when this computer staged
	 * the update (`changes[…].hash`), else null.
	 */
	stagedVersion?: AppVersionView | null;
	/** Its own upload, else the app's upload to this device on the device's first service. */
	upload?: AppUploadInput;
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
	/** Not readable, for any reason (the devices in `never` included, as in `Coverage`). */
	unknown: number;
	locked: string[];
	/** The part of `unknown` that never checked in: nothing can run there, so it is shown on its own (APP §2.18). */
	never: string[];
}

/** `all_unknown`: nothing is readable and at least one device could be (locked, no keys, failed read). */
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
		/** "Hasn't checked in yet". */
		never: EverywhereRow[];
		noAccess: EverywhereRow[];
	};
	coverage: AppCoverage;
	/**
	 * Version foot (APP §2.9): newest version and how many services run it.
	 * `unknown` of the `of` services have a version that can't be told (BG-A1),
	 * so 0 running is "not on any service whose version is known" then.
	 */
	newestRuns: {
		version: AppVersionView;
		services: number;
		of: number;
		unknown?: number;
	} | null;
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
		...(version.sends ? { sends: version.sends } : {}),
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
	// Approving cloud access or spending gives no access to a device's status: keys are not what is missing.
	if (device.relationship === "cloud_approval") return { kind: "noaccess" };
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
	const writes =
		typeof view.offlineWrites === "object" ? view.offlineWrites : undefined;
	// The same "writes need you" as the app headline: paused queues or a conflict at the head.
	const writesNeedYou =
		!!writes && (writes.quarantined || writes.head?.state === "conflict");
	if (rank === 0) return 0;
	if (writesNeedYou) return 1;
	return staged ? Math.min(rank, 2) : rank;
};

/**
 * On a device the viewer owns the hub lists every approval, so none listed
 * means none given; on a shared device it may be someone else's and hidden.
 */
const cloudCell = (
	view: ServiceView,
	mode: AppMode,
	placements: AppDevicePlacements | null | undefined,
	owned: boolean,
): AppCloudCell => {
	if (!placements) return { state: "unknown" };
	const placement = placements.placements.find(
		(value) =>
			value.device_id === view.deviceId &&
			value.placement_id === view.serviceId,
	);
	if (placement) return { state: "approved", placement };
	return mode === "online" && !owned ? { state: "hidden" } : { state: "none" };
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
	const stagedHash =
		staged && lastChange?.kind === "update" ? lastChange.hash : undefined;
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
		...(staged
			? {
					stagedVersion:
						versions.find((value) => sameHash(stagedHash, value.hash)) ?? null,
				}
			: {}),
		data:
			mode === "online"
				? { where: "cloud" }
				: {
						where: "device",
						...(lastChange?.seededAt ? { since: lastChange.seededAt } : {}),
					},
		writes: view.offlineWrites ?? null,
		cloud: cloudCell(
			view,
			mode,
			input.placements,
			device.relationship === "owner",
		),
		lastChange,
		lastKnown: !CURRENT_AGES.includes(view.freshness.age),
		rank: serviceRank(view, staged),
	};
}

function withUploads(
	rows: AppServiceRow[],
	uploads: readonly AppUploadInput[],
): AppServiceRow[] {
	if (!uploads.length) return rows;
	return rows.map((row, index) => {
		const upload =
			uploads.find((value) => value.serviceId === row.serviceId) ??
			(index === 0 ? uploads.find((value) => !value.serviceId) : undefined);
		return upload ? { ...row, upload } : row;
	});
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
	noAccess: boolean,
): MatrixCell {
	const base = { deviceId: device.id, serviceIds: [] as string[] };
	const serving = (group?.services ?? []).filter((row) =>
		row.events?.some((value) => value.event_id === event.id),
	);
	if (noAccess && !serving.length) return { ...base, state: "no_access" };
	if (!readable(device)) {
		const unknown = group?.unknown ?? appUnknownOf(device);
		// Nothing runs on a device that never checked in: not served, with its deploy gate.
		if (unknown.kind === "never")
			return { ...base, state: "not_served", gate: device.deployGate ?? null };
		return unknown.kind === "noaccess"
			? { ...base, state: "no_access" }
			: { ...base, state: "unknown", unknown };
	}
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
				input.noAccess?.includes(id) ?? false,
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
		never: [],
		noAccess: [],
	};
	const coverage: AppCoverage = {
		total: devices.length,
		readable: 0,
		unknown: 0,
		locked: [],
		never: [],
	};
	const noAccess = new Set(input.noAccess ?? []);
	for (const device of devices) {
		const views = readable(device);
		const group = {
			deviceId: device.id,
			name: device.name,
			presence: device.presence,
			relationship: device.relationship,
		};
		const runsApp = views?.some((view) => view.projectId === app.id) ?? false;
		if (
			noAccess.has(device.id) &&
			!runsApp &&
			device.presence.kind !== "never"
		) {
			if (views) coverage.readable++;
			else coverage.unknown++;
			everywhereElse.noAccess.push({
				...everywhereRow(device, app.id, { kind: "noaccess" }),
				gate: null,
			});
			continue;
		}
		if (!views) {
			const unknown = appUnknownOf(device);
			coverage.unknown++;
			if (unknown.kind === "never") {
				coverage.never.push(device.id);
				everywhereElse.never.push(everywhereRow(device, app.id, unknown));
				continue;
			}
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
		const services = withUploads(
			views
				.filter((view) => view.projectId === app.id)
				.map((view) => serviceRow(device, view, input, mode, versions))
				.sort(byRankThenName((row) => row.serviceId)),
			(input.uploads ?? []).filter((upload) => upload.deviceId === device.id),
		);
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
				: coverage.readable || !everywhereElse.unknown.length
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
					unknown: services.filter((row) => !row.version).length,
				}
			: null,
	};
}
