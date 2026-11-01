import { expect, mock, test } from "bun:test";
import type { IBackendState } from "../state/backend-state";
import {
	type ExecuteEventFn,
	ExecutionEngineProvider,
} from "./execution-engine";
import type { IIntercomEvent } from "./schema/events/intercom-event";

function setup() {
	const engine = new ExecutionEngineProvider();
	let finish!: () => void;
	let fail!: (error: Error) => void;
	let emit!: (events: IIntercomEvent[]) => void;
	const executeEvent: ExecuteEventFn = (...args) => {
		if (!args[5]) throw new Error("Event callback missing");
		emit = args[5];
		return new Promise((resolve, reject) => {
			finish = () => resolve(undefined);
			fail = reject;
		});
	};
	engine.setBackend({
		eventState: { executeEvent },
	} as unknown as IBackendState);
	return {
		engine,
		finish: () => finish(),
		fail: (error: Error) => fail(error),
		emit: (events: IIntercomEvent[]) => emit(events),
	};
}

const event: IIntercomEvent = {
	event_id: "reply",
	event_type: "chat_out",
	payload: { response: "Complete answer" },
	timestamp: { secs_since_epoch: 1, nanos_since_epoch: 0 },
};

async function settled(engine: ExecutionEngineProvider) {
	for (
		let count = 0;
		count < 100 && engine.isStreamActive("session");
		count++
	) {
		await new Promise((resolve) => setTimeout(resolve, 1));
	}
	expect(engine.isStreamActive("session")).toBe(false);
}

test("detached completion runs once after the final checkpoint", async () => {
	const run = setup();
	const order: string[] = [];
	const complete = mock(async () => {
		order.push("complete");
	});
	const pending = run.engine.executeEvent("session", {
		appId: "app",
		eventId: "event",
		payload: { id: "node" },
		onIncrementalSave: async (_events, final) => {
			if (final) order.push("save");
		},
		onUnobservedComplete: complete,
	});
	run.engine.subscribeToEventStream(
		"session",
		"chat",
		() => {},
		() => {},
	);
	run.engine.unsubscribeFromEventStream("session", "chat");
	run.emit([event]);
	run.finish();
	await pending;
	await settled(run.engine);
	expect(order).toEqual(["save", "complete"]);
	expect(complete).toHaveBeenCalledWith([event], undefined);
	expect(run.engine.hasStream("session")).toBe(false);
	const replay = mock(() => {});
	run.engine.subscribeToEventStream("session", "returning-chat", replay);
	expect(replay).not.toHaveBeenCalled();
});

test("a subscriber owns finalization when it watches the run finish", async () => {
	const run = setup();
	const fallback = mock(async () => {});
	const observer = mock(async () => {});
	const pending = run.engine.executeEvent("session", {
		appId: "app",
		eventId: "event",
		payload: { id: "node" },
		onUnobservedComplete: fallback,
	});
	run.engine.subscribeToEventStream("session", "chat", () => {}, observer);
	run.finish();
	await pending;
	await settled(run.engine);
	expect(observer).toHaveBeenCalledTimes(1);
	expect(fallback).not.toHaveBeenCalled();
});

test("a subscriber joining during the final save owns completion", async () => {
	const run = setup();
	const fallback = mock(async () => {});
	const observer = mock(async () => {});
	const pending = run.engine.executeEvent("session", {
		appId: "app",
		eventId: "event",
		payload: { id: "node" },
		onIncrementalSave: async () => {
			run.engine.subscribeToEventStream("session", "chat", () => {}, observer);
		},
		onUnobservedComplete: fallback,
	});
	run.finish();
	await pending;
	await settled(run.engine);
	expect(observer).toHaveBeenCalledTimes(1);
	expect(fallback).not.toHaveBeenCalled();
});

test("a returning chat cannot replay while the detached finalizer saves its answer", async () => {
	const run = setup();
	let release!: () => void;
	const saving = new Promise<void>((resolve) => {
		release = resolve;
	});
	const finalize = mock(async () => {
		await saving;
	});
	const pending = run.engine.executeEvent("session", {
		appId: "app",
		eventId: "event",
		payload: { id: "node" },
		onUnobservedComplete: finalize,
	});
	run.emit([event]);
	run.finish();
	await pending;
	await new Promise((resolve) => setTimeout(resolve, 0));
	expect(finalize).toHaveBeenCalledTimes(1);
	const replay = mock(() => {});
	const complete = mock(() => {});
	run.engine.subscribeToEventStream(
		"session",
		"returning-chat",
		replay,
		complete,
	);
	release();
	await settled(run.engine);
	expect(replay).not.toHaveBeenCalled();
	expect(complete).not.toHaveBeenCalled();
	expect(run.engine.hasStream("session")).toBe(false);
});

test("leaving during the final save keeps the session reserved until persistence settles", async () => {
	const run = setup();
	let release!: () => void;
	const saving = new Promise<void>((resolve) => {
		release = resolve;
	});
	const pending = run.engine.executeEvent("session", {
		appId: "app",
		eventId: "event",
		payload: { id: "node" },
		onIncrementalSave: async (_events, final) => {
			if (final) await saving;
		},
	});
	run.engine.subscribeToEventStream("session", "chat", () => {});
	run.emit([event]);
	run.finish();
	await pending;
	run.engine.unsubscribeFromEventStream("session", "chat");
	expect(run.engine.isStreamActive("session")).toBe(true);
	await expect(
		run.engine.executeEvent("session", {
			appId: "app",
			eventId: "event",
			payload: { id: "node" },
		}),
	).rejects.toMatchObject({ isActiveStreamError: true });
	release();
	await settled(run.engine);
});

test("checkpoints finish in order before the final save and completion callback", async () => {
	const run = setup();
	let release!: () => void;
	const blocked = new Promise<void>((resolve) => {
		release = resolve;
	});
	const order: string[] = [];
	const snapshots: number[] = [];
	const pending = run.engine.executeEvent("session", {
		appId: "app",
		eventId: "event",
		payload: { id: "node" },
		saveIntervalEvents: 1,
		saveMinIntervalMs: 0,
		onIncrementalSave: async (events, final) => {
			snapshots.push(events.length);
			if (!final && events.length === 1) await blocked;
			order.push(final ? "final" : `draft-${events.length}`);
		},
	});
	run.engine.subscribeToEventStream(
		"session",
		"chat",
		() => {},
		() => {
			order.push("complete");
		},
	);
	run.emit([event]);
	run.emit([{ ...event, event_id: "second" }]);
	run.finish();
	await pending;
	expect(order).toEqual([]);
	release();
	await settled(run.engine);
	expect(snapshots).toEqual([1, 2, 2]);
	expect(order).toEqual(["draft-1", "draft-2", "final", "complete"]);
});

test("a synchronous dispatch error settles and leaves a retryable session", async () => {
	const engine = new ExecutionEngineProvider();
	engine.setBackend({
		eventState: {
			executeEvent: () => {
				throw new Error("Unavailable");
			},
		},
	} as unknown as IBackendState);
	await expect(
		engine.executeEvent("session", {
			appId: "app",
			eventId: "event",
			payload: { id: "node" },
		}),
	).rejects.toThrow("Unavailable");
	await settled(engine);
	expect(engine.getTasks()[0]).toMatchObject({
		status: "failed",
		error: "Unavailable",
	});
});

test("a throwing live consumer cannot prevent the mounted chat from receiving an event", async () => {
	const run = setup();
	const received = mock(() => {});
	const pending = run.engine.executeEvent("session", {
		appId: "app",
		eventId: "event",
		payload: { id: "node" },
		onLiveEvents: () => {
			throw new Error("Consumer failed");
		},
	});
	run.engine.subscribeToEventStream("session", "chat", received);
	expect(() => run.emit([event])).not.toThrow();
	run.finish();
	await pending;
	await settled(run.engine);
	expect(received).toHaveBeenCalledWith([event]);
});

test("a failed native stop keeps its task retryable", async () => {
	const engine = new ExecutionEngineProvider();
	let finish!: () => void;
	engine.setBackend({
		eventState: {
			executeEvent: (
				_app: string,
				_event: string,
				_payload: unknown,
				_stream: boolean,
				start: (id: string) => void,
			) => {
				start("native-run");
				return new Promise<void>((resolve) => {
					finish = resolve;
				});
			},
			cancelExecution: async () => {
				throw new Error("Native backend unavailable");
			},
		},
	} as unknown as IBackendState);
	const pending = engine.executeEvent("session", {
		appId: "app",
		eventId: "event",
		payload: { id: "node" },
	});
	const task = engine.getTasks()[0];
	await expect(engine.cancelTask(task.taskId)).rejects.toThrow(
		"Native backend unavailable",
	);
	expect(engine.getTasks()[0]).toMatchObject({
		status: "running",
		cancellationRequested: false,
		error: "Native backend unavailable",
	});
	finish();
	await pending;
	await settled(engine);
});
