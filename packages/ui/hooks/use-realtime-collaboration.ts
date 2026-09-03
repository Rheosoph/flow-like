import type { UseQueryResult } from "@tanstack/react-query";
import type { ReactFlowInstance } from "@xyflow/react";
import {
	type RefObject,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import type { RemoteSelectionParticipant } from "../components/flow/flow-node";
import { type IRealtimeAccess, createRealtimeSession } from "../lib";
import { decodeJwtExpiryMs } from "../lib/realtime/authenticated-websocket";
import {
	type PeerPresence,
	createPeerActivityTracker,
	peerPresenceListEqual,
	readPeerPresence,
} from "../lib/realtime/peer-presence";
import type { IBoard } from "../lib/schema/flow/board";

export type {
	PeerEditorPresence,
	PeerPresence,
} from "../lib/realtime/peer-presence";

/** High-frequency cursor data, kept out of React state so cursor motion does not
 *  re-render the board. Consumed via useSyncExternalStore by the cursor overlay. */
export interface PeerCursor {
	clientId: number;
	cursor: { x: number; y: number };
	sub?: string;
	layerPath: string;
}

export interface CursorStore {
	subscribe: (listener: () => void) => () => void;
	getSnapshot: () => PeerCursor[];
}

const EMPTY_CURSORS: PeerCursor[] = [];

/** Rotate the realtime token this long before its `exp` so reconnects never
 *  replay an expired credential. */
const TOKEN_ROTATE_MARGIN_MS = 5 * 60 * 1000;
/** Refresh provider-issued TURN credentials before existing connections need
 * to be rebuilt with an expired credential. */
const ICE_ROTATE_MARGIN_MS = 2 * 60 * 1000;
const ACCESS_REFRESH_RETRY_MS = 15 * 1000;

function realtimeIceExpiryMs(access: IRealtimeAccess): number | null {
	return typeof access.ice_expires_at === "number" &&
		Number.isFinite(access.ice_expires_at) &&
		access.ice_expires_at > 0
		? access.ice_expires_at * 1000
		: null;
}

function cursorsEqual(a: PeerCursor[], b: PeerCursor[]): boolean {
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) {
		const p = a[i];
		const n = b[i];
		if (
			p.clientId !== n.clientId ||
			p.cursor.x !== n.cursor.x ||
			p.cursor.y !== n.cursor.y ||
			p.layerPath !== n.layerPath ||
			p.sub !== n.sub
		) {
			return false;
		}
	}
	return true;
}

interface UseRealtimeCollaborationProps {
	appId: string;
	boardId: string;
	board: UseQueryResult<IBoard>;
	version: [number, number, number] | undefined;
	backend: any;
	/** The authenticated user's sub (subject) from the auth token */
	sub?: string;
	hub: any;
	mousePositionRef: RefObject<{ x: number; y: number }>;
	layerPath: string | undefined;
	screenToFlowPosition: ReactFlowInstance["screenToFlowPosition"];
	commandAwarenessRef: React.MutableRefObject<any>;
	setNodes: any;
}

export function useRealtimeCollaboration({
	appId,
	boardId,
	board,
	version,
	backend,
	sub,
	hub,
	mousePositionRef,
	layerPath,
	screenToFlowPosition,
	commandAwarenessRef,
	setNodes,
}: UseRealtimeCollaborationProps) {
	const [awareness, setAwareness] = useState<any | undefined>(undefined);
	const awarenessRef = useRef<any | undefined>(undefined);
	const [connectionStatus, setConnectionStatus] = useState<
		"connected" | "disconnected" | "reconnecting"
	>("disconnected");
	const sessionRef = useRef<{
		dispose: () => void;
		reconnect: () => Promise<void>;
		refreshAccess: (access: IRealtimeAccess) => void;
	} | null>(null);
	const tokenExpiresAtRef = useRef<number | null>(null);
	const iceExpiresAtRef = useRef<number | null>(null);
	// The room key the live provider was built with; the server rotates it
	// daily and a session on the old key cannot decrypt anyone who joined after.
	const keyIdRef = useRef<string | null>(null);
	const [peerStates, setPeerStates] = useState<PeerPresence[]>([]);
	const remoteSelectionsRef = useRef<Map<string, RemoteSelectionParticipant[]>>(
		new Map(),
	);
	// Local-clock "last did something" per session, for idle badges. Kept out of
	// React state: it changes on every pointer tick.
	const activityTrackerRef = useRef(createPeerActivityTracker());

	// External store for high-frequency cursor data so cursor motion re-renders
	// only the cursor overlay (via useSyncExternalStore), never the whole board.
	const cursorDataRef = useRef<{
		snapshot: PeerCursor[];
		listeners: Set<() => void>;
	}>({ snapshot: EMPTY_CURSORS, listeners: new Set() });
	const cursorStore = useRef<CursorStore>({
		subscribe: (listener: () => void) => {
			cursorDataRef.current.listeners.add(listener);
			return () => {
				cursorDataRef.current.listeners.delete(listener);
			};
		},
		getSnapshot: () => cursorDataRef.current.snapshot,
	}).current;

	// Stable ref for board.refetch so the boardUpdate listener doesn't reinstall every render
	const boardRefetchRef = useRef(board.refetch);
	boardRefetchRef.current = board.refetch;

	// Track the last seen boardUpdate value per peer to detect actual changes
	const lastBoardUpdateRef = useRef<Map<number, number>>(new Map());

	const hasBoardData = !!board.data;

	// Use a ref for signaling servers so changes don't trigger session recreation.
	// Authenticated sessions refuse to run without hub-configured servers, so
	// when none are available yet (hub still loading), setup retries below until
	// the hub provides them. When the hub loads later with the same URL, no
	// reconnect is needed.
	const signalingServersRef = useRef<string[] | undefined>(
		hub.hub?.signaling?.length ? hub.hub.signaling : undefined,
	);
	if (hub.hub?.signaling?.length) {
		signalingServersRef.current = hub.hub.signaling;
	}

	// Track whether the session has been initialized for this board
	const sessionInitializedRef = useRef<string | null>(null);

	// Setup realtime session - only run when board identity changes, not profile updates
	useEffect(() => {
		const sessionKey = `${appId}:${boardId}`;

		// Skip if already initialized for this board and session exists
		if (sessionInitializedRef.current === sessionKey && sessionRef.current) {
			return;
		}

		let disposed = false;
		let rotating = false;
		let rotateTimer: ReturnType<typeof setTimeout> | null = null;
		let setupRetryTimer: ReturnType<typeof setTimeout> | null = null;
		let setupRetryDelayMs = 2000;

		// Re-fetch realtime access and swap the registered signaling credential
		// so the provider's automatic reconnects never replay an expired JWT.
		// Guarded by the decoded expiry so auth-unrelated socket failures with a
		// still-fresh token don't hammer the access endpoint.
		const refreshRealtimeAccess = async () => {
			if (disposed || rotating || !sessionRef.current) return;
			const now = Date.now();
			const tokenExpiresAt = tokenExpiresAtRef.current;
			const iceExpiresAt = iceExpiresAtRef.current;
			const tokenNeedsRefresh =
				tokenExpiresAt === null ||
				tokenExpiresAt - now <= TOKEN_ROTATE_MARGIN_MS;
			const iceNeedsRefresh =
				iceExpiresAt !== null && iceExpiresAt - now <= ICE_ROTATE_MARGIN_MS;
			if (!tokenNeedsRefresh && !iceNeedsRefresh) {
				scheduleAccessRefresh();
				return;
			}
			rotating = true;
			try {
				const access = await backend.boardState.getRealtimeAccess(
					appId,
					boardId,
				);
				if (disposed || !sessionRef.current) return;
				if (keyIdRef.current && access.key_id !== keyIdRef.current) {
					// New room key: the provider's AES key is fixed at construction,
					// so a swapped JWT alone would leave this session deaf to every
					// peer on the new key (and them to us) while both say "Live".
					teardownSession();
					await setup(access);
					return;
				}
				sessionRef.current.refreshAccess(access);
				tokenExpiresAtRef.current = decodeJwtExpiryMs(access.jwt);
				iceExpiresAtRef.current = realtimeIceExpiryMs(access);
				scheduleAccessRefresh();
			} catch (e) {
				console.warn("Realtime access refresh failed:", e);
				if (!disposed) {
					if (rotateTimer !== null) clearTimeout(rotateTimer);
					rotateTimer = setTimeout(() => {
						rotateTimer = null;
						void refreshRealtimeAccess();
					}, ACCESS_REFRESH_RETRY_MS);
				}
			} finally {
				rotating = false;
			}
		};

		const teardownSession = () => {
			try {
				sessionRef.current?.dispose();
			} catch {}
			sessionRef.current = null;
			sessionInitializedRef.current = null;
			keyIdRef.current = null;
			tokenExpiresAtRef.current = null;
			iceExpiresAtRef.current = null;
			awarenessRef.current = undefined;
			commandAwarenessRef.current = undefined;
			setAwareness(undefined);
		};

		const scheduleAccessRefresh = () => {
			if (rotateTimer !== null) clearTimeout(rotateTimer);
			const refreshTimes: number[] = [];
			if (tokenExpiresAtRef.current !== null) {
				refreshTimes.push(tokenExpiresAtRef.current - TOKEN_ROTATE_MARGIN_MS);
			}
			if (iceExpiresAtRef.current !== null) {
				refreshTimes.push(iceExpiresAtRef.current - ICE_ROTATE_MARGIN_MS);
			}
			if (refreshTimes.length === 0) return;
			const refreshAt = Math.min(...refreshTimes);
			rotateTimer = setTimeout(
				() => {
					rotateTimer = null;
					void refreshRealtimeAccess();
				},
				Math.max(refreshAt - Date.now(), 60_000),
			);
		};

		const setup = async (prefetchedAccess?: IRealtimeAccess) => {
			try {
				const offline = await backend.isOffline(appId);

				if (!hasBoardData || typeof version !== "undefined") return;
				if (offline) return;

				const room = sessionKey;
				const access: IRealtimeAccess =
					prefetchedAccess ??
					(await backend.boardState.getRealtimeAccess(appId, boardId));

				const session = await createRealtimeSession({
					room,
					access,
					sub,
					signalingServers: signalingServersRef.current,
					onStatusChange: (status) => {
						setConnectionStatus((prev) => {
							if (prev !== status) {
								console.log(`[FlowBoard] Connection status changed: ${status}`);
							}
							return status;
						});
					},
					onAuthFailure: () => {
						void refreshRealtimeAccess();
					},
				});

				if (disposed) {
					session.dispose();
					return;
				}

				sessionRef.current = {
					dispose: session.dispose,
					reconnect: session.reconnect,
					refreshAccess: session.refreshAccess,
				};
				tokenExpiresAtRef.current = decodeJwtExpiryMs(access.jwt);
				iceExpiresAtRef.current = realtimeIceExpiryMs(access);
				keyIdRef.current = access.key_id ?? null;
				scheduleAccessRefresh();
				awarenessRef.current = session.awareness;
				commandAwarenessRef.current = session.awareness;
				sessionInitializedRef.current = sessionKey;
				setupRetryDelayMs = 2000;
				setAwareness(session.awareness);
				// Don't set "connected" here — let the WebrtcProvider's
				// onStatusChange callback report the actual signaling state
			} catch (e) {
				console.warn("Realtime setup failed:", e);
				setConnectionStatus("disconnected");
				// Cold start can race hub loading or a transient credential-provider
				// failure. Retry with a bounded backoff and never fall back to a public
				// signaling host while a bearer credential is present.
				if (!disposed && setupRetryTimer === null) {
					setupRetryTimer = setTimeout(retrySetup, setupRetryDelayMs);
					setupRetryDelayMs = Math.min(setupRetryDelayMs * 2, 60_000);
				}
			}
		};

		const retrySetup = () => {
			setupRetryTimer = null;
			if (disposed) return;
			if (!signalingServersRef.current?.length) {
				setupRetryTimer = setTimeout(retrySetup, setupRetryDelayMs);
				setupRetryDelayMs = Math.min(setupRetryDelayMs * 2, 60_000);
				return;
			}
			void setup();
		};
		void setup();

		return () => {
			disposed = true;
			if (rotateTimer !== null) clearTimeout(rotateTimer);
			if (setupRetryTimer !== null) clearTimeout(setupRetryTimer);
			sessionInitializedRef.current = null;
			try {
				sessionRef.current?.dispose();
			} catch {}
			sessionRef.current = null;
			tokenExpiresAtRef.current = null;
			iceExpiresAtRef.current = null;
			keyIdRef.current = null;
			awarenessRef.current = undefined;
			commandAwarenessRef.current = undefined;
			setAwareness(undefined);
			setConnectionStatus("disconnected");
		};
		// Board identity plus `sub`: on a cold start the board (persisted cache)
		// is ready before auth, so the first attempt may run without an identity
		// — or fail its access fetch outright. A late `sub` re-runs the setup.
	}, [backend, appId, boardId, hasBoardData, version, sub]);

	// Identity is asserted on the live awareness whenever it changes, not only
	// at session creation.
	useEffect(() => {
		if (!awareness) return;
		awareness.setLocalStateField("sub", sub);
	}, [awareness, sub]);

	// Update peer states
	useEffect(() => {
		if (!awareness) {
			setPeerStates([]);
			activityTrackerRef.current.reset();
			if (cursorDataRef.current.snapshot.length > 0) {
				cursorDataRef.current.snapshot = EMPTY_CURSORS;
				for (const listener of cursorDataRef.current.listeners) listener();
			}
			return;
		}

		let expiryTimer: ReturnType<typeof setTimeout> | null = null;
		const updatePeers = () => {
			const states = awareness.getStates() as Map<number, any>;
			const invalidPeers: Set<number> | undefined = (awareness as any)
				?.__invalidPeers;
			const now = Date.now();
			const next: PeerPresence[] = [];
			const nextCursors: PeerCursor[] = [];
			const tracker = activityTrackerRef.current;
			tracker.observe(states, awareness.clientID);
			let nextExpiry = Number.POSITIVE_INFINITY;
			states.forEach((state, clientId) => {
				const isSelf = clientId === awareness.clientID;
				const isInvalid = invalidPeers?.has(clientId) ?? false;
				if (isSelf || isInvalid) return;
				const activeSeenAt = tracker.activeClickSeenAt(clientId);
				const presence = readPeerPresence(state, clientId, now, activeSeenAt);
				if (presence.activeNodeId && activeSeenAt !== undefined) {
					nextExpiry = Math.min(nextExpiry, activeSeenAt + 3000 - now);
				}
				const cursor = state?.cursor;
				if (
					cursor &&
					typeof cursor.x === "number" &&
					typeof cursor.y === "number"
				) {
					nextCursors.push({
						clientId,
						cursor: { x: cursor.x, y: cursor.y },
						sub: presence.sub,
						layerPath: presence.layerPath,
					});
				}
				next.push(presence);
			});

			// Sort both snapshots by clientId so the index-by-index equality checks
			// (cursorsEqual/presenceEqual) don't see spurious changes when the Map
			// iteration order shifts (e.g. a peer reconnects).
			next.sort((a, b) => a.clientId - b.clientId);
			nextCursors.sort((a, b) => a.clientId - b.clientId);

			// Cursors: publish to the external store (consumed only by the cursor
			// overlay, which re-renders itself without touching the board). Skip when
			// nothing moved so idle 20Hz cursor broadcasts don't re-render the overlay.
			if (!cursorsEqual(cursorDataRef.current.snapshot, nextCursors)) {
				cursorDataRef.current.snapshot = nextCursors;
				for (const listener of cursorDataRef.current.listeners) listener();
			}

			// Presence: only re-render the board when low-frequency presence actually
			// changes (selection, layer, active node, peer set) — not on cursor ticks.
			setPeerStates((prev) =>
				peerPresenceListEqual(prev, next) ? prev : next,
			);

			// A click ages out without any wire traffic: wake up once to clear it.
			if (expiryTimer !== null) clearTimeout(expiryTimer);
			expiryTimer = Number.isFinite(nextExpiry)
				? setTimeout(() => {
						expiryTimer = null;
						scheduleUpdate();
					}, nextExpiry + 1)
				: null;
		};

		// Remote peers broadcast cursors at ~20Hz each. Coalesce the resulting
		// burst of awareness "change" events into a single state update per frame
		// so peer presence no longer re-renders the whole board on every tick.
		let rafId: number | null = null;
		const scheduleUpdate = () => {
			if (rafId !== null) return;
			rafId = requestAnimationFrame(() => {
				rafId = null;
				updatePeers();
			});
		};

		awareness.on("change", scheduleUpdate);
		updatePeers();

		return () => {
			if (rafId !== null) cancelAnimationFrame(rafId);
			if (expiryTimer !== null) clearTimeout(expiryTimer);
			try {
				awareness.off("change", scheduleUpdate);
			} catch {}
		};
	}, [awareness]);

	// Listen for peer board updates — use refs to avoid reinstalling on every render
	useEffect(() => {
		if (!awareness) return;

		const handleBoardUpdate = ({
			added,
			updated,
		}: { added: number[]; updated: number[] }) => {
			const states = awareness.getStates() as Map<number, any>;
			const changedPeers = [...added, ...updated];

			for (const clientId of changedPeers) {
				if (clientId === awareness.clientID) continue;
				const state = states.get(clientId);
				const peerBoardUpdate = state?.boardUpdate as number | undefined;
				if (
					peerBoardUpdate &&
					peerBoardUpdate !== lastBoardUpdateRef.current.get(clientId)
				) {
					lastBoardUpdateRef.current.set(clientId, peerBoardUpdate);
					void boardRefetchRef.current();
					break;
				}
			}
		};

		awareness.on("update", handleBoardUpdate);
		return () => {
			try {
				awareness.off("update", handleBoardUpdate);
			} catch {}
		};
	}, [awareness]);

	// Broadcast cursor position via throttled interval (avoids 60fps rerenders).
	// Skip when the pointer hasn't moved since the last tick so idle peers don't
	// force every other client into a 20Hz no-op awareness change.
	useEffect(() => {
		if (!awareness) return;
		let lastX = Number.NaN;
		let lastY = Number.NaN;
		const interval = setInterval(() => {
			const pos = mousePositionRef.current;
			if (pos.x === lastX && pos.y === lastY) return;
			lastX = pos.x;
			lastY = pos.y;
			const flowPoint = screenToFlowPosition({
				x: pos.x,
				y: pos.y,
			});
			awareness.setLocalStateField("cursor", {
				x: flowPoint.x,
				y: flowPoint.y,
			});
		}, 50);
		return () => clearInterval(interval);
	}, [awareness, screenToFlowPosition]);

	// Broadcast layer path
	useEffect(() => {
		if (!awareness) return;
		awareness.setLocalStateField("layerPath", layerPath ?? "root");
	}, [awareness, layerPath]);

	// Initialize selection state
	useEffect(() => {
		if (!awareness) return;
		awareness.setLocalStateField("selection", { nodes: [] });
	}, [awareness]);

	// Update remote selections on nodes
	useEffect(() => {
		const map = new Map<string, RemoteSelectionParticipant[]>();
		for (const peer of peerStates) {
			if (!peer.selection.nodes.length) continue;
			for (const nodeId of peer.selection.nodes) {
				if (!nodeId) continue;
				const participant: RemoteSelectionParticipant = {
					clientId: peer.clientId,
					sub: peer.sub,
					self: Boolean(sub && peer.sub === sub),
					isActive: peer.activeNodeId === nodeId,
				};
				const existing = map.get(nodeId) ?? [];
				map.set(nodeId, [...existing, participant]);
			}
		}

		// Deduplicate participants by sub (same user with multiple sessions)
		// and sort by sub for stable ordering
		for (const [nodeId, participants] of map.entries()) {
			const seen = new Map<string, RemoteSelectionParticipant>();
			for (const p of participants) {
				const key = p.sub ?? `client:${p.clientId}`;
				const existing = seen.get(key);
				if (!existing || (p.isActive && !existing.isActive)) {
					seen.set(key, p);
				}
			}
			map.set(
				nodeId,
				[...seen.values()].sort((a, b) =>
					(a.sub ?? "").localeCompare(b.sub ?? ""),
				),
			);
		}

		// Check if selections actually changed (compare by sub, not clientId)
		let hasChanges = false;
		if (map.size !== remoteSelectionsRef.current.size) {
			hasChanges = true;
		} else {
			for (const [nodeId, participants] of map.entries()) {
				const prev = remoteSelectionsRef.current.get(nodeId);
				if (!prev || prev.length !== participants.length) {
					hasChanges = true;
					break;
				}
				for (let i = 0; i < participants.length; i++) {
					const p = participants[i];
					const prevP = prev[i];
					if (!prevP || p.sub !== prevP.sub || p.isActive !== prevP.isActive) {
						hasChanges = true;
						break;
					}
				}
				if (hasChanges) break;
			}
		}

		if (!hasChanges) return;

		remoteSelectionsRef.current = map;

		setNodes((nds: any) => {
			if (nds.length === 0) return nds;
			const updated = nds.map((node: any) => {
				if (
					node.type !== "node" &&
					node.type !== "callFunctionNode" &&
					node.type !== "layerNode"
				)
					return node;
				const participants = map.get(node.id) ?? [];
				const hasSelections = participants.length > 0;
				const hadSelections =
					!!node.data.remoteSelections && node.data.remoteSelections.length > 0;

				if (!hasSelections && !hadSelections) return node;

				return {
					...node,
					data: {
						...node.data,
						remoteSelections: hasSelections ? participants : undefined,
					},
				};
			});
			return updated;
		});
	}, [peerStates, setNodes, sub]);

	const reconnect = useCallback(() => {
		sessionRef.current?.reconnect();
	}, []);

	const broadcastActiveNode = useCallback(
		(nodeId: string | undefined) => {
			if (!awareness) return;
			awareness.setLocalStateField("activeNodeId", nodeId);
			awareness.setLocalStateField(
				"activeNodeTs",
				nodeId ? Date.now() : undefined,
			);
		},
		[awareness],
	);

	/** Local-clock time a user (any of their sessions) last did something. */
	const getPeerLastActiveAt = useCallback(
		(peerSub: string) => activityTrackerRef.current.lastActiveAt(peerSub),
		[],
	);
	/** Live activity predicates — local clock only, cheap enough to poll. */
	const isPeerTypingInEditor = useCallback(
		(peerSub: string) => activityTrackerRef.current.isTypingInEditor(peerSub),
		[],
	);
	const isPeerTypingInChat = useCallback(
		(peerSub: string) => activityTrackerRef.current.isTypingInChat(peerSub),
		[],
	);
	const isPeerAway = useCallback(
		(peerSub: string) => activityTrackerRef.current.isAway(peerSub),
		[],
	);

	return {
		awareness,
		connectionStatus,
		peerStates,
		cursorStore,
		reconnect,
		broadcastActiveNode,
		getPeerLastActiveAt,
		isPeerTypingInEditor,
		isPeerTypingInChat,
		isPeerAway,
	};
}
