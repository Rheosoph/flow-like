import type { ExecutionTask } from "../../packages/ui/lib/execution/types";
import type { IIntercomEvent } from "../../packages/ui/lib/schema/events/intercom-event";
import type { FixtureExecutionEngine } from "./execution-context";

const ago = (seconds: number) =>
	new Date(Date.now() - seconds * 1000).toISOString();
export function sampleTasks(): ExecutionTask[] {
	const common = {
		scope: "fixture",
		appId: "workspace",
		eventId: "chat",
		interfaceType: "chat",
		events: [],
		pendingInteractions: [],
	};
	return [
		{
			...common,
			taskId: "approval",
			streamId: "approval-session",
			title: "Review supplier invoices",
			path: "/chat/invoices",
			status: "waiting",
			startedAt: ago(134),
			preview:
				"Three invoices are ready. One needs your approval before I continue.",
			pendingInteractions: [
				{
					requestId: "approval-question",
					status: "pending",
					event: {
						event_id: "approval-question",
						event_type: "interaction_request",
						payload: {
							id: "approval-question",
							name: "Approve the invoice?",
							description:
								"The €2,450 invoice from Northstar Supplies is 8% above the purchase order.",
							status: "pending",
							ttl_seconds: 900,
							expires_at: Math.floor(Date.now() / 1000) + 900,
							interaction_type: {
								type: "single_choice",
								options: [
									{
										id: "approve",
										label: "Approve invoice",
										description: "Continue with the updated amount.",
									},
									{
										id: "review",
										label: "Send for review",
										description: "Keep this invoice on hold.",
									},
								],
							},
						},
					} as IIntercomEvent,
				},
			],
		},
		{
			...common,
			taskId: "research",
			streamId: "research-session",
			title: "Prepare the weekly briefing",
			path: "/chat/briefing",
			status: "running",
			startedAt: ago(48),
			preview: "Comparing this week’s results with the previous quarter…",
		},
		{
			...common,
			taskId: "summary",
			streamId: "summary-session",
			title: "Summarize customer feedback",
			path: "/chat/feedback",
			status: "completed",
			startedAt: ago(620),
			completedAt: Date.now() - 180_000,
			preview:
				"Your summary is ready, with six themes and recommended next steps.",
		},
		{
			...common,
			taskId: "sync",
			streamId: "sync-session",
			title: "Update the inventory report",
			path: "/chat/inventory",
			status: "failed",
			startedAt: ago(900),
			completedAt: Date.now() - 420_000,
			error:
				"The warehouse connection timed out. Your last report is still available.",
		},
	];
}

export class MockExecutionEngine implements FixtureExecutionEngine {
	tasks = sampleTasks();
	listeners = new Set<() => void>();
	calls: {
		action: string;
		taskId: string;
		requestId?: string;
		value?: unknown;
	}[] = [];
	failCancel = false;
	failReply = false;
	addMachineTask = () => {
		this.tasks.push({
			taskId: "page-read",
			streamId: "page-read-session",
			scope: "fixture",
			appId: "workspace",
			eventId: "report",
			title: "Read the inventory page",
			path: "/page/inventory",
			status: "waiting",
			startedAt: ago(12),
			events: [],
			pendingInteractions: [
				{
					requestId: "elements-request",
					status: "pending",
					event: {
						event_id: "elements-request",
						event_type: "a2ui",
						payload: {
							type: "requestElements",
							request_id: "elements-request",
							selectors: ["inventory"],
						},
					} as IIntercomEvent,
				},
			],
		});
		this.notify();
	};
	getTasks = () => this.tasks;
	subscribeToGlobalUpdates = (listener: () => void) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};
	notify = () => {
		this.tasks = [...this.tasks];
		for (const listener of this.listeners) listener();
	};
	reset = () => {
		this.tasks = sampleTasks();
		this.calls = [];
		this.failCancel = false;
		this.failReply = false;
		this.notify();
	};
	cancelTask = async (taskId: string) => {
		if (this.failCancel) throw new Error("Could not stop this run. Try again.");
		this.calls.push({ action: "cancel", taskId });
		this.tasks = this.tasks.map((task) =>
			task.taskId === taskId
				? {
						...task,
						status: "cancelled",
						completedAt: Date.now(),
						pendingInteractions: [],
					}
				: task,
		);
		this.notify();
	};
	dismissTask = (taskId: string) => {
		this.calls.push({ action: "dismiss", taskId });
		this.tasks = this.tasks.filter((task) => task.taskId !== taskId);
		this.notify();
	};
	replyToTask = async (taskId: string, requestId: string, value: unknown) => {
		if (this.failReply)
			throw new Error("Your answer could not be sent. Try again.");
		const task = this.tasks.find((task) => task.taskId === taskId);
		const request = task?.pendingInteractions.find(
			(item) => item.requestId === requestId,
		);
		if (!task || !request || request.status !== "pending") return;
		this.calls.push({ action: "reply", taskId, requestId, value });
		request.status = "submitted";
		task.status = "running";
		task.preview = "Approval received. Continuing with the remaining invoices.";
		this.notify();
	};
}
