import type { IBoard } from "../schema/flow/board";

/**
 * Cross-frame protocol used by the University lesson runtime to capture the
 * current state of a board running in a side-by-side iframe (e.g. /use, /flow).
 *
 * Lesson side → iframe:  FL_REQUEST_BOARD_STATE { requestId }
 * Iframe   → lesson:    FL_BOARD_STATE        { requestId, ... }
 *                       FL_BOARD_BRIDGE_READY (announced by responder)
 *
 * All payloads carry `protocol: "flow-like-board-bridge"` so the listener can
 * reject unrelated messages without crashing on shape mismatches.
 */
export const BOARD_BRIDGE_PROTOCOL = "flow-like-board-bridge" as const;
export const BOARD_BRIDGE_NATIVE_EVENT =
	"flow-like-board-bridge:request-state" as const;

export interface BoardBridgeBaseMessage {
	readonly protocol: typeof BOARD_BRIDGE_PROTOCOL;
}

export interface BoardBridgeRequestMessage extends BoardBridgeBaseMessage {
	readonly type: "FL_REQUEST_BOARD_STATE";
	readonly requestId: string;
}

export interface BoardBridgeReadyMessage extends BoardBridgeBaseMessage {
	readonly type: "FL_BOARD_BRIDGE_READY";
	readonly appId?: string;
	readonly boardId?: string;
}

export interface BoardSnapshotNode {
	readonly id: string;
	readonly nodeTypeId: string;
	readonly name?: string;
	readonly coordinates?: ReadonlyArray<number>;
	readonly pins: Record<string, { readonly value?: unknown }>;
}

export interface BoardSnapshotConnection {
	readonly fromNodeId: string;
	readonly fromPin: string;
	readonly toNodeId: string;
	readonly toPin: string;
}

export interface BoardSnapshot {
	readonly appId: string;
	readonly boardId: string;
	readonly nodes: ReadonlyArray<BoardSnapshotNode>;
	readonly connections: ReadonlyArray<BoardSnapshotConnection>;
}

export interface BoardBridgeStateMessage extends BoardBridgeBaseMessage {
	readonly type: "FL_BOARD_STATE";
	readonly requestId: string;
	readonly snapshot: BoardSnapshot;
}

export interface BoardBridgeNativeRequestDetail {
	readonly resolve: (snapshot: BoardSnapshot) => void;
	readonly reject?: (error: Error) => void;
}

export type BoardBridgeMessage =
	| BoardBridgeRequestMessage
	| BoardBridgeReadyMessage
	| BoardBridgeStateMessage;

export function isBoardBridgeMessage(
	value: unknown,
): value is BoardBridgeMessage {
	return (
		typeof value === "object" &&
		value !== null &&
		(value as { protocol?: unknown }).protocol === BOARD_BRIDGE_PROTOCOL &&
		typeof (value as { type?: unknown }).type === "string"
	);
}

/**
 * Convert a saved IBoard model into the snapshot shape the validator expects.
 * Used both by the in-frame responder and by REST-based fallback paths.
 */
export function snapshotFromBoard(appId: string, board: IBoard): BoardSnapshot {
	const nodes: BoardSnapshotNode[] = Object.values(board.nodes ?? {}).map(
		(n) => {
			const pins: Record<string, { value?: unknown }> = {};
			for (const pin of Object.values(n.pins ?? {})) {
				if (!pin || pin.pin_type === "Output") continue;
				pins[pin.name] = { value: decodePinValue(pin.default_value) };
			}
			return {
				id: n.id,
				nodeTypeId: n.name,
				name: n.friendly_name,
				coordinates: n.coordinates ?? undefined,
				pins,
			};
		},
	);

	const connections: BoardSnapshotConnection[] = [];
	const pinOwner = new Map<string, { nodeId: string; pinName: string }>();
	for (const node of Object.values(board.nodes ?? {})) {
		for (const pin of Object.values(node.pins ?? {})) {
			if (pin) pinOwner.set(pin.id, { nodeId: node.id, pinName: pin.name });
		}
	}
	for (const node of Object.values(board.nodes ?? {})) {
		for (const pin of Object.values(node.pins ?? {})) {
			if (!pin || pin.pin_type !== "Output") continue;
			for (const targetPinId of pin.connected_to ?? []) {
				const target = pinOwner.get(targetPinId);
				if (!target) continue;
				connections.push({
					fromNodeId: node.id,
					fromPin: pin.name,
					toNodeId: target.nodeId,
					toPin: target.pinName,
				});
			}
		}
	}

	return { appId, boardId: board.id, nodes, connections };
}

function decodePinValue(raw: number[] | null | undefined): unknown {
	if (!raw || raw.length === 0) return undefined;
	try {
		const text = new TextDecoder("utf-8", { fatal: false }).decode(
			new Uint8Array(raw),
		);
		try {
			return JSON.parse(text);
		} catch {
			return text;
		}
	} catch {
		return undefined;
	}
}
