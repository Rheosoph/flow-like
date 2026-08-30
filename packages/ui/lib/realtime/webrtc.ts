import { WebrtcProvider } from "y-webrtc";
import * as Y from "yjs";
import {
	type AuthenticatedSignaling,
	prepareAuthenticatedSignaling,
} from "./authenticated-websocket";
import type { IRealtimeAccess } from "./types";

const FALLBACK_SIGNALING_URL = "wss://signaling.flow-like.com";

export interface RealtimeSession {
	doc: Y.Doc;
	provider: any; // WebrtcProvider, typed as any to avoid direct dependency types here
	awareness: any;
	dispose: () => void;
	onStatusChange?: (
		status: "connected" | "disconnected" | "reconnecting",
	) => void;
	reconnect: () => Promise<void>;
	/** Swap the registered signaling credential for a freshly minted one. */
	refreshAccess: (access: IRealtimeAccess) => void;
}

// Global registry to prevent duplicate Y.Doc instances for the same room
const roomRegistry = new Map<
	string,
	{
		doc: Y.Doc;
		provider: any;
		refCount: number;
		disposeAuthentication: () => void;
		rotateAuthentication: (token: string) => void;
	}
>();

export async function createRealtimeSession(args: {
	room: string;
	access: IRealtimeAccess;
	/** The authenticated user's sub (subject) from the auth token */
	sub?: string;
	signalingServers?: string[];
	onStatusChange?: (
		status: "connected" | "disconnected" | "reconnecting",
	) => void;
	/** Invoked when a signaling socket is rejected or closed for a stale
	 *  credential, so the caller can re-fetch access and call refreshAccess. */
	onAuthFailure?: () => void;
}): Promise<RealtimeSession> {
	const { room, access, sub, onStatusChange } = args;

	// Check if a session already exists for this room
	const existing = roomRegistry.get(room);
	if (existing) {
		existing.refCount++;

		const awareness = existing.provider.awareness;
		// Shared with a still-mounted consumer: identity may be (re)asserted,
		// but its live selection broadcast is not ours to reset.
		awareness.setLocalStateField("sub", sub);

		const dispose = () => {
			existing.refCount--;
			if (existing.refCount <= 0) {
				try {
					existing.provider.disconnect();
					existing.provider.destroy();
				} catch (e) {
					console.error("Provider destroy error:", e);
				}
				try {
					existing.doc.destroy();
				} catch (e) {
					console.error("Doc destroy error:", e);
				}
				existing.disposeAuthentication();
				roomRegistry.delete(room);
			}
		};

		const reconnect = async () => {
			if (onStatusChange) onStatusChange("reconnecting");
			try {
				// y-webrtc's Room.disconnect() removes OUR awareness entry and
				// connect() never restores it — and setLocalStateField is a no-op
				// on a null local state. Re-seed it, or nothing publishes again.
				const snapshot = awareness.getLocalState() ?? {};
				existing.provider.disconnect();
				existing.provider.connect();
				awareness.setLocalState({ ...snapshot, sub });
			} catch (e) {
				console.error("[WebRTC] Reconnection failed:", e);
				if (onStatusChange) onStatusChange("disconnected");
			}
		};

		return {
			doc: existing.doc,
			provider: existing.provider,
			awareness,
			dispose,
			reconnect,
			refreshAccess: (nextAccess: IRealtimeAccess) => {
				existing.rotateAuthentication(nextAccess.jwt);
			},
			onStatusChange,
		};
	}

	// Create a new session
	const configuredSignaling = args.signalingServers?.length
		? args.signalingServers
		: null;
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
			args.onAuthFailure,
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
			peerOpts: {},
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
	let lastStatus: "connected" | "disconnected" | undefined;
	let statusCheckInterval: NodeJS.Timeout | undefined;

	const checkConnectionStatus = () => {
		const states = awareness.getStates() as Map<number, any>;
		const currentPeers = states.size - 1; // Exclude self

		if (currentPeers !== connectedPeers) {
			connectedPeers = currentPeers;
			if (connectedPeers > 0 && onStatusChange && lastStatus !== "connected") {
				lastStatus = "connected";
				onStatusChange("connected");
			}
		}

		// Check if signaling websockets are alive
		const signalingConnected = provider.signalingConns?.some(
			(conn: any) => conn.connected,
		);

		if (signalingConnected && lastStatus !== "connected") {
			// Signaling is alive — report connected so users know the session is up
			lastStatus = "connected";
			if (onStatusChange) onStatusChange("connected");
		} else if (
			!signalingConnected &&
			onStatusChange &&
			lastStatus !== "disconnected"
		) {
			lastStatus = "disconnected";
			onStatusChange("disconnected");
		}
	};

	// Check status periodically
	statusCheckInterval = setInterval(checkConnectionStatus, 5000);
	// Run an initial check after a short delay to set the correct status
	setTimeout(checkConnectionStatus, 1000);

	// Register in the global registry
	roomRegistry.set(room, {
		doc,
		provider,
		refCount: 1,
		disposeAuthentication: authenticatedSignaling.dispose,
		rotateAuthentication: authenticatedSignaling.rotate,
	});

	const reconnect = async () => {
		if (onStatusChange) onStatusChange("reconnecting");
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
				sub,
				reconnected: Date.now(),
			});
			// The status is whatever the sockets say once they settle — never
			// asserted here.
			lastStatus = undefined;
			setTimeout(checkConnectionStatus, 1000);
		} catch (e) {
			console.error("[WebRTC] Reconnection failed:", e);
			lastStatus = "disconnected";
			if (onStatusChange) onStatusChange("disconnected");
		}
	};

	const dispose = () => {
		const entry = roomRegistry.get(room);
		if (!entry) return;

		if (statusCheckInterval) {
			clearInterval(statusCheckInterval);
		}

		entry.refCount--;
		if (entry.refCount <= 0) {
			try {
				provider.disconnect();
				provider.destroy();
			} catch (e) {
				console.error("Provider destroy error:", e);
			}
			try {
				doc.destroy();
			} catch (e) {
				console.error("Doc destroy error:", e);
			}
			entry.disposeAuthentication();
			roomRegistry.delete(room);
		}
	};

	return {
		doc,
		provider,
		awareness,
		dispose,
		reconnect,
		refreshAccess: (nextAccess: IRealtimeAccess) => {
			authenticatedSignaling.rotate(nextAccess.jwt);
		},
		onStatusChange,
	};
}
