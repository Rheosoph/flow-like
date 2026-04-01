import type { CallbackManagerForLLMRun } from "@langchain/core/callbacks/manager";
import { Embeddings, type EmbeddingsParams } from "@langchain/core/embeddings";
import {
	BaseChatModel,
	type BaseChatModelParams,
} from "@langchain/core/language_models/chat_models";
import { AIMessage, type BaseMessage } from "@langchain/core/messages";
import type { ChatResult } from "@langchain/core/outputs";
import { stripTrailingSlashes } from "./client.js";

export interface FlowLikeChatModelParams extends BaseChatModelParams {
	baseUrl: string;
	token: string;
	bitId: string;
	temperature?: number;
	maxTokens?: number;
	topP?: number;
	stop?: string[];
}

export interface FlowLikeEmbeddingsParams extends EmbeddingsParams {
	baseUrl: string;
	token: string;
	bitId: string;
}

function buildHeaders(token: string): Record<string, string> {
	const headers: Record<string, string> = {
		"Content-Type": "application/json",
	};
	if (token.startsWith("pat_")) {
		headers["Authorization"] = token;
	} else {
		headers["X-API-Key"] = token;
	}
	return headers;
}

const ROLE_MAP: Record<string, string> = {
	human: "user",
	ai: "assistant",
	system: "system",
	tool: "tool",
};

export class FlowLikeChatModel extends BaseChatModel {
	lc_namespace = ["flow-like"];
	private baseUrl: string;
	private token: string;
	private bitId: string;
	private temperature?: number;
	private maxTokens?: number;
	private topP?: number;
	private stopSequences?: string[];

	constructor(params: FlowLikeChatModelParams) {
		super(params);
		this.baseUrl = stripTrailingSlashes(params.baseUrl);
		this.token = params.token;
		this.bitId = params.bitId;
		this.temperature = params.temperature;
		this.maxTokens = params.maxTokens;
		this.topP = params.topP;
		this.stopSequences = params.stop;
	}

	_llmType(): string {
		return "flow-like";
	}

	async _generate(
		messages: BaseMessage[],
		options: this["ParsedCallOptions"],
		_runManager?: CallbackManagerForLLMRun,
	): Promise<ChatResult> {
		const body: Record<string, unknown> = {
			messages: messages.map((m) => ({
				role: ROLE_MAP[m._getType()] ?? "user",
				content:
					typeof m.content === "string" ? m.content : JSON.stringify(m.content),
			})),
			model: this.bitId,
		};
		if (this.temperature != null) body.temperature = this.temperature;
		if (this.maxTokens != null) body.max_tokens = this.maxTokens;
		if (this.topP != null) body.top_p = this.topP;
		if (this.stopSequences) body.stop = this.stopSequences;

		const res = await fetch(`${this.baseUrl}/api/v1/chat/completions`, {
			method: "POST",
			headers: buildHeaders(this.token),
			body: JSON.stringify(body),
			signal: options?.signal as AbortSignal | undefined,
		});

		if (!res.ok) {
			const text = await res.text().catch(() => "");
			throw new Error(`Flow-Like API error ${res.status}: ${text}`);
		}

		const data = (await res.json()) as Record<string, any>;
		const content = data.choices?.[0]?.message?.content ?? "";

		return {
			generations: [{ text: content, message: new AIMessage(content) }],
		};
	}
}

export class FlowLikeEmbeddings extends Embeddings {
	private baseUrl: string;
	private token: string;
	private bitId: string;

	constructor(params: FlowLikeEmbeddingsParams) {
		super(params);
		this.baseUrl = stripTrailingSlashes(params.baseUrl);
		this.token = params.token;
		this.bitId = params.bitId;
	}

	private async callApi(
		input: string[],
		embedType: "query" | "document",
	): Promise<number[][]> {
		const res = await fetch(`${this.baseUrl}/api/v1/embeddings/embed`, {
			method: "POST",
			headers: buildHeaders(this.token),
			body: JSON.stringify({
				model: this.bitId,
				input,
				embed_type: embedType,
			}),
		});

		if (!res.ok) {
			const text = await res.text().catch(() => "");
			throw new Error(`Flow-Like API error ${res.status}: ${text}`);
		}

		const data = (await res.json()) as Record<string, any>;
		return data.embeddings;
	}

	async embedDocuments(documents: string[]): Promise<number[][]> {
		return this.callApi(documents, "document");
	}

	async embedQuery(document: string): Promise<number[]> {
		const result = await this.callApi([document], "query");
		return result[0];
	}
}
