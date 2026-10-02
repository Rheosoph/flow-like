import { isTransportFailure } from "../../api-error";
import {
	FLEET_READER_RENEW_S,
	type FleetMetrics,
	type FleetRead,
	type FleetStream,
	type StatusObservation,
	readFleet,
	registerFleetReader,
	removeFleetReader,
} from "../fleet";
import { SnapshotIntegrityError, readSavedInventory } from "../inventory";
import { classify } from "../model/freshness";
import type {
	Freshness,
	FreshnessReason,
	FreshnessSignal,
} from "../model/types";
import type { LocalDeviceVault } from "../storage";
import type { BrowserController, DeviceReceipt } from "../types";
import { LiveCallError } from "./errors";
import type {
	ClockModel,
	FleetDeviceState,
	FleetSnapshotReader,
	HubPort,
	KeyPort,
	KeySessionManager,
	WorkspaceDeps,
} from "./types";

/** M-DATA §3.4 cadences. */
export const FLEET_POLL_MS = {
	visible: 30_000,
	hidden: 120_000,
	saved: 60_000,
} as const;
/** Failed reads retry after 30 → 60 → 120 → 300 s and stay at 300 s. */
export const FLEET_BACKOFF_MS = [30_000, 60_000, 120_000, 300_000] as const;
const REGISTRATION_RETRY_MS = 300_000;

/** Seams for tests; production uses the lib functions and the browser. */
export interface FleetReaderIo {
	readFleet: typeof readFleet;
	registerFleetReader: typeof registerFleetReader;
	removeFleetReader: typeof removeFleetReader;
	readSavedInventory: typeof readSavedInventory;
	setTimer(run: () => void, ms: number): unknown;
	clearTimer(handle: unknown): void;
	visible(): boolean;
	onVisibilityChange(listener: () => void): () => void;
}

const BROWSER_IO: FleetReaderIo = {
	readFleet,
	registerFleetReader,
	removeFleetReader,
	readSavedInventory,
	setTimer: (run, ms) => setTimeout(run, ms),
	clearTimer: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
	visible: () => globalThis.document?.visibilityState !== "hidden",
	onVisibilityChange(listener) {
		const document = globalThis.document;
		if (!document) return () => undefined;
		document.addEventListener("visibilitychange", listener);
		return () => document.removeEventListener("visibilitychange", listener);
	},
};

export interface FleetReaderPorts {
	keys: KeyPort & Partial<Pick<KeySessionManager, "list">>;
	hub: HubPort;
	clock: ClockModel;
	/** Non-revoked hub device ids: the `coverage()` universe. Default: devices with keys here plus the ones read. */
	devices?: () => readonly string[];
}

export interface FleetReaderRuntime extends FleetSnapshotReader {
	/** "Renew": sign a new 365-day reader declaration now. Needs the device's keys. */
	renew(deviceId: string): Promise<void>;
	/** "Stop receiving": delete this computer's reader on the hub; status stays "No access" until `renew`. */
	stopReceiving(deviceId: string): Promise<void>;
	/** Stop every timer and discard in-flight reads. */
	dispose(): void;
}

type FailureKind = NonNullable<FleetDeviceState["error"]>["kind"];
interface StreamFailure {
	kind: FailureKind;
	reason: FreshnessReason;
	message: string;
	/** Hub-corrected unix seconds. */
	at: number;
	retryAt: number;
}
interface ReadState {
	/** Local milliseconds. */
	dueAt: number;
	readAt?: number;
	failures: number;
	loaded: boolean;
	error?: StreamFailure;
}
interface StreamEntry {
	stream: FleetStream;
	observation?: StatusObservation;
	metric?: FleetMetrics;
}
type KeyMode = "none" | "locked" | "unlocked";
interface Session {
	generation: number;
	controller: BrowserController;
	vault: LocalDeviceVault;
	receipt: DeviceReceipt;
}
interface Entry {
	deviceId: string;
	watchers: number;
	generation: number;
	mode: KeyMode;
	controller?: BrowserController;
	status: ReadState;
	saved: ReadState;
	streams: Map<string, StreamEntry>;
	data: Omit<FleetDeviceState, "deviceId" | "freshness" | "error">;
	registeredFor?: BrowserController;
	registerRetryAt: number;
	registrationError?: unknown;
	/** "Stop receiving" in this workspace: no registration and no status reads until `renew`. */
	stopped: boolean;
	inflight?: { generation: number; status: boolean; promise: Promise<unknown> };
	timer?: unknown;
	state: FleetDeviceState;
}

const FAILURE_RANK: Record<FailureKind, number> = {
	integrity: 3,
	access: 2,
	network: 1,
};

function statusOf(error: unknown): number | undefined {
	const status = (error as { status?: unknown } | null)?.status;
	return typeof status === "number" ? status : undefined;
}

const ACCESS_ENDED = new Set([403, 410]);

function networkReason(error: unknown, status?: number): FreshnessReason {
	if (status === 429) return "rate_limited";
	if (status === 408 || (error as Error | null)?.name === "RequestTimeoutError")
		return "timeout";
	if ((status ?? 0) >= 500) return "server_error";
	return isTransportFailure(error) ? "network" : "refresh_failed";
}

/** A refusal ends access; everything that is not a verdict on the data or the grant is retried. */
function failureOf(
	error: unknown,
	registered: boolean,
): Pick<StreamFailure, "kind" | "reason"> {
	if (error instanceof SnapshotIntegrityError)
		return { kind: "integrity", reason: "integrity" };
	const status = statusOf(error);
	if (ACCESS_ENDED.has(status ?? 0) || (status === 404 && registered))
		return { kind: "access", reason: "access_ended" };
	return { kind: "network", reason: networkReason(error, status) };
}

function backoffMs(failures: number): number {
	return FLEET_BACKOFF_MS[
		Math.min(failures, FLEET_BACKOFF_MS.length) - 1
	] as number;
}

function newReadState(): ReadState {
	return { dueAt: 0, failures: 0, loaded: false };
}

function newest<T extends { observedAt: number }>(rows: readonly T[]) {
	return rows.reduce<T | undefined>(
		(best, row) => (!best || row.observedAt > best.observedAt ? row : best),
		undefined,
	);
}

function sameJson(a: unknown, b: unknown) {
	return JSON.stringify(a) === JSON.stringify(b);
}

function sameState(a: FleetDeviceState, b: FleetDeviceState) {
	return (
		a.status === b.status &&
		a.metrics === b.metrics &&
		a.saved === b.saved &&
		a.previousBootId === b.previousBootId &&
		sameJson(a.reader, b.reader) &&
		sameJson(a.policy, b.policy) &&
		sameJson(a.error, b.error) &&
		sameJson(a.freshness, b.freshness)
	);
}

/** Status and metrics as shown; the newest status stream sets the time, boot and sequence. */
function streamView(
	streams: Iterable<StreamEntry>,
): Pick<FleetDeviceState, "status" | "metrics"> {
	const kept = [...streams];
	const status = kept.filter((row) => row.stream.kind === "status");
	const latest = newest(status.map((row) => row.stream));
	const metrics = kept.flatMap((row) => (row.metric ? [row.metric] : []));
	return {
		status: latest && {
			observations: status.flatMap((row) =>
				row.observation ? [row.observation] : [],
			),
			observedAt: latest.observedAt,
			bootId: latest.bootId,
			sequence: latest.sequence,
		},
		metrics: metrics.length > 0 ? metrics : undefined,
	};
}

/** The viewer's grant is scoped to another app: the device is not part of that app's coverage. */
function outsideApp(state: FleetDeviceState | undefined, projectId?: string) {
	const scope = state?.policy?.myGrant?.scope;
	if (!projectId || !scope || scope.kind === "device") return false;
	return scope.project_id !== projectId;
}

function coverageBucket(
	mode: KeyMode,
	state: FleetDeviceState | undefined,
): "readable" | "locked" | "noKeys" | undefined {
	if (mode === "none") return "noKeys";
	if (mode === "locked") return "locked";
	return state?.status || state?.saved ? "readable" : undefined;
}

function policyOf(read: FleetRead): FleetDeviceState["policy"] {
	if (read.policyVersion === undefined) return undefined;
	return {
		version: read.policyVersion,
		...(read.myGrant && { myGrant: read.myGrant }),
	};
}

/**
 * M-DATA §3.4: encrypted status and metrics per device, read while something
 * watches the device and its keys are unlocked. Failures keep the last good
 * data with an error and a retry time; integrity failures never show the
 * rejected data and never lock; a refusal is "No access".
 */
export function createFleetSnapshotReader(
	deps: WorkspaceDeps,
	ports: FleetReaderPorts,
	io: FleetReaderIo = BROWSER_IO,
): FleetReaderRuntime {
	const localNow = deps.now ?? Date.now;
	const nowS = () => ports.clock.now() / 1000;
	const entries = new Map<string, Entry>();
	const listeners = new Set<() => void>();
	let disposed = false;

	const emit = () => {
		for (const listener of [...listeners]) listener();
	};
	const statusCadence = () =>
		io.visible() ? FLEET_POLL_MS.visible : FLEET_POLL_MS.hidden;
	const savedCadence = () => Math.max(FLEET_POLL_MS.saved, statusCadence());

	function keyMode(deviceId: string): KeyMode {
		const state = ports.keys.snapshot(deviceId).state;
		if (state === "none") return "none";
		return state === "unlocked" && ports.keys.controller(deviceId)
			? "unlocked"
			: "locked";
	}

	function failureInput(read: ReadState, stopped: boolean) {
		if (stopped) return { noAccess: { code: "not_a_reader" as const } };
		const error = read.error;
		if (!error) return {};
		return error.kind === "access"
			? { noAccess: { code: error.reason } }
			: { error: { code: error.reason, retryAt: error.retryAt } };
	}

	function planeFreshness(
		entry: Entry,
		signal: FreshnessSignal,
		read: ReadState,
		at: number | undefined,
	): Freshness {
		return classify(signal, {
			now: nowS(),
			at: read.loaded ? (at ?? null) : at,
			loaded: read.loaded,
			locked: entry.mode === "locked",
			...(entry.mode === "none" && {
				notLoadedReason: { code: "no_keys_here" as const },
			}),
			...failureInput(read, entry.stopped && read === entry.status),
			skewS: ports.clock.deviceSkewS(entry.deviceId),
		});
	}

	function worstError(entry: Entry): FleetDeviceState["error"] {
		const failure = [entry.status.error, entry.saved.error].reduce<
			StreamFailure | undefined
		>(
			(worst, next) =>
				next && (!worst || FAILURE_RANK[next.kind] > FAILURE_RANK[worst.kind])
					? next
					: worst,
			undefined,
		);
		return (
			failure && {
				kind: failure.kind,
				message: failure.message,
				at: failure.at,
				retryAt: failure.retryAt,
			}
		);
	}

	function update(entry: Entry) {
		const { data } = entry;
		const error = worstError(entry);
		const next: FleetDeviceState = {
			deviceId: entry.deviceId,
			...data,
			freshness: {
				status: planeFreshness(
					entry,
					"fleet_status",
					entry.status,
					data.status?.observedAt,
				),
				metrics: planeFreshness(
					entry,
					"fleet_metrics",
					entry.status,
					data.metrics && newest(data.metrics)?.observedAt,
				),
				saved: planeFreshness(
					entry,
					"saved_inventory",
					entry.saved,
					data.saved?.observedAt,
				),
			},
			...(error && { error }),
		};
		if (sameState(entry.state, next)) return;
		entry.state = next;
		emit();
	}

	/** New read cycle: in-flight reads are discarded and decrypted data is dropped; signed public facts stay. */
	function restart(entry: Entry, keepReader: boolean) {
		entry.generation++;
		entry.inflight = undefined;
		entry.status = newReadState();
		entry.saved = newReadState();
		entry.registeredFor = undefined;
		entry.registerRetryAt = 0;
		entry.registrationError = undefined;
		entry.streams = new Map();
		entry.data = {
			...(keepReader && entry.data.reader && { reader: entry.data.reader }),
			...(entry.data.policy && { policy: entry.data.policy }),
		};
	}

	function createEntry(deviceId: string): Entry {
		const entry: Entry = {
			deviceId,
			watchers: 0,
			generation: 0,
			mode: keyMode(deviceId),
			status: newReadState(),
			saved: newReadState(),
			streams: new Map(),
			data: {},
			registerRetryAt: 0,
			stopped: false,
			state: { deviceId, freshness: {} as FleetDeviceState["freshness"] },
		};
		entry.controller =
			entry.mode === "unlocked" ? ports.keys.controller(deviceId) : undefined;
		entries.set(deviceId, entry);
		update(entry);
		return entry;
	}

	const entryOf = (deviceId: string) =>
		entries.get(deviceId) ?? createEntry(deviceId);

	function sessionOf(entry: Entry): Session | undefined {
		const { deviceId } = entry;
		const controller = ports.keys.controller(deviceId);
		const vault = ports.keys.vault(deviceId);
		const receipt = ports.keys.receipt(deviceId);
		if (
			disposed ||
			entry.mode !== "unlocked" ||
			!controller ||
			controller !== entry.controller ||
			!vault ||
			!receipt
		)
			return undefined;
		return { generation: entry.generation, controller, vault, receipt };
	}

	function requireSession(entry: Entry): Session {
		const session = sessionOf(entry);
		if (!session)
			throw new LiveCallError(
				"keys_locked",
				"Unlock this device to change its encrypted status subscription.",
			);
		return session;
	}

	const isCurrent = (entry: Entry, session: Session) =>
		!disposed &&
		entry.generation === session.generation &&
		ports.keys.controller(entry.deviceId) === session.controller;

	/** A new controller (lock, unlock, "Use here") starts a fresh read cycle; decrypted data never outlives its keys. */
	function syncKeys(entry: Entry) {
		const mode = keyMode(entry.deviceId);
		const controller =
			mode === "unlocked" ? ports.keys.controller(entry.deviceId) : undefined;
		if (controller === entry.controller) {
			if (mode !== entry.mode) {
				entry.mode = mode;
				update(entry);
			}
			return;
		}
		entry.mode = mode;
		entry.controller = controller;
		restart(entry, true);
		update(entry);
		schedule(entry, 0);
	}

	function succeed(read: ReadState, cadenceMs: number) {
		const now = localNow();
		read.readAt = now;
		read.failures = 0;
		read.loaded = true;
		read.error = undefined;
		read.dueAt = now + cadenceMs;
	}

	function fail(read: ReadState, error: unknown, registered: boolean) {
		const now = localNow();
		read.readAt = now;
		read.failures++;
		const delay = backoffMs(read.failures);
		read.dueAt = now + delay;
		read.error = {
			...failureOf(error, registered),
			message: error instanceof Error ? error.message : String(error),
			at: nowS(),
			retryAt: nowS() + delay / 1000,
		};
	}

	function observeClock(entry: Entry, stream: FleetStream, arrivedAt: number) {
		const previous = entry.streams.get(stream.id)?.stream;
		if (
			stream.kind !== "status" ||
			!previous ||
			previous.bootId !== stream.bootId ||
			stream.sequence <= previous.sequence
		)
			return;
		ports.clock.observe(
			"snapshot",
			stream.observedAt,
			arrivedAt,
			entry.deviceId,
		);
	}

	/** Streams the verified reader still authorizes keep their last snapshot until the device publishes the next one. */
	function mergeStreams(entry: Entry, read: FleetRead, arrivedAt: number) {
		const authorized = new Set(read.authorized);
		const merged = new Map<string, StreamEntry>();
		for (const [id, kept] of entry.streams)
			if (authorized.has(id)) merged.set(id, kept);
		let changed = merged.size !== entry.streams.size;
		const observations = [...read.observations];
		const metrics = [...read.metrics];
		for (const stream of read.streams) {
			const kept = entry.streams.get(stream.id)?.stream;
			const fresh =
				!kept ||
				kept.sequence !== stream.sequence ||
				kept.bootId !== stream.bootId;
			observeClock(entry, stream, arrivedAt);
			const value =
				stream.kind === "status"
					? { observation: observations.shift() }
					: { metric: metrics.shift() };
			if (!fresh && merged.has(stream.id)) continue;
			merged.set(stream.id, { stream, ...value });
			changed = true;
		}
		entry.streams = merged;
		return changed;
	}

	function applyStatus(entry: Entry, read: FleetRead, arrivedAt: number) {
		const previousBoot = entry.data.status?.bootId;
		if (mergeStreams(entry, read, arrivedAt)) {
			const { status, metrics } = streamView(entry.streams.values());
			entry.data.status = status;
			entry.data.metrics = metrics;
			if (status && previousBoot && status.bootId !== previousBoot)
				entry.data.previousBootId = previousBoot;
		}
		const reader = {
			revision: read.readerRevision,
			expiresAt: read.readerExpiresAt,
		};
		if (!sameJson(entry.data.reader, reader)) entry.data.reader = reader;
		const policy = policyOf(read);
		if (!sameJson(entry.data.policy, policy)) entry.data.policy = policy;
		if (read.readerExpiresAt - nowS() <= FLEET_READER_RENEW_S)
			entry.registeredFor = undefined;
	}

	/** The first read with these keys registers (or renews) this computer's 365-day reader. */
	async function ensureRegistered(
		entry: Entry,
		session: Session,
		force: boolean,
	) {
		if (entry.registeredFor === session.controller && !force) return;
		if (!force && localNow() < entry.registerRetryAt) return;
		try {
			await io.registerFleetReader(
				deps.api,
				deps.profile,
				deps.scope,
				session.controller,
				session.vault,
				session.receipt,
				() => isCurrent(entry, session),
			);
			if (!isCurrent(entry, session)) return;
			entry.registeredFor = session.controller;
			entry.registerRetryAt = 0;
			entry.registrationError = undefined;
		} catch (error) {
			if (!isCurrent(entry, session)) return;
			entry.registerRetryAt = localNow() + REGISTRATION_RETRY_MS;
			entry.registrationError = error;
		}
	}

	async function readStatus(
		entry: Entry,
		session: Session,
		force: boolean,
	): Promise<unknown> {
		try {
			const crypto = await deps.crypto();
			if (!isCurrent(entry, session)) return undefined;
			await ensureRegistered(entry, session, force);
			if (!isCurrent(entry, session)) return undefined;
			const read = await io.readFleet(
				deps.api,
				deps.profile,
				deps.scope,
				session.controller,
				session.vault,
				crypto,
				() => isCurrent(entry, session),
				{ receipt: session.receipt, now: nowS() },
			);
			if (!isCurrent(entry, session)) return undefined;
			applyStatus(entry, read, localNow());
			succeed(entry.status, statusCadence());
			return undefined;
		} catch (error) {
			if (!isCurrent(entry, session)) return undefined;
			const registered = entry.registeredFor === session.controller;
			const cause = registered ? error : (entry.registrationError ?? error);
			fail(entry.status, cause, registered);
			if (entry.status.error?.kind === "access") {
				entry.streams = new Map();
				entry.data.status = undefined;
				entry.data.metrics = undefined;
				entry.data.previousBootId = undefined;
			}
			return cause;
		} finally {
			if (isCurrent(entry, session)) update(entry);
		}
	}

	async function readSaved(entry: Entry, session: Session) {
		try {
			const observations = await io.readSavedInventory(
				deps.api,
				deps.profile,
				deps.scope,
				session.controller,
				session.vault.grantId,
				() => isCurrent(entry, session),
			);
			if (!isCurrent(entry, session)) return;
			const observedAt = Math.max(
				...observations.map((row) => row.observed_at ?? 0),
			);
			entry.data.saved =
				observations.length > 0
					? { observations, observedAt: observedAt / 1000 }
					: undefined;
			succeed(entry.saved, savedCadence());
		} catch (error) {
			if (!isCurrent(entry, session)) return;
			fail(entry.saved, error, true);
			if (entry.saved.error?.kind === "access") entry.data.saved = undefined;
		} finally {
			if (isCurrent(entry, session)) update(entry);
		}
	}

	function dueReads(entry: Entry, force: boolean) {
		const now = localNow();
		return {
			status: !entry.stopped && (force || now >= entry.status.dueAt),
			saved: force || now >= entry.saved.dueAt,
		};
	}

	async function readCycle(
		entry: Entry,
		session: Session,
		reads: { status: boolean; saved: boolean },
		force: boolean,
	): Promise<unknown> {
		const [failure] = await Promise.all([
			reads.status ? readStatus(entry, session, force) : undefined,
			reads.saved ? readSaved(entry, session) : undefined,
		]);
		return failure;
	}

	/** One read cycle per device and key session; resolves to the status failure, if any. */
	function poll(entry: Entry, force: boolean): Promise<unknown> {
		const inflight =
			entry.inflight?.generation === entry.generation
				? entry.inflight
				: undefined;
		if (inflight && (inflight.status || !force)) return inflight.promise;
		const session = sessionOf(entry);
		if (!session) return Promise.resolve(undefined);
		const reads = dueReads(entry, force);
		const promise = (inflight?.promise ?? Promise.resolve()).then(() =>
			readCycle(entry, session, reads, force),
		);
		const current = {
			generation: entry.generation,
			status: reads.status,
			promise,
		};
		entry.inflight = current;
		void promise.finally(() => {
			if (entry.inflight === current) entry.inflight = undefined;
		});
		return promise;
	}

	function schedule(entry: Entry, delayMs?: number) {
		if (entry.timer !== undefined) io.clearTimer(entry.timer);
		entry.timer = undefined;
		if (disposed || entry.watchers === 0 || entry.mode !== "unlocked") return;
		const dueAt = entry.stopped
			? entry.saved.dueAt
			: Math.min(entry.status.dueAt, entry.saved.dueAt);
		const delay = delayMs ?? Math.max(0, dueAt - localNow());
		entry.timer = io.setTimer(() => {
			entry.timer = undefined;
			void poll(entry, false).finally(() => {
				if (entry.timer === undefined) schedule(entry);
			});
		}, delay);
	}

	/** Back in view: anything older than the visible cadence is read now. */
	function onVisible() {
		if (!io.visible()) return;
		for (const entry of entries.values()) {
			if (entry.watchers === 0) continue;
			for (const [read, cadence] of [
				[entry.status, FLEET_POLL_MS.visible],
				[entry.saved, FLEET_POLL_MS.saved],
			] as const)
				if (read.failures === 0 && read.readAt !== undefined)
					read.dueAt = Math.min(read.dueAt, read.readAt + cadence);
			schedule(entry);
		}
	}

	function universe(): readonly string[] {
		if (ports.devices) return ports.devices();
		const ids = new Set(entries.keys());
		for (const session of ports.keys.list?.() ?? []) ids.add(session.deviceId);
		return [...ids];
	}

	const stopKeys = ports.keys.subscribe(() => {
		for (const entry of entries.values()) syncKeys(entry);
	});
	const stopVisibility = io.onVisibilityChange(onVisible);

	return {
		get: (deviceId) => entries.get(deviceId)?.state,
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		watch(deviceId) {
			const entry = entryOf(deviceId);
			entry.watchers++;
			syncKeys(entry);
			schedule(entry);
			let active = true;
			return () => {
				if (!active) return;
				active = false;
				entry.watchers--;
				if (entry.watchers === 0) schedule(entry);
			};
		},
		async refresh(deviceId) {
			const entry = entryOf(deviceId);
			syncKeys(entry);
			const failure = await poll(entry, true);
			schedule(entry);
			if (failure !== undefined) throw failure;
		},
		async renew(deviceId) {
			const entry = entryOf(deviceId);
			syncKeys(entry);
			const session = requireSession(entry);
			await io.registerFleetReader(
				deps.api,
				deps.profile,
				deps.scope,
				session.controller,
				session.vault,
				session.receipt,
				() => isCurrent(entry, session),
				{ renew: true },
			);
			if (!isCurrent(entry, session)) return;
			entry.stopped = false;
			entry.registeredFor = session.controller;
			entry.registerRetryAt = 0;
			entry.registrationError = undefined;
			entry.status.dueAt = 0;
			update(entry);
			schedule(entry, 0);
		},
		async stopReceiving(deviceId) {
			const entry = entryOf(deviceId);
			syncKeys(entry);
			const session = requireSession(entry);
			await io.removeFleetReader(
				deps.api,
				deps.profile,
				deviceId,
				session.controller.publicBundle().controller_key.x,
			);
			entry.stopped = true;
			restart(entry, false);
			update(entry);
			schedule(entry, 0);
		},
		coverage(projectId) {
			const result = { readable: 0, locked: 0, noKeys: 0, total: 0 };
			for (const deviceId of universe()) {
				const state = entries.get(deviceId)?.state;
				if (outsideApp(state, projectId)) continue;
				result.total++;
				const bucket = coverageBucket(keyMode(deviceId), state);
				if (bucket) result[bucket]++;
			}
			return result;
		},
		dispose() {
			disposed = true;
			stopKeys();
			stopVisibility();
			for (const entry of entries.values()) {
				entry.generation++;
				schedule(entry);
			}
			listeners.clear();
		},
	};
}
