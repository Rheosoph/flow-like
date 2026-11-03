// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { afterEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	toast: Object.assign(vi.fn(), {
		error: vi.fn(),
		success: vi.fn(),
		warning: vi.fn(),
		dismiss: vi.fn(),
	}),
	progress: vi.fn(),
	filter: vi.fn(),
	close: vi.fn(),
	payment: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: mocks.toast }));
vi.mock("../../components/a2ui/elements-request-handler", () => ({
	handleElementsRequestMessage: () => false,
}));
vi.mock("../../components/a2ui/widget-query-handler", () => ({
	handleWidgetQueryMessage: () => false,
}));
vi.mock("../../components/a2ui/micro-widget-host", () => ({
	subscribeMicroWidgetBridges: () => () => {},
}));
vi.mock("../../components/payments/payment-events", () => ({
	dispatchPaymentRequest: mocks.payment,
}));
vi.mock("../../components/ui/progress-toast", () => ({
	showProgressToast: mocks.progress,
	getActiveProgressToasts: () => new Map(),
}));
vi.mock("../../state/chat-activity-store", () => ({
	recordChatActivity: vi.fn(),
}));
vi.mock("../device-bridge", () => ({
	createDeviceCommandSession: () => ({
		filter: mocks.filter,
		close: mocks.close,
	}),
}));

import type { A2UIServerMessage } from "../../components/a2ui/types";
import {
	type IMessage,
	chatDb,
} from "../../components/interfaces/chat-default/chat-db";
import { updateSessionInteractions } from "../../components/interfaces/chat-default/session-interactions";
import { recordChatActivity } from "../../state/chat-activity-store";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import { ExecutionRuntime } from "./runtime";
import { ExecutionRuntimeClient } from "./runtime-client";
import { attachExecutionEffects } from "./session-effects";
import { registerExecutionSurface } from "./surfaces";
import type {
	ExecutionAuthorization,
	ExecutionCommand,
	ExecutionMessage,
	ExecutionTask,
} from "./types";

const cleanups: (() => void)[] = [];
afterEach(() => {
	for (const cleanup of cleanups.splice(0)) cleanup();
	vi.restoreAllMocks();
	vi.clearAllMocks();
});

function fixture() {
	const task: ExecutionTask = {
		taskId: crypto.randomUUID(),
		streamId: crypto.randomUUID(),
		scope: "account",
		appId: "app",
		eventId: "event",
		title: "Task",
		startedAt: new Date().toISOString(),
		status: "running",
		events: [],
		pendingInteractions: [],
	};
	let update = () => {};
	const client = {
		getTasks: () => [task],
		isObserving: () => false,
		subscribe: (listener: () => void) => {
			update = listener;
			return () => {};
		},
		claimEvent: vi.fn(async () => true),
	};
	cleanups.push(attachExecutionEffects(client));
	return { task, client, update: () => update() };
}

test("a completed stream keeps feedback pending until the accepted response arrives", () => {
	const { task, update } = fixture();
	task.pendingInteractions = [
		{
			requestId: "question",
			status: "submitting",
			event: {
				event_id: "question",
				event_type: "interaction_request",
				payload: { id: "question", status: "pending" },
			} as ExecutionTask["events"][number],
		},
	];
	task.status = "completed";
	update();
	updateSessionInteractions(task.streamId, (items) => {
		expect(items[0].status).toBe("pending");
		return items;
	});
	task.pendingInteractions[0].status = "submitted";
	task.pendingInteractions[0].response_value = "accepted in another tab";
	update();
	updateSessionInteractions(task.streamId, (items) => {
		expect(items[0]).toMatchObject({
			status: "responded",
			response_value: "accepted in another tab",
		});
		return items;
	});
});

test("a notification received while hidden can be claimed when this tab becomes visible", async () => {
	const visibility = vi
		.spyOn(document, "visibilityState", "get")
		.mockReturnValue("hidden");
	const { task, client, update } = fixture();
	task.events.push({
		event_id: "toast",
		event_type: "toast",
		payload: { message: "Ready" },
	} as ExecutionTask["events"][number]);
	update();
	expect(client.claimEvent).not.toHaveBeenCalled();
	visibility.mockReturnValue("visible");
	window.dispatchEvent(new Event("focus"));
	await vi.waitFor(() => expect(mocks.toast).toHaveBeenCalledWith("Ready"));
	update();
	expect(client.claimEvent).toHaveBeenCalledTimes(1);
});

test("a terminal run closes only progress notifications created for that run", async () => {
	const { task, update } = fixture();
	task.events.push({
		event_id: "progress",
		event_type: "progress",
		payload: { id: "progress", message: "Building", done: false },
	} as ExecutionTask["events"][number]);
	update();
	await vi.waitFor(() => expect(mocks.progress).toHaveBeenCalledTimes(1));
	task.status = "failed";
	update();
	expect(mocks.progress).toHaveBeenLastCalledWith(
		expect.objectContaining({ id: "progress", done: true, success: false }),
	);
	expect(mocks.close).toHaveBeenCalledTimes(1);
});

test("a detached run ends its progress and device session without a failure, while its requests stay answerable", async () => {
	const { task, update } = fixture();
	await chatDb.sessions.put({
		id: task.streamId,
		appId: "app",
		summarization: "Chat",
		createdAt: 1,
		updatedAt: 1,
	});
	task.chat = {
		responseMessage: { id: "answer" } as IMessage,
		detachedNotice: {
			title: "Still running on the server",
			message: "Live updates stopped.",
		},
	};
	task.events.push({
		event_id: "progress",
		event_type: "progress",
		payload: { id: "progress", message: "Building", done: false },
	} as ExecutionTask["events"][number]);
	task.pendingInteractions = [
		{
			requestId: "question",
			status: "pending",
			event: {
				event_id: "question",
				event_type: "interaction_request",
				payload: { id: "question", status: "pending" },
			} as ExecutionTask["events"][number],
		},
	];
	update();
	await vi.waitFor(() => expect(mocks.progress).toHaveBeenCalledTimes(1));
	task.detached = true;
	update();
	expect(mocks.toast.dismiss).toHaveBeenCalledWith("progress");
	expect(mocks.progress).toHaveBeenCalledTimes(1);
	expect(mocks.close).toHaveBeenCalledTimes(1);
	updateSessionInteractions(task.streamId, (items) => {
		expect(items[0].status).toBe("pending");
		return items;
	});
	await vi.waitFor(() =>
		expect(recordChatActivity).toHaveBeenCalledWith(
			expect.objectContaining({
				messageId: "answer",
				preview: "Still running on the server",
			}),
		),
	);
});

test("board notifications claimed before completion still dispatch once when the claim settles", async () => {
	const { task, client, update } = fixture();
	task.eventId = "";
	for (const [id, type, payload] of [
		["toast", "toast", { message: "Board finished" }],
		["payment", "payment_request", { amount: 10 }],
		[
			"progress",
			"progress",
			{ id: "board-progress", message: "Building", done: false },
		],
	] as const)
		task.events.push({
			event_id: id,
			event_type: type,
			payload,
		} as ExecutionTask["events"][number]);
	update();
	task.status = "completed";
	update();
	await vi.waitFor(() =>
		expect(mocks.toast).toHaveBeenCalledWith("Board finished"),
	);
	expect(mocks.payment).toHaveBeenCalledOnce();
	expect(mocks.progress).toHaveBeenCalledWith(
		expect.objectContaining({
			id: "board-progress",
			done: true,
			success: true,
		}),
	);
	update();
	expect(client.claimEvent).toHaveBeenCalledTimes(3);
	expect(mocks.toast).toHaveBeenCalledOnce();
	expect(mocks.payment).toHaveBeenCalledOnce();
});

function connectInTab(runtime: ExecutionRuntime) {
	const port = {
		onmessage: null as ((event: MessageEvent<ExecutionMessage>) => void) | null,
		postMessage: (command: ExecutionCommand | ExecutionAuthorization) => {
			queueMicrotask(() => {
				void runtime.receive(worker, structuredClone(command));
			});
		},
	};
	const worker = {
		postMessage: (message: ExecutionMessage) => {
			const cloned = structuredClone(message);
			queueMicrotask(() =>
				port.onmessage?.({ data: cloned } as MessageEvent<ExecutionMessage>),
			);
		},
	};
	const client = new ExecutionRuntimeClient("account", port, "in-tab");
	cleanups.push(() => client.close());
	return client;
}

function streamed(events: IIntercomEvent[]): Response {
	const encoder = new TextEncoder();
	return new Response(
		new ReadableStream<Uint8Array>({
			start(controller) {
				for (const event of events)
					controller.enqueue(
						encoder.encode(`data: ${JSON.stringify(event)}\r\n\r\n`),
					);
				controller.close();
			},
		}),
	);
}

function pushRow(childId: string): IIntercomEvent {
	return {
		event_id: childId,
		event_type: "a2ui",
		payload: {
			type: "upsertElement",
			element_id: "page/feed-list",
			value: { type: "pushChild", childId },
		},
		timestamp: { secs_since_epoch: 1, nanos_since_epoch: 0 },
	};
}

test("a page run's output reaches its surface once in the tab that started it", async () => {
	const client = connectInTab(
		new ExecutionRuntime({
			fetch: async () =>
				streamed([
					pushRow("first"),
					pushRow("second"),
					{
						event_id: "done",
						event_type: "completed",
						payload: { status: "completed" },
						timestamp: { secs_since_epoch: 1, nanos_since_epoch: 0 },
					},
				]),
			push: async () => {},
		}),
	);
	await client.ready;
	cleanups.push(attachExecutionEffects(client));
	const pushed: string[] = [];
	const apply = (message: A2UIServerMessage) => {
		const value = (message as { value?: { childId?: string } }).value;
		if (value?.childId) pushed.push(value.childId);
	};
	cleanups.push(
		registerExecutionSurface({
			appId: "app",
			pageId: "page",
			source: () => ({ surfaceId: "page", components: {}, storedValues: {} }),
			onMessage: apply,
		}),
	);
	await client.execute(
		{
			taskId: "page-load",
			streamId: "page-load",
			scope: "account",
			appId: "app",
			eventId: "event",
			title: "Load page",
			startedAt: new Date().toISOString(),
			pageId: "page",
		},
		{ url: "https://example.test/invoke", body: {} },
		{
			onEvents: (events) => {
				for (const event of events)
					if (event.event_type === "a2ui")
						apply(event.payload as A2UIServerMessage);
			},
		},
	);
	window.dispatchEvent(new Event("focus"));
	expect(pushed).toEqual(["first", "second"]);
});

test("a page run started in another tab still catches a mounted page up once", () => {
	const { task, update } = fixture();
	task.pageId = "page";
	task.events.push(pushRow("first"));
	const onMessage = vi.fn();
	cleanups.push(
		registerExecutionSurface({
			appId: "app",
			pageId: "page",
			source: () => ({ surfaceId: "page", components: {}, storedValues: {} }),
			onMessage,
		}),
	);
	update();
	update();
	expect(onMessage).toHaveBeenCalledOnce();
});
