import { z } from "zod";
import {
	type PendingArtifactTransfer,
	legacyArtifactTransferDevices,
	readArtifactTransfer,
	takeLegacyArtifactTransfers,
} from "../artifacts";
import { readCertificateRequests } from "../certificate-issuance";
import { readDeploymentRollout } from "../deployment";
import type { CopyRef, NavTarget } from "../model/types";
import { accountStorageKey } from "../storage";
import type { ManagementCall } from "../telemetry";
import { type PolicyView, managementRejection } from "../types";
import type {
	ActivityAction,
	ActivityDetailCode,
	ActivityItem,
	ActivityKind,
	ActivityState,
	ActivityTracker,
	HubPort,
	KeyPort,
	LivePort,
	ResumeHandle,
	WorkspaceDeps,
} from "./types";

/**
 * The persisted activity tray (M-DATA §3.9, IA §6.3.5, SPEC §3.6, R9).
 * Item times are milliseconds; resume handle times (`issuedAt`, `expiresAt`) are unix seconds, like the device's.
 * Only typed fields are stored: ids, versions and names, never secrets.
 */

export type ActivityStart = Omit<
	ActivityItem,
	"id" | "startedAt" | "updatedAt"
>;

/** One deploy across several devices: one item per target, restored after a reload by its id (`useDeployRun` `resumeId`). */
export interface ActivityRun {
	id: string;
	title: CopyRef<string>;
	/** In target order. */
	itemIds: string[];
	oneAtATime: boolean;
	stopOnFail: boolean;
	startedAt: number;
	updatedAt: number;
}

export interface ActivityRunTracker extends ActivityTracker {
	startRun(run: {
		title: CopyRef<string>;
		oneAtATime: boolean;
		stopOnFail: boolean;
		items: ActivityStart[];
	}): ActivityRun;
	run(runId: string): ActivityRun | undefined;
	/** Every item of the run still kept, dismissed ones included, in target order. */
	runItems(runId: string): ActivityItem[];
	runs(): ActivityRun[];
}

export const ACTIVITY_STORAGE_PREFIX = "flow-like.device-activity.";
const MAX_ITEMS = 100;
const FINISHED_TTL_MS = 7 * 86_400_000;
const JOURNAL_TTL_S = 86_400;

const KINDS: Record<ActivityKind, true> = {
	safe_update: true,
	upload: true,
	access_rules: true,
	account_backup: true,
	agent_update: true,
	reboot: true,
	command: true,
	secret_write: true,
	history_readers: true,
	metric_readers: true,
	offline_write_retry: true,
	signing_request: true,
	setup: true,
};
const STATES: Record<ActivityState, true> = {
	active: true,
	paused: true,
	waiting: true,
	done: true,
	failed: true,
	unknown: true,
};
const ACTIONS: Record<ActivityAction, true> = {
	open: true,
	check_again: true,
	activate: true,
	discard: true,
	resume: true,
	cancel: true,
	dismiss: true,
};
const DETAILS: Record<ActivityDetailCode, true> = {
	files_progress: true,
	bytes_progress: true,
	instances_progress: true,
	resumable_until: true,
	waiting_for_device: true,
	waiting_for_apply: true,
	waiting_for_heartbeat: true,
	waiting_for_backup: true,
	no_reply: true,
	failed: true,
	rolled_back: true,
	rejected: true,
	cancelled: true,
	done: true,
};
const codes = <T extends string>(record: Record<T, true>) =>
	Object.keys(record) as [T, ...T[]];

const id = z.string().min(1).max(256);
const name = z.string().max(256);
const time = z.number().finite().nonnegative();
const copy = <T extends z.ZodTypeAny>(code: T) =>
	z.object({
		code,
		params: z
			.record(z.union([z.string().max(512), z.number().finite()]))
			.optional(),
	});

const handleSchema = z.discriminatedUnion("type", [
	z.object({
		type: z.literal("operation"),
		operationId: id,
		command: name,
		issuedAt: time,
	}),
	z.object({
		type: z.literal("rollout"),
		rolloutId: id,
		placementId: id,
		projectId: id,
	}),
	z.object({
		type: z.literal("transfer"),
		transferId: id,
		projectId: id,
		manifestSha256: z.string().regex(/^[a-f0-9]{64}$/u),
		expiresAt: time,
		confirmed: z.boolean().optional(),
	}),
	z.object({
		type: z.literal("host_operation"),
		kind: z.enum(["reboot", "update_agent"]),
		operationId: id,
		bootIdBefore: name.nullable(),
	}),
	z.object({ type: z.literal("policy"), version: time }),
	z.object({ type: z.literal("account_backup"), revision: time }),
	z.object({
		type: z.literal("secret"),
		operationId: id,
		placementId: id,
		name,
	}),
	z.object({ type: z.literal("csr"), requestId: id }),
	z.object({
		type: z.literal("setup"),
		enrollmentId: id,
		deviceId: id.optional(),
	}),
]) satisfies z.ZodType<ResumeHandle>;

/** Routes are validated by the routing layer; here only their plain-data shape. */
const routeSchema = z
	.object({ screen: z.string().max(32) })
	.catchall(
		z.union([name, z.number().finite(), z.boolean(), z.array(name).max(64)]),
	);

const itemSchema = z.object({
	id,
	kind: z.enum(codes(KINDS)),
	target: z.object({
		deviceId: id,
		deviceName: name.optional(),
		serviceId: id.optional(),
		projectId: id.optional(),
	}),
	state: z.enum(codes(STATES)),
	label: copy(z.enum(codes(KINDS))),
	detail: copy(z.enum(codes(DETAILS))).optional(),
	progress: z
		.union([
			z.literal("indeterminate"),
			z.object({
				done: z.number().finite().nonnegative(),
				total: z.number().finite().nonnegative(),
				unit: z.enum(["files", "bytes", "instances", "seconds"]),
			}),
		])
		.optional(),
	deadlineAt: time.optional(),
	startedAt: time,
	updatedAt: time,
	finishedAt: time.optional(),
	startedBy: z.literal("you"),
	resume: handleSchema.optional(),
	actions: z.array(z.enum(codes(ACTIONS))).max(8),
	href: routeSchema.optional(),
	dismissed: z.boolean().optional(),
});

const runSchema: z.ZodType<ActivityRun> = z.object({
	id,
	title: copy(z.string().min(1).max(128)),
	itemIds: z.array(id).max(MAX_ITEMS),
	oneAtATime: z.boolean(),
	stopOnFail: z.boolean(),
	startedAt: time,
	updatedAt: time,
});

/** Strips every field the contract does not name, so nothing else reaches storage. */
function parseItem(value: unknown): ActivityItem | undefined {
	const parsed = itemSchema.safeParse(value);
	return parsed.success
		? (parsed.data as Omit<typeof parsed.data, "href"> & {
				href?: NavTarget;
			})
		: undefined;
}

function strictItem(value: unknown, operation: string): ActivityItem {
	const item = parseItem(value);
	if (item) return item;
	const issue = itemSchema.safeParse(value).error?.issues[0];
	throw new Error(
		`Activity ${operation} was refused: ${issue?.path.join(".") ?? "item"} ${issue?.message ?? "is invalid"}.`,
	);
}

const FINISHED = new Set<ActivityState>(["done", "failed", "unknown"]);
export const isActivityFinished = (item: Pick<ActivityItem, "state">) =>
	FINISHED.has(item.state);

const JOURNAL_HANDLES = new Set<ResumeHandle["type"]>([
	"operation",
	"host_operation",
	"secret",
]);
const LIVE_HANDLES = new Set<ResumeHandle["type"]>([
	...JOURNAL_HANDLES,
	"rollout",
	"transfer",
	"csr",
]);

function journalIssuedAt(item: ActivityItem, handle: ResumeHandle): number {
	return handle.type === "operation"
		? handle.issuedAt
		: Math.floor(item.startedAt / 1000);
}

function finishedActions(
	item: ActivityItem,
	outcome: ActivityState,
): ActivityAction[] {
	const actions: ActivityAction[] = [];
	if (item.href || item.actions.includes("open")) actions.push("open");
	if (outcome === "unknown" && item.resume) actions.push("check_again");
	actions.push("dismiss");
	return actions;
}

function finished(
	item: ActivityItem,
	outcome: "done" | "failed" | "unknown",
	at: number,
	detail = item.detail,
): ActivityItem {
	return {
		...item,
		state: outcome,
		detail,
		actions: finishedActions(item, outcome),
		updatedAt: Math.max(item.updatedAt, at),
		finishedAt: item.state === outcome ? (item.finishedAt ?? at) : at,
	};
}

/** When the device stops being able to answer for the handle: journal records live 24 h, transfers until their expiry. */
function handleLapsedAt(item: ActivityItem, handle: ResumeHandle) {
	if (JOURNAL_HANDLES.has(handle.type))
		return (journalIssuedAt(item, handle) + JOURNAL_TTL_S) * 1000;
	return handle.type === "transfer"
		? handle.expiresAt * 1000
		: Number.POSITIVE_INFINITY;
}

/** Drops a lapsed handle; an unfinished item can no longer be checked, so it settles. */
function settleLapsed(
	item: ActivityItem,
	handle: ResumeHandle,
	lapsedAt: number,
) {
	const settled = { ...item, resume: undefined };
	if (item.state === "done" || item.state === "failed") return settled;
	return handle.type === "transfer"
		? finished(settled, "failed", lapsedAt, { code: "failed" })
		: finished(settled, "unknown", lapsedAt, { code: "no_reply" });
}

function expire(item: ActivityItem, nowMs: number) {
	const finishedAt = item.finishedAt ?? item.updatedAt;
	if (isActivityFinished(item) && finishedAt + FINISHED_TTL_MS < nowMs)
		return [];
	const handle = item.resume;
	if (!handle) return [item];
	const lapsedAt = handleLapsedAt(item, handle);
	return [lapsedAt < nowMs ? settleLapsed(item, handle, lapsedAt) : item];
}

/** Over the cap, the oldest finished results go first, then the oldest unfinished items. */
function prune(items: ActivityItem[], nowMs: number) {
	const kept = items.flatMap((item) => expire(item, nowMs));
	if (kept.length <= MAX_ITEMS) return kept;
	const keepLonger = (item: ActivityItem) => (isActivityFinished(item) ? 0 : 1);
	const drop = new Set(
		[...kept]
			.sort(
				(a, b) =>
					keepLonger(a) - keepLonger(b) ||
					(a.finishedAt ?? a.updatedAt) - (b.finishedAt ?? b.updatedAt),
			)
			.slice(0, kept.length - MAX_ITEMS)
			.map((item) => item.id),
	);
	return kept.filter((item) => !drop.has(item.id));
}

const ORDER: Record<ActivityState, number> = {
	active: 0,
	paused: 0,
	waiting: 0,
	unknown: 1,
	done: 2,
	failed: 2,
};
function trayOrder(a: ActivityItem, b: ActivityItem): number {
	const group = ORDER[a.state] - ORDER[b.state];
	if (group) return group;
	const at = (item: ActivityItem) =>
		ORDER[item.state] === 0 ? item.startedAt : (item.finishedAt ?? 0);
	return at(b) - at(a);
}

function pausedUpload(
	item: ActivityItem,
	handle: Extract<ResumeHandle, { type: "transfer" }>,
): Partial<ActivityItem> {
	return {
		state: "paused",
		detail: {
			code: "resumable_until",
			params: { until: handle.expiresAt * 1000 },
		},
		resume: handle,
		actions: [
			...(item.href || item.actions.includes("open") ? ["open" as const] : []),
			"resume",
			"discard",
		],
	};
}

interface Stored {
	items: ActivityItem[];
	runs: ActivityRun[];
}

function readStored(
	storageKey: string | undefined,
	nowMs: number,
): Stored | undefined {
	if (!storageKey) return undefined;
	try {
		const raw = globalThis.localStorage?.getItem(storageKey);
		if (raw == null) return undefined;
		const value = JSON.parse(raw) as { items?: unknown; runs?: unknown };
		const items = Array.isArray(value.items)
			? value.items.flatMap((entry) => parseItem(entry) ?? [])
			: [];
		const runs = Array.isArray(value.runs)
			? value.runs.flatMap((entry) => {
					const run = runSchema.safeParse(entry);
					return run.success ? [run.data] : [];
				})
			: [];
		return { items: prune(items, nowMs), runs };
	} catch {
		return undefined;
	}
}

/** False when storage is unavailable or full: the tray then lives on this page only. */
function writeStored(storageKey: string | undefined, next: Stored): boolean {
	if (!storageKey) return true;
	try {
		globalThis.localStorage?.setItem(
			storageKey,
			JSON.stringify({ v: 1, ...next }),
		);
		return true;
	} catch {
		return false;
	}
}

/** Runs only keep the items that are still kept. */
function settleRuns(runs: ActivityRun[], items: ActivityItem[]): ActivityRun[] {
	const kept = new Set(items.map((item) => item.id));
	return runs
		.map((run) => ({
			...run,
			itemIds: run.itemIds.filter((itemId) => kept.has(itemId)),
		}))
		.filter((run) => run.itemIds.length);
}

/* Resume: re-read one handle from the device (keys + live) or the hub. */

type Resolution =
	| {
			finish: "done" | "failed" | "unknown";
			detail?: ActivityItem["detail"];
	  }
	| { patch: Partial<ActivityItem> };

const DONE: Resolution = { finish: "done", detail: { code: "done" } };
const failedWith = (code: ActivityDetailCode): Resolution => ({
	finish: "failed",
	detail: { code },
});
const waitingFor = (code: ActivityDetailCode): Resolution => ({
	patch: { state: "waiting", detail: { code } },
});

type ResumePorts = { live: LivePort; keys: KeyPort; hub: HubPort };
type JournalLookup = { known: false } | { known: true; state: string };

/** The journal answers an unknown or expired id itself, with a coded `invalid` under the lookup's own id. */
async function lookupJournal(
	call: ManagementCall,
	operationId: string,
): Promise<JournalLookup> {
	const response = await call({
		type: "operation",
		operation_id: operationId,
	});
	if (response.operation_id === operationId)
		return { known: true, state: response.state };
	if (managementRejection(response)?.code === "invalid")
		return { known: false };
	throw new Error(
		`The device did not report operation ${operationId} (state ${response.state}).`,
	);
}

const JOURNAL_ENDS: Partial<Record<string, Resolution>> = {
	completed: DONE,
	failed: failedWith("failed"),
	rolled_back: failedWith("rolled_back"),
	rejected: failedWith("rejected"),
};

async function checkJournal(
	call: ManagementCall,
	operationId: string,
	waiting: ActivityDetailCode,
): Promise<Resolution> {
	const lookup = await lookupJournal(call, operationId);
	if (!lookup.known) return failedWith("failed");
	return JOURNAL_ENDS[lookup.state] ?? waitingFor(waiting);
}

const ROLLOUT_ENDS: Partial<Record<string, Resolution>> = {
	healthy: DONE,
	rolled_back: failedWith("rolled_back"),
	failed: failedWith("failed"),
	cancelled: failedWith("cancelled"),
};

async function checkRollout(
	call: ManagementCall,
	handle: Extract<ResumeHandle, { type: "rollout" }>,
): Promise<Resolution> {
	const status = await readDeploymentRollout(call, {
		rollout_id: handle.rolloutId,
		placement_id: handle.placementId,
		project_id: handle.projectId,
	});
	return (
		ROLLOUT_ENDS[status.state] ?? {
			patch: {
				state: "active",
				...(status.deadline_at
					? { deadlineAt: status.deadline_at * 1000 }
					: {}),
			},
		}
	);
}

async function checkTransfer(
	call: ManagementCall,
	item: ActivityItem,
	handle: Extract<ResumeHandle, { type: "transfer" }>,
): Promise<Resolution> {
	const status = await readArtifactTransfer(call, {
		transfer_id: handle.transferId,
		project_id: handle.projectId,
		manifest_sha256: handle.manifestSha256,
	});
	if (!status) return failedWith("failed");
	if (status.state === "committed") return DONE;
	if (status.state === "aborted") return failedWith("cancelled");
	return {
		patch: pausedUpload(item, {
			...handle,
			expiresAt: status.expires_at,
			confirmed: true,
		}),
	};
}

async function checkLive(
	call: ManagementCall,
	item: ActivityItem,
	handle: ResumeHandle,
): Promise<Resolution | undefined> {
	switch (handle.type) {
		case "operation":
			return checkJournal(call, handle.operationId, "waiting_for_apply");
		case "secret":
		case "host_operation":
			return checkJournal(call, handle.operationId, "waiting_for_device");
		case "rollout":
			return checkRollout(call, handle);
		case "transfer":
			return checkTransfer(call, item, handle);
		case "csr": {
			const requests = await readCertificateRequests(call);
			return requests.some((request) => request.request_id === handle.requestId)
				? { patch: { state: "waiting" } }
				: DONE;
		}
		default:
			return undefined;
	}
}

async function checkHub(
	hub: HubPort,
	item: ActivityItem,
	handle: ResumeHandle,
): Promise<Resolution | undefined> {
	const device = encodeURIComponent(item.target.deviceId);
	switch (handle.type) {
		case "policy": {
			const view = await hub.fetch<PolicyView>(
				`devices/${device}/management/policy`,
			);
			return view.applied_version >= handle.version
				? DONE
				: waitingFor("waiting_for_device");
		}
		case "account_backup": {
			const backup = await hub.fetch<{ revision?: unknown }>(
				`devices/controller-vaults/${device}`,
			);
			return Number(backup.revision) >= handle.revision
				? DONE
				: waitingFor("waiting_for_backup");
		}
		case "setup": {
			if (!handle.deviceId) return undefined;
			const row = await hub.fetch<{ last_seen_at?: unknown }>(
				`devices/${encodeURIComponent(handle.deviceId)}`,
			);
			return typeof row.last_seen_at === "number"
				? DONE
				: waitingFor("waiting_for_heartbeat");
		}
		default:
			return undefined;
	}
}

/** A reboot is over once the live session reports another OS boot. */
function rebooted(live: LivePort, item: ActivityItem): boolean {
	const handle = item.resume;
	if (handle?.type !== "host_operation" || handle.kind !== "reboot")
		return false;
	const state = live.state(item.target.deviceId);
	return (
		handle.bootIdBefore !== null &&
		state.kind === "live" &&
		state.bootId !== handle.bootIdBefore
	);
}

/** Undefined when the handle cannot be checked now (keys locked, nothing to ask). */
async function resolveHandle(
	ports: ResumePorts,
	item: ActivityItem,
	handle: ResumeHandle,
): Promise<Resolution | undefined> {
	if (rebooted(ports.live, item)) return DONE;
	if (!LIVE_HANDLES.has(handle.type)) return checkHub(ports.hub, item, handle);
	const deviceId = item.target.deviceId;
	if (ports.keys.snapshot(deviceId).state !== "unlocked") return undefined;
	const release = ports.live.acquire(deviceId, "operation");
	try {
		return await checkLive(
			ports.live.call(deviceId, { lane: "poll", idempotent: true }),
			item,
			handle,
		);
	} finally {
		release();
	}
}

/** A pre-C8 upload hint as a paused, resumable tray item. */
function legacyUploadItem(
	deviceId: string,
	transfer: PendingArtifactTransfer,
	at: number,
): ActivityItem {
	const base: ActivityItem = {
		id: crypto.randomUUID(),
		kind: "upload",
		target: { deviceId, projectId: transfer.project_id },
		state: "paused",
		label: { code: "upload" },
		startedAt: at,
		updatedAt: at,
		startedBy: "you",
		actions: [],
	};
	return strictItem(
		{
			...base,
			...pausedUpload(base, {
				type: "transfer",
				transferId: transfer.transfer_id,
				projectId: transfer.project_id,
				manifestSha256: transfer.manifest_sha256,
				expiresAt: Math.floor(transfer.expires_at),
				...(transfer.confirmed === undefined
					? {}
					: { confirmed: transfer.confirmed }),
			}),
		},
		"import",
	);
}

/** Adds the hints the tray does not track yet. */
function withLegacyUploads(
	items: ActivityItem[],
	deviceId: string,
	transfers: PendingArtifactTransfer[],
	at: number,
): ActivityItem[] {
	const known = new Set(
		items.flatMap((item) =>
			item.resume?.type === "transfer" ? [item.resume.transferId] : [],
		),
	);
	return [
		...items,
		...transfers
			.filter((transfer) => !known.has(transfer.transfer_id))
			.map((transfer) => legacyUploadItem(deviceId, transfer, at)),
	];
}

function storageKeyFor(deps: WorkspaceDeps): string | undefined {
	try {
		return `${ACTIVITY_STORAGE_PREFIX}${accountStorageKey(deps.scope)}`;
	} catch {
		return undefined;
	}
}

export function createActivityTracker(
	deps: WorkspaceDeps,
	ports: ResumePorts,
): ActivityRunTracker {
	const now = deps.now ?? Date.now;
	const storageKey = storageKeyFor(deps);
	/** Items started or updated by this tracker: a flow on this page drives them. */
	const driven = new Set<string>();
	const checking = new Set<string>();
	const listeners = new Set<() => void>();
	const finishListeners = new Set<(item: ActivityItem) => void>();
	let stored: Stored = readStored(storageKey, now()) ?? {
		items: [],
		runs: [],
	};
	let cache: ActivityItem[] | undefined;
	/** After a refused write the stored copy lacks this page's items, so it is not read back. */
	let saved = true;

	/** Announces items that reached a finished state; expiry bookkeeping is not announced. */
	function notify(previous: ActivityItem[], touched?: ReadonlySet<string>) {
		cache = undefined;
		const before = new Map(previous.map((item) => [item.id, item.state]));
		for (const item of stored.items)
			if (
				isActivityFinished(item) &&
				before.get(item.id) !== item.state &&
				(!touched || touched.has(item.id))
			)
				for (const listener of finishListeners) listener(item);
		for (const listener of listeners) listener();
	}

	/** Read-modify-write against storage, so other tabs' items survive this tab's changes. */
	function commit(
		touched: ReadonlySet<string>,
		mutate: (current: Stored) => Stored,
	): void {
		const previous = stored.items;
		const current = saved ? readStored(storageKey, now()) : undefined;
		const next = mutate(current ?? stored);
		const items = prune(next.items, now());
		stored = { items, runs: settleRuns(next.runs, items) };
		saved = writeStored(storageKey, stored);
		notify(previous, touched);
	}

	function patchItem(
		itemId: string,
		change: (item: ActivityItem) => ActivityItem | undefined,
	): void {
		commit(new Set([itemId]), (current) => ({
			...current,
			items: current.items.flatMap((item) =>
				item.id === itemId ? (change(item) ?? []) : [item],
			),
		}));
	}

	function onStorage(event: StorageEvent): void {
		if (event.key !== storageKey || !saved) return;
		const previous = stored.items;
		stored = readStored(storageKey, now()) ?? { items: [], runs: [] };
		notify(previous);
	}

	/** An upload nothing on this page drives any more is paused, not running. */
	function view(item: ActivityItem): ActivityItem {
		const handle = item.resume;
		return item.state === "active" &&
			handle?.type === "transfer" &&
			!driven.has(item.id)
			? { ...item, ...pausedUpload(item, handle) }
			: item;
	}

	function newItem(input: ActivityStart, at: number): ActivityItem {
		const item = strictItem(
			{ ...input, id: crypto.randomUUID(), startedAt: at, updatedAt: at },
			"start",
		);
		driven.add(item.id);
		return isActivityFinished(item)
			? { ...item, finishedAt: at, actions: finishedActions(item, item.state) }
			: item;
	}

	async function check(item: ActivityItem): Promise<void> {
		const handle = item.resume;
		if (!handle || checking.has(item.id)) return;
		checking.add(item.id);
		try {
			const resolution = await resolveHandle(ports, item, handle);
			if (!resolution) return;
			if ("finish" in resolution)
				tracker.finish(item.id, resolution.finish, resolution.detail);
			else
				patchItem(item.id, (current) => ({
					...current,
					...resolution.patch,
					updatedAt: now(),
				}));
		} catch {
			// A failed re-read leaves the item as it was; the next resume tries again.
		} finally {
			checking.delete(item.id);
		}
	}

	/** Pre-C8 upload hints move into the tray of the account that holds keys for that device. */
	function migrateLegacyTransfers(deviceId?: string): void {
		const devices = legacyArtifactTransferDevices().filter(
			(device) =>
				(deviceId === undefined || device === deviceId) &&
				ports.keys.snapshot(device).state !== "none",
		);
		for (const device of devices) {
			const transfers = takeLegacyArtifactTransfers(device);
			const at = now();
			if (transfers.length)
				commit(new Set(), (current) => ({
					...current,
					items: withLegacyUploads(current.items, device, transfers, at),
				}));
		}
	}

	function resumable(item: ActivityItem, deviceId?: string): boolean {
		return (
			item.resume !== undefined &&
			(deviceId === undefined || item.target.deviceId === deviceId) &&
			item.state !== "done" &&
			item.state !== "failed" &&
			!(item.state === "active" && driven.has(item.id))
		);
	}

	const tracker: ActivityRunTracker = {
		list() {
			cache ??= stored.items
				.filter((item) => !item.dismissed)
				.map(view)
				.sort(trayOrder);
			return cache;
		},
		subscribe(listener) {
			if (!listeners.size && typeof window !== "undefined")
				window.addEventListener("storage", onStorage);
			listeners.add(listener);
			return () => {
				listeners.delete(listener);
				if (!listeners.size && typeof window !== "undefined")
					window.removeEventListener("storage", onStorage);
			};
		},
		start(input) {
			const item = newItem(input, now());
			commit(new Set([item.id]), (current) => ({
				...current,
				items: [...current.items, item],
			}));
			return item.id;
		},
		update(itemId, patch) {
			const at = now();
			driven.add(itemId);
			patchItem(itemId, (item) => {
				const next = strictItem(
					{
						...item,
						...patch,
						id: item.id,
						startedAt: item.startedAt,
						updatedAt: at,
					},
					"update",
				);
				if (!isActivityFinished(next))
					return { ...next, finishedAt: undefined };
				return next.state === item.state
					? next
					: finished(next, next.state as "done" | "failed" | "unknown", at);
			});
		},
		finish(itemId, outcome, detail) {
			const at = now();
			patchItem(itemId, (item) =>
				finished(item, outcome, at, detail ?? item.detail),
			);
		},
		dismiss(itemId) {
			patchItem(itemId, (item) =>
				isActivityFinished(item) ? undefined : { ...item, dismissed: true },
			);
		},
		async resume(deviceId) {
			migrateLegacyTransfers(deviceId);
			const byDevice = new Map<string, ActivityItem[]>();
			for (const item of stored.items)
				if (resumable(item, deviceId))
					byDevice.set(item.target.deviceId, [
						...(byDevice.get(item.target.deviceId) ?? []),
						item,
					]);
			await Promise.all(
				[...byDevice.values()].map(async (items) => {
					for (const item of items) await check(item);
				}),
			);
		},
		onFinishedElsewhere(listener) {
			finishListeners.add(listener);
			return () => {
				finishListeners.delete(listener);
			};
		},
		startRun({ title, oneAtATime, stopOnFail, items }) {
			const at = now();
			const created = items.map((input) => newItem(input, at));
			const run: ActivityRun = {
				id: crypto.randomUUID(),
				title,
				itemIds: created.map((item) => item.id),
				oneAtATime,
				stopOnFail,
				startedAt: at,
				updatedAt: at,
			};
			commit(new Set(run.itemIds), (current) => ({
				items: [...current.items, ...created],
				runs: [...current.runs, run],
			}));
			return run;
		},
		run(runId) {
			return stored.runs.find((run) => run.id === runId);
		},
		runItems(runId) {
			const run = tracker.run(runId);
			if (!run) return [];
			const items = new Map(stored.items.map((item) => [item.id, item]));
			return run.itemIds.flatMap((itemId) => {
				const item = items.get(itemId);
				return item ? [view(item)] : [];
			});
		},
		runs() {
			return stored.runs;
		},
	};
	return tracker;
}
