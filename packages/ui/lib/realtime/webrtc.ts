import { type JWK, importJWK, jwtVerify } from "jose";
import { WebrtcProvider } from "y-webrtc";
import * as Y from "yjs";
import type { IJwks, IRealtimeAccess } from "./types";

export interface RealtimeSession {
	doc: Y.Doc;
	provider: any; // WebrtcProvider, typed as any to avoid direct dependency types here
	awareness: any;
	dispose: () => void;
	onStatusChange?: (
		status: "connected" | "disconnected" | "reconnecting",
	) => void;
	reconnect: () => Promise<void>;
}

// Global registry to prevent duplicate Y.Doc instances for the same room
const roomRegistry = new Map<
	string,
	{ doc: Y.Doc; provider: any; refCount: number }
>();

async function verifyPeerJwt(token: string, jwks: IJwks): Promise<boolean> {
	try {
		// Resolve key by kid if present, else try first EC key
		const [header] = token.split(".");
		const kid = (() => {
			try {
				const json = JSON.parse(
					atob(header.replace(/-/g, "+").replace(/_/g, "/")),
				);
				return json.kid as string | undefined;
			} catch {
				return undefined;
			}
		})();

		const key: JWK | undefined = jwks.keys.find((k) =>
			kid ? k.kid === kid : k.kty === "EC",
		);
		if (!key) return false;

		const cryptoKey = await importJWK(key as JWK, key.alg || "ES256");
		await jwtVerify(token, cryptoKey, {
			algorithms: ["ES256"],
			// We accept iss/aud validation at the backend; here we only verify signature
		});
		return true;
	} catch {
		return false;
	}
}

export async function createRealtimeSession(args: {
	room: string;
	access: IRealtimeAccess;
	jwks?: IJwks;
	/** The authenticated user's sub (subject) from the auth token */
	sub?: string;
	signalingServers?: string[];
	onStatusChange?: (
		status: "connected" | "disconnected" | "reconnecting",
	) => void;
}): Promise<RealtimeSession> {
	const { room, access, jwks, sub, onStatusChange } = args;

	// Check if a session already exists for this room
	const existing = roomRegistry.get(room);
	if (existing) {
		console.log(`[WebRTC] Reusing existing session for room: ${room}`);
		existing.refCount++;

		const awareness = existing.provider.awareness;
		awareness.setLocalStateField("sub", sub);
		awareness.setLocalStateField("token", access.jwt);
		awareness.setLocalStateField("selection", { nodes: [] });

		const dispose = () => {
			existing.refCount--;
			console.log(
				`[WebRTC] Decremented refCount for room ${room}: ${existing.refCount}`,
			);
			if (existing.refCount <= 0) {
				console.log(`[WebRTC] Destroying session for room: ${room}`);
				try {
					existing.provider.destroy();
				} catch (e) {
					console.error("Provider destroy error:", e);
				}
				try {
					existing.doc.destroy();
				} catch (e) {
					console.error("Doc destroy error:", e);
				}
				roomRegistry.delete(room);
			}
		};

		const reconnect = async () => {
			console.log(`[WebRTC] Reconnect called for existing session: ${room}`);
			if (onStatusChange) onStatusChange("reconnecting");
			// Provider should auto-reconnect, just reset awareness state
			awareness.setLocalStateField("sub", sub);
			awareness.setLocalStateField("token", access.jwt);
			if (onStatusChange) onStatusChange("connected");
		};

		return {
			doc: existing.doc,
			provider: existing.provider,
			awareness,
			dispose,
			reconnect,
			onStatusChange,
		};
	}

	// Create a new session
	console.log(`[WebRTC] Creating new session for room: ${room}`);
	const doc = new Y.Doc();
	if (!args.signalingServers) {
		console.warn("No signaling servers provided, using default");
	} else {
		console.log("Using signaling servers:", args.signalingServers);
	}

	const provider = new WebrtcProvider(room, doc, {
		password: access.encryption_key,
		maxConns: 20 + Math.floor(Math.random() * 15),
		signaling: args.signalingServers ?? ["wss://signaling.flow-like.com"],
		filterBcConns: true,
		peerOpts: {},
	});

	const awareness = provider.awareness;
	awareness.setLocalStateField("sub", sub);
	awareness.setLocalStateField("token", access.jwt);
	awareness.setLocalStateField("selection", { nodes: [] });

	// Optional: validate peers' JWTs when their state arrives; mark invalid
	const invalidPeers = new Set<number>();
	(awareness as any).__invalidPeers = invalidPeers;
	if (jwks) {
		awareness.on(
			"update",
			async ({ added, updated }: { added: number[]; updated: number[] }) => {
				const states = awareness.getStates() as Map<number, any>;
				const toCheck = [...added, ...updated];
				for (const clientId of toCheck) {
					const state = states.get(clientId);
					const token = state?.token as string | undefined;
					if (!token) continue;
					const ok = await verifyPeerJwt(token, jwks);
					if (!ok) invalidPeers.add(clientId);
				}
			},
		);
	}

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

		// Check for signaling server connection
		if (provider.room?.webrtcConns) {
			const hasConnections = Object.keys(provider.room.webrtcConns).length > 0;
			if (!hasConnections && connectedPeers === 0 && onStatusChange && lastStatus !== "disconnected") {
				lastStatus = "disconnected";
				onStatusChange("disconnected");
			}
		}
	};

	// Check status periodically
	statusCheckInterval = setInterval(checkConnectionStatus, 5000);

	// Register in the global registry
	roomRegistry.set(room, { doc, provider, refCount: 1 });

	const reconnect = async () => {
		console.log(`[WebRTC] Attempting to reconnect for room: ${room}`);
		if (onStatusChange) onStatusChange("reconnecting");

		try {
			// Reinitialize awareness state
			awareness.setLocalStateField("sub", sub);
			awareness.setLocalStateField("token", access.jwt);
			awareness.setLocalStateField("selection", { nodes: [] });
			awareness.setLocalStateField("cursor", undefined);

			// Trigger awareness update to broadcast to peers
			awareness.setLocalStateField("reconnected", Date.now());

			if (onStatusChange) onStatusChange("connected");
			console.log(`[WebRTC] Reconnected to room: ${room}`);
		} catch (e) {
			console.error(`[WebRTC] Reconnection failed:`, e);
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
		console.log(
			`[WebRTC] Decremented refCount for room ${room}: ${entry.refCount}`,
		);
		if (entry.refCount <= 0) {
			console.log(`[WebRTC] Destroying session for room: ${room}`);
			try {
				provider.destroy();
			} catch (e) {
				console.error("Provider destroy error:", e);
			}
			try {
				doc.destroy();
			} catch (e) {
				console.error("Doc destroy error:", e);
			}
			roomRegistry.delete(room);
		}
	};

	return { doc, provider, awareness, dispose, reconnect, onStatusChange };
}
