import type {
	IApplyFlowScriptResponse,
	IBoard,
	IBoardState,
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
} from "../../../";
import type { IJwks, IRealtimeAccess } from "../../../";
import type { SurfaceComponent } from "../../../components/a2ui/types";
import type {
	ChatImage,
	CopilotScope,
	UIActionContext,
	UnifiedChatMessage,
	UnifiedCopilotResponse,
} from "../../../lib/schema/copilot";

export class EmptyBoardState implements IBoardState {
	getBoards(appId: string): Promise<IBoard[]> {
		throw new Error("Method not implemented.");
	}
	getCatalog(appId: string): Promise<INode[]> {
		throw new Error("Method not implemented.");
	}
	getBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		forceFresh?: boolean,
	): Promise<IBoard> {
		throw new Error("Method not implemented.");
	}
	getRealtimeAccess(appId: string, boardId: string): Promise<IRealtimeAccess> {
		throw new Error("Method not implemented.");
	}
	getRealtimeJwks(appId: string, boardId: string): Promise<IJwks> {
		throw new Error("Method not implemented.");
	}
	createBoardVersion(
		appId: string,
		boardId: string,
		versionType: IVersionType,
	): Promise<[number, number, number]> {
		throw new Error("Method not implemented.");
	}
	getBoardVersions(
		appId: string,
		boardId: string,
	): Promise<[number, number, number][]> {
		throw new Error("Method not implemented.");
	}
	deleteBoard(appId: string, boardId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getOpenBoards(): Promise<[string, string, string][]> {
		throw new Error("Method not implemented.");
	}
	getBoardSettings(): Promise<IConnectionMode> {
		throw new Error("Method not implemented.");
	}
	executeBoard(
		appId: string,
		boardId: string,
		payload: IRunPayload,
		streamState?: boolean,
		eventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
	): Promise<ILogMetadata | undefined> {
		throw new Error("Method not implemented.");
	}
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
	): Promise<ILogMetadata[]> {
		throw new Error("Method not implemented.");
	}
	queryRun(
		logMeta: ILogMetadata,
		query: string,
		offset?: number,
		limit?: number,
	): Promise<ILog[]> {
		throw new Error("Method not implemented.");
	}
	undoBoard(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	redoBoard(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	upsertBoard(
		appId: string,
		boardId: string,
		name: string,
		description: string,
		logLevel: ILogLevel,
		stage: IExecutionStage,
		executionMode?: IExecutionMode,
		template?: IBoard,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	closeBoard(boardId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	executeCommand(
		appId: string,
		boardId: string,
		command: IGenericCommand,
	): Promise<IGenericCommand> {
		throw new Error("Method not implemented.");
	}
	executeCommands(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<IGenericCommand[]> {
		throw new Error("Method not implemented.");
	}

	applyFlowScript(
		appId: string,
		boardId: string,
		flowscript: string,
		currentLayer?: string,
		catalogNodes?: INode[],
		allowDeletions?: boolean,
	): Promise<IApplyFlowScriptResponse> {
		throw new Error("Method not implemented.");
	}

	getFlowScript(
		appId: string,
		boardId: string,
		version?: [number, number, number],
		anchors?: boolean,
	): Promise<string> {
		throw new Error("Method not implemented.");
	}

	getExecutionElements(
		appId: string,
		boardId: string,
		pageId: string,
		wildcard?: boolean,
	): Promise<Record<string, unknown>> {
		throw new Error("Method not implemented.");
	}

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
	): Promise<UnifiedCopilotResponse> {
		throw new Error("Method not implemented.");
	}
}
