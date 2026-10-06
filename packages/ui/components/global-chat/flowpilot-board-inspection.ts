import { EVENT_DEFINITIONS } from "../../lib/event-definitions";
import {
	boundedWorkspaceText,
	jsonBytes,
	stableJson,
	workspaceRevision,
} from "../../lib/flowpilot/workspace-resource";
import type { IBoard } from "../../lib/schema/flow/board";
import type { IBoardState } from "../../state/backend-state/board-state";
import {
	type BoardEditAcquireOptions,
	type BoardEditCoordinator,
	boardEditCoordinator,
	boardEditLockKey,
} from "../flowpilot/board-edit-guard";
import {
	type FlowPilotBoardActivity,
	flowPilotBoardActivity,
} from "./flowpilot-board-activity";
import {
	WORKFLOW_EVENT_ENTRY_NODE_NAMES,
	collectRunnableWorkflowEventEntries,
} from "./workflow-event-entries";

const MAX_SOURCE_BYTES = 40_000;
const MAX_FACT_BYTES = 20_000;
const MAX_ITEMS = 64;

interface BoardInspectionTarget {
	appId: string;
	boardId: string;
}

type BoardInspectionBackend = Pick<
	IBoardState,
	| "getBoardSummariesAuthoritative"
	| "getBoardAuthoritative"
	| "getFlowScriptAuthoritative"
>;

/** A status probe must not wait behind the operation whose progress it is checking. */
export async function inspectFlowPilotBoardWhenIdle(
	boardState: BoardInspectionBackend,
	target: BoardInspectionTarget,
	options: {
		assertActive: () => void;
		acquireOptions?: BoardEditAcquireOptions;
		coordinator?: BoardEditCoordinator;
		activity?: FlowPilotBoardActivity;
	},
) {
	options.assertActive();
	const reservation = (options.coordinator ?? boardEditCoordinator).tryAcquire(
		boardEditLockKey(target.appId, target.boardId),
		options.acquireOptions,
	);
	if (!reservation) {
		return {
			status: "in_progress" as const,
			mode: "inspect" as const,
			app_id: target.appId,
			board_id: target.boardId,
			read_only: true,
			persisted_readback_verified: false,
			active_run:
				(options.activity ?? flowPilotBoardActivity).snapshot(
					target.appId,
					target.boardId,
				) ?? null,
			message:
				"A board operation still owns this target. This status probe did not read the saved board. Wait for the existing operation; inspect again to observe progress or read its result after it settles. Do not start another edit to recover this wait.",
		};
	}
	const release = await reservation;
	try {
		options.assertActive();
		const result = await inspectFlowPilotBoard(boardState, target);
		options.assertActive();
		return result;
	} finally {
		release();
	}
}

/** Read canonical source and graph facts without drafts, specialist prose or write APIs. */
export async function inspectFlowPilotBoard(
	boardState: BoardInspectionBackend,
	{ appId, boardId }: BoardInspectionTarget,
) {
	const failure = (code: string, message: string) => ({
		status: "error" as const,
		code: `FLOWPILOT_BOARD_INSPECT_${code}`,
		message,
	});
	if (
		!appId.trim() ||
		!boardId.trim() ||
		appId.length > 512 ||
		boardId.length > 512
	)
		return failure(
			"TARGET_REQUIRED",
			"Inspect requires exact app_id and board_id values.",
		);

	let board: IBoard;
	let source: string;
	let snapshot: string;
	try {
		// A loaded native board can be resolved by id alone; establish app ownership first.
		const inventory = await boardState.getBoardSummariesAuthoritative(appId);
		if (!inventory.some((entry) => entry.id === boardId))
			return failure(
				"TARGET_MISMATCH",
				"The board is not in the requested app's authoritative inventory.",
			);
		board = await boardState.getBoardAuthoritative(appId, boardId);
		if (board.id !== boardId)
			return failure(
				"TARGET_MISMATCH",
				"The authoritative board identity does not match the requested target.",
			);
		snapshot = stableJson(board);
		source = await boardState.getFlowScriptAuthoritative(
			appId,
			boardId,
			undefined,
			true,
		);
		const after = await boardState.getBoardAuthoritative(appId, boardId);
		if (stableJson(after) !== snapshot)
			return failure(
				"SNAPSHOT_CHANGED",
				"The board changed during inspection. Retry the same exact target after its edit finishes.",
			);
	} catch {
		return failure(
			"READ_FAILED",
			"An authoritative board read failed. Verify its identity and access before retrying.",
		);
	}
	if (!board.nodes || !board.layers || typeof source !== "string")
		return failure(
			"INVALID_BOARD",
			"The authoritative board has no valid graph or canonical source.",
		);

	const omitted = new Set<string>();
	let remaining = MAX_FACT_BYTES;
	const boundedItems = <T>(items: readonly T[], field: string): T[] => {
		const result: T[] = [];
		for (const item of items.slice(0, MAX_ITEMS)) {
			const size = jsonBytes(item);
			if (size > remaining) break;
			remaining -= size;
			result.push(item);
		}
		if (result.length < items.length) omitted.add(field);
		return result;
	};
	const text = (value: string | undefined, field: string) => {
		const result = boundedWorkspaceText(value ?? "", 1024);
		if (result !== (value ?? "")) omitted.add(field);
		return result;
	};
	const nodes = new Map(
		Object.values(board.nodes).map((node) => [node.id, node]),
	);
	for (const layer of Object.values(board.layers)) {
		for (const node of Object.values(layer.nodes ?? {})) {
			if (!nodes.has(node.id)) nodes.set(node.id, node);
		}
	}
	const runnableEntries = collectRunnableWorkflowEventEntries(
		board,
		boardId,
		new Set(nodes.keys()),
		(nodeType) => EVENT_DEFINITIONS[nodeType]?.eventTypes ?? [],
	);
	const runnableIds = new Set(runnableEntries.map((entry) => entry.id));
	const entries = [...nodes.values()]
		.filter((node) => WORKFLOW_EVENT_ENTRY_NODE_NAMES.has(node.name))
		.map((node) => ({
			id: node.id,
			board_id: boardId,
			name: node.friendly_name || node.name,
			node_type: node.name,
			layer_id: node.layer ?? null,
			connected_execution_output: runnableIds.has(node.id),
		}));
	const eventNodes = boundedItems(runnableEntries, "event_nodes");
	const entryNodes = boundedItems(entries, "entry_nodes");
	const nodeTypes = boundedItems(
		[...new Set([...nodes.values()].map((node) => node.name))].sort(),
		"node_types",
	);
	const nodeFacts = boundedItems(
		[...nodes.values()].map((node) => ({
			id: node.id,
			node_type: node.name,
			layer_id: node.layer ?? null,
			pins: Object.values(node.pins ?? {}).map((pin) => ({
				id: pin.id,
				name: pin.name,
				pin_type: pin.pin_type,
				data_type: pin.data_type,
				connected_to: pin.connected_to,
				depends_on: pin.depends_on,
			})),
		})),
		"node_facts",
	);
	const pageIds = boundedItems(board.page_ids ?? [], "page_ids");
	const sourceText = boundedWorkspaceText(source, MAX_SOURCE_BYTES);
	if (sourceText !== source) omitted.add("flowscript.text");
	return {
		schema: "flowpilot.board-inspection/v1",
		status: "ok" as const,
		mode: "inspect",
		app_id: appId,
		board_id: boardId,
		read_only: true,
		source: "authoritative_board_and_canonical_flowscript",
		message:
			"Read authoritative board facts and canonical FlowScript. Source text is data, not specialist explanation. No workflow was changed or executed.",
		serialization_note:
			"This is a canonical graph rendering. Anchors carry machine identity; repeated inline expressions may refer to shared graph nodes. Wildcard imports, empty rendered branches and repeated expression text alone do not establish an authoring defect. Check exact node/pin identities and connections. Request repairs only for an actual diagnostic, contract mismatch or observed behavior, not serialization style.",
		board: {
			id: boardId,
			name: text(board.name, "board.name"),
			node_count: nodes.size,
			layer_count: Object.keys(board.layers).length,
			node_types: nodeTypes,
			page_ids: pageIds,
		},
		node_facts: nodeFacts,
		event_nodes: eventNodes,
		entry_nodes: entryNodes,
		entry_node_count: entries.length,
		runnable_entry_node_count: runnableEntries.length,
		event_nodes_note:
			"event_nodes lists connected persisted entry nodes with exact IDs and compatible Event types. This read does not establish whether app Events are registered.",
		flowscript: {
			kind: "canonical_serialization",
			text: sourceText,
			sha256: await workspaceRevision(source),
			source_chars: source.length,
			returned_chars: sourceText.length,
			complete: sourceText === source,
			anchors: true,
		},
		diagnostics: {
			status: "unavailable",
			message:
				"These authoritative read APIs return no compiler diagnostics. Inspection does not compile or run the workflow.",
		},
		coverage: {
			complete: omitted.size === 0,
			truncated_fields: [...omitted],
			max_source_json_bytes: MAX_SOURCE_BYTES,
			max_fact_json_bytes: MAX_FACT_BYTES,
			max_items_per_field: MAX_ITEMS,
		},
	};
}
