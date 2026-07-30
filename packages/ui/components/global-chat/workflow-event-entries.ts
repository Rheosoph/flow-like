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

interface WorkflowBoardResultEnvelopeInput {
	specialistMessage: string | undefined;
	appliedCommands: number;
	persistedReadbackVerified: boolean;
	eventNodes: readonly RunnableWorkflowEventEntry[];
}

interface WorkflowBoardResultEnvelope {
	message: string | undefined;
	specialist_message?: string;
	applied_commands: number;
	board_persisted?: true;
	persisted_readback_verified?: true;
	event_nodes?: readonly RunnableWorkflowEventEntry[];
	event_registration_required?: true;
	event_nodes_note?: string;
}

const EVENT_REGISTRATION_NOTE =
	"These entry nodes exist on the board but are NOT app Events yet, so nothing triggers them. In a LATER assistant turn (never the same tool batch as flowpilot_board), call upsert_event once per entry the user asked for, and set_page_load_event for a page's load entry. The build is incomplete until every requested entry is registered.";

/**
 * Build the host-owned portion of a successful board result. Nested specialists report their
 * compiler workspace as "queued" before the host applies it, so their prose cannot be the
 * authoritative persistence status after the host has applied and read the board back.
 */
export function buildWorkflowBoardResultEnvelope({
	specialistMessage,
	appliedCommands,
	persistedReadbackVerified,
	eventNodes,
}: WorkflowBoardResultEnvelopeInput): WorkflowBoardResultEnvelope {
	const eventRegistration =
		eventNodes.length > 0
			? {
					event_nodes: eventNodes,
					event_registration_required: true as const,
					event_nodes_note: EVENT_REGISTRATION_NOTE,
				}
			: {};

	if (appliedCommands <= 0 || !persistedReadbackVerified) {
		return {
			message: specialistMessage,
			applied_commands: appliedCommands,
			...eventRegistration,
		};
	}

	return {
		message: `Applied and persisted ${appliedCommands} checked board change${appliedCommands === 1 ? "" : "s"}; canonical FlowScript readback from the persisted board succeeded.${eventNodes.length > 0 ? " Workflow entry nodes are ready, but their required app Event registration is still outstanding." : ""}`,
		specialist_message: specialistMessage,
		applied_commands: appliedCommands,
		board_persisted: true as const,
		persisted_readback_verified: true as const,
		...eventRegistration,
	};
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
