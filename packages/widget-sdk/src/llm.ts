import type { JsonSchema } from "./contract";
import { LLM_SCHEMAS } from "./llm-schemas";

/**
 * Flow-Like model types a widget can exchange with model and agent nodes. The
 * bundler turns an `@llm` type into the contract marker
 * `{"type":"object","x-flow-like-type":"llm","x-llm":"<kind>"}`. Query Widget
 * gives such a pin the exact schema of the native Rust type, so model outputs
 * connect directly, and `validateSchema` checks values against that schema.
 * These TypeScript shapes follow `flow_like_model_provider` serialization.
 */
export type LlmKind = keyof typeof LLM_SCHEMAS;

export type LlmRole = "system" | "user" | "assistant" | "function" | "tool";

export interface LlmImageUrl {
	url: string;
	detail?: string | null;
	media_type?: string | null;
	additional_params?: unknown;
}

interface LlmMediaMembers {
	media_type?: string | null;
	additional_params?: unknown;
}

/** One ordered text or media part of a message. */
export type LlmContent =
	| { type: "text"; text: string }
	| { type: "image_url"; image_url: LlmImageUrl }
	| ({ type: "audio_url"; audio_url: string } & LlmMediaMembers)
	| ({ type: "video_url"; video_url: string } & LlmMediaMembers)
	| ({ type: "document_url"; document_url: string } & LlmMediaMembers);

export interface LlmUrlCitation {
	end_index: number;
	start_index: number;
	title: string;
	url: string;
	content?: string | null;
}

export interface LlmAnnotation {
	type: string;
	url_citation?: LlmUrlCitation | null;
}

export interface LlmToolCall {
	id: string;
	type: string;
	function: { name: string; arguments: string };
}

export interface LlmHistoryMessage {
	role: LlmRole;
	content: string | LlmContent[];
	name?: string | null;
	tool_calls?: LlmToolCall[] | null;
	tool_call_id?: string | null;
	annotations?: LlmAnnotation[] | null;
}

/**
 * A Flow-Like History: the input of model and agent nodes.
 * @llm History
 */
export interface LlmHistory {
	model: string;
	messages: LlmHistoryMessage[];
	preset?: string | null;
	stream?: boolean | null;
	stream_options?: { include_usage: boolean } | null;
	max_completion_tokens?: number | null;
	top_p?: number | null;
	temperature?: number | null;
	thinking?: "off" | "low" | "mid" | "high" | null;
	seed?: number | null;
	presence_penalty?: number | null;
	frequency_penalty?: number | null;
	user?: string | null;
	stop?: string[] | null;
	response_format?: unknown;
	n?: number | null;
	/** Tool definitions; see the native History schema for their parameters. */
	tools?: unknown[] | null;
	tool_choice?: unknown;
	usage?: { include: boolean } | null;
}

export interface LlmUsage {
	completion_tokens: number;
	prompt_tokens: number;
	total_tokens: number;
	cost?: number | null;
	prompt_tokens_details?: Record<string, number | null> | null;
	completion_tokens_details?: Record<string, number | null> | null;
	upstream_inference_cost?: Record<string, number | null> | null;
}

export interface LlmTopLogProbs {
	token: string;
	logprob: number;
	bytes?: number[] | null;
}

export interface LlmTokenLogProbs extends LlmTopLogProbs {
	top_logprobs?: LlmTopLogProbs[] | null;
}

export interface LlmLogProbs {
	content?: LlmTokenLogProbs[] | null;
	refusal?: LlmTokenLogProbs[] | null;
}

export interface LlmFunctionCall {
	index?: number | null;
	id: string;
	type?: string | null;
	function: { name: string; arguments: string };
}

export interface LlmResponseMessage {
	role: string;
	content?: string | null;
	/** Ordered media-aware content; text-only responses use `content`. */
	content_parts?: LlmContent[];
	refusal?: string | null;
	annotations?: LlmAnnotation[] | null;
	audio?: {
		data: string;
		expires_at?: number | null;
		id: string;
		transcript?: string | null;
	} | null;
	reasoning?: string | null;
	tool_calls: LlmFunctionCall[];
}

/**
 * A Flow-Like Response: the final output of model and agent nodes.
 * @llm Response
 */
export interface LlmResponse {
	id?: string | null;
	choices: {
		index: number;
		finish_reason: string;
		message: LlmResponseMessage;
		logprobs?: LlmLogProbs | null;
	}[];
	created?: number | null;
	model?: string | null;
	service_tier?: string | null;
	system_fingerprint?: string | null;
	object?: string | null;
	usage: LlmUsage;
}

export interface LlmDeltaFunctionCall {
	index?: number | null;
	id?: string | null;
	type?: string | null;
	function: { name?: string | null; arguments?: string | null };
}

export interface LlmDelta {
	role?: string | null;
	content?: string | null;
	content_parts?: LlmContent[] | null;
	tool_calls?: LlmDeltaFunctionCall[] | null;
	refusal?: string | null;
	reasoning?: string | null;
}

/**
 * A Flow-Like ResponseChunk: one streamed delta from model and agent nodes.
 * @llm ResponseChunk
 */
export interface LlmResponseChunk {
	id: string;
	choices: {
		index: number;
		delta?: LlmDelta | null;
		finish_reason?: string | null;
		logprobs?: LlmLogProbs | null;
	}[];
	created?: number | null;
	model?: string | null;
	service_tier?: string | null;
	system_fingerprint?: string | null;
	usage?: LlmUsage | null;
	x_prefill_progress?: number | null;
}

export function isLlmKind(value: unknown): value is LlmKind {
	return typeof value === "string" && Object.hasOwn(LLM_SCHEMAS, value);
}

// Keys whose values are data, not schemas, and keys whose values map names to schemas.
const DATA_KEYS = new Set(["default", "examples", "enum", "const"]);
const SCHEMA_MAPS = new Set([
	"properties",
	"patternProperties",
	"$defs",
	"definitions",
]);

function inline(
	value: unknown,
	defs: Record<string, unknown>,
	stack: string[],
	schemaMap = false,
): unknown {
	// The validator reads schema objects only; `true` accepts everything.
	if (value === true) return {};
	if (Array.isArray(value))
		return value.map((item) => inline(item, defs, stack));
	if (typeof value !== "object" || value === null) return value;
	if (schemaMap) {
		return Object.fromEntries(
			Object.entries(value).map(([key, entry]) => [
				key,
				inline(entry, defs, stack),
			]),
		);
	}
	const {
		$ref,
		$defs: _defs,
		$schema: _schema,
		...rest
	} = value as Record<string, unknown>;
	const result: JsonSchema = {};
	for (const [key, entry] of Object.entries(rest)) {
		result[key] = DATA_KEYS.has(key)
			? entry
			: inline(entry, defs, stack, SCHEMA_MAPS.has(key));
	}
	if (typeof $ref !== "string") return result;
	const name = $ref.startsWith("#/$defs/")
		? $ref.slice("#/$defs/".length)
		: undefined;
	if (name === undefined || !Object.hasOwn(defs, name)) {
		throw new Error(`Unresolvable ${$ref} in the Flow-Like LLM schemas`);
	}
	// Tool parameter schemas nest themselves; below their first level they stay open.
	if (stack.includes(name)) return result;
	return {
		...(inline(defs[name], defs, [...stack, name]) as JsonSchema),
		...result,
	};
}

const standalone = new Map<LlmKind, JsonSchema>();

/** The native schema for a Flow-Like model type with its `$defs` inlined, as `validateSchema` needs. */
export function llmSchema(kind: LlmKind): JsonSchema {
	let schema = standalone.get(kind);
	if (!schema) {
		const source = LLM_SCHEMAS[kind];
		schema = inline(
			source,
			(source.$defs ?? {}) as Record<string, unknown>,
			[],
		) as JsonSchema;
		standalone.set(kind, schema);
	}
	return schema;
}

/**
 * The native schema a contract `llm` marker names, or undefined for other
 * schemas. Throws for a malformed marker so it is never widened.
 */
export function markedLlmSchema(schema: JsonSchema): JsonSchema | undefined {
	const marked = schema["x-flow-like-type"] === "llm";
	if (!marked && !("x-llm" in schema)) return undefined;
	const kind = schema["x-llm"];
	if (!marked || !isLlmKind(kind))
		throw new Error("Invalid Flow-Like LLM type marker");
	return llmSchema(kind);
}
