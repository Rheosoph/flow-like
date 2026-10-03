import {
	type AppServiceRow,
	type AppVersionView,
	type AppView,
	type LocalServiceChange,
	type MatrixRow,
	isClaimedKind,
} from "../../../../lib/device-management/model/app-plan";
import { versionName } from "../../../../lib/device-management/model/app-versions";
import {
	type DeployPlan,
	planLatestFlows,
} from "../../../../lib/device-management/model/deploy-plan";
import { rolloutEndsAt } from "../../../../lib/device-management/model/device-view";
import type { HeadlineApp } from "../../../../lib/device-management/model/headline";
import type {
	AppDevicePlacements,
	GateFailure,
	OnlineAccess,
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

export { versionName };

/* This computer's records (BG12, BG-A4 interim): the activity tray is the only log of what was sent. */

const seconds = (ms: number) => Math.floor(ms / 1000);

const CHANGE_KIND: Partial<
	Record<ActivityItem["kind"], LocalServiceChange["kind"]>
> = { safe_update: "update", upload: "deploy" };

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

/**
 * Eligible events no readable device serves; an unknown cell keeps an event
 * out (unknown ≠ not deployed). A schedule or a bot is never one: the hub or
 * the desktop app runs it until someone moves it to a device.
 */
export function eventsNowhere(view: AppView): string[] {
	if (!view.services.length) return [];
	return view.events.rows
		.filter(
			(row) =>
				!isClaimedKind(row.eligibility.kind) && servedSomewhere(row) === false,
		)
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
 * The flows an update turns into a version when it starts: an event that
 * follows Latest is updated to the flow as it is now, and a flow with edits
 * no published version holds gets one. Said before the update starts.
 */
export function flowsToPublish(plans: readonly DeployPlan[]): string[] {
	const boards = new Set<string>();
	for (const plan of plans)
		for (const { eventId, boardId } of planLatestFlows(plan)) {
			const event = plan.app?.events.find((row) => row.id === eventId);
			const flow = typeof event?.flow === "object" ? event.flow : null;
			if (flow && flow.current === null) boards.add(boardId);
		}
	return [...boards].sort();
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

export const pinText = (value: readonly number[]) => value.join(".");

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
