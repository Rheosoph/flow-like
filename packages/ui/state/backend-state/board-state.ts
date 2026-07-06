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
	ChatImage,
	CopilotScope,
	UIActionContext,
	UnifiedChatMessage,
	UnifiedCopilotResponse,
} from "../../lib/schema/copilot";
import type { BoardCommand } from "../../lib/schema/flow/copilot";
import type { IPrerunBoardResponse } from "./types";

export interface IApplyFlowScriptResponse {
	commands: IGenericCommand[];
	board_commands: BoardCommand[];
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

	getExecutionElements(
		appId: string,
		boardId: string,
		pageId: string,
		wildcard?: boolean,
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
	): Promise<UnifiedCopilotResponse>;

	/** Pre-run analysis: get required runtime variables and OAuth for a board */
	prerunBoard?(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IPrerunBoardResponse>;
}
