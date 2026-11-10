import { describe, expect, test } from "bun:test";
import {
	TURN_IDLE_CLOSE_MS,
	TURN_INACTIVE_CLOSE_MS,
	TURN_MAX_SPEECH_SAMPLES,
	TURN_MAX_STREAMS_PER_PORT,
	type TurnDispatchPort,
	type TurnModels,
	createTurnDispatcher,
} from "./call-turn-dispatch";
import type {
	TurnModelRole,
	TurnWorkerRequest,
	TurnWorkerResponse,
} from "./call-turn-protocol";

class FakePort implements TurnDispatchPort {
	received: TurnWorkerResponse[] = [];
	closed = false;
	onmessage: ((event: MessageEvent<TurnWorkerRequest>) => void) | null = null;
	postMessage(message: TurnWorkerResponse) {
		if (this.closed) throw new Error("port is closed");
		this.received.push(message);
	}
	close() {
		this.closed = true;
	}
	send(message: TurnWorkerRequest) {
		this.onmessage?.({ data: message } as MessageEvent<TurnWorkerRequest>);
	}
}

function manualTimers() {
	const pending = new Map<number, { callback: () => void; ms: number }>();
	let next = 0;
	return {
		timers: {
			set: (callback: () => void, ms: number) => {
				pending.set(++next, { callback, ms });
				return next;
			},
			clear: (handle: unknown) => {
				pending.delete(handle as number);
			},
		},
		fire(ms: number) {
			for (const [id, timer] of [...pending]) {
				if (timer.ms !== ms) continue;
				pending.delete(id);
				timer.callback();
			}
		},
	};
}

const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

function setup(failing: TurnModelRole[] = []) {
	const calls: string[] = [];
	const models: TurnModels = {
		load: async (role) => {
			calls.push(`load:${role}`);
			if (failing.includes(role)) throw new Error(`${role} is offline`);
		},
		speech: async (stream, samples) => {
			calls.push(`speech:${stream}:${samples.length}`);
			return 0.7;
		},
		turn: async (samples) => {
			calls.push(`turn:${samples.length}`);
			return 0.9;
		},
		reset: (stream) => {
			calls.push(`reset:${stream}`);
		},
		release: async () => {
			calls.push("release");
		},
	};
	const clock = manualTimers();
	const closed: string[] = [];
	const dispatcher = createTurnDispatcher(
		models,
		() => closed.push("closed"),
		clock.timers,
	);
	const connect = () => {
		const port = new FakePort();
		dispatcher.attach(port);
		return port;
	};
	return { calls, clock, closed, connect, failing };
}

describe("createTurnDispatcher", () => {
	test("a model loads once, however many tabs ask for it", async () => {
		const h = setup();
		const first = h.connect();
		const second = h.connect();
		first.send({ type: "init", role: "speech" });
		second.send({ type: "init", role: "speech" });
		first.send({ type: "init", role: "speech" });
		await settle();
		expect(h.calls).toEqual(["load:speech"]);
		expect(first.received).toEqual([
			{ type: "ready", role: "speech" },
			{ type: "ready", role: "speech" },
		]);
		expect(second.received).toEqual([{ type: "ready", role: "speech" }]);
	});

	test("answers stay on the asking port and streams of two tabs never mix", async () => {
		const h = setup();
		const first = h.connect();
		const second = h.connect();
		first.send({ type: "init", role: "speech" });
		first.send({ type: "init", role: "turn" });
		second.send({ type: "init", role: "speech" });
		first.send({
			type: "speech",
			id: 1,
			stream: "mic",
			samples: new Float32Array(4),
		});
		second.send({
			type: "speech",
			id: 1,
			stream: "mic",
			samples: new Float32Array(8),
		});
		first.send({ type: "turn", id: 2, samples: new Float32Array(16) });
		await settle();
		expect(h.calls.filter((call) => !call.startsWith("load"))).toEqual([
			"speech:1:mic:4",
			"speech:2:mic:8",
			"turn:16",
		]);
		expect(first.received.slice(2)).toEqual([
			{ type: "result", id: 1, probability: 0.7 },
			{ type: "result", id: 2, probability: 0.9 },
		]);
		expect(second.received.slice(1)).toEqual([
			{ type: "result", id: 1, probability: 0.7 },
		]);
	});

	test("a model that failed to load answers null and is not fetched again per block", async () => {
		const h = setup(["speech"]);
		const port = h.connect();
		port.send({ type: "init", role: "speech" });
		await settle();
		expect(port.received).toEqual([
			{ type: "failed", role: "speech", message: "speech is offline" },
		]);
		for (let id = 1; id <= 3; id++) {
			port.send({
				type: "speech",
				id,
				stream: "mic",
				samples: new Float32Array(4),
			});
		}
		await settle();
		expect(h.calls).toEqual(["load:speech"]);
		expect(port.received.slice(1)).toEqual([
			{ type: "result", id: 1, probability: null },
			{ type: "result", id: 2, probability: null },
			{ type: "result", id: 3, probability: null },
		]);
		h.failing.length = 0;
		port.send({ type: "init", role: "speech" });
		await settle();
		expect(port.received.at(-1)).toEqual({ type: "ready", role: "speech" });
	});

	test("a model that failed leaves the other one answering", async () => {
		const h = setup(["turn"]);
		const port = h.connect();
		port.send({ type: "init", role: "speech" });
		port.send({ type: "init", role: "turn" });
		port.send({
			type: "speech",
			id: 1,
			stream: "mic",
			samples: new Float32Array(4),
		});
		port.send({ type: "turn", id: 2, samples: new Float32Array(4) });
		await settle();
		expect(port.received).toEqual([
			{ type: "ready", role: "speech" },
			{ type: "failed", role: "turn", message: "turn is offline" },
			{ type: "result", id: 1, probability: 0.7 },
			{ type: "result", id: 2, probability: null },
		]);
	});

	test("oversized audio is refused instead of being run", async () => {
		const h = setup();
		const port = h.connect();
		port.send({ type: "init", role: "speech" });
		port.send({
			type: "speech",
			id: 1,
			stream: "mic",
			samples: new Float32Array(TURN_MAX_SPEECH_SAMPLES + 1),
		});
		await settle();
		expect(port.received.at(-1)).toEqual({
			type: "result",
			id: 1,
			probability: null,
		});
		expect(h.calls).toEqual(["load:speech"]);
	});

	test("a tab keeps a bounded number of voice-activity streams", async () => {
		const h = setup();
		const port = h.connect();
		port.send({ type: "init", role: "speech" });
		for (let stream = 0; stream <= TURN_MAX_STREAMS_PER_PORT; stream++) {
			port.send({
				type: "speech",
				id: stream + 1,
				stream: `s${stream}`,
				samples: new Float32Array(4),
			});
		}
		await settle();
		expect(h.calls.filter((call) => call.startsWith("reset"))).toEqual([
			"reset:1:s0",
		]);
	});

	test("a disconnecting tab frees its streams, and the last one frees the models", async () => {
		const h = setup();
		const first = h.connect();
		const second = h.connect();
		first.send({ type: "init", role: "speech" });
		first.send({
			type: "speech",
			id: 1,
			stream: "mic",
			samples: new Float32Array(4),
		});
		await settle();
		first.send({ type: "disconnect" });
		expect(first.closed).toBe(true);
		expect(h.calls.at(-1)).toBe("reset:1:mic");
		h.clock.fire(TURN_IDLE_CLOSE_MS);
		expect(h.closed).toEqual([]);
		second.send({ type: "disconnect" });
		const returning = h.connect();
		h.clock.fire(TURN_IDLE_CLOSE_MS);
		await settle();
		expect(h.closed).toEqual([]);
		returning.send({ type: "disconnect" });
		h.clock.fire(TURN_IDLE_CLOSE_MS);
		await settle();
		expect(h.calls.at(-1)).toBe("release");
		expect(h.closed).toEqual(["closed"]);
	});

	test("a tab that vanished without disconnecting does not keep the models forever", async () => {
		const h = setup();
		const port = h.connect();
		port.send({ type: "init", role: "speech" });
		await settle();
		h.clock.fire(TURN_INACTIVE_CLOSE_MS);
		expect(h.closed).toEqual([]);
		h.clock.fire(TURN_INACTIVE_CLOSE_MS);
		await settle();
		expect(port.received.at(-1)).toEqual({ type: "idle" });
		expect(port.closed).toBe(true);
		expect(h.calls.at(-1)).toBe("release");
		expect(h.closed).toEqual(["closed"]);

		const late = h.connect();
		expect(late.received).toEqual([{ type: "idle" }]);
		expect(late.closed).toBe(true);
	});
});
