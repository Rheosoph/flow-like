import { createAppMethods } from "./apps.js";
import { resolveAuth } from "./auth.js";
import { createBitMethods } from "./bits.js";
import { createBoardMethods } from "./boards.js";
import { createChatMethods } from "./chat.js";
import { type HttpClient, createHttpClient } from "./client.js";
import { createDatabaseMethods } from "./database.js";
import { createEmbeddingMethods } from "./embeddings.js";
import { FlowLikeError } from "./errors.js";
import { createEventMethods } from "./events.js";
import { createExecutionMethods } from "./execution.js";
import { createFileMethods } from "./files.js";
import { createSinkMethods } from "./sinks.js";
import type { FlowLikeClientOptions } from "./types.js";
import { createWorkflowMethods } from "./workflows.js";

export type { Connection as LanceConnection } from "@lancedb/lancedb";

export class FlowLikeClient {
	private readonly http: HttpClient;
	private readonly baseUrl: string;
	private readonly token: string;

	readonly triggerWorkflow;
	readonly triggerWorkflowAsync;
	readonly triggerEvent;
	readonly triggerEventAsync;
	readonly listFiles;
	readonly uploadFile;
	readonly downloadFile;
	readonly deleteFile;
	readonly presignData;
	readonly getDbCredentials;
	readonly getDbCredentialsRaw;
	readonly createLanceConnection;
	readonly listTables;
	readonly getTableSchema;
	readonly queryTable;
	readonly addToTable;
	readonly deleteFromTable;
	readonly countItems;
	readonly getRunStatus;
	readonly pollExecution;
	readonly triggerHttpSink;
	readonly chatCompletions;
	readonly chatCompletionsStream;
	readonly getUsage;
	readonly embed;
	readonly listApps;
	readonly getApp;
	readonly createApp;
	readonly health;
	readonly searchBits;
	readonly getBit;
	readonly listLlms;
	readonly listEmbeddingModels;
	readonly listBoards;
	readonly getBoard;
	readonly upsertBoard;
	readonly deleteBoard;
	readonly prerunBoard;
	readonly getBoardVersions;
	readonly versionBoard;
	readonly executeCommands;

	constructor(options?: FlowLikeClientOptions) {
		const baseUrl = options?.baseUrl ?? process.env.FLOW_LIKE_BASE_URL;

		if (!baseUrl) {
			throw new FlowLikeError(
				"No base URL provided. Set FLOW_LIKE_BASE_URL or pass baseUrl in options.",
			);
		}

		const auth = resolveAuth(options?.pat, options?.apiKey);
		this.baseUrl = baseUrl;
		this.token = auth.token;
		this.http = createHttpClient(baseUrl, auth);

		const workflows = createWorkflowMethods(this.http);
		this.triggerWorkflow = workflows.triggerWorkflow;
		this.triggerWorkflowAsync = workflows.triggerWorkflowAsync;

		const events = createEventMethods(this.http);
		this.triggerEvent = events.triggerEvent;
		this.triggerEventAsync = events.triggerEventAsync;

		const files = createFileMethods(this.http);
		this.listFiles = files.listFiles;
		this.uploadFile = files.uploadFile;
		this.downloadFile = files.downloadFile;
		this.deleteFile = files.deleteFile;
		this.presignData = files.presignData;

		const database = createDatabaseMethods(this.http);
		this.getDbCredentials = database.getDbCredentials;
		this.getDbCredentialsRaw = database.getDbCredentialsRaw;
		this.createLanceConnection = database.createLanceConnection.bind(database);
		this.listTables = database.listTables;
		this.getTableSchema = database.getTableSchema;
		this.queryTable = database.queryTable;
		this.addToTable = database.addToTable;
		this.deleteFromTable = database.deleteFromTable;
		this.countItems = database.countItems;

		const execution = createExecutionMethods(this.http);
		this.getRunStatus = execution.getRunStatus;
		this.pollExecution = execution.pollExecution;

		const sinks = createSinkMethods(this.http);
		this.triggerHttpSink = sinks.triggerHttpSink;

		const chat = createChatMethods(this.http);
		this.chatCompletions = chat.chatCompletions;
		this.chatCompletionsStream = chat.chatCompletionsStream;
		this.getUsage = chat.getUsage;

		const embeddings = createEmbeddingMethods(this.http);
		this.embed = embeddings.embed;

		const apps = createAppMethods(this.http);
		this.listApps = apps.listApps;
		this.getApp = apps.getApp;
		this.createApp = apps.createApp;
		this.health = apps.health;

		const bits = createBitMethods(this.http);
		this.searchBits = bits.searchBits;
		this.getBit = bits.getBit;
		this.listLlms = bits.listLlms;
		this.listEmbeddingModels = bits.listEmbeddingModels;

		const boards = createBoardMethods(this.http);
		this.listBoards = boards.listBoards;
		this.getBoard = boards.getBoard;
		this.upsertBoard = boards.upsertBoard;
		this.deleteBoard = boards.deleteBoard;
		this.prerunBoard = boards.prerunBoard;
		this.getBoardVersions = boards.getBoardVersions;
		this.versionBoard = boards.versionBoard;
		this.executeCommands = boards.executeCommands;
	}

	async asLangChainChat(
		bitId: string,
		options?: {
			temperature?: number;
			maxTokens?: number;
			topP?: number;
			stop?: string[];
		},
	) {
		const { FlowLikeChatModel } = await import("./langchain.js");
		return new FlowLikeChatModel({
			baseUrl: this.baseUrl,
			token: this.token,
			bitId,
			...options,
		});
	}

	async asLangChainEmbeddings(bitId: string) {
		const { FlowLikeEmbeddings } = await import("./langchain.js");
		return new FlowLikeEmbeddings({
			baseUrl: this.baseUrl,
			token: this.token,
			bitId,
		});
	}
}

export type { SSEChunk } from "./client.js";
export * from "./types.js";
export * from "./errors.js";
export type {
	FlowLikeChatModelParams,
	FlowLikeEmbeddingsParams,
} from "./langchain.js";
