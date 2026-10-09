import { createProjectMethods } from "./projects.js";
import { createDeviceMethods } from "./devices.js";
import { createAppMethods } from "./apps.js";
import { resolveAuth } from "./auth.js";
import { createBitMethods } from "./bits.js";
import { createBoardMethods } from "./boards.js";
import { createChatMethods } from "./chat.js";
import {
	type HttpClient,
	type RequestOptions,
	createHttpClient,
	normalizeBaseUrl,
} from "./client.js";
import { createDatabaseMethods } from "./database.js";
import { createEmbeddingMethods } from "./embeddings.js";
import { FlowLikeError } from "./errors.js";
import { createEventMethods } from "./events.js";
import { createExecutionMethods } from "./execution.js";
import { createFileMethods } from "./files.js";
import { createSinkMethods } from "./sinks.js";
import type { FlowLikeClientOptions } from "./types.js";
import { createWorkflowMethods } from "./workflows.js";
import type { FlowLikeChat, FlowLikeEmbeddingModel } from "./integrations.js";

export type * from "./integrations.js";

export class FlowLikeClient {
	private readonly http: HttpClient;
	private readonly baseUrl: string;
	private readonly token: string;

	constructor(options?: FlowLikeClientOptions) {
		const baseUrl = options?.baseUrl ?? process.env.FLOW_LIKE_BASE_URL;

		if (!baseUrl) {
			throw new FlowLikeError(
				"No base URL provided. Set FLOW_LIKE_BASE_URL or pass baseUrl in options.",
			);
		}

		const auth = resolveAuth(options?.pat, options?.apiKey);
		this.baseUrl = normalizeBaseUrl(baseUrl);
		this.token = auth.token;
		this.http = createHttpClient(baseUrl, auth);

		for (const methods of [
			createWorkflowMethods(this.http),
			createEventMethods(this.http),
			createFileMethods(this.http),
			createDatabaseMethods(this.http),
			createExecutionMethods(this.http),
			createSinkMethods(this.http),
			createChatMethods(this.http),
			createEmbeddingMethods(this.http),
			createAppMethods(this.http),
			createBitMethods(this.http),
			createBoardMethods(this.http),
			createProjectMethods(this.http),
			createDeviceMethods(this.http),
		]) {
			for (const [name, method] of Object.entries(methods))
				Object.defineProperty(this, name, {
					value: method.bind(methods),
					enumerable: true,
				});
		}
	}

	/** Access a newer public API route using backend JSON field names. Paths begin below /api/v1. */
	request<T = unknown>(
		method: string,
		path: string,
		options?: RequestOptions,
	): Promise<T> {
		return this.http.request<T>(method, path, options);
	}

	async asLangChainChat(
		bitId: string,
		options?: {
			temperature?: number;
			maxTokens?: number;
			topP?: number;
			stop?: string[];
		},
	): Promise<FlowLikeChat> {
		const { FlowLikeChatModel } = await import("./langchain.js");
		return new FlowLikeChatModel({
			baseUrl: this.baseUrl,
			token: this.token,
			bitId,
			...options,
		});
	}

	async asLangChainEmbeddings(bitId: string): Promise<FlowLikeEmbeddingModel> {
		const { FlowLikeEmbeddings } = await import("./langchain.js");
		return new FlowLikeEmbeddings({
			baseUrl: this.baseUrl,
			token: this.token,
			bitId,
		});
	}
}

export interface FlowLikeClient
	extends ReturnType<typeof createWorkflowMethods>,
		ReturnType<typeof createEventMethods>,
		ReturnType<typeof createFileMethods>,
		ReturnType<typeof createDatabaseMethods>,
		ReturnType<typeof createExecutionMethods>,
		ReturnType<typeof createSinkMethods>,
		ReturnType<typeof createChatMethods>,
		ReturnType<typeof createEmbeddingMethods>,
		ReturnType<typeof createAppMethods>,
		ReturnType<typeof createBitMethods>,
		ReturnType<typeof createBoardMethods>,
		ReturnType<typeof createProjectMethods>,
		ReturnType<typeof createDeviceMethods> {}

export type { SSEChunk, RequestOptions, QueryParams } from "./client.js";
export * from "./types.js";
export * from "./errors.js";
