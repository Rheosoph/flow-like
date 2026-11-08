import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
	BrowserEmotionClient,
	type EmotionWorkerPort,
	createEmotionWorkerPort,
	portFromSharedWorker,
} from "./client";
import type { EmotionWorkerRequest, EmotionWorkerResponse } from "./protocol";
import { expressionScores } from "./scoring";

class WorkerPort implements EmotionWorkerPort {
	constructor(private readyImmediately = true) {}
	warmups = 0;
	requests: EmotionWorkerRequest[] = [];
	onmessage: EmotionWorkerPort["onmessage"] = null;
	onerror: EmotionWorkerPort["onerror"] = null;
	terminate = vi.fn();
	postMessage(request: EmotionWorkerRequest) {
		if (request.warmup) {
			this.warmups++;
			if (this.readyImmediately) this.reply({ type: "ready" });
			return;
		}
		this.requests.push(request);
	}
	reply(message: EmotionWorkerResponse) {
		this.onmessage?.({ data: message } as MessageEvent<EmotionWorkerResponse>);
	}
	finish(index: number) {
		this.reply({
			type: "result",
			id: this.requests[index].id,
			scores: expressionScores({ joy: 0.9 }),
		});
	}
}

beforeEach(() => {
	vi.useFakeTimers();
	vi.setSystemTime(10_000);
});
afterEach(() => {
	vi.clearAllTimers();
	vi.useRealTimers();
});

describe("browser emotion queue", () => {
	it("releases the resident model after the last composer closes, with a handoff grace period", () => {
		const ports: WorkerPort[] = [];
		const client = new BrowserEmotionClient(() => {
			const port = new WorkerPort();
			ports.push(port);
			return port;
		});
		const unsubscribe = client.subscribe(() => {});
		client.warmup();
		unsubscribe();
		vi.advanceTimersByTime(59_000);
		expect(ports[0].terminate).not.toHaveBeenCalled();
		const unsubscribeNext = client.subscribe(() => {});
		vi.advanceTimersByTime(60_000);
		expect(ports[0].terminate).not.toHaveBeenCalled();
		unsubscribeNext();
		vi.advanceTimersByTime(60_000);
		expect(ports[0].terminate).toHaveBeenCalledOnce();
		const states: string[] = [];
		client.subscribe((state) => states.push(state.status));
		client.warmup();
		expect(ports).toHaveLength(2);
		expect(states).toEqual(["idle", "loading", "ready"]);
	});

	it("prewarms once without draft text and classifies only the latest draft after loading", async () => {
		const port = new WorkerPort(false);
		const factory = vi.fn(() => port);
		const client = new BrowserEmotionClient(factory);
		const owner = {};
		client.warmup();
		client.warmup();
		expect(port.warmups).toBe(1);
		expect(port.requests).toHaveLength(0);
		const first = client.predict(owner, { draft: "old", messages: [] });
		const latest = client.predict(owner, { draft: "latest", messages: [] });
		expect(await first).toBeNull();
		expect(port.requests).toHaveLength(0);
		port.reply({ type: "ready" });
		expect(port.requests.map((request) => request.draft)).toEqual(["latest"]);
		port.finish(0);
		await latest;
		client.cancel(owner);
		client.warmup();
		expect(factory).toHaveBeenCalledOnce();
	});

	it("reports a failed background model load without requiring a draft", () => {
		const port = new WorkerPort(false);
		const client = new BrowserEmotionClient(() => port);
		const states: string[] = [];
		client.subscribe((state) => states.push(state.status));
		client.warmup();
		port.reply({ type: "error", id: 0 });
		expect(states).toEqual(["idle", "loading", "unavailable"]);
		expect(port.terminate).toHaveBeenCalledOnce();
	});
	it("loads lazily and keeps only the newest request per composer behind active inference", async () => {
		const port = new WorkerPort();
		const factory = vi.fn(() => port);
		const client = new BrowserEmotionClient(factory);
		const owner = {};
		expect(factory).not.toHaveBeenCalled();
		const first = client.predict(owner, { draft: "first", messages: [] });
		const second = client.predict(owner, { draft: "second", messages: [] });
		const latest = client.predict(owner, { draft: "latest", messages: [] });
		expect(await first).toBeNull();
		expect(await second).toBeNull();
		expect(port.requests.map((request) => request.draft)).toEqual(["first"]);
		port.finish(0);
		expect(port.requests.map((request) => request.draft)).toEqual([
			"first",
			"latest",
		]);
		port.finish(1);
		expect((await latest)?.happy).toBe(0.9);
		expect(factory).toHaveBeenCalledTimes(1);
	});

	it("cancels disposed sessions and ignores unrelated response IDs", async () => {
		const port = new WorkerPort();
		const client = new BrowserEmotionClient(() => port);
		const old = {};
		const next = {};
		const first = client.predict(old, {
			draft: "old conversation",
			messages: [],
		});
		client.cancel(old);
		const second = client.predict(next, {
			draft: "new conversation",
			messages: [],
		});
		port.reply({
			type: "result",
			id: 99,
			scores: expressionScores({ anger: 1 }),
		});
		expect(port.requests).toHaveLength(1);
		port.finish(0);
		port.finish(1);
		expect(await first).toBeNull();
		expect((await second)?.happy).toBe(0.9);
	});

	it("fails neutrally and retries on later activity after a cooldown", async () => {
		const ports: WorkerPort[] = [];
		const client = new BrowserEmotionClient(() => {
			const port = new WorkerPort();
			ports.push(port);
			return port;
		});
		const states: string[] = [];
		client.subscribe((state) => states.push(state.status));
		const owner = {};
		const request = client.predict(owner, { draft: "hello", messages: [] });
		ports[0].reply({ type: "error", id: ports[0].requests[0].id });
		expect(await request).toBeNull();
		expect(states.at(-1)).toBe("unavailable");
		expect(ports[0].terminate).toHaveBeenCalledOnce();
		expect(
			await client.predict(owner, { draft: "hello again", messages: [] }),
		).toBeNull();
		expect(ports).toHaveLength(1);
		vi.advanceTimersByTime(60_000);
		const retry = client.predict(owner, { draft: "retry", messages: [] });
		expect(ports).toHaveLength(2);
		ports[0].reply({ type: "progress", progress: 0.8 });
		expect(states.at(-1)).toBe("ready");
		ports[1].finish(0);
		expect((await retry)?.happy).toBe(0.9);
	});

	it("clears stalled inference and never starts a worker for empty input", async () => {
		const port = new WorkerPort();
		const factory = vi.fn(() => port);
		const client = new BrowserEmotionClient(factory);
		expect(await client.predict({}, { draft: " ", messages: [] })).toBeNull();
		expect(factory).not.toHaveBeenCalled();
		const warmup = client.predict({}, { draft: "warm", messages: [] });
		port.finish(0);
		await warmup;
		const stalled = client.predict({}, { draft: "stalled", messages: [] });
		vi.advanceTimersByTime(30_000);
		expect(await stalled).toBeNull();
		expect(port.terminate).toHaveBeenCalledOnce();
	});
});

describe("emotion worker connection", () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it("closes only the shared port so other tabs keep the model", () => {
		const messages: unknown[] = [];
		const close = vi.fn();
		const port = {
			start: vi.fn(),
			close,
			postMessage: (message: unknown) => messages.push(message),
			onmessage: null as ((event: MessageEvent) => void) | null,
		};
		const worker = {
			port,
			onerror: null as ((event: ErrorEvent) => void) | null,
		};
		const adapter = portFromSharedWorker(worker as unknown as SharedWorker);
		adapter.postMessage({ id: 1, warmup: true, draft: "", messages: [] });
		expect(port.start).toHaveBeenCalledOnce();
		expect(messages).toHaveLength(1);
		adapter.terminate();
		expect(close).toHaveBeenCalledOnce();
		expect(messages.at(-1)).toEqual({ type: "disconnect" });
	});

	it("replays warmup in a dedicated worker after an asynchronous shared startup failure", async () => {
		const port = {
			start: vi.fn(),
			close: vi.fn(),
			postMessage: vi.fn(),
			onmessage: null,
		};
		const shared = {
			port,
			onerror: null as ((event: ErrorEvent) => void) | null,
		};
		const dedicated = new WorkerPort();
		const factory = vi.fn(() => dedicated);
		const connection = portFromSharedWorker(
			shared as unknown as SharedWorker,
			factory,
		);
		const client = new BrowserEmotionClient(() => connection);
		const states: string[] = [];
		client.subscribe((state) => states.push(state.status));
		const result = client.predict({}, { draft: "hello", messages: [] });
		expect(port.postMessage).toHaveBeenCalledWith({
			id: 0,
			warmup: true,
			draft: "",
			messages: [],
		});
		shared.onerror?.({} as ErrorEvent);
		expect(factory).toHaveBeenCalledOnce();
		expect(port.close).toHaveBeenCalledOnce();
		expect(dedicated.warmups).toBe(1);
		expect(dedicated.requests.map((request) => request.draft)).toEqual([
			"hello",
		]);
		dedicated.finish(0);
		expect((await result)?.happy).toBe(0.9);
		expect(states).toEqual(["idle", "loading", "ready"]);
		connection.terminate();
		expect(dedicated.terminate).toHaveBeenCalledOnce();
	});

	it("reports shared runtime errors after readiness without starting another model", () => {
		const port = {
			start: vi.fn(),
			close: vi.fn(),
			postMessage: vi.fn(),
			onmessage: null as
				| ((event: MessageEvent<EmotionWorkerResponse>) => void)
				| null,
		};
		const shared = {
			port,
			onerror: null as ((event: ErrorEvent) => void) | null,
		};
		const factory = vi.fn(() => new WorkerPort());
		const connection = portFromSharedWorker(
			shared as unknown as SharedWorker,
			factory,
		);
		const onerror = vi.fn();
		connection.onerror = onerror;
		port.onmessage?.({
			data: { type: "ready" },
		} as MessageEvent<EmotionWorkerResponse>);
		shared.onerror?.({} as ErrorEvent);
		expect(onerror).toHaveBeenCalledOnce();
		expect(factory).not.toHaveBeenCalled();
	});

	it("falls back to a dedicated worker when the shared worker cannot start", () => {
		class DedicatedWorker extends WorkerPort {}
		vi.stubGlobal("window", globalThis);
		vi.stubGlobal(
			"SharedWorker",
			class {
				constructor() {
					throw new Error("module shared workers are unavailable");
				}
			},
		);
		vi.stubGlobal("Worker", DedicatedWorker);
		expect(createEmotionWorkerPort()).toBeInstanceOf(DedicatedWorker);
	});
});
