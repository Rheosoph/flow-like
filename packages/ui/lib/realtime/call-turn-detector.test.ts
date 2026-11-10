import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
	BrowserCallTurnDetector,
	type CallModelPlan,
	TURN_ANSWER_BUDGET_MS,
	TURN_IDLE_DISPOSE_MS,
	TURN_LOAD_TIMEOUT_MS,
	TURN_MAX_LATE_ANSWERS,
	TURN_MAX_TIMEOUTS,
	TURN_REQUEST_TIMEOUT_MS,
	TURN_RETRY_AFTER_MS,
	type TurnWorkerPort,
	callModelPlan,
} from "./call-turn-detector";
import { TURN_WINDOW_SAMPLES } from "./call-turn-features";
import type {
	TurnModelRole,
	TurnWorkerRequest,
	TurnWorkerResponse,
} from "./call-turn-protocol";

class FakeWorker implements TurnWorkerPort {
	posted: TurnWorkerRequest[] = [];
	terminated = false;
	onmessage: ((event: MessageEvent<TurnWorkerResponse>) => void) | null = null;
	onerror: ((event: ErrorEvent) => void) | null = null;
	constructor(readonly role: TurnModelRole) {}
	postMessage(message: TurnWorkerRequest) {
		this.posted.push(message);
	}
	terminate() {
		this.terminated = true;
	}
	reply(message: TurnWorkerResponse) {
		this.onmessage?.({ data: message } as MessageEvent<TurnWorkerResponse>);
	}
	ready() {
		this.reply({ type: "ready", role: this.role });
	}
}

function setup(plan: CallModelPlan = { speech: true, turn: true }) {
	const workers: FakeWorker[] = [];
	const clock = { now: 1_000 };
	const detector = new BrowserCallTurnDetector(
		(role) => {
			const worker = new FakeWorker(role);
			workers.push(worker);
			return worker;
		},
		() => clock.now,
		TURN_IDLE_DISPOSE_MS,
		() => plan,
	);
	const worker = (role: TurnModelRole) =>
		[...workers].reverse().find((w) => w.role === role) as FakeWorker;
	const count = (role: TurnModelRole) =>
		workers.filter((w) => w.role === role).length;
	return { detector, workers, worker, count, clock };
}

beforeEach(() => {
	vi.useFakeTimers();
});

afterEach(() => {
	vi.useRealTimers();
});

describe("BrowserCallTurnDetector", () => {
	test("starts nothing until a listener acquires it", async () => {
		const { detector, workers } = setup();
		expect(workers).toHaveLength(0);
		expect(detector.ready).toBe(false);
		expect(await detector.speech(new Float32Array(512))).toBeNull();
		expect(await detector.turnComplete(new Float32Array(512))).toBeNull();
		detector.wake();
		expect(workers).toHaveLength(0);
	});

	test("each model gets its own worker, once, and is ready when it says so", () => {
		const { detector, workers, worker } = setup();
		detector.acquire();
		detector.acquire();
		expect(workers.map((w) => w.role).sort()).toEqual(["speech", "turn"]);
		expect(worker("speech").posted).toEqual([{ type: "init", role: "speech" }]);
		expect(worker("turn").posted).toEqual([{ type: "init", role: "turn" }]);
		expect(detector.ready).toBe(false);
		worker("speech").ready();
		expect(detector.ready).toBe(true);
		expect(detector.turnReady).toBe(false);
		worker("turn").ready();
		expect(detector.turnReady).toBe(true);
	});

	test("a device plan without the turn model starts the voice-activity worker only", async () => {
		const { detector, workers, worker } = setup({ speech: true, turn: false });
		detector.acquire();
		expect(workers.map((w) => w.role)).toEqual(["speech"]);
		worker("speech").ready();
		expect(detector.ready).toBe(true);
		expect(await detector.turnComplete(new Float32Array(4))).toBeNull();
		expect(workers).toHaveLength(1);
	});

	test("a device plan without models never creates a worker", async () => {
		const { detector, workers } = setup({ speech: false, turn: false });
		detector.acquire();
		await detector.warm();
		detector.wake();
		expect(workers).toHaveLength(0);
		expect(detector.ready).toBe(false);
	});

	test("answers requests by id, in the order they were asked", async () => {
		const { detector, worker } = setup();
		detector.acquire();
		worker("speech").ready();
		const first = detector.speech(new Float32Array(4), "a");
		const second = detector.speech(new Float32Array(4), "a");
		const posted = worker("speech").posted.filter((m) => m.type === "speech");
		expect(posted.map((m) => m.stream)).toEqual(["a", "a"]);
		const [one, two] = posted.map((m) => m.id);
		worker("speech").reply({ type: "result", id: two, probability: 0.2 });
		worker("speech").reply({ type: "result", id: one, probability: 0.9 });
		expect(await first).toBe(0.9);
		expect(await second).toBe(0.2);
	});

	test("a turn check carries at most the last 8 s", () => {
		const { detector, worker } = setup();
		detector.acquire();
		worker("turn").ready();
		const samples = new Float32Array(TURN_WINDOW_SAMPLES + 100).fill(0.5);
		samples[100] = 0.2;
		void detector.turnComplete(samples);
		const request = worker("turn").posted.at(-1);
		expect(request?.type).toBe("turn");
		if (request?.type !== "turn") return;
		expect(request.samples).toHaveLength(TURN_WINDOW_SAMPLES);
		expect(request.samples[0]).toBeCloseTo(0.2);
		expect(request.samples.buffer.byteLength).toBe(TURN_WINDOW_SAMPLES * 4);
	});

	test("a model that fails drops its worker and waits longer each time", () => {
		const { detector, worker, count, clock } = setup();
		detector.acquire();
		worker("turn").ready();
		worker("speech").reply({
			type: "failed",
			role: "speech",
			message: "offline",
		});
		expect(worker("speech").terminated).toBe(true);
		expect(detector.ready).toBe(false);
		expect(detector.turnReady).toBe(true);
		clock.now += TURN_RETRY_AFTER_MS[0] - 1;
		detector.wake();
		expect(count("speech")).toBe(1);
		clock.now += 1;
		detector.wake();
		expect(count("speech")).toBe(2);
		worker("speech").onerror?.({} as ErrorEvent);
		clock.now += TURN_RETRY_AFTER_MS[1] - 1;
		detector.wake();
		expect(count("speech")).toBe(2);
		clock.now += 1;
		detector.wake();
		expect(count("speech")).toBe(3);
		worker("speech").ready();
		worker("speech").onerror?.({} as ErrorEvent);
		clock.now += TURN_RETRY_AFTER_MS[0];
		detector.wake();
		expect(count("speech")).toBe(4);
		expect(count("turn")).toBe(1);
	});

	test("a turn model that fails leaves voice activity running", async () => {
		const { detector, worker } = setup();
		const warmed = vi.fn();
		void detector.warm().then(warmed);
		worker("speech").ready();
		await vi.advanceTimersByTimeAsync(0);
		expect(warmed).not.toHaveBeenCalled();
		worker("turn").reply({ type: "failed", role: "turn", message: "offline" });
		await vi.advanceTimersByTimeAsync(0);
		expect(warmed).toHaveBeenCalledOnce();
		expect(detector.ready).toBe(true);
		expect(detector.turnReady).toBe(false);
		expect(worker("speech").terminated).toBe(false);
		expect(worker("turn").terminated).toBe(true);
	});

	test("a worker that cannot be created counts as failed", async () => {
		const detector = new BrowserCallTurnDetector(
			() => {
				throw new Error("no workers here");
			},
			Date.now,
			TURN_IDLE_DISPOSE_MS,
			() => ({ speech: true, turn: true }),
		);
		await detector.warm();
		expect(detector.ready).toBe(false);
		expect(await detector.speech(new Float32Array(4))).toBeNull();
	});

	test("a worker that never answers is given up, and stops holding a warm-up", async () => {
		const { detector, worker, count, clock } = setup();
		const warmed = vi.fn();
		void detector.warm().then(warmed);
		worker("speech").ready();
		await vi.advanceTimersByTimeAsync(TURN_LOAD_TIMEOUT_MS - 1);
		expect(warmed).not.toHaveBeenCalled();
		expect(worker("turn").terminated).toBe(false);
		await vi.advanceTimersByTimeAsync(1);
		expect(warmed).toHaveBeenCalledOnce();
		expect(worker("turn").terminated).toBe(true);
		expect(detector.ready).toBe(true);
		const release = detector.acquire();
		expect(count("turn")).toBe(1);
		clock.now += TURN_RETRY_AFTER_MS[0];
		detector.wake();
		expect(count("turn")).toBe(2);
		release();
	});

	test("unanswered requests time out, and repeated timeouts drop the worker", async () => {
		const { detector, worker } = setup();
		detector.acquire();
		worker("speech").ready();
		for (let attempt = 0; attempt < TURN_MAX_TIMEOUTS; attempt++) {
			const pending = detector.speech(new Float32Array(4));
			await vi.advanceTimersByTimeAsync(TURN_REQUEST_TIMEOUT_MS);
			expect(await pending).toBeNull();
		}
		expect(detector.ready).toBe(false);
		expect(worker("speech").terminated).toBe(true);
	});

	test("the workers outlive the last listener by a grace period, then are given up", () => {
		const { detector, workers, worker } = setup();
		const release = detector.acquire();
		worker("speech").ready();
		release();
		release();
		vi.advanceTimersByTime(TURN_IDLE_DISPOSE_MS - 1);
		expect(detector.ready).toBe(true);
		const again = detector.acquire();
		vi.advanceTimersByTime(TURN_IDLE_DISPOSE_MS * 2);
		expect(detector.ready).toBe(true);
		again();
		vi.advanceTimersByTime(TURN_IDLE_DISPOSE_MS);
		expect(detector.ready).toBe(false);
		expect(workers.every((w) => w.terminated)).toBe(true);
	});

	test("a worker that freed its model is asked for again only when a call needs it", async () => {
		const { detector, worker, count } = setup();
		detector.acquire();
		worker("speech").ready();
		worker("turn").ready();
		const pending = detector.speech(new Float32Array(4));
		worker("speech").reply({ type: "idle" });
		expect(await pending).toBeNull();
		expect(detector.ready).toBe(false);
		expect(count("speech")).toBe(1);
		detector.wake();
		detector.wake();
		expect(count("speech")).toBe(2);
		worker("speech").ready();
		expect(detector.ready).toBe(true);

		worker("turn").reply({ type: "idle" });
		expect(detector.turnReady).toBe(false);
		expect(count("turn")).toBe(1);
		expect(await detector.turnComplete(new Float32Array(4))).toBeNull();
		expect(count("turn")).toBe(1);
		detector.wake();
		expect(count("turn")).toBe(2);
	});

	test("turn answers that keep arriving too late free the turn model for the rest of the page", async () => {
		const { detector, worker, count, clock } = setup();
		const release = detector.acquire();
		worker("speech").ready();
		worker("turn").ready();
		const answer = async (afterMs: number) => {
			const asked = detector.turnComplete(new Float32Array(4));
			const request = worker("turn").posted.at(-1);
			clock.now += afterMs;
			if (request?.type === "turn") {
				worker("turn").reply({
					type: "result",
					id: request.id,
					probability: 0.9,
				});
			}
			return asked;
		};
		for (let late = 1; late < TURN_MAX_LATE_ANSWERS; late++) {
			expect(await answer(TURN_ANSWER_BUDGET_MS + 1)).toBe(0.9);
		}
		expect(await answer(TURN_ANSWER_BUDGET_MS)).toBe(0.9);
		expect(detector.turnReady).toBe(true);
		expect(detector.lastTurnMs).toBe(TURN_ANSWER_BUDGET_MS);
		const info = vi.spyOn(console, "info").mockImplementation(() => {});
		for (let late = 0; late < TURN_MAX_LATE_ANSWERS; late++) {
			expect(await answer(TURN_ANSWER_BUDGET_MS + 1)).toBe(0.9);
		}
		expect(info).toHaveBeenCalledOnce();
		info.mockRestore();
		expect(detector.turnReady).toBe(false);
		expect(worker("turn").terminated).toBe(true);
		expect(detector.ready).toBe(true);
		expect(await detector.turnComplete(new Float32Array(4))).toBeNull();
		detector.wake();

		release();
		vi.advanceTimersByTime(TURN_IDLE_DISPOSE_MS);
		detector.acquire();
		expect(count("turn")).toBe(1);
		expect(count("speech")).toBe(2);
	});
});

describe("callModelPlan", () => {
	const desktop = {
		hardwareConcurrency: 8,
		maxTouchPoints: 0,
		userAgent:
			"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari/605.1.15",
	};

	test("a desktop with enough memory, or one that does not report it, loads both models", () => {
		expect(callModelPlan(desktop)).toEqual({ speech: true, turn: true });
		expect(callModelPlan({ ...desktop, deviceMemory: 8 })).toEqual({
			speech: true,
			turn: true,
		});
	});

	test("less memory or fewer cores keeps voice activity only", () => {
		expect(callModelPlan({ ...desktop, deviceMemory: 4 })).toEqual({
			speech: true,
			turn: false,
		});
		expect(callModelPlan({ ...desktop, hardwareConcurrency: 2 })).toEqual({
			speech: true,
			turn: false,
		});
	});

	test("very little memory loads no model", () => {
		expect(callModelPlan({ ...desktop, deviceMemory: 2 })).toEqual({
			speech: false,
			turn: false,
		});
	});

	test("phones and tablets keep voice activity only", () => {
		for (const device of [
			{ hardwareConcurrency: 8, userAgentData: { mobile: true } },
			{
				hardwareConcurrency: 6,
				userAgent:
					"Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) Mobile/15E148",
			},
			{
				hardwareConcurrency: 8,
				deviceMemory: 8,
				userAgent: "Mozilla/5.0 (Linux; Android 16; Pixel 10) Chrome/140",
			},
			{ ...desktop, maxTouchPoints: 5 },
		]) {
			expect(callModelPlan(device)).toEqual({ speech: true, turn: false });
		}
	});
});
