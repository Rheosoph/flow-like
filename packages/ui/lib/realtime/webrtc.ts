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

interface IceConfigurableProvider {
	peerOpts?: RealtimePeerOptions;
	room?: {
		webrtcConns?: {
			values: () => Iterable<{ destroy?: () => void }>;
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

/** Apply freshly minted ICE credentials to future peer connections. Active
 * peer connections keep the configuration captured at construction, so they
 * are recycled when the effective server list changes. */
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

	const connections = provider.room?.webrtcConns?.values();
	if (connections) {
		for (const connection of Array.from(connections)) {
			try {
				connection.destroy?.();
			} catch (error) {
				console.error("[WebRTC] Failed to recycle a peer connection:", error);
			}
		}
	}

	return true;
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
}

// Global registry to prevent duplicate Y.Doc instances for the same room
const roomRegistry = new Map<string, RoomRegistryEntry>();

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
	};
}

export async function createRealtimeSession(args: {
	room: string;
	access: IRealtimeAccess;
	/** The authenticated user's sub (subject) from the auth token */
	sub?: string;
	signalingServers?: string[];
	onStatusChange?: RealtimeStatusListener;
	/** Invoked when a signaling socket is rejected or closed for a stale
	 *  credential, so the caller can re-fetch access and call refreshAccess. */
	onAuthFailure?: () => void;
}): Promise<RealtimeSession> {
	const { room, access, sub, onStatusChange } = args;

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
		authenticatedSignaling = await prepareAuthenticatedSignaling(
			configuredSignaling,
			room,
			access.jwt,
			() => {
				for (const listener of authFailureListeners) listener();
			},
		);
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

	registryEntry = {
		doc,
		provider,
		keyId: access.key_id,
		refCount: 1,
		disposeAuthentication: authenticatedSignaling.dispose,
		refreshAccess: (nextAccess: IRealtimeAccess) => {
			authenticatedSignaling.rotate(nextAccess.jwt);
			applyRealtimeIceServers(provider, nextAccess.ice_servers);
		},
		reconnect,
		statusListeners,
		authFailureListeners,
		statusCheckInterval: null,
		statusCheckTimeout: null,
		lastStatus: undefined,
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
