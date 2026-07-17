export const WORKFLOW_EVENT_ENTRY_NODE_NAMES = new Set([
	"events_simple",
	"events_generic",
	"events_chat",
]);

interface PinLike {
	id?: string;
	connected_to?: string[];
	data_type?: string;
	pin_type?: string;
}

interface NodeLike {
	id: string;
	name: string;
	friendly_name?: string;
	pins?: Record<string, PinLike>;
}

interface LayerLike {
	nodes?: Record<string, NodeLike>;
	pins?: Record<string, PinLike>;
}

export interface WorkflowBoardLike {
	nodes?: Record<string, NodeLike>;
	layers?: Record<string, LayerLike>;
}

export interface RunnableWorkflowEventEntry {
	id: string;
	board_id: string;
	name: string;
	node_type: string;
	supported_event_types: string[];
	created_this_run: boolean;
}

/** A submitted/preview/no-change workspace must never enter the apply path. */
export function shouldApplyFlowScriptWorkspace(status: string | undefined) {
	return status === "queued";
}

function collectPersistedPinIds(board: WorkflowBoardLike): Set<string> {
	const pinIds = new Set<string>();
	const collectPins = (pins: Record<string, PinLike> | undefined) => {
		for (const [key, pin] of Object.entries(pins ?? {})) {
			pinIds.add(key);
			if (pin.id) pinIds.add(pin.id);
		}
	};

	for (const node of Object.values(board.nodes ?? {})) collectPins(node.pins);
	for (const layer of Object.values(board.layers ?? {})) {
		collectPins(layer.pins);
		for (const node of Object.values(layer.nodes ?? {})) collectPins(node.pins);
	}
	return pinIds;
}

function hasConnectedExecutionOutput(
	node: NodeLike,
	persistedPinIds: Set<string>,
): boolean {
	return Object.values(node.pins ?? {}).some(
		(pin) =>
			pin.pin_type === "Output" &&
			pin.data_type === "Execution" &&
			(pin.connected_to ?? []).some((pinId) => persistedPinIds.has(pinId)),
	);
}

/**
 * Check one persisted node before exposing or scheduling it as an app Event.
 * This deliberately requires a real execution edge: a bare Event entry is a no-op,
 * even when its node type would otherwise be compatible with the requested sink.
 */
export function isRunnableWorkflowEventEntry(
	board: WorkflowBoardLike,
	nodeId: string,
): boolean {
	const node = board.nodes?.[nodeId];
	return Boolean(
		node &&
			WORKFLOW_EVENT_ENTRY_NODE_NAMES.has(node.name) &&
			hasConnectedExecutionOutput(node, collectPersistedPinIds(board)),
	);
}

/**
 * Return only persisted workflow entries that actually lead into an execution graph.
 * Empty entries are not safe Event targets: registering one would schedule/expose a no-op.
 */
export function collectRunnableWorkflowEventEntries(
	board: WorkflowBoardLike,
	boardId: string,
	preExistingEventNodeIds: ReadonlySet<string>,
	supportedEventTypes: (nodeType: string) => readonly string[],
): RunnableWorkflowEventEntry[] {
	const persistedPinIds = collectPersistedPinIds(board);
	return Object.values(board.nodes ?? {})
		.filter(
			(node) =>
				WORKFLOW_EVENT_ENTRY_NODE_NAMES.has(node.name) &&
				hasConnectedExecutionOutput(node, persistedPinIds),
		)
		.map((node) => ({
			id: node.id,
			board_id: boardId,
			name: node.friendly_name || node.name,
			node_type: node.name,
			supported_event_types: [...supportedEventTypes(node.name)],
			created_this_run: !preExistingEventNodeIds.has(node.id),
		}));
}
