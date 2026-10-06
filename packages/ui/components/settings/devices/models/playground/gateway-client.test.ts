import { describe, expect, test } from "bun:test";
import type { DeviceServiceStream } from "../../../../../lib/device-management/tunnel";
import type { BrowserFetch } from "../../../../../lib/service-runtime/transport";
import {
	AnswerBrokeOffError,
	type ChatDelta,
	GATEWAY_HEAD_TIMEOUT_MS,
	GatewayError,
	SseParser,
	embedText,
	gatewayFetch,
	gatewayStalled,
	invokeSystemOne,
	modelGatewayOf,
	streamChat,
} from "./gateway-client";

const encoder = new TextEncoder();

/** A response body that yields `chunks` one read at a time; `state` tells how it ended. */
function body(
	chunks: readonly string[],
	state: { read: number; cancelled: boolean } = { read: 0, cancelled: false },
) {
	return new ReadableStream<Uint8Array>({
		pull(controller) {
			const chunk = chunks[state.read++];
			if (chunk === undefined) controller.close();
			else controller.enqueue(encoder.encode(chunk));
		},
		cancel() {
			state.cancelled = true;
		},
	});
}

interface Seen {
	path: string;
	init: RequestInit | undefined;
}

function fetcherOf(response: () => Response) {
	const seen: Seen[] = [];
	const fetcher: BrowserFetch = async (input, init) => {
		seen.push({ path: String(input), init });
		return response();
	};
	return { fetcher, seen };
}

const event = (value: unknown) => `data: ${JSON.stringify(value)}\n\n`;

/** A clock that returns the given milliseconds in order, then keeps the last. */
function clock(...values: number[]) {
	let index = 0;
	return () => values[Math.min(index++, values.length - 1)] as number;
}

function ignore() {
	return undefined;
}

describe("server-sent events", () => {
	test("one data payload per event, wherever the chunks split", () => {
		const parser = new SseParser();
		expect(parser.push("da")).toEqual([]);
		expect(parser.push("ta: hel")).toEqual([]);
		expect(parser.push("lo\n")).toEqual([]);
		expect(parser.push("\ndata: next\n\n")).toEqual(["hello", "next"]);
	});

	test("CR, LF and CRLF all end lines; a CRLF split across chunks is one line end", () => {
		const parser = new SseParser();
		expect(parser.push("data: a\r\n\r\n")).toEqual(["a"]);
		expect(parser.push("data: b\r")).toEqual([]);
		expect(parser.push("\n\r\n")).toEqual(["b"]);
		expect(parser.push("data: c\r\rdata: d\n\n")).toEqual(["c", "d"]);
	});

	test("data lines join, comments and other fields are skipped, the end flushes", () => {
		const parser = new SseParser();
		expect(
			parser.push(": keepalive\nevent: chunk\nid: 7\ndata: a\ndata:b\n\n"),
		).toEqual(["a\nb"]);
		expect(parser.push("data: tail", true)).toEqual(["tail"]);
		expect(parser.push("", true)).toEqual([]);
	});
});

describe("chat", () => {
	const chunks = [
		event({ choices: [{ delta: { role: "assistant" } }] }),
		event({ choices: [{ delta: { reasoning_content: "Hm." } }] }),
		event({ choices: [{ delta: { content: "Hel" } }] }),
		event({ choices: [{ delta: { content: "lo" } }] }),
		event({
			choices: [],
			usage: { prompt_tokens: 12, completion_tokens: 3 },
		}),
		"data: [DONE]\n\n",
	];

	test("streams the answer and measures the first token and the speed after it", async () => {
		const { fetcher, seen } = fetcherOf(
			() =>
				new Response(body(chunks), {
					headers: { "content-type": "text/event-stream" },
				}),
		);
		const deltas: ChatDelta[] = [];
		const measure = await streamChat(fetcher, {
			model: "qwen3-8b",
			messages: [{ role: "user", content: "Hi" }],
			onDelta: (delta) => deltas.push(delta),
			now: clock(1_000, 1_190, 1_250, 1_290, 1_400),
		});
		expect(deltas).toEqual([
			{ reasoning: "Hm." },
			{ content: "Hel" },
			{ content: "lo" },
		]);
		expect(measure).toEqual({
			ttftMs: 190,
			tokensPerSecond: 20,
			promptTokens: 12,
			completionTokens: 3,
			totalMs: 400,
		});
		expect(seen[0]?.path).toBe("/v1/chat/completions");
		expect(seen[0]?.init?.method).toBe("POST");
		expect(JSON.parse(String(seen[0]?.init?.body))).toEqual({
			model: "qwen3-8b",
			messages: [{ role: "user", content: "Hi" }],
			stream: true,
			stream_options: { include_usage: true },
		});
	});

	test("the engine's own speed wins over the measured one", async () => {
		const { fetcher } = fetcherOf(
			() =>
				new Response(
					body([
						event({ choices: [{ delta: { content: "a" } }] }),
						event({ choices: [{ delta: { content: "b" } }] }),
						event({
							choices: [],
							usage: { prompt_tokens: 1, completion_tokens: 2 },
							timings: { predicted_per_second: 86.4 },
						}),
						"data: [DONE]\n\n",
					]),
				),
		);
		const measure = await streamChat(fetcher, {
			model: "m",
			messages: [],
			onDelta: ignore,
			now: clock(0, 100, 900, 1_000),
		});
		expect(measure.tokensPerSecond).toBe(86.4);
	});

	test("a refusal carries the status and the device's own sentence", async () => {
		const { fetcher } = fetcherOf(
			() =>
				new Response(
					JSON.stringify({
						error: {
							message: "This device hosts no model called ghost",
							type: "model_not_found",
						},
					}),
					{ status: 404 },
				),
		);
		const failure = await streamChat(fetcher, {
			model: "ghost",
			messages: [],
			onDelta: ignore,
		}).catch((error: unknown) => error);
		expect(failure).toBeInstanceOf(GatewayError);
		expect(failure).toMatchObject({
			status: 404,
			reason: "This device hosts no model called ghost",
		});
		const bare = await streamChat(
			fetcherOf(() => new Response("upstream down", { status: 502 })).fetcher,
			{ model: "m", messages: [], onDelta: ignore },
		).catch((error: unknown) => error);
		expect(bare).toMatchObject({ status: 502, reason: undefined });
	});

	test("the stream is read to its end past [DONE], so the tunnel stream closes", async () => {
		const state = { read: 0, cancelled: false };
		const { fetcher } = fetcherOf(
			() => new Response(body([...chunks, ": closing\n\n"], state)),
		);
		await streamChat(fetcher, { model: "m", messages: [], onDelta: ignore });
		expect(state.read).toBe(chunks.length + 2);
		expect(state.cancelled).toBe(false);
	});

	/** Streams `chunks`; resolves with what failed the answer and what was shown of it. */
	async function failedAnswer(chunks: readonly string[]) {
		const state = { read: 0, cancelled: false };
		const { fetcher } = fetcherOf(() => new Response(body(chunks, state)));
		const deltas: ChatDelta[] = [];
		const failure = await streamChat(fetcher, {
			model: "m",
			messages: [],
			onDelta: (delta) => deltas.push(delta),
		}).then(
			() => undefined,
			(error: unknown) => error,
		);
		return { failure, deltas, state };
	}

	test("an error event after the answer began fails it with the engine's sentence, though [DONE] follows (MLX)", async () => {
		const { failure, deltas, state } = await failedAnswer([
			event({ choices: [{ delta: { content: "Hel" } }] }),
			event({ error: { message: "MLX generation failed: out of memory" } }),
			"data: [DONE]\n\n",
		]);
		expect(failure).toBeInstanceOf(GatewayError);
		expect(failure).toMatchObject({
			status: 502,
			reason: "MLX generation failed: out of memory",
		});
		expect(deltas).toEqual([{ content: "Hel" }]);
		expect(state.cancelled).toBe(true);
	});

	test("llama.cpp's error event carries its own status and ends the stream without [DONE]", async () => {
		const { failure } = await failedAnswer([
			event({ choices: [{ delta: { content: "Hel" } }] }),
			event({
				error: {
					code: 500,
					message: "Context size has been exceeded.",
					type: "server_error",
				},
			}),
		]);
		expect(failure).toMatchObject({
			status: 500,
			reason: "Context size has been exceeded.",
		});
	});

	test("a stream that ends before the model finished is no answer (the gateway closes it cleanly when the engine breaks off)", async () => {
		const { failure, deltas } = await failedAnswer([
			event({ choices: [{ delta: { content: "Hel" } }] }),
			event({ choices: [{ delta: { content: "lo" } }] }),
		]);
		expect(failure).toBeInstanceOf(AnswerBrokeOffError);
		expect(deltas).toHaveLength(2);
		const { failure: empty } = await failedAnswer([]);
		expect(empty).toBeInstanceOf(AnswerBrokeOffError);
	});

	test("a finish reason completes the answer when [DONE] never comes", async () => {
		const { failure } = await failedAnswer([
			event({ choices: [{ delta: { content: "Hi" } }] }),
			event({ choices: [{ delta: {}, finish_reason: "stop" }] }),
			event({
				choices: [],
				usage: { prompt_tokens: 3, completion_tokens: 1 },
			}),
		]);
		expect(failure).toBeUndefined();
	});

	test("an event that isn't a chat chunk fails the answer with a sentence and cancels the stream", async () => {
		const state = { read: 0, cancelled: false };
		const { fetcher } = fetcherOf(
			() =>
				new Response(body(["data: {oops\n\n", event({ choices: [] })], state)),
		);
		await expect(
			streamChat(fetcher, { model: "m", messages: [], onDelta: ignore }),
		).rejects.toThrow(/isn.t a chat completion chunk/);
		expect(state.cancelled).toBe(true);
	});
});

describe("embeddings", () => {
	test("the vector's size, its first values and the input speed", async () => {
		const vector = Array.from({ length: 768 }, (_, index) => index / 1000);
		const { fetcher, seen } = fetcherOf(() =>
			Response.json({
				object: "list",
				data: [{ object: "embedding", index: 0, embedding: vector }],
				usage: { prompt_tokens: 9 },
			}),
		);
		const measure = await embedText(fetcher, {
			model: "nomic-embed-v1.5",
			text: "invoice total",
			now: clock(0, 45),
		});
		expect(measure).toEqual({
			dimensions: 768,
			preview: vector.slice(0, 8),
			promptTokens: 9,
			totalMs: 45,
			tokensPerSecond: 200,
		});
		expect(seen[0]?.path).toBe("/v1/embeddings");
		expect(JSON.parse(String(seen[0]?.init?.body))).toEqual({
			model: "nomic-embed-v1.5",
			input: "invoice total",
		});
	});

	test("an answer without a vector is an error that names the field", async () => {
		const { fetcher } = fetcherOf(() => Response.json({ data: [] }));
		await expect(embedText(fetcher, { model: "m", text: "x" })).rejects.toThrow(
			/invalid embedding \(data:/,
		);
	});
});

describe("the gateway stream", () => {
	test("found only once the live session offers it, then opened for the device", async () => {
		expect(modelGatewayOf({}, "edge")).toBeUndefined();
		const calls: unknown[][] = [];
		const live = {
			async openModelGateway(...args: unknown[]) {
				calls.push(args);
				return {} as DeviceServiceStream;
			},
		};
		const open = modelGatewayOf(live, "edge");
		const signal = new AbortController().signal;
		await open?.(signal);
		await open?.();
		expect(calls).toEqual([
			["edge", { signal }],
			["edge", {}],
		]);
	});

	test("an answer may start after the device's queue and a model load, and a head that never comes reads as stalled", async () => {
		expect(GATEWAY_HEAD_TIMEOUT_MS).toBeGreaterThanOrEqual(
			60_000 + 30 * 60_000,
		);
		const original = globalThis.setTimeout;
		const timers: { callback: () => void; duration?: number }[] = [];
		globalThis.setTimeout = ((callback: () => void, duration?: number) => {
			timers.push({ callback, duration });
			return 1;
		}) as typeof setTimeout;
		try {
			const silent = {
				finished: false,
				closed: new Promise<void>(() => {}),
				read: () => new Promise<Uint8Array | null>(() => {}),
				write: async () => {},
				reset() {},
			} as unknown as DeviceServiceStream;
			const fetcher = gatewayFetch(
				async () => silent,
				() => ignore,
			);
			const failure = fetcher("/v1/chat/completions", {
				method: "POST",
				body: "{}",
			}).catch((error: unknown) => error);
			for (let turn = 0; turn < 40; turn++) await Promise.resolve();
			expect(timers.at(-1)?.duration).toBe(GATEWAY_HEAD_TIMEOUT_MS);
			timers.at(-1)?.callback();
			expect(gatewayStalled(await failure)).toBe(true);
		} finally {
			globalThis.setTimeout = original;
		}
	});

	test("each request holds the device while its stream is open", async () => {
		const answer = '{"ok":true}';
		let close!: () => void;
		const closed = new Promise<void>((resolve) => {
			close = resolve;
		});
		const chunks = [
			`HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: ${answer.length}\r\n\r\n`,
			answer,
		];
		const stream = {
			finished: false,
			closed,
			async read() {
				const chunk = chunks.shift();
				if (chunk === undefined) {
					close();
					return null;
				}
				return encoder.encode(chunk);
			},
			async write() {},
			reset() {
				close();
			},
		} as unknown as DeviceServiceStream;
		const holds: string[] = [];
		const fetcher = gatewayFetch(
			async () => stream,
			() => {
				holds.push("hold");
				return () => holds.push("release");
			},
		);
		const response = await fetcher("/v1/models");
		expect(holds).toEqual(["hold"]);
		expect(await response.json()).toEqual({ ok: true });
		await closed;
		await Promise.resolve();
		expect(holds).toEqual(["hold", "release"]);
	});
});

test("SystemOne uses native nonstreaming requests and rejects chat fields", async () => {
	const answer = {
		model: "decisions",
		answers: {
			sentiment: {
				type: "choice",
				choice: "negative",
				confidence: 0.99,
				probabilities: { negative: 0.99, positive: 0.01 },
			},
		},
	};
	const { fetcher, seen } = fetcherOf(() => Response.json(answer));
	const request = {
		state: "Late delivery",
		questions: {
			sentiment: {
				type: "choice",
				instructions: "Sentiment?",
				criteria: { negative: null, positive: null },
			},
		},
	};
	expect(
		await invokeSystemOne(fetcher, { model: "decisions", request }),
	).toEqual(answer);
	expect(seen[0]?.path).toBe("/v1/systemone");
	expect(JSON.parse(String(seen[0]?.init?.body))).toEqual({
		model: "decisions",
		...request,
	});
	await expect(
		invokeSystemOne(fetcher, {
			model: "decisions",
			request: { ...request, stream: true },
		}),
	).rejects.toThrow("SystemOne request");
	expect(seen).toHaveLength(1);
});
