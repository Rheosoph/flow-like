export type BoardChangeStatus = "added" | "removed" | "changed" | "moved";

export type BoardChangeKind =
	| "node"
	| "layer"
	| "variable"
	| "comment"
	| "board";

export type BoardChangeDetailKind = "value" | "wire" | "name" | "setting";

export interface IBoardChangeDetail {
	kind: BoardChangeDetailKind;
	/** Pin name or property key the detail is about. */
	field: string;
	/** Pin friendly name, or the English property name (translate by `field`). */
	label: string;
	/** Set for pin details: the label is board data, not a property name. */
	role?: "pin" | "runs-after" | "feeds";
	before?: string;
	after?: string;
	/** Sensitive pin or secret variable: the values are withheld. */
	masked?: boolean;
}

export interface IBoardChange {
	/** `${kind}:${id}` — stable across renders of the same diff. */
	key: string;
	kind: BoardChangeKind;
	id: string;
	status: BoardChangeStatus;
	title: string;
	/** Layer the item sits in; `undefined` is the board root. */
	layerId?: string;
	details: IBoardChangeDetail[];
	move?: { from: [number, number]; to: [number, number] };
	/** Canvas ids that show this change: node ids, a layer's id, a comment id. */
	focusIds: string[];
}

export type BoardEdgeStatus = "added" | "removed";

export interface IBoardDiff {
	changes: IBoardChange[];
	counts: Record<BoardChangeStatus, number>;
	/** Everything except changes that only moved something on the canvas. */
	logicCount: number;
	byKey: Map<string, IBoardChange>;
	/** Status of every node, layer and comment id that differs. */
	itemStatus: Map<string, BoardChangeStatus>;
	/** Edge id as `parseBoard` builds it (`${fromPinId}-${toPinId}`). */
	edgeStatus: Map<string, BoardEdgeStatus>;
	/** Layers holding at least one logic change, including in nested layers. */
	layersWithChanges: Set<string>;
}
