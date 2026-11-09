import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { EmotionWorkerRequest, EmotionWorkerResponse } from "./protocol";

const model = vi.hoisted(() => ({
	tokenizer: vi.fn(),
	createSession: vi.fn(),
}));

vi.mock("@huggingface/transformers", () => ({
	AutoTokenizer: { from_pretrained: model.tokenizer },
	env: { backends: { onnx: { wasm: {} } } },
}));
vi.mock("onnxruntime-web/webgpu", () => ({
	env: { wasm: {} },
	InferenceSession: { create: model.createSession },
}));

function port() {
	return {
		postMessage: vi.fn<(message: EmotionWorkerResponse) => void>(),
		onmessage: null as
			| ((event: MessageEvent<EmotionWorkerRequest>) => void)
			| null,
		start: vi.fn(),
	};
}

function warmup(target: ReturnType<typeof port>, id = 0) {
	target.onmessage?.(
		new MessageEvent("message", {
			data: { id, warmup: true, draft: "", messages: [] },
		}),
	);
}

beforeEach(() => {
	vi.resetModules();
	vi.clearAllMocks();
	model.tokenizer.mockResolvedValue({});
	model.createSession.mockResolvedValue({ outputNames: ["probabilities"] });
	vi.stubGlobal(
		"fetch",
		vi.fn(async () => new Response(new Uint8Array([1, 2]))),
	);
	vi.stubGlobal("caches", undefined);
});

afterEach(() => {
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

describe("emotion worker loading", () => {
	it.each(["open", "match", "read", "put"])(
		"continues without persistent storage when cache %s fails",
		async (failure) => {
			const unavailable = () =>
				Promise.reject(new Error("storage unavailable"));
			const cache = {
				match: vi.fn(async () => undefined as Response | undefined),
				put: vi.fn(async () => {}),
			};
			const open = vi.fn(async () => cache);
			if (failure === "open") open.mockImplementation(unavailable);
			if (failure === "match") cache.match.mockImplementation(unavailable);
			if (failure === "read") {
				const cached = new Response();
				vi.spyOn(cached, "arrayBuffer").mockImplementation(unavailable);
				cache.match.mockResolvedValue(cached);
			}
			if (failure === "put") cache.put.mockImplementation(unavailable);
			vi.stubGlobal("caches", { open });
			const scope = port();
			vi.stubGlobal("self", scope);
			await import("./emotion.worker");
			warmup(scope);
			await vi.waitFor(() =>
				expect(scope.postMessage).toHaveBeenCalledWith({ type: "ready" }),
			);
			expect(fetch).toHaveBeenCalledOnce();
			expect(model.createSession.mock.calls[0][0]).toEqual(
				new Uint8Array([1, 2]).buffer,
			);
		},
	);

	it("uses a cached model without downloading it again", async () => {
		vi.stubGlobal("caches", {
			open: async () => ({
				match: async () => new Response(new Uint8Array([3, 4])),
			}),
		});
		const scope = port();
		vi.stubGlobal("self", scope);
		await import("./emotion.worker");
		warmup(scope);
		await vi.waitFor(() =>
			expect(scope.postMessage).toHaveBeenCalledWith({ type: "ready" }),
		);
		expect(fetch).not.toHaveBeenCalled();
		expect(model.createSession.mock.calls[0][0]).toEqual(
			new Uint8Array([3, 4]).buffer,
		);
	});

	it("retries failed initialization for a later tab on the same shared worker", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		model.createSession.mockRejectedValueOnce(new Error("temporary failure"));
		const scope = { onconnect: null as ((event: MessageEvent) => void) | null };
		vi.stubGlobal("self", scope);
		await import("./emotion.worker");
		const first = port();
		const second = port();
		scope.onconnect?.({ ports: [first] } as unknown as MessageEvent);
		scope.onconnect?.({ ports: [second] } as unknown as MessageEvent);
		warmup(first, 1);
		await vi.waitFor(() =>
			expect(first.postMessage).toHaveBeenCalledWith({ type: "error", id: 1 }),
		);
		warmup(second, 2);
		await vi.waitFor(() =>
			expect(second.postMessage).toHaveBeenCalledWith({ type: "ready" }),
		);
		expect(model.createSession).toHaveBeenCalledTimes(2);
		expect(first.start).toHaveBeenCalledOnce();
		expect(second.start).toHaveBeenCalledOnce();
	});
});
