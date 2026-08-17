import type { IBoard, INode, IPin } from "@flow-like/flow-like-ui";
import { isEqual } from "lodash-es";

interface SystemTimeLike {
	secs_since_epoch?: number;
	nanos_since_epoch?: number;
}

export interface MergeBoardResult {
	merged: IBoard;
	/** True when `merged` differs from the local board beyond `updated_at`. */
	changed: boolean;
}

const systemTimeToNumber = (time?: SystemTimeLike): number => {
	if (!time) return 0;
	return (
		(time.secs_since_epoch ?? 0) * 1_000_000_000 + (time.nanos_since_epoch ?? 0)
	);
};

const hasIncompletePageIds = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): boolean =>
	(remoteBoard.page_ids?.length ?? 0) === 0 &&
	(localBoard?.page_ids?.length ?? 0) > 0;

export const shouldApplyRemoteBoard = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): boolean => {
	if (!localBoard) return true;

	if (hasIncompletePageIds(remoteBoard, localBoard)) {
		return false;
	}

	const remoteUpdated = systemTimeToNumber(remoteBoard.updated_at);
	const localUpdated = systemTimeToNumber(localBoard.updated_at);

	if (remoteUpdated > 0 && localUpdated > 0 && remoteUpdated < localUpdated) {
		return false;
	}

	return true;
};

export const getRemoteBoardSkipReason = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): string | null => {
	if (!localBoard) return null;

	if (hasIncompletePageIds(remoteBoard, localBoard)) {
		return "remote page_ids empty while local board still has pages";
	}

	const remoteUpdated = systemTimeToNumber(remoteBoard.updated_at);
	const localUpdated = systemTimeToNumber(localBoard.updated_at);

	if (remoteUpdated > 0 && localUpdated > 0 && remoteUpdated < localUpdated) {
		return "remote board updated_at is older than local board";
	}

	return null;
};

const isSensitivePin = (pin: IPin | undefined): boolean =>
	pin?.options?.sensitive === true;

/**
 * The server strips secret variable values and sensitive pin literals from every board
 * response, so a remote `null` on one of those is "not disclosed", not "cleared". Local
 * execution needs the real values, so they are carried over from the local copy. This has to
 * run before node merging: the remote node's runtime hash still reflects the value it does not
 * carry, so the hash fast path would otherwise take the stripped node as-is.
 */
const preserveSecretValues = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): IBoard => {
	if (!localBoard) return remoteBoard;

	for (const [varId, remoteVar] of Object.entries(remoteBoard.variables)) {
		const localVar = localBoard.variables[varId];
		if (
			localVar?.secret &&
			remoteVar.secret &&
			remoteVar.default_value == null &&
			localVar.default_value != null
		) {
			remoteVar.default_value = localVar.default_value;
		}
	}

	const preservePins = (
		remotePins: Record<string, IPin> | undefined,
		localPins: Record<string, IPin> | undefined,
	) => {
		if (!remotePins || !localPins) return;
		for (const [pinId, remotePin] of Object.entries(remotePins)) {
			const localPin = localPins[pinId];
			if (
				isSensitivePin(remotePin) &&
				isSensitivePin(localPin) &&
				remotePin.default_value == null &&
				localPin.default_value != null
			) {
				remotePin.default_value = localPin.default_value;
			}
		}
	};
	for (const [nodeId, remoteNode] of Object.entries(remoteBoard.nodes ?? {})) {
		preservePins(remoteNode.pins, localBoard.nodes?.[nodeId]?.pins);
	}
	for (const [layerId, remoteLayer] of Object.entries(remoteBoard.layers ?? {})) {
		const localLayer = localBoard.layers?.[layerId];
		preservePins(remoteLayer.pins, localLayer?.pins);
		for (const [nodeId, remoteNode] of Object.entries(remoteLayer.nodes ?? {})) {
			preservePins(remoteNode.pins, localLayer?.nodes?.[nodeId]?.pins);
		}
	}

	return remoteBoard;
};

const comparableNodeWithoutRuntimeHash = (
	node: INode,
	localNode?: INode,
): INode => {
	const comparable = structuredClone(node);
	comparable.hash = undefined;

	if (comparable.wasm == null && localNode?.wasm != null) {
		comparable.wasm = structuredClone(localNode.wasm);
	}

	return comparable;
};

const preserveNodeRuntimeFields = (
	remoteNode: INode,
	localNode?: INode,
): INode => {
	if (!localNode) return remoteNode;

	// Identical runtime hashes already prove content equality — skip the
	// clone + deep-compare below, which dominates merge cost on large boards.
	if (localNode.hash != null && remoteNode.hash === localNode.hash) {
		if (remoteNode.wasm == null && localNode.wasm != null) {
			remoteNode.wasm = structuredClone(localNode.wasm);
		}
		return remoteNode;
	}

	const nodesMatchIgnoringRuntimeHash = isEqual(
		comparableNodeWithoutRuntimeHash(remoteNode, localNode),
		comparableNodeWithoutRuntimeHash(localNode),
	);

	if (
		localNode.hash != null &&
		(remoteNode.hash == null || nodesMatchIgnoringRuntimeHash)
	) {
		remoteNode.hash = localNode.hash;
	}

	if (remoteNode.wasm == null && localNode.wasm != null) {
		remoteNode.wasm = structuredClone(localNode.wasm);
	}

	return remoteNode;
};

const preserveBoardRuntimeFields = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): IBoard => {
	if (!localBoard) return remoteBoard;

	for (const [nodeId, remoteNode] of Object.entries(remoteBoard.nodes)) {
		preserveNodeRuntimeFields(remoteNode, localBoard.nodes[nodeId]);
	}

	for (const [layerId, remoteLayer] of Object.entries(remoteBoard.layers)) {
		const localLayer = localBoard.layers[layerId];
		if (!localLayer) continue;

		for (const [nodeId, remoteNode] of Object.entries(remoteLayer.nodes)) {
			preserveNodeRuntimeFields(remoteNode, localLayer.nodes[nodeId]);
		}
	}

	return remoteBoard;
};

const cloneBoard = (board: IBoard): IBoard => structuredClone(board);

export const mergeRemoteBoard = (
	remoteBoard: IBoard,
	localBoard?: IBoard,
): IBoard => {
	const merged = preserveBoardRuntimeFields(
		preserveSecretValues(cloneBoard(remoteBoard), localBoard),
		localBoard,
	);

	if (hasIncompletePageIds(merged, localBoard)) {
		merged.page_ids = localBoard?.page_ids ?? merged.page_ids;
	}

	return merged;
};

export const boardsDifferIgnoringUpdatedAt = (
	incomingBoard: IBoard,
	currentBoard?: IBoard,
): boolean => {
	if (!currentBoard) return true;

	const comparableBoard = cloneBoard(incomingBoard);
	comparableBoard.updated_at = currentBoard.updated_at;

	return !isEqual(comparableBoard, currentBoard);
};

export function mergeBoardWithLocal(
	remoteBoard: IBoard,
	localBoard?: IBoard,
): MergeBoardResult {
	const merged = mergeRemoteBoard(remoteBoard, localBoard);
	const changed = boardsDifferIgnoringUpdatedAt(merged, localBoard);
	return { merged, changed };
}
