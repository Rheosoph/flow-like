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
import { createRealtimeSession } from "../lib";
import { normalizeSelectionNodes } from "../lib/flow-board-utils";
import type { IBoard } from "../lib/schema/flow/board";

export interface PeerPresence {
	clientId: number;
	cursor?: { x: number; y: number };
	/** The sub (subject) from the auth token - use this to resolve user info via API */
	sub?: string;
	layerPath: string;
	selection: { nodes: string[] };
	/** The node the user just clicked/focused — cleared after a short timeout */
	activeNodeId?: string;
	/** Timestamp of the last active node click for freshness detection */
	activeNodeTs?: number;
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
	} | null>(null);
	const [peerStates, setPeerStates] = useState<PeerPresence[]>([]);
	const remoteSelectionsRef = useRef<Map<string, RemoteSelectionParticipant[]>>(
		new Map(),
	);

	// Stable ref for board.refetch so the boardUpdate listener doesn't reinstall every render
	const boardRefetchRef = useRef(board.refetch);
	boardRefetchRef.current = board.refetch;

	// Track the last seen boardUpdate value per peer to detect actual changes
	const lastBoardUpdateRef = useRef<Map<number, number>>(new Map());

	const hasBoardData = !!board.data;

	// Use a ref for signaling servers so changes don't trigger session recreation.
	// The session is created once with whatever servers are available (fallback kicks in
	// inside createRealtimeSession when undefined). When the hub loads later with the
	// same URL, no reconnect is needed.
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
		const setup = async () => {
			try {
				const offline = await backend.isOffline(appId);

				if (!hasBoardData || typeof version !== "undefined") return;
				if (offline) return;

				const room = sessionKey;
				const [access, jwks] = await Promise.all([
					backend.boardState.getRealtimeAccess(appId, boardId),
					backend.boardState
						.getRealtimeJwks(appId, boardId)
						.catch(() => undefined as any),
				]);

				const session = await createRealtimeSession({
					room,
					access,
					jwks,
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
				});

				if (disposed) {
					session.dispose();
					return;
				}

				sessionRef.current = {
					dispose: session.dispose,
					reconnect: session.reconnect,
				};
				awarenessRef.current = session.awareness;
				commandAwarenessRef.current = session.awareness;
				sessionInitializedRef.current = sessionKey;
				setAwareness(session.awareness);
				// Don't set "connected" here — let the WebrtcProvider's
				// onStatusChange callback report the actual signaling state
			} catch (e) {
				console.warn("Realtime setup failed:", e);
				setConnectionStatus("disconnected");
			}
		};
		void setup();

		return () => {
			disposed = true;
			sessionInitializedRef.current = null;
			try {
				sessionRef.current?.dispose();
			} catch {}
			sessionRef.current = null;
			awarenessRef.current = undefined;
			commandAwarenessRef.current = undefined;
			setAwareness(undefined);
			setConnectionStatus("disconnected");
		};
		// Only depend on board identity and essential data, not profile updates
		// Profile updates are handled by a separate effect that updates awareness
	}, [backend, appId, boardId, hasBoardData, version]);

	// Update peer states
	useEffect(() => {
		if (!awareness) {
			setPeerStates([]);
			return;
		}

		const updatePeers = () => {
			const states = awareness.getStates() as Map<number, any>;
			const invalidPeers: Set<number> | undefined = (awareness as any)
				?.__invalidPeers;
			const now = Date.now();
			const next: PeerPresence[] = [];
			states.forEach((state, clientId) => {
				const isSelf = clientId === awareness.clientID;
				const isInvalid = invalidPeers?.has(clientId) ?? false;
				if (isSelf || isInvalid) return;
				const cursor = state?.cursor;
				const activeNodeTs = state?.activeNodeTs as number | undefined;
				const activeNodeFresh = activeNodeTs && now - activeNodeTs < 3000;
				next.push({
					clientId,
					cursor: cursor ? { x: cursor.x, y: cursor.y } : undefined,
					sub: state?.sub,
					layerPath: state?.layerPath ?? "root",
					selection: {
						nodes: normalizeSelectionNodes(state?.selection?.nodes),
					},
					activeNodeId: activeNodeFresh
						? (state?.activeNodeId as string | undefined)
						: undefined,
					activeNodeTs: activeNodeFresh ? activeNodeTs : undefined,
				});
			});
			setPeerStates(next);
		};

		const handleChange = () => updatePeers();
		awareness.on("change", handleChange);
		updatePeers();

		return () => {
			try {
				awareness.off("change", handleChange);
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

	// Broadcast cursor position via throttled interval (avoids 60fps rerenders)
	useEffect(() => {
		if (!awareness) return;
		const interval = setInterval(() => {
			const pos = mousePositionRef.current;
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
				if (node.type !== "node" && node.type !== "callFunctionNode")
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
	}, [peerStates, setNodes]);

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

	return {
		awareness,
		connectionStatus,
		peerStates,
		reconnect,
		broadcastActiveNode,
	};
}
