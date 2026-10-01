import { readCertificates } from "../certificates";
import { classify } from "../model/freshness";
import type {
	CopyRef,
	Freshness,
	FreshnessReason,
	FreshnessSignal,
} from "../model/types";
import { readOfflineQueues } from "../offline-queue";
import { GroupMetricsReader, type ManagementCall } from "../telemetry";
import {
	type ManagementRejection,
	type ManagementResponse,
	managementRejection,
} from "../types";
import { LiveCallError, classifyDeviceError } from "./errors";
import type {
	CallLane,
	KeyPort,
	LivePort,
	LiveSessionManager,
	LiveStreams,
	StreamGap,
	StreamSpec,
	StreamState,
	WorkspaceDeps,
} from "./types";

/** Default cadences (M-DATA §3.3); `0` = on connect and on `refresh()` only. */
export const STREAM_EVERY_MS: Record<StreamSpec["kind"], number> = {
	metrics: 5_000,
	project_metrics: 5_000,
	logs: 5_000,
	messages: 5_000,
	offline_queues: 30_000,
	certificates: 0,
	group_metrics: 3_000,
};
const SIGNAL: Record<StreamSpec["kind"], FreshnessSignal> = {
	metrics: "live_metrics",
	project_metrics: "project_metrics",
	logs: "logs",
	messages: "logs",
	offline_queues: "offline_queues",
	certificates: "certificates_live",
	group_metrics: "live_metrics",
};
/** Records kept per log or message stream; older pages are re-read by `loadOlder`. */
export const RECORD_BUFFER = 500;
const PAGE_LIMIT = 100;
const MAX_GAPS = 64;
const MAX_PAGES = 4096;

export interface TelemetryRecord {
	sequence: number;
	timestamp: number;
	kind: string;
	data: Record<string, unknown>;
}

export interface RecordStream {
	records: TelemetryRecord[];
	/** `retention_limit` and similar page fields of the latest read. */
	meta: Record<string, unknown>;
}

export interface GroupMetricsSample {
	sequence: number;
	sample?: Record<string, unknown>;
	state: string;
	receiptPending: boolean;
}

type RecordSpec = Extract<StreamSpec, { kind: "logs" | "messages" }>;
/** The full live manager also offers `subscribe`, `inspection` and `devices` (resume, auto-load, lock). */
type LiveSource = LivePort &
	Partial<Pick<LiveSessionManager, "subscribe" | "inspection">> & {
		devices?(): string[];
	};

export interface StreamPorts {
	/** Needed for group metrics (controller, vault and receipt of the key session). */
	keys?: KeyPort;
	schedule?: (run: () => void, ms: number) => () => void;
	/** Hub-corrected epoch milliseconds; defaults to `deps.now`. */
	now?: () => number;
	/** Replaces the MLS group read (tests). */
	readGroup?: (
		deviceId: string,
		scope: string,
		call: ManagementCall,
	) => Promise<GroupMetricsSample | null>;
}

export interface LiveStreamsImpl extends LiveStreams {
	/** Re-read now (manual refresh for `everyMs: 0` streams). */
	refresh(deviceId: string, spec: StreamSpec): Promise<void>;
	/** Current buffer without subscribing (attention input), `undefined` before the first read. */
	peek<T>(deviceId: string, spec: StreamSpec): StreamState<T> | undefined;
	/** Lock: drop every buffer of the device (or all devices). */
	clear(deviceId?: string): void;
	dispose(): void;
}

interface Subscriber {
	spec: StreamSpec;
	listener: (state: StreamState<unknown>) => void;
	frozen?: { data: unknown; appended: number };
	release: () => void;
}

/** The lines a "load older" extends: the shared buffer, or one paused viewer's own lines. */
interface RecordView {
	read(): RecordStream | undefined;
	write(next: RecordStream): void;
}

interface Buffer {
	deviceId: string;
	key: string;
	spec: StreamSpec;
	subscribers: Set<Subscriber>;
	data?: unknown;
	readAt?: number;
	error?: { code: FreshnessReason; at: number };
	rejected?: ManagementRejection;
	stopped: boolean;
	locked: boolean;
	gaps: StreamGap[];
	cursor: number;
	/** `after` and first record of every page read, so `loadOlder` can re-read trimmed pages. */
	pages: { after: number; first: number }[];
	/** Records kept: `RECORD_BUFFER` plus what `loadOlder` brought back while someone is viewing. */
	keep: number;
	appended: number;
	outboxDropped?: number;
	running?: Promise<boolean>;
	cancelTimer?: () => void;
	sessionAt?: number;
}

type KeyOf = {
	[K in StreamSpec["kind"]]: (spec: Extract<StreamSpec, { kind: K }>) => string;
};
const KEY: KeyOf = {
	metrics: (spec) => `metrics|${spec.placementId ?? ""}`,
	logs: (spec) => `logs|${spec.placementId ?? ""}`,
	messages: (spec) =>
		`messages|${spec.placementId ?? ""}|${spec.projectId ?? ""}`,
	project_metrics: (spec) => `project_metrics|${spec.projectId}`,
	offline_queues: (spec) => `offline_queues|${spec.placementId}`,
	certificates: () => "certificates",
	group_metrics: (spec) => `group_metrics|${spec.scope}`,
};

function streamKey(spec: StreamSpec): string {
	return (KEY[spec.kind] as (value: StreamSpec) => string)(spec);
}

function isRecordSpec(spec: StreamSpec): spec is RecordSpec {
	return spec.kind === "logs" || spec.kind === "messages";
}

function asRecords(value: unknown): TelemetryRecord[] {
	if (!Array.isArray(value)) return [];
	return value.filter(
		(row): row is TelemetryRecord =>
			!!row &&
			typeof row === "object" &&
			Number.isSafeInteger((row as TelemetryRecord).sequence) &&
			!!(row as TelemetryRecord).data &&
			typeof (row as TelemetryRecord).data === "object",
	);
}

function counter(value: unknown): number | undefined {
	return Number.isSafeInteger(value) && Number(value) >= 0
		? Number(value)
		: undefined;
}

function recordGaps(
	records: TelemetryRecord[],
	result: Record<string, unknown>,
	at: number,
): StreamGap[] {
	const gaps: StreamGap[] = [];
	const evicted = counter(result.evicted_through);
	if (evicted !== undefined)
		gaps.push({ kind: "evicted_through", through: evicted, at });
	for (const row of records) {
		const dropped = counter(row.data.dropped_lines);
		if (dropped)
			gaps.push({
				kind: "dropped_lines",
				through: row.sequence,
				count: dropped,
				at: row.timestamp,
			});
		if (row.data.truncated === true)
			gaps.push({
				kind: "truncated",
				through: row.sequence,
				at: row.timestamp,
			});
	}
	return gaps;
}

function failureReason(error: unknown): FreshnessReason {
	const code = classifyDeviceError(error).code;
	if (code === "device_unreachable") return "device_unreachable";
	if (code === "timeout") return "timeout";
	if (code === "session_closed" || code === "expired") return "session_closed";
	return "refresh_failed";
}

function rejectionReason(
	rejection: ManagementRejection,
):
	| { noAccess: CopyRef<FreshnessReason> }
	| { unsupported: CopyRef<FreshnessReason> }
	| undefined {
	if (rejection.code === "unsupported")
		return { unsupported: { code: "agent_update_needed" } };
	if (rejection.code === "unauthorized")
		return { noAccess: { code: "needs_capability" } };
	if (rejection.code === "host_policy")
		return { noAccess: { code: "needs_linux" } };
	return undefined;
}

class Streams implements LiveStreamsImpl {
	private readonly buffers = new Map<string, Map<string, Buffer>>();
	private readonly sessions = new Map<
		string,
		{ connectedAt?: number; inspected: boolean; autoLoaded?: number }
	>();
	private readonly schedule: NonNullable<StreamPorts["schedule"]>;
	private readonly nowMs: () => number;
	private readonly stopLive: () => void;
	private readonly stopKeys: () => void;

	constructor(
		private readonly deps: WorkspaceDeps,
		private readonly live: LiveSource,
		private readonly ports: StreamPorts,
	) {
		this.schedule =
			ports.schedule ??
			((run, ms) => {
				const timer = setTimeout(run, ms);
				return () => clearTimeout(timer);
			});
		this.nowMs = ports.now ?? deps.now ?? Date.now;
		this.stopLive = live.subscribe?.(() => this.liveChanged()) ?? (() => {});
		this.stopKeys = ports.keys?.subscribe(() => this.keysChanged()) ?? (() => {});
	}

	subscribe<T>(
		deviceId: string,
		spec: StreamSpec,
		listener: (state: StreamState<T>) => void,
	): () => void {
		const buffer = this.buffer(deviceId, spec);
		const subscriber: Subscriber = {
			spec,
			listener: listener as (state: StreamState<unknown>) => void,
			release: this.live.acquire(deviceId, "stream"),
		};
		buffer.subscribers.add(subscriber);
		this.emitTo(buffer, subscriber);
		if (
			buffer.data === undefined ||
			(!buffer.cancelTimer && this.everyMs(buffer) > 0)
		)
			this.kick(buffer);
		return () => {
			if (!buffer.subscribers.delete(subscriber)) return;
			subscriber.release();
			if (buffer.subscribers.size === 0) {
				buffer.cancelTimer?.();
				buffer.cancelTimer = undefined;
				buffer.keep = RECORD_BUFFER;
			}
		};
	}

	async loadOlder(deviceId: string, spec: RecordSpec): Promise<void> {
		const buffer = this.buffer(deviceId, spec);
		for (const view of this.recordViews(buffer, spec))
			await this.prepend(buffer, view);
		this.emit(buffer);
	}

	private recordViews(buffer: Buffer, spec: RecordSpec): RecordView[] {
		const paused =
			spec.kind === "logs" && !spec.follow
				? [...buffer.subscribers].filter((subscriber) => subscriber.frozen)
				: [];
		if (paused.length)
			return paused.map((subscriber) => ({
				read: () => subscriber.frozen?.data as RecordStream | undefined,
				write: (next) => {
					if (subscriber.frozen)
						subscriber.frozen = { ...subscriber.frozen, data: next };
				},
			}));
		return [
			{
				read: () => buffer.data as RecordStream | undefined,
				write: (next) => {
					buffer.data = next;
					buffer.keep = Math.max(buffer.keep, next.records.length);
				},
			},
		];
	}

	/** Re-reads the page before the view's first record; a view that moved on meanwhile is left as it is. */
	private async prepend(buffer: Buffer, view: RecordView): Promise<void> {
		const first = view.read()?.records[0]?.sequence;
		if (first === undefined) return;
		const page = buffer.pages.filter((row) => row.first < first).pop();
		if (!page) return;
		const result = await this.readPage(buffer, page.after);
		const current = view.read();
		if (!result || current?.records[0]?.sequence !== first) return;
		const older = asRecords(result.records).filter(
			(row) => row.sequence < first,
		);
		if (older.length)
			view.write({
				records: [...older, ...current.records],
				meta: current.meta,
			});
	}

	async refresh(deviceId: string, spec: StreamSpec): Promise<void> {
		const buffer = this.buffer(deviceId, spec);
		buffer.stopped = false;
		await this.run(buffer);
	}

	peek<T>(deviceId: string, spec: StreamSpec): StreamState<T> | undefined {
		const buffer = this.buffers.get(deviceId)?.get(streamKey(spec));
		if (!buffer || buffer.readAt === undefined) return undefined;
		return this.state(buffer, undefined) as StreamState<T>;
	}

	clear(deviceId?: string): void {
		for (const [id, buffers] of this.buffers) {
			if (deviceId && id !== deviceId) continue;
			for (const buffer of buffers.values()) {
				buffer.data = undefined;
				buffer.readAt = undefined;
				buffer.error = undefined;
				buffer.rejected = undefined;
				buffer.gaps = [];
				buffer.cursor = 0;
				buffer.pages = [];
				buffer.keep = RECORD_BUFFER;
				buffer.appended = 0;
				buffer.outboxDropped = undefined;
				buffer.locked = true;
				for (const subscriber of buffer.subscribers)
					subscriber.frozen = undefined;
				this.emit(buffer);
			}
			this.sessions.delete(id);
		}
	}

	dispose(): void {
		this.stopLive();
		this.stopKeys();
		for (const buffers of this.buffers.values())
			for (const buffer of buffers.values()) {
				buffer.cancelTimer?.();
				for (const subscriber of buffer.subscribers) subscriber.release();
				buffer.subscribers.clear();
			}
		this.buffers.clear();
	}

	/* Buffers and emission. */

	private buffer(deviceId: string, spec: StreamSpec): Buffer {
		let device = this.buffers.get(deviceId);
		if (!device) {
			device = new Map();
			this.buffers.set(deviceId, device);
		}
		const key = streamKey(spec);
		let buffer = device.get(key);
		if (!buffer) {
			buffer = {
				deviceId,
				key,
				spec,
				subscribers: new Set(),
				stopped: false,
				locked: this.ports.keys ? !this.ports.keys.controller(deviceId) : false,
				gaps: [],
				cursor: 0,
				pages: [],
				keep: RECORD_BUFFER,
				appended: 0,
			};
			device.set(key, buffer);
		}
		return buffer;
	}

	private sessionOpen(deviceId: string): boolean {
		const kind = this.live.state(deviceId).kind;
		return kind === "live" || kind === "renewing";
	}

	private freshness(buffer: Buffer): Freshness {
		const reasons = buffer.rejected
			? rejectionReason(buffer.rejected)
			: undefined;
		return classify(SIGNAL[buffer.spec.kind], {
			now: Math.floor(this.nowMs() / 1000),
			at: buffer.readAt,
			loaded: buffer.readAt !== undefined,
			sessionOpen: this.sessionOpen(buffer.deviceId),
			locked: buffer.locked,
			error: buffer.error ? { code: buffer.error.code } : undefined,
			dataFrom: buffer.readAt,
			...reasons,
			...(buffer.rejected && !reasons
				? { error: { code: "rejected" as const } }
				: {}),
		});
	}

	/** A paused log viewer (`follow: false`) keeps the records it had and counts what arrived since. */
	private state(
		buffer: Buffer,
		frozen: Subscriber["frozen"],
	): StreamState<unknown> {
		return {
			data: frozen ? frozen.data : buffer.data,
			freshness: this.freshness(buffer),
			gaps: buffer.gaps,
			...(isRecordSpec(buffer.spec)
				? { behind: frozen ? buffer.appended - frozen.appended : 0 }
				: {}),
			...(buffer.rejected ? { rejected: buffer.rejected } : {}),
		};
	}

	private emitTo(buffer: Buffer, subscriber: Subscriber): void {
		const paused =
			subscriber.spec.kind === "logs" && subscriber.spec.follow === false;
		if (!paused) subscriber.frozen = undefined;
		else subscriber.frozen ??= { data: buffer.data, appended: buffer.appended };
		subscriber.listener(this.state(buffer, subscriber.frozen));
	}

	private emit(buffer: Buffer): void {
		for (const subscriber of buffer.subscribers)
			this.emitTo(buffer, subscriber);
	}

	/* Polling. */

	private everyMs(buffer: Buffer): number {
		let every = Number.POSITIVE_INFINITY;
		for (const { spec } of buffer.subscribers)
			every = Math.min(
				every,
				"everyMs" in spec && spec.everyMs !== undefined
					? spec.everyMs
					: STREAM_EVERY_MS[spec.kind],
			);
		return Number.isFinite(every) ? every : STREAM_EVERY_MS[buffer.spec.kind];
	}

	private kick(buffer: Buffer): void {
		buffer.cancelTimer?.();
		buffer.cancelTimer = undefined;
		this.run(buffer).then(
			(more) => this.plan(buffer, more ? 0 : undefined),
			() => this.plan(buffer),
		);
	}

	private plan(buffer: Buffer, delayMs?: number): void {
		if (buffer.subscribers.size === 0 || buffer.stopped) return;
		const every = delayMs ?? this.everyMs(buffer);
		if (every <= 0 && delayMs === undefined) return;
		buffer.cancelTimer?.();
		buffer.cancelTimer = this.schedule(() => {
			buffer.cancelTimer = undefined;
			this.kick(buffer);
		}, every);
	}

	/** Resolves `true` when a page came back full and more is waiting. */
	private run(buffer: Buffer): Promise<boolean> {
		if (buffer.running) return buffer.running;
		if (this.live.state(buffer.deviceId).kind !== "live" || buffer.stopped)
			return Promise.resolve(false);
		const running = this.read(buffer).finally(() => {
			buffer.running = undefined;
		});
		buffer.running = running;
		return running;
	}

	private async read(buffer: Buffer): Promise<boolean> {
		try {
			const full = await this.fetch(buffer);
			buffer.locked = false;
			buffer.error = undefined;
			buffer.readAt = Math.floor(this.nowMs() / 1000);
			this.emit(buffer);
			return full;
		} catch (error) {
			if (error instanceof LiveCallError && error.code === "keys_locked") {
				this.clear(buffer.deviceId);
				return false;
			}
			buffer.error = {
				code: failureReason(error),
				at: Math.floor(this.nowMs() / 1000),
			};
			this.emit(buffer);
			return false;
		}
	}

	private call(buffer: Buffer): ManagementCall {
		return this.live.call(buffer.deviceId, { lane: "poll" });
	}

	/** True when the read hit its page limit and more is waiting. */
	private async fetch(buffer: Buffer): Promise<boolean> {
		const spec = buffer.spec;
		switch (spec.kind) {
			case "metrics":
				return this.single(buffer, {
					type: "metrics",
					placement_id: spec.placementId,
				});
			case "project_metrics":
				return this.single(buffer, {
					type: "project_metrics",
					project_id: spec.projectId,
				});
			case "logs":
			case "messages":
				return this.follow(buffer);
			case "offline_queues":
				buffer.data = await readOfflineQueues(
					this.call(buffer),
					spec.placementId,
				);
				return false;
			case "certificates":
				buffer.data = await readCertificates(this.call(buffer));
				return false;
			case "group_metrics":
				await this.group(buffer, spec.scope);
				return false;
		}
	}

	private rejectedBy(buffer: Buffer, response: ManagementResponse): boolean {
		const rejection = managementRejection(response);
		if (response.state !== "rejected") {
			buffer.rejected = undefined;
			return false;
		}
		buffer.rejected = rejection ?? {
			code: "failed",
			error: "The device rejected this read.",
			retryable: false,
		};
		if (rejection && !rejection.retryable) buffer.stopped = true;
		return true;
	}

	private async single(
		buffer: Buffer,
		command: Record<string, unknown>,
	): Promise<boolean> {
		const response = await this.call(buffer)(command);
		if (!this.rejectedBy(buffer, response)) buffer.data = response.result;
		return false;
	}

	private async readPage(
		buffer: Buffer,
		after: number,
	): Promise<Record<string, unknown> | undefined> {
		const spec = buffer.spec as RecordSpec;
		const response = await this.call(buffer)({
			type: spec.kind,
			placement_id: spec.placementId,
			...(spec.kind === "messages"
				? { project_id: spec.placementId ? null : spec.projectId }
				: {}),
			after,
			limit: spec.limit ?? PAGE_LIMIT,
		});
		return this.rejectedBy(buffer, response) ? undefined : response.result;
	}

	private async follow(buffer: Buffer): Promise<boolean> {
		const after = buffer.cursor;
		const result = await this.readPage(buffer, after);
		if (!result) return false;
		const records = asRecords(result.records).filter(
			(row) => row.sequence > after,
		);
		const next = counter(result.next);
		if (next !== undefined && next >= after) buffer.cursor = next;
		if (records.length)
			buffer.pages = [
				...buffer.pages,
				{ after, first: records[0].sequence },
			].slice(-MAX_PAGES);
		const at = Math.floor(this.nowMs() / 1000);
		this.addGaps(buffer, [
			...recordGaps(records, result, at),
			...this.outboxGap(buffer, result, at),
		]);
		const previous = (buffer.data as RecordStream | undefined)?.records ?? [];
		const meta = Object.fromEntries(
			Object.entries(result).filter(
				([key]) => key !== "records" && key !== "next",
			),
		);
		buffer.data = {
			records: [...previous, ...records].slice(-buffer.keep),
			meta,
		} satisfies RecordStream;
		buffer.appended += records.length;
		return records.length >= ((buffer.spec as RecordSpec).limit ?? PAGE_LIMIT);
	}

	private outboxGap(
		buffer: Buffer,
		result: Record<string, unknown>,
		at: number,
	): StreamGap[] {
		const dropped = counter(result.outbox_dropped);
		if (dropped === undefined) return [];
		const previous = buffer.outboxDropped;
		buffer.outboxDropped = dropped;
		const added = dropped - (previous ?? 0);
		return added > 0 ? [{ kind: "outbox_dropped", count: added, at }] : [];
	}

	private addGaps(buffer: Buffer, gaps: StreamGap[]): void {
		const fresh = gaps.filter(
			(gap) =>
				!buffer.gaps.some(
					(known) =>
						known.kind === gap.kind &&
						known.through === gap.through &&
						gap.through !== undefined,
				),
		);
		if (fresh.length) buffer.gaps = [...buffer.gaps, ...fresh].slice(-MAX_GAPS);
	}

	private async group(buffer: Buffer, scope: string): Promise<void> {
		const read =
			this.ports.readGroup ??
			((deviceId, groupScope, call) =>
				this.readGroup(deviceId, groupScope, call));
		const options: { signal?: AbortSignal; lane?: CallLane } = { lane: "poll" };
		const sample = await this.live.exclusive(
			buffer.deviceId,
			(call) => read(buffer.deviceId, scope, call),
			options,
		);
		if (!sample) return;
		const previous = buffer.data as GroupMetricsSample | undefined;
		buffer.data = { ...sample, sample: sample.sample ?? previous?.sample };
	}

	private async readGroup(
		deviceId: string,
		scope: string,
		call: ManagementCall,
	): Promise<GroupMetricsSample | null> {
		const keys = this.ports.keys;
		const controller = keys?.controller(deviceId);
		const vault = keys?.vault(deviceId);
		const receipt = keys?.receipt(deviceId);
		if (!controller || !vault || !receipt)
			throw new LiveCallError("keys_locked", "Unlock this device first.");
		const module = await this.deps.crypto();
		const manifest = module.verifyDeviceReceipt(
			receipt,
			vault.manifestJws,
			vault.ownerControllerKey ?? controller.publicBundle().controller_key,
		);
		const reader = await GroupMetricsReader.open(
			controller,
			this.deps.scope,
			manifest,
			receipt,
			scope,
		);
		try {
			let latest = null as GroupMetricsSample | null;
			for (let count = 0; count < 4; count++) {
				const result = await reader.read(call, 0);
				if (!result) break;
				latest = {
					sequence: result.sequence,
					state: result.state,
					receiptPending: result.receiptPending,
					sample: result.sample ?? latest?.sample,
				};
				if (result.receiptPending || result.state === "removed") break;
			}
			return latest;
		} finally {
			reader.close();
		}
	}

	/* Session changes: resume on connect, auto-load offline queues and certificates, clear on lock. */

	/** Decrypted buffers never outlive the keys they were read with, whatever the session got to read. */
	private keysChanged(): void {
		for (const [deviceId, buffers] of this.buffers) {
			if (this.ports.keys?.controller(deviceId)) continue;
			if ([...buffers.values()].some((buffer) => !buffer.locked))
				this.clear(deviceId);
		}
	}

	private liveChanged(): void {
		for (const deviceId of new Set([
			...this.buffers.keys(),
			...this.sessions.keys(),
			...(this.live.devices?.() ?? []),
		]))
			this.deviceChanged(deviceId);
	}

	private deviceChanged(deviceId: string): void {
		const state = this.live.state(deviceId);
		const inspection = this.live.inspection?.(deviceId);
		const session = this.sessions.get(deviceId) ?? { inspected: false };
		this.sessions.set(deviceId, session);
		if (session.inspected && !inspection && state.kind === "idle") {
			this.clear(deviceId);
			return;
		}
		session.inspected = !!inspection;
		if (state.kind !== "live") return;
		if (session.connectedAt !== state.connectedAt) {
			session.connectedAt = state.connectedAt;
			this.resume(deviceId);
		}
		if (
			inspection &&
			session.autoLoaded !== state.connectedAt &&
			inspection.readAt >= state.connectedAt
		) {
			session.autoLoaded = state.connectedAt;
			this.autoLoad(deviceId);
		}
	}

	private resume(deviceId: string): void {
		for (const buffer of this.buffers.get(deviceId)?.values() ?? []) {
			buffer.stopped = false;
			if (buffer.subscribers.size) this.kick(buffer);
		}
	}

	/** IA §2.1: offline queues per placement and the certificate list load on connect. */
	private autoLoad(deviceId: string): void {
		const value = this.live.inspection?.(deviceId)?.value;
		if (!value) return;
		const specs: StreamSpec[] = value.placements.map((placement) => ({
			kind: "offline_queues",
			placementId: placement.id,
		}));
		if (value.certificate_management === 1)
			specs.push({ kind: "certificates" });
		for (const spec of specs)
			this.run(this.buffer(deviceId, spec)).catch(() => {});
	}
}

export function createLiveStreams(
	deps: WorkspaceDeps,
	live: LiveSource,
	ports: StreamPorts = {},
): LiveStreamsImpl {
	return new Streams(deps, live, ports);
}
