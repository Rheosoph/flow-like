import type { Nullable } from "../schema/auto-import";
import { type IContent, IContentType, type IRole } from "../schema/llm/history";
import type {
	IChoice,
	ICompletionTokenDetails,
	IFunctionCall,
	IPromptTokenDetails,
	IResponse,
	IResponseFunction,
	IResponseMessage,
	IUsage,
} from "../schema/llm/response";
import type { IDelta, IResponseChunk } from "../schema/llm/response-chunk";

export class FunctionCall implements IFunctionCall {
	index?: number | null = null;
	id = "";
	type?: string | null = null;
	function: IResponseFunction = {
		name: null,
		arguments: null,
	};
}

export class ResponseMessage implements IResponseMessage {
	content?: Nullable<string>;
	content_parts: IContent[] = [];
	reasoning?: Nullable<string>;
	refusal?: Nullable<string>;
	toolCalls: IFunctionCall[] = [];
	role = "";

	public static fromJson(json: string): ResponseMessage {
		const object = JSON.parse(json);
		return ResponseMessage.fromObject(object);
	}

	public toJson(): string {
		const object = this.toObject();
		return JSON.stringify(object);
	}

	public static fromObject(obj: IResponseMessage): ResponseMessage {
		const response_message = new ResponseMessage();

		for (const key of Object.keys(obj)) {
			(response_message as any)[key] = obj[key];
		}

		return response_message;
	}

	public toObject(): IResponseMessage {
		const obj: Record<string, any> = {};
		Object.keys(this).forEach((key) => {
			if (typeof (this as any)[key] !== "function") {
				obj[key] = (this as any)[key];
			}
		});
		return obj as IResponseMessage;
	}

	private applyDeltaToolCalls(delta: IDelta): this {
		if (!delta.toolCalls) return this;

		for (const functionCall of delta.toolCalls) {
			const existingToolCall = this.toolCalls.find(
				(c) => c.index === functionCall.index,
			);

			if (existingToolCall) {
				existingToolCall.id = functionCall.id;
				if (functionCall.type) {
					existingToolCall.type =
						(existingToolCall.type ?? "") + functionCall.type;
				}
				if (functionCall.function.name) {
					existingToolCall.function.name =
						(existingToolCall.function.name ?? "") + functionCall.function.name;
				}
				if (functionCall.function.arguments) {
					existingToolCall.function.arguments =
						(existingToolCall.function.arguments ?? "") +
						functionCall.function.arguments;
				}
				continue;
			}

			this.toolCalls.push({ ...functionCall });
		}

		return this;
	}

	public applyDelta(delta: IDelta): this {
		if (delta.content) {
			const structuredPartsContainText = delta.content_parts?.some(
				(part) => part.type === IContentType.Text,
			);
			if (this.content_parts.length > 0 && !structuredPartsContainText) {
				const lastPart = this.content_parts.at(-1);
				if (lastPart?.type === IContentType.Text) {
					this.content_parts = [
						...this.content_parts.slice(0, -1),
						{
							...lastPart,
							text: (lastPart.text ?? "") + delta.content,
						},
					];
				} else {
					this.content_parts = [
						...this.content_parts,
						{
							type: IContentType.Text,
							text: delta.content,
						},
					];
				}
			}
			this.content = (this.content ?? "") + delta.content;
		}
		if (delta.content_parts?.length) {
			let precedingParts = this.content_parts;
			const incomingContainsText = delta.content_parts.some(
				(part) => part.type === IContentType.Text,
			);
			if (
				precedingParts.length === 0 &&
				this.content &&
				!incomingContainsText
			) {
				precedingParts = [
					{
						type: IContentType.Text,
						text: this.content,
					},
				];
			}
			this.content_parts = [...precedingParts, ...delta.content_parts];
		}
		if (delta.reasoning) {
			this.reasoning = (this.reasoning ?? "") + delta.reasoning;
		}
		if (delta.refusal) {
			this.refusal = (this.refusal ?? "") + delta.refusal;
		}
		if (delta.role && delta.role !== this.role) {
			this.role += delta.role;
		}

		return this.applyDeltaToolCalls(delta);
	}
}

export class Usage implements IUsage {
	completion_tokens = 0;
	prompt_tokens = 0;
	total_tokens = 0;
	prompt_tokens_details?: Nullable<IPromptTokenDetails>;
	completion_tokens_details?: Nullable<ICompletionTokenDetails>;
}

export class Response implements IResponse {
	choices: IChoice[] = [];
	created?: Nullable<number>;
	id?: Nullable<string>;
	model?: Nullable<string>;
	object?: Nullable<string>;
	service_tier?: Nullable<string>;
	system_fingerprint?: Nullable<string>;
	usage: IUsage = new Usage();

	public static default(): Response {
		return new Response();
	}

	public static fromJson(json: string): Response {
		return JSON.parse(json);
	}

	public toJson(): string {
		return JSON.stringify(this);
	}

	public static fromObject(obj: IResponse): Response {
		const response = new Response();

		for (const key of Object.keys(obj)) {
			(response as any)[key] = obj[key];
		}

		return response;
	}

	private pushChunkValues(chunk: IResponseChunk): void {
		if (chunk.created) this.created = chunk.created;
		if (chunk.model) this.model = chunk.model;
		if (chunk.serviceTier) this.service_tier = chunk.serviceTier;
		if (chunk.systemFingerprint)
			this.system_fingerprint = chunk.systemFingerprint;

		if (chunk.usage) {
			this.usage.completionTokens += chunk.usage.completionTokens;
			this.usage.promptTokens += chunk.usage.promptTokens;
			this.usage.totalTokens += chunk.usage.totalTokens;
		}
	}

	public pushChunk(chunk: IResponseChunk): void {
		this.pushChunkValues(chunk);

		for (const choice of chunk.choices) {
			const existingChoice = this.choices.find((c) => c.index === choice.index);

			if (existingChoice) {
				if (choice.delta)
					existingChoice.message = ResponseMessage.fromObject(
						existingChoice.message,
					)
						.applyDelta(choice.delta)
						.toObject();
				if (choice.logprobs) existingChoice.logprobs = choice.logprobs;
				if (choice.finishReason)
					existingChoice.finishReason = choice.finishReason;
				continue;
			}

			const message = new ResponseMessage();

			if (choice.delta) message.applyDelta(choice.delta);

			this.choices.push({
				finish_reason: choice.finishReason,
				index: choice.index,
				logprobs: choice.logprobs,
				message,
			});
		}
	}

	public lastMessageOfRole(role: IRole): ResponseMessage | null {
		const message =
			this.choices
				.map((c) => c.message)
				.reverse()
				.find((m) => m.role === role) ?? null;
		if (!message) return null;
		return ResponseMessage.fromObject(message);
	}

	public lastNMessages(
		n: number,
		includeSystemPrompt?: boolean,
	): ResponseMessage[] {
		const messages = [];
		includeSystemPrompt = includeSystemPrompt ?? false;

		if (includeSystemPrompt) {
			const systemPrompt =
				this.choices.map((c) => c.message).find((m) => m.role === "system") ??
				null;
			if (systemPrompt) messages.push(ResponseMessage.fromObject(systemPrompt));
		}

		const nLast = this.choices
			.map((c) => c.message)
			.reverse()
			.slice(0, n)
			.map((m) => ResponseMessage.fromObject(m));
		messages.push(...nLast);
		return messages;
	}
}
