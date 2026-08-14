import { create } from "zustand";
import type {
	CanvasSettings,
	SurfaceComponent,
} from "../components/a2ui/types";
import type {
	FlowScriptApplyOptions,
	FlowScriptApplyResultLike,
} from "../components/flow/flow-copilot/types";
import type { ILogMetadata } from "../lib";
import type { BoardEditReceiptHistoryMode } from "../lib/flowpilot/board-edit-job-delivery";
import type { FlowIrCommitToken } from "../lib/schema/copilot";
import type { IBoard } from "../lib/schema/flow/board";
import type { BoardCommand } from "../lib/schema/flow/copilot";
import type { INode } from "../lib/schema/flow/node";
import type { IApplyFlowIrCommitResponse } from "./backend-state/board-state";

/**
 * Live context published by an open flow board while it is mounted, so the global
 * assistant can read the board and apply edits through the board's own pipeline
 * (layer-aware, undo history, refetch, awareness) instead of detached backend calls.
 */
export interface AssistantBoardSurface {
	/** App the open board belongs to. */
	appId: string;
	/** Id of the open board. */
	boardId: string;
	/** Latest board data as fetched by the board's query (undefined while loading). */
	board: IBoard | undefined;
	/** Layer the user is currently editing (undefined = root layer). */
	currentLayer: string | undefined;
	/** Node catalog already fetched by the board, if available. */
	catalogNodes: INode[] | undefined;
	/** Ids of the flow nodes currently selected on the canvas. */
	selectedNodeIds: string[];
	/** Run/log metadata currently attached to the board, if any. */
	runContext: ILogMetadata | undefined;
	/** Layer-aware FlowScript apply routed through the board's command execution. */
	applyFlowScript: (
		flowscript: string,
		options?: FlowScriptApplyOptions,
	) => Promise<FlowScriptApplyResultLike | undefined>;
	/** Atomically applies the exact retained compiled workflow batch under a live-board CAS. */
	applyFlowIrCommit: (
		token: FlowIrCommitToken,
		deliveryId?: string,
		historyMode?: BoardEditReceiptHistoryMode,
	) => Promise<IApplyFlowIrCommitResponse>;
	/** Executes board copilot commands through the board's command pipeline. */
	executeCommands: (commands: BoardCommand[]) => Promise<void>;
	/** Pans/zooms the canvas to the given node. */
	focusNode: (nodeId: string) => void;
	/** Replaces the canvas selection with the given node ids. */
	selectNodes: (nodeIds: string[]) => void;
	/** Clears the run context attached to the board. */
	clearRunContext: () => void;
}

/**
 * Live context published by an open widget/page builder while it is mounted, so the
 * global assistant can generate UI components directly into the visible canvas.
 */
export interface AssistantWidgetSurface {
	/** Builder surface id (a2ui canvas id / active page id). */
	surfaceId: string;
	/** Distinguishes a persisted page builder from a reusable-widget builder. */
	kind: "page" | "widget";
	/** App the builder is editing, when known. */
	appId?: string;
	/** Owning board for a page builder, when known. */
	boardId?: string;
	/** Persisted page being edited when kind is `page`. */
	pageId?: string;
	/** Persisted reusable widget being edited when kind is `widget`. */
	widgetId?: string;
	/** Components currently on the canvas. */
	currentComponents: SurfaceComponent[];
	/** Canvas settings currently on the surface, customCss included. The assistant needs the live
	 * stylesheet to edit it; `applyComponents` merges what it sends back over this. */
	currentCanvasSettings?: CanvasSettings;
	/** Ids of the components currently selected in the builder. */
	selectedComponentIds: string[];
	/** Captures the canvas as a PNG data URL (null when capture fails). */
	captureScreenshot: () => Promise<string | null>;
	/** Applies staged components (optionally with canvas settings) to the canvas. */
	applyComponents: (
		components?: SurfaceComponent[],
		canvasSettings?: {
			backgroundColor?: string;
			padding?: string;
			customCss?: string;
		},
	) => void;
	/** Stages generated components as pending in the builder (preview/apply bar). */
	componentsGenerated: (components: SurfaceComponent[]) => void;
}

/**
 * Live context published by an open Data Studio page while it is mounted, so the global assistant
 * knows which app's data / ontology the user is looking at. The routing policy still checks that
 * app's configured Events before choosing a direct Data Studio fallback.
 */
export interface AssistantDataStudioSurface {
	/** App whose Data Studio is open. */
	appId: string;
	/** Human name of the app, when known. */
	appName?: string;
	/** Overlay/ontology currently selected on the page, if any. */
	overlayId?: string;
	/** Human name of the selected overlay, when known. */
	overlayName?: string;
	/** Table currently selected in the explorer, if any. */
	selectedTable?: string;
	/** Names of the overlays available in the app, for grounding. */
	overlayNames?: string[];
}

export interface AssistantSurfaceState {
	/** Board surface currently mounted and editable, or null when no board is open. */
	boardSurface: AssistantBoardSurface | null;
	/** Widget builder surface currently mounted, or null when no builder is open. */
	widgetSurface: AssistantWidgetSurface | null;
	/** Data Studio page currently open, or null when the user is not on one. */
	dataStudioSurface: AssistantDataStudioSurface | null;
	/** Monotonic counter bumped each time a surface asks the assistant UI to open. */
	openAssistantRequested: number;
	/**
	 * Prompt handed along with the latest open request (e.g. "Explain these nodes" deep links).
	 * The host chat consumes it once as a draft via consumeAssistantPrompt().
	 */
	pendingAssistantPrompt: string | null;
	/** Registers the live board surface (null to clear on unmount). */
	setBoardSurface: (surface: AssistantBoardSurface | null) => void;
	/** Registers the live widget surface (null to clear on unmount). */
	setWidgetSurface: (surface: AssistantWidgetSurface | null) => void;
	/** Registers the open Data Studio page (null to clear on unmount). */
	setDataStudioSurface: (surface: AssistantDataStudioSurface | null) => void;
	/** Asks the host app to open the global assistant (consumers watch openAssistantRequested). */
	requestOpenAssistant: (prompt?: string) => void;
	/** Returns the pending prompt once and clears it, so it is only turned into a draft a single time. */
	consumeAssistantPrompt: () => string | null;
}

export const useAssistantSurface = create<AssistantSurfaceState>(
	(set, get) => ({
		boardSurface: null,
		widgetSurface: null,
		dataStudioSurface: null,
		openAssistantRequested: 0,
		pendingAssistantPrompt: null,
		setBoardSurface: (surface) => set({ boardSurface: surface }),
		setWidgetSurface: (surface) => set({ widgetSurface: surface }),
		setDataStudioSurface: (surface) => set({ dataStudioSurface: surface }),
		requestOpenAssistant: (prompt) =>
			set((state) => ({
				openAssistantRequested: state.openAssistantRequested + 1,
				pendingAssistantPrompt: prompt ?? null,
			})),
		consumeAssistantPrompt: () => {
			const { pendingAssistantPrompt } = get();
			if (pendingAssistantPrompt) set({ pendingAssistantPrompt: null });
			return pendingAssistantPrompt;
		},
	}),
);

/** Imperative store access (getState/subscribe) for non-React consumers like tool bridges. */
export const assistantSurfaceStore = useAssistantSurface;
