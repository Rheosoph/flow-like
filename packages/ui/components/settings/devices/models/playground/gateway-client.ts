import { z } from "zod";
import type { DeviceServiceStream } from "../../../../../lib/device-management/tunnel";
import { createTunnelFetch } from "../../../../../lib/device-management/tunnel-fetch";
import { TUNNEL_HTTP_HEAD_STALLED } from "../../../../../lib/device-management/tunnel-http";
import type { BrowserFetch } from "../../../../../lib/service-runtime/transport";

/*
 * The playground's client of a device's model gateway (plan §3.4, §3.7): the
 * OpenAI-compatible routes over a tunnel stream opened with the
 * `model_gateway` target. Chat streams server-sent events; the times are
 * measured here, the token counts come from the gateway's usage chunk.
 */

/** Opens one HTTP stream to the device's model gateway. */
export type GatewayOpen = (
	signal?: AbortSignal,
) => Promise<DeviceServiceStream>;

type OpenGateway = (
	deviceId: string,
	options?: { signal?: AbortSignal },
) => Promise<DeviceServiceStream>;

/** The live session's gateway opener once it offers one; `undefined` before. */
export function modelGatewayOf(
	live: object,
	deviceId: string,
): GatewayOpen | undefined {
	const open = (live as { openModelGateway?: unknown }).openModelGateway;
	if (typeof open !== "function") return undefined;
	return (signal) =>
		(open as OpenGateway).call(live, deviceId, signal ? { signal } : {});
}

/**
 * How long the gateway may take to start an answer. It answers once the
 * request left its queue (60 s at most) and the model is loaded, which the
 * device allows a minute plus 50 MB/s of weights for, 30 minutes at most.
 */
export const GATEWAY_HEAD_TIMEOUT_MS = 32 * 60_000;

/** `fetch` over gateway streams; `hold` runs while each stream is open (the keys' idle lock waits). */
export function gatewayFetch(
	open: GatewayOpen,
	hold: () => () => void,
	signal?: AbortSignal,
): BrowserFetch {
	return createTunnelFetch({
		open: async (inner) => {
			const stream = await open(inner);
			const release = hold();
			void stream.closed.then(release, release);
			return stream;
		},
		signal,
		headTimeoutMs: GATEWAY_HEAD_TIMEOUT_MS,
	});
}

/** The gateway never started its answer: the device may still be loading the model. */
export const gatewayStalled = (error: unknown) =>
	error instanceof Error && error.message === TUNNEL_HTTP_HEAD_STALLED;

/** A refusal of the gateway or its engine, with the status and the device's own sentence. */
export class GatewayError extends Error {
	constructor(
		readonly status: number,
		readonly reason: string | undefined,
	) {
		super(
			`The model gateway answered ${status}${reason ? `: ${reason}` : ""}.`,
		);
		this.name = "GatewayError";
	}
}

/** The stream ended before the model finished: neither `[DONE]` nor a finish reason arrived. */
export class AnswerBrokeOffError extends Error {
	constructor() {
		super("The answer broke off before the model finished.");
		this.name = "AnswerBrokeOffError";
	}
}

const REASON_MAX = 1024;

const errorBody = z.object({
	error: z.object({ message: z.string().max(REASON_MAX) }).passthrough(),
});

async function refusal(response: Response) {
	const text = await response.text().catch(() => "");
	let reason: string | undefined;
	try {
		reason = errorBody.parse(JSON.parse(text)).error.message;
	} catch {
		reason = undefined;
	}
	return new GatewayError(response.status, reason);
}

async function post(
	fetcher: BrowserFetch,
	path: string,
	body: unknown,
	signal?: AbortSignal,
) {
	const response = await fetcher(path, {
		method: "POST",
		headers: { "content-type": "application/json" },
		body: JSON.stringify(body),
		signal,
	});
	if (!response.ok) throw await refusal(response);
	return response;
}

/* Server-sent events. */

const LINE_END = /\r\n|\r|\n/;

/** The `data` of each complete event (SSE): comments and other fields are skipped, lines may end in CR, LF or both. */
export class SseParser {
	private buffer = "";
	private data: string[] = [];

	push(text: string, final = false): string[] {
		this.buffer += text;
		const events: string[] = [];
		for (;;) {
			const match = LINE_END.exec(this.buffer);
			if (!match) break;
			const lone = match[0] === "\r" && match.index === this.buffer.length - 1;
			if (lone && !final) break;
			this.line(this.buffer.slice(0, match.index), events);
			this.buffer = this.buffer.slice(match.index + match[0].length);
		}
		if (final) {
			if (this.buffer) this.line(this.buffer, events);
			this.buffer = "";
			this.line("", events);
		}
		return events;
	}

	private line(line: string, events: string[]) {
		if (line === "") {
			if (this.data.length) events.push(this.data.join("\n"));
			this.data = [];
			return;
		}
		const colon = line.indexOf(":");
		if (colon === 0) return;
		const field = colon < 0 ? line : line.slice(0, colon);
		if (field !== "data") return;
		const value = colon < 0 ? "" : line.slice(colon + 1);
		this.data.push(value.startsWith(" ") ? value.slice(1) : value);
	}
}

async function* sseData(body: ReadableStream<Uint8Array>) {
	const reader = body.getReader();
	const decoder = new TextDecoder();
	const parser = new SseParser();
	try {
		for (;;) {
			const { done, value } = await reader.read();
			const text = done
				? decoder.decode()
				: decoder.decode(value, { stream: true });
			yield* parser.push(text, done);
			if (done) return;
		}
	} finally {
		reader.releaseLock();
	}
}

/* Chat. */

export interface ChatTurn {
	role: "user" | "assistant";
	content: string;
}

export interface ChatDelta {
	content?: string;
	/** A thinking model's reasoning, streamed before its answer. */
	reasoning?: string;
}

export interface ChatMeasure {
	/** Milliseconds from sending to the first generated token. */
	ttftMs?: number;
	/** Generated tokens per second after the first: the engine's figure when it reports one. */
	tokensPerSecond?: number;
	promptTokens?: number;
	completionTokens?: number;
	totalMs: number;
}

const chunkSchema = z
	.object({
		choices: z
			.array(
				z
					.object({
						delta: z
							.object({
								content: z.string().nullish(),
								reasoning_content: z.string().nullish(),
							})
							.passthrough()
							.nullish(),
						finish_reason: z.string().nullish(),
					})
					.passthrough(),
			)
			.nullish(),
		usage: z
			.object({
				prompt_tokens: z.number().nonnegative().optional(),
				completion_tokens: z.number().nonnegative().optional(),
			})
			.passthrough()
			.nullish(),
		timings: z
			.object({ predicted_per_second: z.number().positive().optional() })
			.passthrough()
			.nullish(),
		error: z.unknown().optional(),
	})
	.passthrough();

type Chunk = z.infer<typeof chunkSchema>;

const streamedErrorSchema = z.union([
	z.string(),
	z
		.object({ message: z.string().optional(), code: z.unknown().optional() })
		.passthrough(),
]);

const httpStatus = z.number().int().min(400).max(599);

/** An error event after the answer began (llama.cpp, the MLX shim): its HTTP status when it names one, else 502. */
function streamedFailure(error: unknown) {
	const detail = streamedErrorSchema.safeParse(error).data;
	const message = typeof detail === "string" ? detail : detail?.message;
	const code = typeof detail === "object" ? detail.code : undefined;
	return new GatewayError(
		httpStatus.safeParse(code).data ?? 502,
		message?.slice(0, REASON_MAX) || undefined,
	);
}

function parseChunk(data: string): Chunk {
	try {
		return chunkSchema.parse(JSON.parse(data));
	} catch (error) {
		throw new Error(
			`The model gateway streamed an event that isn't a chat completion chunk (${error instanceof Error ? error.message : String(error)}).`,
		);
	}
}

function deltaOf(chunk: Chunk): ChatDelta | undefined {
	const delta = chunk.choices?.[0]?.delta;
	const content = delta?.content ?? undefined;
	const reasoning = delta?.reasoning_content ?? undefined;
	if (!content && !reasoning) return undefined;
	return {
		...(content ? { content } : {}),
		...(reasoning ? { reasoning } : {}),
	};
}

/** What one streamed answer measured: first token, the last token, the counts. */
class ChatTimer {
	private first?: number;
	private last?: number;
	private chunks = 0;
	private usage: { prompt?: number; completion?: number } = {};
	private engineSpeed?: number;

	constructor(
		private readonly now: () => number,
		private readonly started = now(),
	) {}

	token() {
		const at = this.now();
		this.first ??= at;
		this.last = at;
		this.chunks++;
	}

	absorb(chunk: Chunk) {
		if (chunk.usage?.prompt_tokens !== undefined)
			this.usage.prompt = chunk.usage.prompt_tokens;
		if (chunk.usage?.completion_tokens !== undefined)
			this.usage.completion = chunk.usage.completion_tokens;
		if (chunk.timings?.predicted_per_second !== undefined)
			this.engineSpeed = chunk.timings.predicted_per_second;
	}

	measure(): ChatMeasure {
		const generated = this.usage.completion ?? this.chunks;
		const decodeMs =
			this.first === undefined || this.last === undefined
				? 0
				: this.last - this.first;
		const speed =
			this.engineSpeed ??
			(generated > 1 && decodeMs > 0
				? ((generated - 1) * 1000) / decodeMs
				: undefined);
		return {
			...(this.first === undefined
				? {}
				: { ttftMs: this.first - this.started }),
			...(speed === undefined ? {} : { tokensPerSecond: speed }),
			...(this.usage.prompt === undefined
				? {}
				: { promptTokens: this.usage.prompt }),
			...(this.chunks ? { completionTokens: generated } : {}),
			totalMs: this.now() - this.started,
		};
	}
}

export interface ChatInput {
	model: string;
	messages: readonly ChatTurn[];
	signal?: AbortSignal;
	onDelta: (delta: ChatDelta) => void;
	/** Milliseconds on a monotonic clock. */
	now?: () => number;
}

/** One streamed chat completion; resolves with its measurements once the model finished. */
export async function streamChat(
	fetcher: BrowserFetch,
	input: ChatInput,
): Promise<ChatMeasure> {
	const timer = new ChatTimer(input.now ?? (() => performance.now()));
	const response = await post(
		fetcher,
		"/v1/chat/completions",
		{
			model: input.model,
			messages: input.messages,
			stream: true,
			stream_options: { include_usage: true },
		},
		input.signal,
	);
	const stream = response.body;
	if (!stream)
		throw new Error("The model gateway answered the chat without a body.");
	try {
		await readChat(stream, timer, input.onDelta);
	} catch (error) {
		await stream.cancel().catch(() => undefined);
		throw error;
	}
	return timer.measure();
}

/**
 * Reads to the end of the stream, past `[DONE]`: the tunnel stream closes, and
 * releases the device, only there. An error event fails the answer, and so
 * does an end before `[DONE]` or a finish reason (the gateway closes the
 * stream cleanly when its engine breaks off).
 */
async function readChat(
	stream: ReadableStream<Uint8Array>,
	timer: ChatTimer,
	onDelta: (delta: ChatDelta) => void,
) {
	let finished = false;
	for await (const data of sseData(stream)) {
		if (data === "[DONE]") {
			finished = true;
			continue;
		}
		const chunk = parseChunk(data);
		if (chunk.error !== undefined && chunk.error !== null)
			throw streamedFailure(chunk.error);
		timer.absorb(chunk);
		if (chunk.choices?.some((choice) => choice.finish_reason)) finished = true;
		const delta = deltaOf(chunk);
		if (!delta) continue;
		timer.token();
		onDelta(delta);
	}
	if (!finished) throw new AnswerBrokeOffError();
}

/* Embeddings. */

export interface EmbedMeasure {
	dimensions: number;
	/** The first values of the vector. */
	preview: number[];
	promptTokens?: number;
	totalMs: number;
	/** Input tokens per second of the whole request. */
	tokensPerSecond?: number;
}

const PREVIEW = 8;

const embeddingSchema = z.object({
	data: z
		.array(z.object({ embedding: z.array(z.number()).min(1) }).passthrough())
		.min(1),
	usage: z
		.object({ prompt_tokens: z.number().nonnegative().optional() })
		.passthrough()
		.nullish(),
});

export interface EmbedInput {
	model: string;
	text: string;
	signal?: AbortSignal;
	now?: () => number;
}

/** One embedding of `text`, with its size and timing. */
export async function embedText(
	fetcher: BrowserFetch,
	input: EmbedInput,
): Promise<EmbedMeasure> {
	const now = input.now ?? (() => performance.now());
	const started = now();
	const response = await post(
		fetcher,
		"/v1/embeddings",
		{ model: input.model, input: input.text },
		input.signal,
	);
	const parsed = embeddingSchema.safeParse(await response.json());
	const totalMs = now() - started;
	if (!parsed.success)
		throw new Error(
			`The model gateway returned an invalid embedding (${parsed.error.issues[0]?.path.join(".") || "answer"}: ${parsed.error.issues[0]?.message ?? "invalid"}).`,
		);
	const vector = parsed.data.data[0]?.embedding ?? [];
	const tokens = parsed.data.usage?.prompt_tokens;
	return {
		dimensions: vector.length,
		preview: vector.slice(0, PREVIEW),
		...(tokens === undefined ? {} : { promptTokens: tokens }),
		totalMs,
		...(tokens && totalMs > 0
			? { tokensPerSecond: (tokens * 1000) / totalMs }
			: {}),
	};
}
