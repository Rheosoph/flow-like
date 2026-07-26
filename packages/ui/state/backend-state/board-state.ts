import type { SurfaceComponent } from "../../components/a2ui/types";
import type {
	IBoard,
	IConnectionMode,
	IExecutionMode,
	IExecutionStage,
	IGenericCommand,
	IIntercomEvent,
	ILog,
	ILogLevel,
	ILogMetadata,
	INode,
	IRunContext,
	IRunPayload,
	IVersionType,
} from "../../lib";
import type { IJwks, IRealtimeAccess } from "../../lib";
import type {
	BoardEditJob,
	BoardEditJobDeliveryClaim,
	BoardEditJobResolution,
	ChatImage,
	CopilotScope,
	CopilotToolContext,
	FlowIrCommitDisposition,
	FlowIrCommitDispositionResult,
	FlowIrCommitToken,
	UIActionContext,
	UnifiedChatMessage,
	UnifiedCopilotResponse,
} from "../../lib/schema/copilot";
import type { BoardCommand } from "../../lib/schema/flow/copilot";
import type { IPrerunBoardResponse } from "./types";

export interface IApplyFlowScriptResponse {
	commands: IGenericCommand[];
	board_commands: BoardCommand[];
	/** Non-blocking source repairs; reload canonical FlowScript when present. */
	corrections?: string[];
	diagnostics: string[];
	/** Aggregate-only authoritative count after the host refreshes the applied board. */
	final_board_node_count?: number;
}

export interface IApplyFlowIrCommitResponse extends IApplyFlowScriptResponse {
	status: "applied" | "stale" | "error";
	code?: string;
	message: string;
	/** True when native Apply returned a persisted receipt without mutating now. */
	replayed?: boolean;
	/** True only after durable remote/outbox sync and idempotent renderer history recording. */
	delivery_complete?: boolean;
}

export interface IFlowScriptDiagnostic {
	message: string;
	/** 1-based line number. */
	line: number;
	/** 1-based column number. */
	col: number;
	severity: "error" | "warning";
}

export interface ICheckFlowScriptReconcileResponse {
	parse_valid: boolean;
	reconcile_valid: boolean;
	idempotent: boolean;
	command_count: number;
	corrections: string[];
	diagnostics: string[];
}

export interface IBoardState {
	getBoards(appId: string): Promise<IBoard[]>;
	getCatalog(appId: string): Promise<INode[]>;
	getBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		forceFresh?: boolean,
	): Promise<IBoard>;

	// Realtime collaboration
	getRealtimeAccess(appId: string, boardId: string): Promise<IRealtimeAccess>;
	getRealtimeJwks(appId: string, boardId: string): Promise<IJwks>;
	createBoardVersion(
		appId: string,
		boardId: string,
		versionType: IVersionType,
	): Promise<[number, number, number]>;
	getBoardVersions(
		appId: string,
		boardId: string,
	): Promise<[number, number, number][]>;
	deleteBoard(appId: string, boardId: string): Promise<void>;
	// [AppId, BoardId, BoardName]
	getOpenBoards(): Promise<[string, string, string][]>;
	getBoardSettings(): Promise<IConnectionMode>;
	ensureAppPackagesInstalledForExecution?(appId: string): Promise<void>;

	executeBoard(
		appId: string,
		boardId: string,
		payload: IRunPayload,
		streamState?: boolean,
		eventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
	): Promise<ILogMetadata | undefined>;

	executeBoardRemote?(
		appId: string,
		boardId: string,
		payload: IRunPayload,
		streamState?: boolean,
		eventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
	): Promise<ILogMetadata | undefined>;

	listRuns(
		appId: string,
		boardId: string,
		nodeId?: string,
		from?: number,
		to?: number,
		status?: ILogLevel,
		lastMeta?: ILogMetadata,
		offset?: number,
		limit?: number,
		/** Load per-node activity summaries (heatmap only — extra query). */
		includeNodes?: boolean,
	): Promise<ILogMetadata[]>;
	queryRun(
		logMeta: ILogMetadata,
		query: string,
		offset?: number,
		limit?: number,
	): Promise<ILog[]>;

	undoBoard(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<void>;
	redoBoard(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<void>;

	upsertBoard(
		appId: string,
		boardId: string,
		name: string,
		description: string,
		logLevel: ILogLevel,
		stage: IExecutionStage,
		executionMode?: IExecutionMode,
		template?: IBoard,
	): Promise<void>;

	closeBoard(boardId: string): Promise<void>;

	executeCommand(
		appId: string,
		boardId: string,
		command: IGenericCommand,
	): Promise<IGenericCommand>;

	executeCommands(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<IGenericCommand[]>;

	applyFlowScript(
		appId: string,
		boardId: string,
		flowscript: string,
		currentLayer?: string,
		catalogNodes?: INode[],
		allowDeletions?: boolean,
	): Promise<IApplyFlowScriptResponse>;

	/** Render the board as FlowScript source text (anchored by default for stable round-trips). */
	getFlowScript(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		anchors?: boolean,
	): Promise<string>;

	/**
	 * Authoritative parse-only FlowScript validation via the native (Rust) parser, returning
	 * positioned diagnostics. Non-mutating, safe to call while typing. Optional — only present
	 * where a native parser exists (Flow-Like Studio / desktop); elsewhere the editor relies on
	 * the client-side structural linter.
	 */
	lintFlowScript?(flowscript: string): Promise<IFlowScriptDiagnostic[]>;

	/**
	 * Compile FlowScript against the authoritative live board and app-scoped catalog without
	 * applying it. Desktop-only; intended for post-generation validation and diagnostics.
	 */
	checkFlowScriptReconcile?(
		appId: string,
		boardId: string,
		flowscript: string,
	): Promise<ICheckFlowScriptReconcileResponse>;

	getExecutionElements(
		appId: string,
		boardId: string,
		pageId: string,
		wildcard?: boolean,
		version?: [number, number, number],
	): Promise<Record<string, unknown>>;

	/** Unified copilot chat that can handle board, UI, or both */
	copilot_chat(
		scope: CopilotScope,
		board: IBoard | null,
		catalogNodes: INode[] | undefined,
		selectedNodeIds: string[],
		currentSurface: SurfaceComponent[] | null,
		selectedComponentIds: string[],
		userPrompt: string,
		history: UnifiedChatMessage[],
		requestImages?: ChatImage[],
		onToken?: (token: string) => void,
		modelId?: string,
		reasoningEffort?: string,
		token?: string,
		runContext?: IRunContext,
		actionContext?: UIActionContext,
		/**
		 * Sub-agent run spawned while another copilot session is mid-turn (e.g. the global
		 * assistant's flowpilot_board). Agent-CLI backends use this to isolate the run in its
		 * own CLI process — the copilot CLI serializes requests within one process.
		 */
		nested?: boolean,
		/**
		 * Read-only sub-run (flowpilot_board explain): the board copilot answers a question about
		 * the board and emits no edits. Keeps it out of workflow-edit mode so its answer is streamed
		 * and returned instead of being coerced into producing (and failing to produce) an edit.
		 */
		readOnly?: boolean,
		/** Scope nested runtime tools to the delegated app/board. */
		toolContext?: CopilotToolContext,
		/** Stable invocation id used for cooperative cancellation of a long-running native chat. */
		requestId?: string,
		/** Immutable user-authored request, excluding host mode/run-context wrappers. */
		rawUserPrompt?: string,
		/** App owning this copilot surface; omitted for global/user-only chat. */
		appId?: string,
	): Promise<UnifiedCopilotResponse>;

	/** Best-effort cooperative cancellation; desktop providers may terminate their child process. */
	cancelCopilotChat?(requestId: string): Promise<void>;

	/** Resolve the native compiled workflow review reservation around Apply/Dismiss. */
	flowIrCommitDisposition?(
		token: FlowIrCommitToken,
		disposition: FlowIrCommitDisposition,
	): Promise<FlowIrCommitDispositionResult>;

	/** Atomically CAS-check and apply the exact batch retained by a compiled workflow token. */
	applyFlowIrCommit?(
		appId: string,
		token: FlowIrCommitToken,
		/** Stable native job id used to deduplicate renderer/server receipt replay. */
		deliveryId?: string,
	): Promise<IApplyFlowIrCommitResponse>;

	/** Create/recover a provider-neutral host review for an exact compiled workflow batch. */
	createBoardEditJob?(
		appId: string,
		requestId: string | undefined,
		token: FlowIrCommitToken,
	): Promise<BoardEditJob>;

	/** Rehydrate unresolved reviews after a panel navigation or renderer reload. */
	listBoardEditJobs?(
		appId?: string,
		boardId?: string,
		includeTerminal?: boolean,
	): Promise<BoardEditJob[]>;

	/** Read one native review job, primarily for idempotent resolution polling. */
	getBoardEditJob?(jobId: string): Promise<BoardEditJob | undefined>;

	/** Apply or deny the exact retained batch owned by the native job. */
	resolveBoardEditJob?(
		jobId: string,
		approved: boolean,
	): Promise<BoardEditJobResolution>;

	/** Claim one renderer delivery of an already-applied native receipt. */
	claimBoardEditJobDelivery?(jobId: string): Promise<BoardEditJobDeliveryClaim>;

	/** Mark renderer sync/history replay complete for the exact delivery lease. */
	ackBoardEditJobDelivery?(
		jobId: string,
		deliveryLeaseId: string,
	): Promise<BoardEditJob>;

	/** Pre-run analysis: get required runtime variables and OAuth for a board */
	prerunBoard?(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IPrerunBoardResponse>;
}
