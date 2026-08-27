/**
 * Pure bookkeeping behind the canvas presence signals — drag ghosts, pings,
 * summons, run outcomes and join/leave — over the board's Yjs awareness.
 *
 * Everything is parameterised by `now()`/`schedule()`/`raf()` so it runs under
 * a fake clock in tests; `hooks/use-realtime-signals.ts` wires it to the live
 * awareness. Freshness is always first-sight on the LOCAL clock: whatever a
 * session already carries when this client starts observing is history, and a
 * peer's `ts` is never compared to our clock.
 */

import {
	DRAG_FIELD,
	type DragPayload,
	LAST_RUN_FIELD,
	PING_FIELD,
	PING_TTL_MS,
	type PingEmoji,
	type RunStatus,
	SUMMON_FIELD,
	sanitizeDrag,
	sanitizeLastRun,
	sanitizePing,
	sanitizeSummon,
} from "./presence-signals";

export type AwarenessStates = Map<number, Record<string, unknown>>;

export interface SignalAwareness {
	clientID: number;
	getStates: () => AwarenessStates;
	getLocalState?: () => Record<string, unknown> | null;
	on: (event: "change", cb: () => void) => void;
	off: (event: "change", cb: () => void) => void;
}

export interface SignalFieldSetter {
	setLocalStateField: (field: string, value: unknown) => void;
}

/** useSyncExternalStore-shaped store, same contract as the canvas cursorStore. */
export interface SignalStore<T> {
	subscribe: (listener: () => void) => () => void;
	getSnapshot: () => T;
}

export interface WritableSignalStore<T> extends SignalStore<T> {
	set: (next: T) => void;
}

export interface PeerDrag {
	clientId: number;
	sub?: string;
	layerPath: string;
	nodes: DragPayload["nodes"];
}

export interface PeerPing {
	key: string;
	clientId: number;
	sub?: string;
	layerPath: string;
	x: number;
	y: number;
	emoji?: PingEmoji;
	/** Local clock time this client first saw the ping. */
	seenAt: number;
}

export interface PeerSummon {
	clientId: number;
	sub?: string;
	x: number;
	y: number;
	zoom: number;
	layerPath: string;
}

export interface PeerRun {
	clientId: number;
	sub?: string;
	runId: string;
	status: RunStatus;
	executed: number;
}

export interface PeerPresenceEvent {
	sub: string;
	kind: "joined" | "left";
}

export const EMPTY_PEER_DRAGS: PeerDrag[] = [];
export const EMPTY_PEER_PINGS: PeerPing[] = [];
/** A reconnect flap shorter than this fires neither `left` nor `joined`. */
export const PRESENCE_EVENT_DEBOUNCE_MS = 1500;
/** Drag positions leave at most 20 Hz, like the cursor. */
export const DRAG_PUBLISH_MIN_INTERVAL_MS = 50;

interface Timing {
	now: () => number;
	schedule: (cb: () => void, ms: number) => unknown;
	cancel: (handle: unknown) => void;
}

interface FrameTiming {
	raf: (cb: () => void) => number;
	caf: (handle: number) => void;
}

export type SignalTimingOptions = Partial<Timing>;

function resolveTiming(options?: SignalTimingOptions): Timing {
	return {
		now: options?.now ?? Date.now,
		schedule:
			options?.schedule ??
			((cb: () => void, ms: number) => setTimeout(cb, ms) as unknown),
		cancel:
			options?.cancel ??
			((handle: unknown) =>
				clearTimeout(handle as ReturnType<typeof setTimeout>)),
	};
}

function resolveFrameTiming(options?: Partial<FrameTiming>): FrameTiming {
	return {
		raf:
			options?.raf ?? ((cb: () => void) => requestAnimationFrame(() => cb())),
		caf: options?.caf ?? ((handle: number) => cancelAnimationFrame(handle)),
	};
}

function subOf(state: Record<string, unknown> | undefined): string | undefined {
	return typeof state?.sub === "string" ? state.sub : undefined;
}

function layerPathOf(state: Record<string, unknown> | undefined): string {
	return typeof state?.layerPath === "string" && state.layerPath
		? state.layerPath
		: "root";
}

function invalidPeersOf(awareness: SignalAwareness): Set<number> | undefined {
	return (awareness as { __invalidPeers?: Set<number> }).__invalidPeers;
}

export function createSignalStore<T>(initial: T): WritableSignalStore<T> {
	const listeners = new Set<() => void>();
	let snapshot = initial;
	return {
		subscribe: (listener) => {
			listeners.add(listener);
			return () => {
				listeners.delete(listener);
			};
		},
		getSnapshot: () => snapshot,
		set: (next) => {
			if (next === snapshot) return;
			snapshot = next;
			for (const listener of listeners) listener();
		},
	};
}

/* ── Drag ghosts ───────────────────────────────────────────────────────── */

/** Every OTHER session's sanitized drag, sorted by clientId. */
export function deriveDrags(
	states: AwarenessStates,
	selfClientId: number,
	invalidPeers?: Set<number>,
): PeerDrag[] {
	const drags: PeerDrag[] = [];
	states.forEach((state, clientId) => {
		if (clientId === selfClientId || invalidPeers?.has(clientId)) return;
		const drag = sanitizeDrag(state?.[DRAG_FIELD]);
		if (!drag) return;
		drags.push({
			clientId,
			sub: subOf(state),
			layerPath: layerPathOf(state),
			nodes: drag.nodes,
		});
	});
	drags.sort((a, b) => a.clientId - b.clientId);
	return drags;
}

export function peerDragsEqual(
	a: readonly PeerDrag[],
	b: readonly PeerDrag[],
): boolean {
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) {
		const p = a[i];
		const n = b[i];
		if (
			p.clientId !== n.clientId ||
			p.sub !== n.sub ||
			p.layerPath !== n.layerPath ||
			p.nodes.length !== n.nodes.length
		)
			return false;
		for (let j = 0; j < p.nodes.length; j++) {
			const pn = p.nodes[j];
			const nn = n.nodes[j];
			if (pn.id !== nn.id || pn.x !== nn.x || pn.y !== nn.y) return false;
		}
	}
	return true;
}

export interface DragPublisher {
	/** Throttled to the trailing edge; the latest positions always win. */
	publish: (nodes: DragPayload["nodes"]) => void;
	/** Drop: withdraw the drag from the wire immediately and cancel any pending tick. */
	clear: () => void;
	dispose: () => void;
}

export function createDragPublisher(
	options: {
		awareness: SignalFieldSetter;
		minIntervalMs?: number;
	} & SignalTimingOptions,
): DragPublisher {
	const { now, schedule, cancel } = resolveTiming(options);
	const minInterval = options.minIntervalMs ?? DRAG_PUBLISH_MIN_INTERVAL_MS;
	let lastPublishAt = Number.NEGATIVE_INFINITY;
	let pending: DragPayload["nodes"] | undefined;
	let timer: unknown | null = null;
	let onWire = false;

	const clear = () => {
		if (timer !== null) {
			cancel(timer);
			timer = null;
		}
		pending = undefined;
		if (!onWire) return;
		onWire = false;
		options.awareness.setLocalStateField(DRAG_FIELD, undefined);
	};

	const flush = () => {
		timer = null;
		const nodes = pending;
		pending = undefined;
		if (!nodes) return;
		lastPublishAt = now();
		const payload = sanitizeDrag({ nodes, ts: lastPublishAt });
		if (!payload) {
			clear();
			return;
		}
		onWire = true;
		options.awareness.setLocalStateField(DRAG_FIELD, payload);
	};

	return {
		publish: (nodes) => {
			pending = nodes;
			if (timer !== null) return;
			const wait = lastPublishAt + minInterval - now();
			if (wait <= 0) {
				flush();
				return;
			}
			timer = schedule(flush, wait);
		},
		clear,
		dispose: clear,
	};
}

/* ── Pings ─────────────────────────────────────────────────────────────── */

export function peerPingsEqual(
	a: readonly PeerPing[],
	b: readonly PeerPing[],
): boolean {
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) if (a[i].key !== b[i].key) return false;
	return true;
}

/**
 * A ping enters when a session's `seq` changes after the tracker started and
 * leaves PING_TTL_MS after it was first seen, both on the local clock. The
 * local session is included (via `localState` when it is not in `states`) so
 * the sender sees their own ripple.
 */
export function createPingTracker(now: () => number = Date.now) {
	const lastSeq = new Map<number, number>();
	let live: PeerPing[] = EMPTY_PEER_PINGS;
	let seeded = false;
	return {
		observe(
			states: AwarenessStates,
			selfClientId: number,
			localState?: Record<string, unknown> | null,
			invalidPeers?: Set<number>,
		): { pings: PeerPing[]; nextExpiryMs: number } {
			const at = now();
			const next = live.filter((ping) => at - ping.seenAt < PING_TTL_MS);
			const visit = (
				state: Record<string, unknown> | undefined,
				clientId: number,
			) => {
				const ping = sanitizePing(state?.[PING_FIELD]);
				if (!ping || lastSeq.get(clientId) === ping.seq) return;
				lastSeq.set(clientId, ping.seq);
				if (!seeded) return;
				next.push({
					key: `${clientId}:${ping.seq}`,
					clientId,
					sub: subOf(state),
					layerPath: ping.layerPath,
					x: ping.x,
					y: ping.y,
					...(ping.emoji ? { emoji: ping.emoji } : {}),
					seenAt: at,
				});
			};
			let sawSelf = false;
			states.forEach((state, clientId) => {
				if (invalidPeers?.has(clientId)) return;
				if (clientId === selfClientId) sawSelf = true;
				visit(state, clientId);
			});
			if (!sawSelf && localState) visit(localState, selfClientId);
			seeded = true;
			if (!peerPingsEqual(live, next)) live = next;
			let nextExpiryMs = Number.POSITIVE_INFINITY;
			for (const ping of live) {
				nextExpiryMs = Math.min(nextExpiryMs, ping.seenAt + PING_TTL_MS - at);
			}
			return { pings: live, nextExpiryMs };
		},
	};
}

/* ── First-sight signals (summon, last run) ────────────────────────────── */

export interface FirstSight<T> {
	clientId: number;
	sub?: string;
	payload: T;
}

/**
 * Emits a session's payload once per distinct key, skipping whatever was on
 * the wire at the first observe. Keys are remembered even after a session
 * drops out, so an awareness timeout + re-add never replays a stale signal.
 */
export function createFirstSightTracker<T>(
	read: (state: Record<string, unknown> | undefined) => T | undefined,
	keyOf: (payload: T) => string,
) {
	const seen = new Map<number, string>();
	let seeded = false;
	return {
		observe(
			states: AwarenessStates,
			selfClientId: number,
			invalidPeers?: Set<number>,
		): FirstSight<T>[] {
			const fresh: FirstSight<T>[] = [];
			states.forEach((state, clientId) => {
				if (clientId === selfClientId || invalidPeers?.has(clientId)) return;
				const payload = read(state);
				if (!payload) return;
				const key = keyOf(payload);
				if (seen.get(clientId) === key) return;
				seen.set(clientId, key);
				if (seeded) fresh.push({ clientId, sub: subOf(state), payload });
			});
			seeded = true;
			return fresh;
		},
	};
}

/* ── Join / leave ──────────────────────────────────────────────────────── */

/**
 * Diffs the set of DISTINCT subs across sessions (two windows are one user).
 * The set present at the first observe is never announced; transitions settle
 * after `debounceMs` so a reconnect flap cancels itself out. The local user's
 * own sub never fires.
 */
export function createPresenceEventTracker(
	options: {
		selfSub?: string;
		onEvent: (event: PeerPresenceEvent) => void;
		debounceMs?: number;
	} & SignalTimingOptions,
) {
	const { schedule, cancel } = resolveTiming(options);
	const debounceMs = options.debounceMs ?? PRESENCE_EVENT_DEBOUNCE_MS;
	const announced = new Set<string>();
	const pending = new Map<
		string,
		{ kind: PeerPresenceEvent["kind"]; handle: unknown }
	>();
	let seeded = false;

	const cancelPending = (sub: string) => {
		const entry = pending.get(sub);
		if (!entry) return;
		cancel(entry.handle);
		pending.delete(sub);
	};

	const settle = (sub: string, kind: PeerPresenceEvent["kind"]) => {
		pending.delete(sub);
		if (kind === "joined") announced.add(sub);
		else announced.delete(sub);
		options.onEvent({ sub, kind });
	};

	const defer = (sub: string, kind: PeerPresenceEvent["kind"]) => {
		pending.set(sub, {
			kind,
			handle: schedule(() => settle(sub, kind), debounceMs),
		});
	};

	return {
		observe(
			states: AwarenessStates,
			selfClientId: number,
			invalidPeers?: Set<number>,
		) {
			const present = new Set<string>();
			states.forEach((state, clientId) => {
				if (clientId === selfClientId || invalidPeers?.has(clientId)) return;
				const sub = subOf(state);
				if (sub && sub !== options.selfSub) present.add(sub);
			});
			if (!seeded) {
				seeded = true;
				for (const sub of present) announced.add(sub);
				return;
			}
			for (const sub of present) {
				if (announced.has(sub)) {
					if (pending.get(sub)?.kind === "left") cancelPending(sub);
				} else if (!pending.has(sub)) {
					defer(sub, "joined");
				}
			}
			for (const sub of announced) {
				if (!present.has(sub) && !pending.has(sub)) defer(sub, "left");
			}
			for (const [sub, entry] of pending) {
				if (entry.kind === "joined" && !present.has(sub)) cancelPending(sub);
			}
		},
		dispose() {
			for (const entry of pending.values()) cancel(entry.handle);
			pending.clear();
		},
	};
}

/* ── Composite observer ────────────────────────────────────────────────── */

export interface PresenceSignalsObserverOptions
	extends SignalTimingOptions,
		Partial<FrameTiming> {
	/** Local user's sub: never announced as joined/left. */
	selfSub?: string;
	/** Stable stores owned by the caller; created here when omitted. */
	dragStore?: WritableSignalStore<PeerDrag[]>;
	pingStore?: WritableSignalStore<PeerPing[]>;
	onSummon?: (summon: PeerSummon) => void;
	onPeerRun?: (run: PeerRun) => void;
	onPeerPresenceEvent?: (event: PeerPresenceEvent) => void;
	presenceDebounceMs?: number;
}

export interface PresenceSignalsObserver {
	dragStore: SignalStore<PeerDrag[]>;
	pingStore: SignalStore<PeerPing[]>;
	dispose: () => void;
}

/**
 * One awareness subscription for every canvas signal. Awareness "change"
 * bursts coalesce to one recompute per animation frame; the stores only
 * notify when their snapshot actually changed, and ping expiry wakes the
 * observer on its own timer since nothing on the wire changes when a ping
 * merely ages out.
 */
export function createPresenceSignalsObserver(
	awareness: SignalAwareness,
	options: PresenceSignalsObserverOptions = {},
): PresenceSignalsObserver {
	const timing = resolveTiming(options);
	const { raf, caf } = resolveFrameTiming(options);
	const dragStore =
		options.dragStore ?? createSignalStore<PeerDrag[]>(EMPTY_PEER_DRAGS);
	const pingStore =
		options.pingStore ?? createSignalStore<PeerPing[]>(EMPTY_PEER_PINGS);
	const pings = createPingTracker(timing.now);
	const summons = createFirstSightTracker(
		(state) => sanitizeSummon(state?.[SUMMON_FIELD]),
		(payload) => String(payload.seq),
	);
	const runs = createFirstSightTracker(
		(state) => sanitizeLastRun(state?.[LAST_RUN_FIELD]),
		(payload) => `${payload.runId}:${payload.ts}`,
	);
	const presence = createPresenceEventTracker({
		selfSub: options.selfSub,
		debounceMs: options.presenceDebounceMs,
		onEvent: (event) => options.onPeerPresenceEvent?.(event),
		...timing,
	});
	let rafId: number | null = null;
	let expiryTimer: unknown | null = null;
	let disposed = false;

	const recompute = () => {
		const states = awareness.getStates();
		const invalidPeers = invalidPeersOf(awareness);
		const self = awareness.clientID;

		const drags = deriveDrags(states, self, invalidPeers);
		if (!peerDragsEqual(dragStore.getSnapshot(), drags)) dragStore.set(drags);

		const observed = pings.observe(
			states,
			self,
			awareness.getLocalState?.(),
			invalidPeers,
		);
		pingStore.set(observed.pings);
		if (expiryTimer !== null) {
			timing.cancel(expiryTimer);
			expiryTimer = null;
		}
		if (Number.isFinite(observed.nextExpiryMs)) {
			expiryTimer = timing.schedule(() => {
				expiryTimer = null;
				scheduleRecompute();
			}, observed.nextExpiryMs + 1);
		}

		for (const { clientId, sub, payload } of summons.observe(
			states,
			self,
			invalidPeers,
		)) {
			options.onSummon?.({
				clientId,
				sub,
				x: payload.x,
				y: payload.y,
				zoom: payload.zoom,
				layerPath: payload.layerPath,
			});
		}
		for (const { clientId, sub, payload } of runs.observe(
			states,
			self,
			invalidPeers,
		)) {
			options.onPeerRun?.({
				clientId,
				sub,
				runId: payload.runId,
				status: payload.status,
				executed: payload.executed,
			});
		}
		presence.observe(states, self, invalidPeers);
	};

	const scheduleRecompute = () => {
		if (rafId !== null || disposed) return;
		rafId = raf(() => {
			rafId = null;
			recompute();
		});
	};

	awareness.on("change", scheduleRecompute);
	recompute();

	return {
		dragStore,
		pingStore,
		dispose: () => {
			disposed = true;
			if (rafId !== null) caf(rafId);
			if (expiryTimer !== null) {
				timing.cancel(expiryTimer);
				expiryTimer = null;
			}
			presence.dispose();
			try {
				awareness.off("change", scheduleRecompute);
			} catch {}
		},
	};
}
