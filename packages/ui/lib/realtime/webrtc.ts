import { WebrtcProvider } from "y-webrtc";
import * as Y from "yjs";
import {
	type AuthenticatedSignaling,
	prepareAuthenticatedSignaling,
} from "./authenticated-websocket";
import type { IRealtimeAccess, IRealtimeIceServer } from "./types";

const FALLBACK_SIGNALING_URL = "wss://signaling.flow-like.com";

interface RealtimePeerOptions extends Record<string, unknown> {
	config?: RealtimePeerConfiguration;
}

interface RealtimePeerConfiguration extends Record<string, unknown> {
	iceServers?: IRealtimeIceServer[];
}

interface RetirableConnection {
	peer?: unknown;
	destroy?: () => void;
}

interface IceConfigurableProvider {
	peerOpts?: RealtimePeerOptions;
	room?: {
		webrtcConns?: {
			values: () => Iterable<RetirableConnection>;
		};
	} | null;
}

function cloneIceServers(
	iceServers: IRealtimeIceServer[],
): IRealtimeIceServer[] {
	return iceServers.map((server) => ({
		urls: Array.isArray(server.urls) ? [...server.urls] : server.urls,
		...(server.username === undefined ? {} : { username: server.username }),
		...(server.credential === undefined
			? {}
			: { credential: server.credential }),
	}));
}

function iceServersEqual(
	current: IRealtimeIceServer[] | undefined,
	next: IRealtimeIceServer[] | undefined,
): boolean {
	if (current === undefined || next === undefined) return current === next;
	if (!Array.isArray(current) || current.length !== next.length) return false;

	for (let index = 0; index < current.length; index++) {
		const currentServer = current[index];
		const nextServer = next[index];
		if (
			currentServer.username !== nextServer.username ||
			currentServer.credential !== nextServer.credential
		) {
			return false;
		}

		const currentUrls = Array.isArray(currentServer.urls)
			? currentServer.urls
			: [currentServer.urls];
		const nextUrls = Array.isArray(nextServer.urls)
			? nextServer.urls
			: [nextServer.urls];
		if (
			currentUrls.length !== nextUrls.length ||
			currentUrls.some((url, urlIndex) => url !== nextUrls[urlIndex])
		) {
			return false;
		}
	}

	return true;
}

/** Build simple-peer options while preserving its built-in ICE defaults when
 * the API does not provide an override. An explicit empty list remains an
 * explicit override. */
export function peerOptsForIceServers(
	iceServers: IRealtimeIceServer[] | undefined,
): Record<string, unknown> {
	return iceServers === undefined
		? {}
		: { config: { iceServers: cloneIceServers(iceServers) } };
}

/** Expiry of an access's ICE credentials in epoch milliseconds, when it names one. */
export function realtimeIceExpiryMs(access: IRealtimeAccess): number | null {
	return typeof access.ice_expires_at === "number" &&
		Number.isFinite(access.ice_expires_at) &&
		access.ice_expires_at > 0
		? access.ice_expires_at * 1000
		: null;
}

/** Hand freshly minted ICE servers to the peer connections opened from now on.
 * Every access carries its own relay credentials, so a live connection is never
 * closed for a new list: it keeps running on the one it was built with. */
export function applyRealtimeIceServers(
	provider: IceConfigurableProvider,
	iceServers: IRealtimeIceServer[] | undefined,
): boolean {
	const currentIceServers = provider.peerOpts?.config?.iceServers;
	if (iceServersEqual(currentIceServers, iceServers)) {
		return false;
	}

	let nextPeerOpts: RealtimePeerOptions = { ...(provider.peerOpts ?? {}) };
	if (iceServers === undefined) {
		const currentConfig = nextPeerOpts.config;
		nextPeerOpts = Object.fromEntries(
			Object.entries(nextPeerOpts).filter(([key]) => key !== "config"),
		);
		if (currentConfig) {
			const remainingConfig = Object.fromEntries(
				Object.entries(currentConfig).filter(([key]) => key !== "iceServers"),
			);
			if (Object.keys(remainingConfig).length > 0) {
				nextPeerOpts.config = remainingConfig;
			}
		}
	} else {
		nextPeerOpts.config = {
			...(nextPeerOpts.config ?? {}),
			iceServers: cloneIceServers(iceServers),
		};
	}
	provider.peerOpts = nextPeerOpts;
	return true;
}

type StatsReport = Record<string, unknown>;

interface StatsSource {
	getStats(): Promise<{ forEach(visit: (report: StatsReport) => void): void }>;
}

/** Whether this side of the connection sends through a TURN relay. */
export async function usesRelay(connection: StatsSource): Promise<boolean> {
	const reports = new Map<unknown, StatsReport>();
	// biome-ignore lint/complexity/noForEach: forEach is all an RTCStatsReport is typed to offer
	(await connection.getStats()).forEach((report) => {
		reports.set(report.id, report);
	});
	const all = [...reports.values()];
	const transport = all.find(
		(report) =>
			report.type === "transport" &&
			report.selectedCandidatePairId !== undefined,
	);
	const pair = transport
		? reports.get(transport.selectedCandidatePairId)
		: all.find(
				(report) =>
					report.type === "candidate-pair" &&
					(report.selected === true ||
						(report.nominated === true && report.state === "succeeded")),
			);
	return reports.get(pair?.localCandidateId)?.candidateType === "relay";
}

function liveConnections(
	provider: IceConfigurableProvider,
): RetirableConnection[] {
	return [...(provider.room?.webrtcConns?.values() ?? [])];
}

/**
 * A relayed connection runs on the credential it was built with and ends when
 * that lapses, so it is closed shortly before and y-webrtc reconnects it on the
 * current one. A direct connection holds no credential and stays.
 */
export async function retireRelayedConnections(
	provider: IceConfigurableProvider,
	connections: readonly RetirableConnection[],
): Promise<void> {
	for (const connection of connections) {
		const source = (connection.peer as { _pc?: StatsSource } | undefined)?._pc;
		if (!source || !liveConnections(provider).includes(connection)) continue;
		try {
			if (
				(await usesRelay(source)) &&
				liveConnections(provider).includes(connection)
			) {
				connection.destroy?.();
			}
		} catch (error) {
			console.warn("[WebRTC] Could not retire a relayed connection:", error);
		}
	}
}

const RELAY_RETIRE_MARGIN_MS = 60_000;

const AUTH_REFRESH_MIN_MS = 2_000;
const AUTH_REFRESH_MAX_MS = 60_000;
const AUTH_REFRESH_RESET_MS = 2 * AUTH_REFRESH_MAX_MS;

/**
 * A refused signaling socket reconnects every few seconds for as long as the
 * room is open. The first refusal asks for a fresh credential at once; repeats
 * wait twice as long each time, so a server that keeps refusing is not
 * answered with one access request per reconnect.
 */
export function createAuthFailureGate(notify: () => void) {
	let delay = 0;
	let notifiedAt = Number.NEGATIVE_INFINITY;
	let timer: ReturnType<typeof setTimeout> | null = null;
	const fire = () => {
		timer = null;
		notifiedAt = Date.now();
		delay = Math.min(
			Math.max(delay * 2, AUTH_REFRESH_MIN_MS),
			AUTH_REFRESH_MAX_MS,
		);
		notify();
	};
	return {
		signal() {
			if (timer !== null) return;
			const sinceLast = Date.now() - notifiedAt;
			if (sinceLast > AUTH_REFRESH_RESET_MS) delay = 0;
			if (sinceLast >= delay) fire();
			else timer = setTimeout(fire, delay - sinceLast);
		},
		dispose() {
			if (timer !== null) clearTimeout(timer);
			timer = null;
		},
	};
}

export interface RealtimeSession {
	doc: Y.Doc;
	provider: any; // WebrtcProvider, typed as any to avoid direct dependency types here
	awareness: any;
	dispose: () => void;
	onStatusChange?: RealtimeStatusListener;
	reconnect: () => Promise<void>;
	/** Swap the registered signaling credential for a freshly minted one. */
	refreshAccess: (access: IRealtimeAccess) => void;
}

type RealtimeStatus = "connected" | "disconnected" | "reconnecting";
type RealtimeStatusListener = (status: RealtimeStatus) => void;

interface RoomRegistryEntry {
	doc: Y.Doc;
	provider: any;
	keyId: string;
	refCount: number;
	disposeAuthentication: () => void;
	refreshAccess: (access: IRealtimeAccess) => void;
	reconnect: (sub?: string) => Promise<void>;
	statusListeners: Set<RealtimeStatusListener>;
	authFailureListeners: Set<() => void>;
	statusCheckInterval: ReturnType<typeof setInterval> | null;
	statusCheckTimeout: ReturnType<typeof setTimeout> | null;
	lastStatus: Exclude<RealtimeStatus, "reconnecting"> | undefined;
	/** When the ICE credentials new connections are built with lapse. */
	iceExpiresAt: number | null;
	retirements: Set<ReturnType<typeof setTimeout>>;
	/** Settles when the last consumer let go and the room closed. */
	released: Promise<void>;
	markReleased: () => void;
}

// Global registry to prevent duplicate Y.Doc instances for the same room
const roomRegistry = new Map<string, RoomRegistryEntry>();
// Two consumers opening one room at once must end up sharing it, so openings queue per room.
const roomOpenings = new Map<string, Promise<unknown>>();
const ROOM_REKEY_WAIT_MS = 3_000;

function emitStatus(entry: RoomRegistryEntry, status: RealtimeStatus): void {
	for (const listener of entry.statusListeners) listener(status);
}

function refreshRegistryEntry(
	entry: RoomRegistryEntry,
	access: IRealtimeAccess,
): void {
	if (entry.keyId !== access.key_id) {
		throw new Error(
			"The shared realtime room uses an older encryption key and must reconnect",
		);
	}
	entry.refreshAccess(access);
}

/**
 * Peers on two room keys cannot reach each other. A consumer that arrives with
 * another key asks whoever holds the room to fetch its access again: a holder
 * on the older key rebuilds, and its letting go lets this opening go ahead on
 * the new key. A holder that keeps the room leaves the arrival on the stale one.
 */
async function awaitRekey(entry: RoomRegistryEntry): Promise<void> {
	for (const listener of [...entry.authFailureListeners]) listener();
	let timer: ReturnType<typeof setTimeout> | undefined;
	await Promise.race([
		entry.released,
		new Promise<void>((resolve) => {
			timer = setTimeout(resolve, ROOM_REKEY_WAIT_MS);
		}),
	]);
	clearTimeout(timer);
}

function createRoomRelease(
	room: string,
	entry: RoomRegistryEntry,
	onStatusChange?: RealtimeStatusListener,
	onAuthFailure?: () => void,
): () => void {
	let released = false;
	return () => {
		if (released) return;
		released = true;
		if (onStatusChange) entry.statusListeners.delete(onStatusChange);
		if (onAuthFailure) entry.authFailureListeners.delete(onAuthFailure);
		if (roomRegistry.get(room) !== entry) return;

		entry.refCount--;
		if (entry.refCount > 0) return;
		roomRegistry.delete(room);
		if (entry.statusCheckInterval !== null) {
			clearInterval(entry.statusCheckInterval);
		}
		if (entry.statusCheckTimeout !== null) {
			clearTimeout(entry.statusCheckTimeout);
		}
		for (const timer of entry.retirements) clearTimeout(timer);
		entry.retirements.clear();

		try {
			entry.provider.disconnect();
			entry.provider.destroy();
		} catch (error) {
			console.error("Provider destroy error:", error);
		}
		try {
			entry.doc.destroy();
		} catch (error) {
			console.error("Doc destroy error:", error);
		}
		entry.disposeAuthentication();
		entry.markReleased();
	};
}

interface RealtimeSessionArgs {
	room: string;
	access: IRealtimeAccess;
	/** The authenticated user's sub (subject) from the auth token */
	sub?: string;
	signalingServers?: string[];
	onStatusChange?: RealtimeStatusListener;
	/** Invoked when the access went stale: a signaling socket was rejected or
	 *  closed for its credential, or another consumer brought a newer room key.
	 *  The caller re-fetches access and calls refreshAccess or rebuilds. */
	onAuthFailure?: () => void;
}

export function createRealtimeSession(
	args: RealtimeSessionArgs,
): Promise<RealtimeSession> {
	const opening = (roomOpenings.get(args.room) ?? Promise.resolve())
		.catch(() => undefined)
		.then(() => openRealtimeSession(args));
	roomOpenings.set(args.room, opening);
	const settled = () => {
		if (roomOpenings.get(args.room) === opening) {
			roomOpenings.delete(args.room);
		}
	};
	opening.then(settled, settled);
	return opening;
}

async function openRealtimeSession(
	args: RealtimeSessionArgs,
): Promise<RealtimeSession> {
	const { room, access, sub, onStatusChange } = args;

	const held = roomRegistry.get(room);
	if (held && held.keyId !== access.key_id) await awaitRekey(held);

	// Check if a session already exists for this room
	const existing = roomRegistry.get(room);
	if (existing) {
		refreshRegistryEntry(existing, access);
		existing.refCount++;
		if (onStatusChange) existing.statusListeners.add(onStatusChange);
		if (args.onAuthFailure) {
			existing.authFailureListeners.add(args.onAuthFailure);
		}
		if (onStatusChange && existing.lastStatus) {
			onStatusChange(existing.lastStatus);
		}

		const awareness = existing.provider.awareness;
		// Shared with a still-mounted consumer: identity may be (re)asserted,
		// but its live selection broadcast is not ours to reset.
		awareness.setLocalStateField("sub", sub);

		const dispose = createRoomRelease(
			room,
			existing,
			onStatusChange,
			args.onAuthFailure,
		);

		return {
			doc: existing.doc,
			provider: existing.provider,
			awareness,
			dispose,
			reconnect: () => existing.reconnect(sub),
			refreshAccess: (nextAccess: IRealtimeAccess) => {
				if (roomRegistry.get(room) === existing) {
					refreshRegistryEntry(existing, nextAccess);
				}
			},
			onStatusChange,
		};
	}

	// Create a new session
	const configuredSignaling = args.signalingServers?.length
		? args.signalingServers
		: null;
	const statusListeners = new Set<RealtimeStatusListener>();
	if (onStatusChange) statusListeners.add(onStatusChange);
	const authFailureListeners = new Set<() => void>();
	if (args.onAuthFailure) authFailureListeners.add(args.onAuthFailure);
	let authenticatedSignaling: AuthenticatedSignaling;
	if (access?.jwt) {
		// Authenticated flow: the JWT is a live bearer credential and must only
		// ever be presented to endpoints the deployment configured — never to the
		// hardcoded public fallback host.
		if (!configuredSignaling) {
			throw new Error(
				"No signaling servers are configured for this deployment; refusing to send the realtime credential to the public fallback host",
			);
		}
		const authFailures = createAuthFailureGate(() => {
			for (const listener of authFailureListeners) listener();
		});
		const signaling = await prepareAuthenticatedSignaling(
			configuredSignaling,
			room,
			access.jwt,
			authFailures.signal,
		);
		authenticatedSignaling = {
			...signaling,
			dispose: () => {
				authFailures.dispose();
				signaling.dispose();
			},
		};
	} else {
		// Legacy unauthenticated path: no credential to protect, fallback allowed.
		if (!configuredSignaling) {
			console.warn("No signaling servers provided, using default");
		}
		authenticatedSignaling = {
			signaling: configuredSignaling ?? [FALLBACK_SIGNALING_URL],
			rotate: () => {},
			dispose: () => {},
		};
	}
	const doc = new Y.Doc();
	let provider: any;
	try {
		provider = new WebrtcProvider(room, doc, {
			password: access.encryption_key,
			maxConns: 20 + Math.floor(Math.random() * 15),
			signaling: authenticatedSignaling.signaling,
			filterBcConns: true,
			peerOpts: peerOptsForIceServers(access.ice_servers),
		});
	} catch (error) {
		authenticatedSignaling.dispose();
		doc.destroy();
		throw error;
	}

	const awareness = provider.awareness;
	awareness.setLocalStateField("sub", sub);
	awareness.setLocalStateField("selection", { nodes: [] });

	// Monitor connection status
	let connectedPeers = 0;
	let registryEntry: RoomRegistryEntry;

	const checkConnectionStatus = () => {
		const states = awareness.getStates() as Map<number, any>;
		const currentPeers = states.size - 1; // Exclude self

		if (currentPeers !== connectedPeers) {
			connectedPeers = currentPeers;
			if (connectedPeers > 0 && registryEntry.lastStatus !== "connected") {
				registryEntry.lastStatus = "connected";
				emitStatus(registryEntry, "connected");
			}
		}

		// Check if signaling websockets are alive
		const signalingConnected = provider.signalingConns?.some(
			(conn: any) => conn.connected,
		);

		if (signalingConnected && registryEntry.lastStatus !== "connected") {
			// Signaling is alive — report connected so users know the session is up
			registryEntry.lastStatus = "connected";
			emitStatus(registryEntry, "connected");
		} else if (
			!signalingConnected &&
			registryEntry.lastStatus !== "disconnected"
		) {
			registryEntry.lastStatus = "disconnected";
			emitStatus(registryEntry, "disconnected");
		}
	};

	const reconnect = async (nextSub?: string) => {
		emitStatus(registryEntry, "reconnecting");
		try {
			// Actually drive the transport: drop the signaling sockets and the
			// room, then rejoin. y-webrtc's Room.disconnect() removes OUR
			// awareness entry and connect() never restores it (and every
			// setLocalStateField is a no-op on a null local state), so the
			// fields are snapshotted and re-seeded — peers keep the clicker's
			// selection and everything publishes again afterwards.
			const snapshot = awareness.getLocalState() ?? {};
			provider.disconnect();
			provider.connect();
			awareness.setLocalState({
				...snapshot,
				sub: nextSub ?? sub,
				reconnected: Date.now(),
			});
			// The status is whatever the sockets say once they settle — never
			// asserted here.
			registryEntry.lastStatus = undefined;
			if (registryEntry.statusCheckTimeout !== null) {
				clearTimeout(registryEntry.statusCheckTimeout);
			}
			registryEntry.statusCheckTimeout = setTimeout(() => {
				registryEntry.statusCheckTimeout = null;
				if (roomRegistry.get(room) === registryEntry) checkConnectionStatus();
			}, 1000);
		} catch (e) {
			console.error("[WebRTC] Reconnection failed:", e);
			registryEntry.lastStatus = "disconnected";
			emitStatus(registryEntry, "disconnected");
		}
	};

	let markReleased = () => {};
	const released = new Promise<void>((resolve) => {
		markReleased = resolve;
	});
	registryEntry = {
		doc,
		provider,
		released,
		markReleased,
		keyId: access.key_id,
		refCount: 1,
		disposeAuthentication: authenticatedSignaling.dispose,
		refreshAccess: (nextAccess: IRealtimeAccess) => {
			authenticatedSignaling.rotate(nextAccess.jwt);
			const outgoing = liveConnections(provider);
			const lapsesAt = registryEntry.iceExpiresAt;
			if (!applyRealtimeIceServers(provider, nextAccess.ice_servers)) return;
			registryEntry.iceExpiresAt = realtimeIceExpiryMs(nextAccess);
			if (lapsesAt === null || outgoing.length === 0) return;
			const timer = setTimeout(
				() => {
					registryEntry.retirements.delete(timer);
					void retireRelayedConnections(provider, outgoing);
				},
				Math.max(lapsesAt - RELAY_RETIRE_MARGIN_MS - Date.now(), 0),
			);
			registryEntry.retirements.add(timer);
		},
		reconnect,
		statusListeners,
		authFailureListeners,
		statusCheckInterval: null,
		statusCheckTimeout: null,
		lastStatus: undefined,
		iceExpiresAt: realtimeIceExpiryMs(access),
		retirements: new Set(),
	};
	roomRegistry.set(room, registryEntry);
	registryEntry.statusCheckInterval = setInterval(checkConnectionStatus, 5000);
	registryEntry.statusCheckTimeout = setTimeout(() => {
		registryEntry.statusCheckTimeout = null;
		if (roomRegistry.get(room) === registryEntry) checkConnectionStatus();
	}, 1000);

	const dispose = createRoomRelease(
		room,
		registryEntry,
		onStatusChange,
		args.onAuthFailure,
	);

	return {
		doc,
		provider,
		awareness,
		dispose,
		reconnect,
		refreshAccess: (nextAccess: IRealtimeAccess) => {
			if (roomRegistry.get(room) === registryEntry) {
				refreshRegistryEntry(registryEntry, nextAccess);
			}
		},
		onStatusChange,
	};
}
