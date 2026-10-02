import { sha256 } from "@noble/hashes/sha2";
import { eventEligibility } from "../../../../lib/device-management/deployment";
import type {
	AppEventInput,
	AppServiceRow,
	AppVersionInput,
	AppVersionPin,
	AppVersionView,
	AppView,
	LocalServiceChange,
	MatrixRow,
	VersionDiffRow,
} from "../../../../lib/device-management/model/app-plan";
import { rolloutEndsAt } from "../../../../lib/device-management/model/device-view";
import type { HeadlineApp } from "../../../../lib/device-management/model/headline";
import type {
	AppDevicePlacements,
	GateFailure,
	OnlineAccess,
	PlacementEvent,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { OfflineQueueStatus } from "../../../../lib/device-management/offline-queue";
import type {
	ActivityItem,
	ActivityRun,
} from "../../../../lib/device-management/workspace/types";
import type { DeviceResources } from "../../../../lib/device-resources";

/*
 * App › Devices facts the W1 model has no source for yet (plan §4.3 W3-N4
 * "Model state and gaps"). Pure; folded into `DM/model/app-plan.ts` by W4.
 */

export const VERSION_CAP = 4;
export const ATTENTION_CAP = 5;
export const GROUP_CAP = 8;
export const UNKNOWN_GROUP_CAP = 2;
export const ELSEWHERE_CAP = 5;
export const EVENT_NAME_CAP = 2;
/** Services listed under "Runs on" before "and N more services". */
export const VERSION_RUN_CAP = 6;
/** Unknown devices named one by one in a version row; more become one sentence. */
export const UNKNOWN_FOLD = 2;
/** Rows of a table or card list before "Show N more". */
export const ROW_CAP = 8;
export const HIDDEN_APPROVAL_CAP = 3;
/** Names spelled out in a "N more: …" line. */
export const NAME_CAP = 3;
/** Names spelled out inside a sentence. */
export const SENTENCE_NAME_CAP = 4;

type Pin = Omit<AppVersionPin, "eventId">;
type Triple = readonly number[];

const sameTriple = (a: Triple, b: Triple) =>
	a.length === b.length && a.every((value, index) => value === b[index]);

const samePin = (a: Pin, b: Pin) =>
	sameTriple(a.eventVersion, b.eventVersion) &&
	sameTriple(a.boardVersion, b.boardVersion);

const compareTriple = (a: Triple, b: Triple) => {
	for (let index = 0; index < Math.max(a.length, b.length); index++) {
		const delta = (a[index] ?? 0) - (b[index] ?? 0);
		if (delta) return delta;
	}
	return 0;
};

const pinOf = (event: PlacementEvent): AppVersionPin => ({
	eventId: event.event_id,
	eventVersion: event.event_version,
	boardVersion: event.board_version,
});

/** "1.5.0" → "v1.5.0"; any other text is shown as the app wrote it. */
export function versionLabel(text: string | null | undefined): string | null {
	const value = text?.trim();
	if (!value) return null;
	return /^\d/.test(value) ? `v${value}` : value;
}

/** A7: the newest app version pins every event that can run on a device at the versions published now. */
export function newestPins(events: readonly AppEventInput[]): AppVersionPin[] {
	return events.flatMap((event) => {
		const rule = eventEligibility(event, {
			ineligibleReason: event.ineligibleReason,
		});
		return rule.eligible && rule.eventVersion && rule.boardVersion
			? [
					{
						eventId: event.id,
						eventVersion: rule.eventVersion,
						boardVersion: rule.boardVersion,
					},
				]
			: [];
	});
}

/** A stable identifier of a pin set, the same on every computer. */
export function definitionHash(pins: readonly AppVersionPin[]): string {
	const rows = [...pins]
		.sort((a, b) => a.eventId.localeCompare(b.eventId))
		.map((pin) => [pin.eventId, pin.eventVersion, pin.boardVersion]);
	const bytes = new TextEncoder().encode(JSON.stringify(rows));
	return Array.from(sha256(bytes), (byte) =>
		byte.toString(16).padStart(2, "0"),
	).join("");
}

export interface VersionSources {
	/** The app's own version text. */
	label?: string | null;
	/** Unix seconds the app last changed. */
	changedAt?: number | null;
	events: readonly AppEventInput[];
	/** Readable services of this app. */
	services: readonly Pick<ServiceView, "appVersion" | "events">[];
	/** Unix seconds this computer sent a revision, by hash. */
	sentAt?: Readonly<Record<string, number>>;
}

interface Observed {
	hash: string;
	pins: Map<string, AppVersionPin>;
}

function observedRevisions(services: VersionSources["services"]): Observed[] {
	const found = new Map<string, Observed>();
	for (const service of services) {
		const hash = service.appVersion?.hash;
		if (!hash || !service.events?.length) continue;
		const entry = found.get(hash) ?? { hash, pins: new Map() };
		for (const event of service.events)
			entry.pins.set(event.event_id, pinOf(event));
		found.set(hash, entry);
	}
	return [...found.values()].sort((a, b) => a.hash.localeCompare(b.hash));
}

/** Every pin the revision serves is the newest one of its event. */
function isCurrent(
	pins: Iterable<AppVersionPin>,
	newest: ReadonlyMap<string, AppVersionPin>,
): boolean {
	let seen = 0;
	for (const pin of pins) {
		const target = newest.get(pin.eventId);
		if (!target || !samePin(pin, target)) return false;
		seen++;
	}
	return seen > 0;
}

/** +1 when `pin` is the newer of the two, −1 when it is the older one. */
const pinOrder = (pin: Pin, other: Pin) =>
	Math.sign(compareTriple(pin.eventVersion, other.eventVersion)) ||
	Math.sign(compareTriple(pin.boardVersion, other.boardVersion));

/** How many of the events both revisions serve are newer in `a` than in `b`, minus the older ones. */
function pinScore(a: Observed, b: Observed): number {
	let score = 0;
	for (const [eventId, pin] of a.pins) {
		const other = b.pins.get(eventId);
		if (other) score += pinOrder(pin, other);
	}
	return score;
}

/** Newer first: by when this computer sent them, else by the versions they pin. */
function byAge(sentAt: VersionSources["sentAt"] = {}) {
	const sent = (entry: Observed) => sentAt[entry.hash] ?? 0;
	return (a: Observed, b: Observed) =>
		sent(b) - sent(a) || -pinScore(a, b) || a.hash.localeCompare(b.hash);
}

/**
 * The hub keeps no history of app versions and a device reports only the
 * revision it runs. The list is therefore: what the app pins now, then every
 * older revision a readable service still runs, newest first.
 */
export function versionInputs(sources: VersionSources): AppVersionInput[] {
	const pins = newestPins(sources.events);
	const newest = new Map(pins.map((pin) => [pin.eventId, pin]));
	const observed = observedRevisions(sources.services);
	const current = observed.find((entry) =>
		isCurrent(entry.pins.values(), newest),
	);
	const older = observed
		.filter((entry) => !isCurrent(entry.pins.values(), newest))
		.sort(byAge(sources.sentAt));
	if (!pins.length && !older.length) return [];
	return [
		{
			hash: current?.hash ?? definitionHash(pins),
			label: versionLabel(sources.label),
			builtAt: sources.changedAt ?? null,
			by: null,
			pins,
		},
		...older.map((entry) => ({
			hash: entry.hash,
			label: null,
			builtAt: sources.sentAt?.[entry.hash] ?? null,
			by: null,
			pins: [...entry.pins.values()],
		})),
	];
}

function rowIsCurrent(row: AppServiceRow, newest: AppVersionView): boolean {
	if (!row.events?.length) return false;
	const pins = new Map(newest.pins.map((pin) => [pin.eventId, pin]));
	return isCurrent(row.events.map(pinOf), pins);
}

/**
 * An older revision is known only through the events its services serve, so
 * "new in" can't be told from "not served there": only changed pins count,
 * plus events the newest version no longer has.
 */
function knownDiff(
	version: AppVersionView,
	newestIndex: number,
): VersionDiffRow[] | null {
	if (!version.diff) return null;
	return version.diff.filter(
		(row) =>
			row.kind === "changed" ||
			(row.kind === "removed" && version.index === newestIndex),
	);
}

/**
 * Aligns the model's hash match with the pins: a service that serves the
 * newest version of each of its events runs the newest app version, whatever
 * revision carried it there.
 */
export function refineView(view: AppView): AppView {
	const newest = view.versions[0];
	if (!newest) return view;
	const versions = view.versions.map((version) => ({
		...version,
		diff: knownDiff(version, 0),
		runningOn: [] as AppVersionView["runningOn"],
	}));
	const byHash = new Map(versions.map((version) => [version.hash, version]));
	const refine = (row: AppServiceRow): AppServiceRow => {
		const version = rowIsCurrent(row, newest)
			? versions[0]
			: row.version
				? (byHash.get(row.version.hash) ?? null)
				: null;
		if (version)
			version.runningOn.push({
				deviceId: row.deviceId,
				serviceId: row.serviceId,
				conv: row.view.conv,
				lastKnown: row.lastKnown,
			});
		return { ...row, version, behind: version ? version.index : null };
	};
	const groups = view.groups.map((group) => ({
		...group,
		services: group.services.map(refine),
	}));
	const services = groups.flatMap((group) => group.services);
	const unknownOn = [
		...groups.filter((group) => group.unknown).map((group) => group.deviceId),
		...services
			.filter((row) => !row.version)
			.map((row) => `${row.deviceId}/${row.serviceId}`),
	];
	for (const version of versions) version.unknownOn = unknownOn;
	return {
		...view,
		groups,
		services,
		versions,
		howRuns: { ...view.howRuns, newest: versions[0] },
		events: {
			...view.events,
			rows: view.events.rows.map((row) => ({ ...row, newIn: null })),
			ineligible: view.events.ineligible.map((row) => ({
				...row,
				newIn: null,
			})),
		},
		newestRuns: {
			version: versions[0],
			services: versions[0].runningOn.length,
			of: services.length,
		},
	};
}

/**
 * A shared device whose access covers other apps only says nothing about this
 * app, locked or not: it is neither unknown-until-unlocked nor not deployed,
 * it is "no access" (APP §2.12). The model sees only the device's key state.
 */
export function withoutAccess(
	view: AppView,
	noAccess: readonly string[],
): AppView {
	const ids = new Set(noAccess);
	const { everywhereElse } = view;
	const moved = [...everywhereElse.unknown, ...everywhereElse.notDeployed]
		.filter((row) => ids.has(row.deviceId))
		.map((row) => ({
			...row,
			gate: null,
			unknown: { kind: "noaccess" as const },
		}));
	if (!moved.length) return view;
	const unknown = everywhereElse.unknown.filter(
		(row) => !ids.has(row.deviceId),
	);
	const groups = view.groups.filter(
		(group) => !(group.unknown && ids.has(group.deviceId)),
	);
	const listed = new Set(everywhereElse.noAccess.map((row) => row.deviceId));
	return {
		...view,
		groups,
		layout:
			view.layout === "all_unknown" && !unknown.length ? "never" : view.layout,
		versions: view.versions.map((version) => ({
			...version,
			unknownOn: version.unknownOn.filter((key) => !ids.has(key)),
		})),
		events: {
			...view.events,
			rows: view.events.rows.map((row) => ({
				...row,
				cells: Object.fromEntries(
					Object.entries(row.cells).map(([deviceId, cell]) => [
						deviceId,
						ids.has(deviceId) && cell.serviceIds.length === 0
							? {
									state: "no_access" as const,
									deviceId,
									serviceIds: [],
								}
							: cell,
					]),
				),
			})),
		},
		everywhereElse: {
			...everywhereElse,
			unknown,
			notDeployed: everywhereElse.notDeployed.filter(
				(row) => !ids.has(row.deviceId),
			),
			noAccess: [
				...everywhereElse.noAccess,
				...moved.filter((row) => !listed.has(row.deviceId)),
			],
		},
	};
}

/** "v1.5.0", or the short hash when the version has no name. */
export function versionName(
	version: Pick<AppVersionView, "label" | "short">,
): string {
	return version.label ?? version.short;
}

/* This computer's records (BG12, BG-A4 interim): the activity tray is the only log of what was sent. */

const seconds = (ms: number) => Math.floor(ms / 1000);

const CHANGE_KIND: Partial<
	Record<ActivityItem["kind"], LocalServiceChange["kind"]>
> = { safe_update: "update", upload: "deploy" };

/** When this computer sent each revision of the app, by hash. */
export function revisionsSent(
	items: readonly ActivityItem[],
	appId: string,
): Record<string, number> {
	const sent: Record<string, number> = {};
	for (const item of items) {
		const handle = item.resume;
		if (handle?.type !== "transfer" || handle.projectId !== appId) continue;
		if (item.state !== "done" || item.finishedAt === undefined) continue;
		const at = seconds(item.finishedAt);
		sent[handle.manifestSha256] = Math.max(
			sent[handle.manifestSha256] ?? 0,
			at,
		);
	}
	return sent;
}

/** The last change this computer made to each service of the app, keyed `${deviceId}/${serviceId}`. */
export function changesOf(
	items: readonly ActivityItem[],
	appId: string,
	me: string,
): Record<string, LocalServiceChange> {
	const changes: Record<string, LocalServiceChange> = {};
	const seeded: Record<string, number> = {};
	const done = items
		.filter(
			(item) =>
				item.state === "done" &&
				item.finishedAt !== undefined &&
				item.target.projectId === appId &&
				!!item.target.serviceId,
		)
		.sort((a, b) => (a.finishedAt ?? 0) - (b.finishedAt ?? 0));
	for (const item of done) {
		const kind =
			item.detail?.code === "rolled_back" ? "rollback" : CHANGE_KIND[item.kind];
		if (!kind) continue;
		const key = `${item.target.deviceId}/${item.target.serviceId}`;
		const at = seconds(item.finishedAt ?? 0);
		if (item.kind === "upload") seeded[key] ??= at;
		const handle = item.resume;
		changes[key] = {
			at,
			kind,
			by: me,
			...(handle?.type === "transfer" ? { hash: handle.manifestSha256 } : {}),
			...(seeded[key] === undefined ? {} : { seededAt: seeded[key] }),
		};
	}
	return changes;
}

/** The app version a staged update switches to, when this computer recorded it (the device's rollout status has none). */
export function stagedVersionOf(
	row: Pick<AppServiceRow, "staged" | "lastChange">,
	versions: readonly AppVersionView[],
): AppVersionView | null {
	if (!row.staged) return null;
	const hash = row.lastChange?.kind === "update" ? row.lastChange.hash : null;
	if (!hash) return null;
	return versions.find((version) => version.hash === hash) ?? null;
}

/** When the device discards a staged update on its own (unix seconds). */
export function stagedEndsAt(row: Pick<AppServiceRow, "view">) {
	const rollout = row.view.rollout;
	return rollout ? rolloutEndsAt(rollout) : undefined;
}

export interface ServiceUpload {
	item: ActivityItem;
	state: "active" | "paused";
	done: number;
	total: number;
	/** Unix seconds the device keeps the partial upload. */
	expiresAt: number;
}

const isTransfer = (item: ActivityItem, appId: string) =>
	item.resume?.type === "transfer" && item.resume.projectId === appId;

/** Uploads of this app that are running or can be resumed, newest first. */
export function appUploads(
	items: readonly ActivityItem[],
	appId: string,
): ServiceUpload[] {
	return items
		.filter(
			(item) =>
				isTransfer(item, appId) &&
				(item.state === "active" ||
					item.state === "paused" ||
					item.state === "waiting"),
		)
		.sort((a, b) => b.updatedAt - a.updatedAt)
		.map((item) => {
			const progress =
				typeof item.progress === "object" ? item.progress : undefined;
			return {
				item,
				state: item.state === "paused" ? "paused" : "active",
				done: progress?.done ?? 0,
				total: progress?.total ?? 0,
				expiresAt: item.resume?.type === "transfer" ? item.resume.expiresAt : 0,
			};
		});
}

/** The upload shown on a service row: the one for it, else the app's upload on the first service of that device. */
export function uploadOf(
	row: Pick<AppServiceRow, "deviceId" | "serviceId">,
	services: readonly Pick<AppServiceRow, "deviceId" | "serviceId">[],
	uploads: readonly ServiceUpload[],
): ServiceUpload | null {
	const onDevice = uploads.filter(
		(upload) => upload.item.target.deviceId === row.deviceId,
	);
	const own = onDevice.find(
		(upload) => upload.item.target.serviceId === row.serviceId,
	);
	if (own) return own;
	const first = services.find((service) => service.deviceId === row.deviceId);
	if (first?.serviceId !== row.serviceId) return null;
	return onDevice.find((upload) => !upload.item.target.serviceId) ?? null;
}

/* Headline input (APP §7.4): app facts the device model doesn't hold. */

function servedSomewhere(row: MatrixRow): boolean | null {
	const cells = Object.values(row.cells);
	if (cells.some((cell) => cell.state === "served" || cell.state === "staged"))
		return true;
	return cells.some((cell) => cell.state === "unknown") ? null : false;
}

/** Eligible events no readable device serves; an unknown cell keeps an event out (unknown ≠ not deployed). */
export function eventsNowhere(view: AppView): string[] {
	if (!view.services.length) return [];
	return view.events.rows
		.filter((row) => servedSomewhere(row) === false)
		.map((row) => row.name);
}

export function headlineApp(view: AppView): HeadlineApp {
	const newest = view.versions[0];
	const older = view.services.filter((row) => (row.behind ?? 0) > 0).length;
	return {
		appId: view.app.id,
		name: view.app.name,
		localOnly: view.app.localOnly,
		events: {
			total: view.events.rows.length + view.events.ineligible.length,
			eligible: view.events.rows.length,
			nowhere: eventsNowhere(view),
		},
		...(newest ? { latestLabel: versionName(newest) } : {}),
		...(older ? { olderServices: older } : {}),
	};
}

/* Update everywhere (APP §2.16). */

export interface UpdateRow {
	deviceId: string;
	serviceId: string;
	from: AppVersionView | null;
	/** Why this row can't be ticked now: an update is switching over, or one is staged. */
	blocked: "busy" | "staged" | null;
	/** Unix seconds a running update ends by. */
	busyUntil?: number;
}

const BUSY = new Set(["update_in_progress", "converging"]);

/** Services that don't run the newest version; one whose version is unknown is offered too. */
export function updateRows(view: AppView): UpdateRow[] {
	return view.services
		.filter((row) => row.behind !== 0)
		.map((row) => {
			const busy = !row.staged && BUSY.has(row.view.conv);
			const until =
				busy && row.view.rollout ? rolloutEndsAt(row.view.rollout) : undefined;
			return {
				deviceId: row.deviceId,
				serviceId: row.serviceId,
				from: row.version,
				blocked: row.staged ? "staged" : busy ? "busy" : null,
				...(until ? { busyUntil: until } : {}),
			};
		});
}

export type UpdateAllGate =
	| "no_devices"
	| "unreadable"
	| "nothing_deployed"
	| "run_active"
	| "all_newest"
	| "all_staged"
	| null;

/**
 * The header button's gates, in the order of APP §2.4; a running update only
 * gates its own row in the sheet. While no device is readable the reason is
 * "unlock first", never "not on any device" (unknown ≠ not deployed).
 */
export function updateAllGate(
	view: AppView,
	runActive: boolean,
): UpdateAllGate {
	if (view.layout === "no_devices") return "no_devices";
	if (view.layout === "all_unknown") return "unreadable";
	if (!view.services.length) return "nothing_deployed";
	if (runActive) return "run_active";
	const rows = updateRows(view);
	if (!rows.length) return "all_newest";
	return rows.every((row) => row.blocked === "staged") ? "all_staged" : null;
}

/**
 * One plan updates one service per device, so the ticked services are split
 * into as few plans as that allows: the n-th service of every device shares one.
 */
export function updateBatches<T extends { deviceId: string }>(
	rows: readonly T[],
): T[][] {
	const batches: T[][] = [];
	const seen = new Map<string, number>();
	for (const row of rows) {
		const index = seen.get(row.deviceId) ?? 0;
		seen.set(row.deviceId, index + 1);
		batches[index] ??= [];
		batches[index].push(row);
	}
	return batches;
}

/* In progress (APP §2.8). */

const RUN_OPEN = new Set(["active", "paused", "waiting"]);

export interface AppRun {
	run: ActivityRun;
	items: ActivityItem[];
	done: number;
	total: number;
	open: boolean;
}

/** Every tracked run with its items of this app. */
function runEntries(
	runs: readonly ActivityRun[],
	items: readonly ActivityItem[],
	appId: string,
): AppRun[] {
	return runs.map((run) => {
		const ids = new Set(run.itemIds);
		const own = items.filter(
			(item) => ids.has(item.id) && item.target.projectId === appId,
		);
		return {
			run,
			items: own,
			done: own.filter((item) => item.state === "done").length,
			total: own.length,
			open: own.some((item) => RUN_OPEN.has(item.state)),
		};
	});
}

/** Multi-device runs of this app that this computer tracks, newest first; `items` is the tray's list. */
export function appRuns(
	runs: readonly ActivityRun[],
	items: readonly ActivityItem[],
	appId: string,
): AppRun[] {
	return runEntries(runs, items, appId)
		.filter((entry) => entry.total > 1 && entry.open)
		.sort((a, b) => b.run.updatedAt - a.run.updatedAt);
}

/** Runs of this app that still have work open, a run on one device included. */
export function openRunIds(
	runs: readonly ActivityRun[],
	items: readonly ActivityItem[],
	appId: string,
): Set<string> {
	return new Set(
		runEntries(runs, items, appId)
			.filter((entry) => entry.open)
			.map((entry) => entry.run.id),
	);
}

/* Lists (R10, R11). */

export interface Capped<T> {
	shown: T[];
	rest: T[];
}

export function capList<T>(list: readonly T[], cap: number): Capped<T> {
	return { shown: list.slice(0, cap), rest: list.slice(cap) };
}

/** The first `cap` names and how many were left out: "a, b, c and 4 more". */
export function capNames(
	names: readonly string[],
	cap: number,
): { names: string[]; more: number } {
	return names.length <= cap
		? { names: [...names], more: 0 }
		: { names: names.slice(0, cap), more: names.length - cap };
}

/** Event names of a service, in the app's event order; ids the app no longer has stay as ids. */
export function eventNames(
	row: Pick<AppServiceRow, "events">,
	names: ReadonlyMap<string, string>,
): string[] | null {
	if (!row.events) return null;
	return row.events.map((event) => names.get(event.event_id) ?? event.event_id);
}

/** Other services of the app that keep running when `row` stops, as "service on device". */
export function runsElsewhere(
	view: AppView,
	row: Pick<AppServiceRow, "deviceId" | "serviceId">,
): { serviceId: string; deviceId: string }[] {
	return view.services
		.filter(
			(other) =>
				!(
					other.deviceId === row.deviceId && other.serviceId === row.serviceId
				) &&
				other.view.desired === "running" &&
				other.view.conv !== "crash_looping" &&
				other.view.conv !== "failed_stopped",
		)
		.map((other) => ({
			serviceId: other.serviceId,
			deviceId: other.deviceId,
		}));
}

export const pinText = (value: Triple) => value.join(".");

/** "€7.41", "€25": cents only when there are any, so two amounts fit a table cell. */
export function shortMoney(micros: number, locale: string): string {
	const value = micros / 1_000_000;
	return new Intl.NumberFormat(locale, {
		style: "currency",
		currency: "EUR",
		minimumFractionDigits: Number.isInteger(value) ? 0 : 2,
		maximumFractionDigits: 2,
	}).format(value);
}

/* Write buffer of an online service (APP §2.9 Data): the queues when they were read live, else the status summary. */

export interface WritesFacts {
	/** Changes waiting in queues that still send. */
	waiting: number;
	/** Queues whose next change conflicts with newer cloud data. */
	conflicts: number;
	/** Changes in queues that are paused because cloud access changed. */
	paused: number;
}

export function writesFacts(
	row: Pick<AppServiceRow, "writes">,
	queues: readonly OfflineQueueStatus[] | undefined,
): WritesFacts | null {
	if (queues) {
		const sending = queues.filter((queue) => !queue.quarantined);
		return {
			waiting: sending.reduce((sum, queue) => sum + queue.pending_count, 0),
			conflicts: sending.filter((queue) => queue.head?.state === "conflict")
				.length,
			paused: queues
				.filter((queue) => queue.quarantined)
				.reduce((sum, queue) => sum + queue.pending_count, 0),
		};
	}
	const writes = row.writes;
	if (typeof writes !== "object" || !writes) return null;
	return writes.quarantined
		? { waiting: 0, conflicts: 0, paused: writes.pending }
		: {
				waiting: writes.pending,
				conflicts: writes.head?.state === "conflict" ? 1 : 0,
				paused: 0,
			};
}

/* Gates on links into the deploy wizard: its Where step unlocks and connects, so those never block a link. */

const WIZARD_HANDLES = new Set([
	"locked_change",
	"unlock_to_check_permissions",
	"unlocking",
	"connect_first",
	"connecting",
	"service_must_be_stopped",
]);

export function blocksDeploy(
	gate: GateFailure | null | undefined,
): gate is GateFailure {
	return !!gate && !WIZARD_HANDLES.has(gate.copy.code);
}

/* Cloud access per service (APP §2.13): E20, or one read per device on an older hub (BG-A6a). */

export interface AppApproval {
	deviceId: string;
	serviceId: string;
	grantId: string;
	active: boolean;
	/** Unix seconds the approval really ends (the hub's effective end when it sends one). */
	endsAt: number;
	files: OnlineAccess | null;
	models: string[];
	maxInstances: number;
	approvedBy: string | null;
	/** Instances holding a cloud lease now; unknown on an older hub. */
	leases?: number;
	billing: {
		id: string;
		limit: number;
		used: number;
		reserved: number;
		endsAt: number;
		payerIsMe: boolean;
	} | null;
}

export type CloudState =
	| { state: "approved"; approval: AppApproval }
	| { state: "hidden" | "none" | "unknown" };

function fromResources(
	row: Pick<AppServiceRow, "deviceId" | "serviceId" | "mode">,
	appId: string,
	resources: DeviceResources,
	me: string,
	owned: boolean,
): CloudState {
	const grants = resources.grants
		.filter(
			(grant) =>
				grant.placement_id === row.serviceId &&
				(grant.app_id === appId || grant.project_id === appId),
		)
		.sort(
			(a, b) =>
				Number(b.status === "active") - Number(a.status === "active") ||
				b.expires_at - a.expires_at,
		);
	const grant = grants[0];
	if (!grant)
		return { state: row.mode === "online" && !owned ? "hidden" : "none" };
	const billing = resources.billing
		.filter((entry) => entry.grant_id === grant.grant_id)
		.sort(
			(a, b) =>
				Number(b.status === "active") - Number(a.status === "active") ||
				b.expires_at - a.expires_at,
		)[0];
	return {
		state: "approved",
		approval: {
			deviceId: row.deviceId,
			serviceId: row.serviceId,
			grantId: grant.grant_id,
			active: grant.status === "active",
			endsAt: grant.effective_expires_at ?? grant.expires_at,
			files: grant.online_access ?? null,
			models: grant.model_ids,
			maxInstances: grant.max_instances,
			approvedBy: grant.approved_by_user_id ?? grant.delegating_user_id ?? null,
			leases: resources.instances.filter(
				(lease) => lease.grant_id === grant.grant_id,
			).length,
			billing:
				billing && billing.status === "active"
					? {
							id: billing.billing_grant_id,
							limit: billing.limit_micros,
							used: billing.used_micros,
							reserved: billing.reserved_micros,
							endsAt: billing.expires_at,
							payerIsMe: billing.payer_id === me,
						}
					: null,
		},
	};
}

/** An approval as the hub's per-app list (E20) reports it. */
function fromPlacement(
	row: Pick<AppServiceRow, "deviceId" | "serviceId">,
	placement: AppDevicePlacements["placements"][number],
): AppApproval {
	const { grant, billing, instances } = placement;
	return {
		deviceId: row.deviceId,
		serviceId: row.serviceId,
		grantId: grant.grant_id,
		active: grant.status === "active",
		endsAt: grant.effective_expires_at || grant.expires_at,
		files: grant.online_access,
		models: grant.model_ids,
		maxInstances: grant.max_instances,
		approvedBy: grant.approved_by_user_id,
		leases: instances.active,
		billing: billing
			? {
					id: billing.billing_grant_id,
					limit: billing.limit_micros,
					used: billing.used_micros,
					reserved: billing.reserved_micros,
					endsAt: billing.expires_at,
					payerIsMe: billing.payer_is_me,
				}
			: null,
	};
}

/**
 * What the viewer may know about a service's cloud access. On a device the
 * viewer owns the hub lists every approval, so none listed means none given;
 * on a shared device it may be someone else's, which the viewer can't see.
 */
export function cloudOf(
	row: Pick<AppServiceRow, "deviceId" | "serviceId" | "mode" | "cloud">,
	appId: string,
	resources: DeviceResources | undefined,
	me: string,
	owned = false,
): CloudState {
	const cell = row.cloud;
	if (cell.state === "approved")
		return { state: "approved", approval: fromPlacement(row, cell.placement) };
	const state = cell.state === "hidden" && owned ? "none" : cell.state;
	// E20 lists only approvals with an app: the model access of a local-only app is in the device's own list.
	if (state === "hidden" || !resources) return { state };
	const own = fromResources(row, appId, resources, me, owned);
	return own.state === "approved" || state === "unknown" ? own : { state };
}

/** Every service's cloud access, in the order of the page's service rows. */
export function approvalsOf(
	view: Pick<AppView, "services" | "app">,
	resources: (deviceId: string) => DeviceResources | undefined,
	me: string,
	owned: (deviceId: string) => boolean = () => false,
): { row: AppServiceRow; cloud: CloudState }[] {
	return view.services.map((row) => ({
		row,
		cloud: cloudOf(
			row,
			view.app.id,
			resources(row.deviceId),
			me,
			owned(row.deviceId),
		),
	}));
}
