export interface ResolveAppEventTypeOptions {
	pageId?: string;
	requestedEventType?: string;
	existingEventType?: string;
	supportedWorkflowEventTypes?: readonly string[];
	defaultWorkflowEventType?: string;
}

export interface ResolveAppEventTargetOptions {
	requestedPageId?: string;
	requestedBoardId?: string;
	requestedNodeId?: string;
	existingPageId?: string | null;
	existingBoardId?: string;
	existingNodeId?: string;
}

export type ResolvedAppEventTarget =
	| {
			ok: true;
			kind: "page" | "workflow";
			pageId: string;
			boardId: string;
			nodeId: string;
			preserveExistingPageMetadata: boolean;
	  }
	| { ok: false; message: string };

const PAGE_WORKFLOW_TARGET_CONFLICT =
	"page_id and node_id target different Event kinds. Create the PAGE event with page_id + route only, and register workflow entries in separate upsert_event calls with board_id + node_id.";
const MISSING_EVENT_TARGET =
	"Provide page_id for a page event, OR board_id + node_id for an events_simple/events_generic/events_chat entry node returned by flowpilot_board.";

/**
 * Resolve the two supported Event target forms while keeping a page's optional owner-board
 * metadata separate from a workflow entry binding.
 */
export function resolveAppEventTarget({
	requestedPageId,
	requestedBoardId,
	requestedNodeId,
	existingPageId,
	existingBoardId,
	existingNodeId,
}: ResolveAppEventTargetOptions): ResolvedAppEventTarget {
	const requestedPage = requestedPageId?.trim() ?? "";
	const existingPage = existingPageId?.trim() ?? "";
	const pageId = requestedPage || existingPage;
	const requestedBoard = requestedBoardId?.trim() ?? "";
	const requestedNode = requestedNodeId?.trim() ?? "";

	if (pageId && requestedNode) {
		return { ok: false, message: PAGE_WORKFLOW_TARGET_CONFLICT };
	}

	if (pageId) {
		const existingBoard = existingBoardId?.trim() ?? "";
		const preserveExistingPageMetadata =
			Boolean(existingPage) &&
			pageId === existingPage &&
			(!requestedBoard || requestedBoard === existingBoard);
		return {
			ok: true,
			kind: "page",
			pageId,
			boardId:
				requestedBoard ||
				(preserveExistingPageMetadata ? existingBoard : "") ||
				"",
			nodeId: "",
			preserveExistingPageMetadata,
		};
	}

	const boardId = requestedBoard || existingBoardId?.trim() || "";
	const nodeId = requestedNode || existingNodeId?.trim() || "";
	if (!boardId || !nodeId) {
		return { ok: false, message: MISSING_EVENT_TARGET };
	}

	return {
		ok: true,
		kind: "workflow",
		pageId: "",
		boardId,
		nodeId,
		preserveExistingPageMetadata: false,
	};
}

/**
 * Page routes and workflow interfaces are different Event targets. A page route always uses the
 * dedicated persisted discriminator, regardless of a model-supplied workflow interface type.
 */
export function resolveAppEventType({
	pageId,
	requestedEventType,
	existingEventType,
	supportedWorkflowEventTypes,
	defaultWorkflowEventType,
}: ResolveAppEventTypeOptions): string {
	if (pageId?.trim()) return "page";

	const requested = requestedEventType?.trim();
	if (requested) return requested;

	const existing = existingEventType?.trim();
	if (existing && supportedWorkflowEventTypes?.includes(existing)) {
		return existing;
	}

	return defaultWorkflowEventType?.trim() || "quick_action";
}

/** Canonical fields that remove workflow-interface state from a persisted page route Event. */
export function pageEventPersistenceReset(pageId: string | undefined) {
	if (!pageId?.trim()) return undefined;
	return {
		nodeId: "",
		config: [] as number[],
		inputs: [] as never[],
		canary: null,
	};
}
