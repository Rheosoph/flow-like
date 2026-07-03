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

	/** Render the board as FlowScript source text (anchored by default for stable round-trips). */
	getFlowScript(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		anchors?: boolean,
	): Promise<string>;

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
	): Promise<UnifiedCopilotResponse>;

	/** Pre-run analysis: get required runtime variables and OAuth for a board */
	prerunBoard?(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IPrerunBoardResponse>;
}
