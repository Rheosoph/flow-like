// @vitest-environment happy-dom
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
	dispatchPaymentRequest: vi.fn(),
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

import { updateSessionInteractions } from "../../components/interfaces/chat-default/session-interactions";
import { attachExecutionEffects } from "./session-effects";
import type { ExecutionTask } from "./types";

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
