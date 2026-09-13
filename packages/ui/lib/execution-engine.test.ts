import { describe, expect, mock, test } from "bun:test";
import { createFrontendStateStore } from "../components/a2ui/frontend-state";
import type { IBackendState } from "../state/backend-state";
import {
	type ExecuteEventFn,
	ExecutionEngineProvider,
} from "./execution-engine";
import type { IIntercomEvent } from "./schema/events/intercom-event";

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
