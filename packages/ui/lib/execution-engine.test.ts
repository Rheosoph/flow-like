import { describe, expect, mock, test } from "bun:test";
import { createFrontendStateStore } from "../components/a2ui/frontend-state";
import type { IBackendState } from "../state/backend-state";
import {
	type ExecuteEventFn,
	ExecutionEngineProvider,
} from "./execution-engine";
import { buildChatMessageError } from "./flowpilot/chat-error";
import type { IIntercomEvent } from "./schema/events/intercom-event";
import type { ILogMetadata } from "./schema/flow/log-metadata";

describe("ExecutionEngineProvider live events", () => {
	test("rejects a stale native request before publishing or dispatching a run", async () => {
		const engine = new ExecutionEngineProvider();
		const executeEvent = mock(async () => undefined);
		engine.setBackend({
			eventState: { executeEvent },
		} as unknown as IBackendState);
		await expect(
			engine.executeEvent("expired", {
				appId: "app",
				eventId: "event",
				payload: { id: "node" },
				beforeDispatch: () => {
					throw new Error("Native request expired");
				},
			}),
		).rejects.toThrow("Native request expired");
		expect(executeEvent).not.toHaveBeenCalled();
		expect(engine.getActiveExecutions()).toEqual([]);
	});
	test("forwards the live identity guard to the Event transport", async () => {
		const engine = new ExecutionEngineProvider();
		let receivedGuard: (() => void) | undefined;
		const executeEvent: ExecuteEventFn = async (...args) => {
			receivedGuard = args[8];
			return undefined;
		};
		engine.setBackend({
			eventState: { executeEvent },
		} as unknown as IBackendState);
		let current = true;
		const beforeDispatch = () => {
			if (!current) throw new Error("Account changed");
		};
		await engine.executeEvent("native", {
			appId: "app",
			eventId: "event",
			payload: { id: "node" },
			beforeDispatch,
		});
		expect(receivedGuard).toBe(beforeDispatch);
		current = false;
		expect(() => receivedGuard?.()).toThrow("Account changed");
	});
	test("publishes active Event metadata, excludes other backends, and retires finished runs", async () => {
		const engine = new ExecutionEngineProvider();
		let start!: (id: string) => void;
		let finish!: () => void;
		const executeEvent: ExecuteEventFn = (
			_app,
			_event,
			_payload,
			_stream,
			onStart,
		) => {
			if (!onStart) throw new Error("Missing execution start callback");
			start = onStart;
			return new Promise((resolve) => {
				finish = () => resolve(undefined);
			});
		};
		const backend = {
			eventState: { executeEvent },
		} as unknown as IBackendState;
		engine.setBackend(backend);
		engine.setExecutionScope("account-a");
		const running = engine.executeEvent("native-session", {
			appId: "app",
			eventId: "event",
			title: "Translation",
			payload: { id: "node", payload: { secret: "private input" } },
		});
		expect(engine.getActiveExecutions()).toEqual([
			{
				streamId: "native-session",
				scope: "account-a",
				appId: "app",
				eventId: "event",
				title: "Translation",
				startedAt: expect.any(String),
			},
		]);
		start("run-id");
		expect(engine.getActiveExecutions()[0].runId).toBe("run-id");
		engine.setExecutionScope("account-b");
		expect(engine.getActiveExecutions()).toEqual([]);
		engine.setExecutionScope("account-a");
		expect(JSON.stringify(engine.getActiveExecutions())).not.toContain(
			"private input",
		);
		engine.setBackend({
			eventState: { executeEvent },
		} as unknown as IBackendState);
		expect(engine.getActiveExecutions()).toEqual([]);
		engine.setBackend(backend);
		expect(engine.getActiveExecutions()).toHaveLength(1);
		finish();
		await running;
		expect(engine.getActiveExecutions()).toEqual([]);
	});
	test("applies state once while detached and does not replay it on subscription or completion", async () => {
		const engine = new ExecutionEngineProvider();
		const state = createFrontendStateStore(undefined);
		let emit!: (events: IIntercomEvent[]) => void;
		let finish!: () => void;
		const executeEvent: ExecuteEventFn = (
			_appId,
			_eventId,
			_payload,
			_streamState,
			_onExecutionStart,
			callback,
		) => {
			if (!callback) throw new Error("Missing event callback");
			emit = callback;
			return new Promise((resolve) => {
				finish = () => resolve(undefined);
			});
		};
		engine.setBackend({
			eventState: { executeEvent },
		} as unknown as IBackendState);
		const onLiveEvents = mock((events: IIntercomEvent[]) => {
			for (const event of events) state.handleMessage(event.payload);
		});
		const running = engine.executeEvent("chat-session", {
			appId: "app",
			eventId: "chat",
			payload: { id: "chat" },
			onLiveEvents,
		});
		const globalUpdate = {
			event_id: "global-update",
			event_type: "a2ui",
			payload: { type: "setGlobalState", key: "count", value: 1 },
		} as IIntercomEvent;
		const pageUpdate = {
			event_id: "page-update",
			event_type: "a2ui",
			payload: {
				type: "setPageState",
				page_id: "chat",
				key: "tab",
				value: "all",
			},
		} as IIntercomEvent;
		emit([globalUpdate, pageUpdate]);
		expect(state.getSnapshot().globalState.count).toBe(1);
		expect(state.getSnapshot().pageStates.chat.tab).toBe("all");

		// A widget can write newer state before the chat subscribes again.
		state.setGlobalState("count", 2);
		const onReplay = mock((_events: IIntercomEvent[]) => {});
		engine.subscribeToEventStream("chat-session", "chat-view", onReplay);
		expect(onReplay).toHaveBeenCalledWith([globalUpdate, pageUpdate]);
		expect(state.getSnapshot().globalState.count).toBe(2);
		emit([globalUpdate, pageUpdate]);
		expect(onLiveEvents).toHaveBeenCalledTimes(1);
		expect(state.getSnapshot().globalState.count).toBe(2);

		engine.unsubscribeFromEventStream("chat-session", "chat-view");
		const nextUpdate = {
			...globalUpdate,
			event_id: "next-update",
			payload: { type: "setGlobalState", key: "count", value: 3 },
		};
		emit([nextUpdate]);
		expect(state.getSnapshot().globalState.count).toBe(3);
		finish();
		await running;
		state.setGlobalState("count", 4);
		engine.subscribeToEventStream("chat-session", "finished-view", onReplay);
		expect(onLiveEvents).toHaveBeenCalledTimes(2);
		expect(state.getSnapshot().globalState.count).toBe(4);
	});
});

describe("ExecutionEngineProvider unclaimed streams", () => {
	test("keeps the newest finished streams nobody came back for, and every watched one", async () => {
		const engine = new ExecutionEngineProvider();
		engine.setBackend({
			eventState: { executeEvent: async () => undefined },
		} as unknown as IBackendState);
		engine.subscribeToEventStream("watched", "chat", () => {});
		const abandoned = Array.from({ length: 10 }, (_, index) => `left-${index}`);
		for (const streamId of ["watched", ...abandoned]) {
			await engine.executeEvent(streamId, {
				appId: "app",
				eventId: "event",
				payload: { id: "node" },
			});
		}

		expect(engine.hasStream("watched")).toBe(true);
		expect(abandoned.filter((streamId) => engine.hasStream(streamId))).toEqual(
			abandoned.slice(2),
		);
	});
});

describe("ExecutionEngineProvider stream lifecycle", () => {
	function controlledEngine() {
		const engine = new ExecutionEngineProvider();
		let emit!: (events: IIntercomEvent[]) => void;
		let finish!: (metadata?: ILogMetadata) => void;
		let fail!: (error: unknown) => void;
		const executeEvent: ExecuteEventFn = (_a, _e, _p, _s, _start, callback) => {
			if (!callback) throw new Error("Missing event callback");
			emit = callback;
			return new Promise((resolve, reject) => {
				finish = resolve;
				fail = reject;
			});
		};
		engine.setBackend({
			eventState: { executeEvent },
		} as unknown as IBackendState);
		const run = (streamId: string) =>
			engine.executeEvent(streamId, {
				appId: "app",
				eventId: "event",
				payload: { id: "node" },
			});
		return {
			engine,
			run,
			emit: (events: IIntercomEvent[]) => emit(events),
			finish: (metadata?: ILogMetadata) => finish(metadata),
			fail: (error: unknown) => fail(error),
		};
	}
	const token = (id: string) =>
		({
			event_id: id,
			event_type: "chat_stream_partial",
			payload: {},
		}) as IIntercomEvent;
	const terminal = (payload: unknown, event_type = "completed") =>
		({ event_id: event_type, event_type, payload }) as IIntercomEvent;
	const metadata = (log_level: number, error_message?: string) =>
		({ run_id: "run", log_level, error_message }) as ILogMetadata;

	test.each([
		[
			"native failure",
			[terminal({ status: "failed", log_level: 4 })],
			metadata(4),
		],
		["hosted failure", [terminal({ status: "failed" })], undefined],
		["legacy metadata", [], metadata(3)],
		["empty error event", [terminal({}, "error")], undefined],
	] as const)(
		"surfaces %s to active and returning subscribers",
		async (_name, events, meta) => {
			const { engine, run, emit, finish } = controlledEngine();
			const completed = mock(
				(_events: IIntercomEvent[], _failure?: unknown) => {},
			);
			const running = run("chat-session");
			engine.subscribeToEventStream(
				"chat-session",
				"chat",
				() => {},
				completed,
			);
			emit([...events]);
			finish(meta);
			// Failed workflow results keep the transport's resolved return value.
			expect(await running).toBe(meta);
			await new Promise((resolve) => setTimeout(resolve, 0));
			const failure = completed.mock.calls[0]?.[1];
			expect(failure).toBeInstanceOf(Error);
			expect((failure as Error).message).toContain("The workflow failed");
			expect(completed).toHaveBeenCalledTimes(1);
			const late = mock((_events: IIntercomEvent[], _failure?: unknown) => {});
			engine.subscribeToEventStream("chat-session", "late", () => {}, late);
			expect(late).toHaveBeenCalledWith([...events], failure);

			const next = run("chat-session");
			finish(metadata(1));
			await next;
			await new Promise((resolve) => setTimeout(resolve, 0));
			expect(completed).toHaveBeenLastCalledWith([], undefined);
		},
	);

	test.each([
		[[terminal({ message: "DependencyFailed" }, "error")], undefined],
		[[terminal("DependencyFailed", "error")], undefined],
		[[terminal({ error: "DependencyFailed" }, "error")], undefined],
		[
			[terminal({ status: "failed", error_message: "DependencyFailed" })],
			undefined,
		],
		[[], metadata(4, "DependencyFailed")],
		[
			[terminal({ message: "Execution failed" }, "error")],
			metadata(4, "DependencyFailed"),
		],
	] as const)(
		"keeps the available run error text: %j",
		async (events, meta) => {
			const { engine, run, emit, finish } = controlledEngine();
			const completed = mock(
				(_events: IIntercomEvent[], _failure?: unknown) => {},
			);
			const running = run("chat-session");
			engine.subscribeToEventStream(
				"chat-session",
				"chat",
				() => {},
				completed,
			);
			emit([...events]);
			finish(meta);
			await running;
			expect(completed.mock.calls[0]?.[1]).toMatchObject({
				message: "DependencyFailed",
			});
		},
	);

	test.each([
		[[], undefined],
		[[], metadata(1)],
		[[], metadata(2)],
		[[terminal({ status: "completed", log_level: 3 })], metadata(3)],
	] as const)(
		"does not mark successful or warning-only runs as failed: %j",
		async (events, meta) => {
			const { engine, run, emit, finish } = controlledEngine();
			const completed = mock(
				(_events: IIntercomEvent[], _failure?: unknown) => {},
			);
			const running = run("chat-session");
			engine.subscribeToEventStream(
				"chat-session",
				"chat",
				() => {},
				completed,
			);
			emit([...events]);
			finish(meta);
			await running;
			expect(completed).toHaveBeenCalledWith([...events], undefined);
		},
	);

	test("renders an explicitly cancelled run as stopped despite its fatal logs", async () => {
		const { engine, run, emit, finish } = controlledEngine();
		const completed = mock(
			(_events: IIntercomEvent[], _failure?: unknown) => {},
		);
		const running = run("chat-session");
		engine.subscribeToEventStream("chat-session", "chat", () => {}, completed);
		emit([terminal({ status: "cancelled", log_level: 4 })]);
		finish(metadata(4));
		await running;
		expect(
			buildChatMessageError("bits", completed.mock.calls[0]?.[1]).kind,
		).toBe("cancelled");
	});

	test("reports a timeout when the terminal status supplies no error text", async () => {
		const { engine, run, emit, finish } = controlledEngine();
		const completed = mock(
			(_events: IIntercomEvent[], _failure?: unknown) => {},
		);
		const running = run("chat-session");
		engine.subscribeToEventStream("chat-session", "chat", () => {}, completed);
		emit([terminal({ status: "timeout" })]);
		finish();
		await running;
		expect(completed.mock.calls[0]?.[1]).toMatchObject({
			message: "The workflow timed out before it could finish this response.",
		});
	});

	test("a subscription alone is not a running stream", () => {
		const { engine } = controlledEngine();
		engine.subscribeToEventStream("waiting", "chat", () => {});
		expect(engine.isStreamActive("waiting")).toBe(false);
		engine.unsubscribeFromEventStream("waiting", "chat");
		expect(engine.getBackgroundStreams()).toEqual([]);
	});

	test("the running card lists a detached stream only while it runs", async () => {
		const { engine, run, finish } = controlledEngine();
		const running = run("chat-session");
		expect(
			engine.getBackgroundStreams().map((stream) => stream.streamId),
		).toEqual(["chat-session"]);
		finish();
		await running;
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(engine.hasStream("chat-session")).toBe(true);
		expect(engine.getBackgroundStreams()).toEqual([]);
	});

	test("a failing subscriber neither ends the run nor starves the others", async () => {
		const { engine, run, emit, finish } = controlledEngine();
		const running = run("chat-session");
		const seen: string[] = [];
		const completed = mock(() => {});
		engine.subscribeToEventStream(
			"chat-session",
			"broken",
			() => {
				throw new Error("bad payload");
			},
			async () => {
				throw new Error("save failed");
			},
		);
		engine.subscribeToEventStream(
			"chat-session",
			"healthy",
			(events) => seen.push(...events.map((event) => event.event_id)),
			completed,
		);
		expect(() => emit([token("a"), token("b")])).not.toThrow();
		finish();
		await running;
		await Promise.resolve();
		expect(seen).toEqual(["a", "b"]);
		expect(completed).toHaveBeenCalledTimes(1);
	});

	test("completion carries why a run failed, and the next run starts without it", async () => {
		const { engine, run, finish, fail } = controlledEngine();
		const completed = mock(
			(_events: IIntercomEvent[], _failure?: unknown) => {},
		);
		const failure = { error: "Automation approval is required" };
		const failing = run("chat-session");
		engine.subscribeToEventStream("chat-session", "chat", () => {}, completed);
		fail(failure);
		await expect(failing).rejects.toBe(failure);
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(completed).toHaveBeenLastCalledWith([], failure);

		const late = mock((_events: IIntercomEvent[], _failure?: unknown) => {});
		engine.subscribeToEventStream("chat-session", "late", () => {}, late);
		expect(late).toHaveBeenLastCalledWith([], failure);

		const succeeding = run("chat-session");
		finish();
		await succeeding;
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(completed).toHaveBeenLastCalledWith([], undefined);
	});
});
