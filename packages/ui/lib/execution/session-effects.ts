import { toast } from "sonner";
import { handleElementsRequestMessage } from "../../components/a2ui/elements-request-handler";
import { subscribeMicroWidgetBridges } from "../../components/a2ui/micro-widget-host";
import { handleWidgetQueryMessage } from "../../components/a2ui/widget-query-handler";
import { chatDb } from "../../components/interfaces/chat-default/chat-db";
import { addSessionInteractions } from "../../components/interfaces/chat-default/session-interactions";
import { dispatchPaymentRequest } from "../../components/payments/payment-events";
import {
	type ProgressToastData,
	getActiveProgressToasts,
	showProgressToast,
} from "../../components/ui/progress-toast";
import { recordChatActivity } from "../../state/chat-activity-store";
import { createDeviceCommandSession } from "../device-bridge";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import type { IInteractionRequest } from "../schema/interaction";
import {
	deliverExecutionSurfaceEvents,
	executionElementSource,
	subscribeExecutionSurfaces,
} from "./surfaces";
import { type ExecutionTask, isExecutionFinished } from "./types";

interface EffectClient {
	getTasks(): ExecutionTask[];
	subscribe(listener: () => void): () => void;
	claimEvent(taskId: string, eventId: string): Promise<boolean>;
}

/** Tab-only effects are claimed separately from the worker's ordered event processing. */
export function attachExecutionEffects(client: EffectClient): () => void {
	const sessions = new Map<
		string,
		ReturnType<typeof createDeviceCommandSession>
	>();
	const observed = new Map<string, Set<string>>();
	const finished = new Set<string>();
	const progress = new Map<string, Map<string, ProgressToastData>>();
	let disposed = false;
	const visible = () =>
		typeof document !== "undefined" && document.visibilityState !== "hidden";

	const notify = async (task: ExecutionTask, event: IIntercomEvent) => {
		if (
			!visible() ||
			!(await client.claimEvent(task.taskId, event.event_id)) ||
			disposed
		)
			return;
		const current = client
			.getTasks()
			.find((item) => item.taskId === task.taskId);
		if (!current || isExecutionFinished(current)) return;
		const payload = event.payload;
		if (event.event_type === "toast") {
			const message =
				typeof payload?.message === "string" ? payload.message : "";
			if (!message) return;
			const kind = payload?.level ?? payload?.variant ?? payload?.type;
			if (kind === "error") toast.error(message);
			else if (kind === "success") toast.success(message);
			else if (kind === "warning") toast.warning(message);
			else toast(message);
		} else if (
			event.event_type === "progress" &&
			typeof payload?.id === "string" &&
			typeof payload?.message === "string"
		) {
			const data = {
				id: payload.id,
				message: payload.message,
				progress: payload.progress,
				done: !!payload.done,
				success: payload.success !== false,
			};
			let owned = progress.get(task.taskId);
			if (!owned) {
				owned = new Map();
				progress.set(task.taskId, owned);
			}
			if (data.done) owned.delete(data.id);
			else owned.set(data.id, data);
			showProgressToast(data);
		} else {
			dispatchPaymentRequest(event);
		}
	};

	const update = () => {
		if (disposed) return;
		const tasks = client.getTasks();
		const retained = new Set(tasks.map((task) => task.taskId));
		for (const [id, session] of sessions) {
			if (!retained.has(id)) {
				session.close();
				sessions.delete(id);
				observed.delete(id);
				finished.delete(id);
			}
		}
		for (const task of tasks) {
			const done = isExecutionFinished(task);
			try {
				deliverExecutionSurfaceEvents(task);
			} catch (error) {
				console.warn("Execution page update failed", error);
			}
			const interactions = task.pendingInteractions.flatMap((request) => {
				if (request.event.event_type !== "interaction_request") return [];
				const original = request.event.payload as IInteractionRequest;
				return [
					{
						...original,
						status:
							request.status === "submitted"
								? "responded"
								: request.status === "submitting"
									? "pending"
									: request.status === "expired"
										? "expired"
										: done || request.status === "cancelled"
											? "cancelled"
											: "pending",
						response_value: request.response_value,
					} as IInteractionRequest,
				];
			});
			if (interactions.length)
				addSessionInteractions(task.streamId, interactions);
			let seen = observed.get(task.taskId);
			if (!seen) {
				seen = new Set();
				observed.set(task.taskId, seen);
			}
			if (!done) {
				let session = sessions.get(task.taskId);
				if (!session) {
					session = createDeviceCommandSession({
						appId: task.appId,
						eventId: task.eventId,
						executionTarget: "remote",
					});
					sessions.set(task.taskId, session);
				}
				for (const event of task.events) {
					if (seen.has(event.event_id)) continue;
					if (
						event.event_type === "a2ui" &&
						event.payload?.type === "deviceCommand"
					) {
						if (visible()) {
							session.filter([event]);
							seen.add(event.event_id);
						}
						continue;
					}
					const notification = [
						"toast",
						"progress",
						"payment_request",
						"payment_simulation",
					].includes(event.event_type);
					if (notification && !visible()) continue;
					seen.add(event.event_id);
					if (event.event_type === "run_initiated") session.filter([event]);
					if (notification) {
						void notify(task, event).catch((error) =>
							console.warn("Execution notification failed", error),
						);
					}
				}
				// Requests are retried when a matching surface mounts, without repeating an action already claimed.
				for (const request of task.pendingInteractions) {
					if (
						request.status !== "pending" ||
						request.event.event_type !== "a2ui"
					)
						continue;
					if (handleWidgetQueryMessage(request.event.payload)) continue;
					handleElementsRequestMessage(request.event.payload, () =>
						executionElementSource(
							task.appId,
							task.pageId,
							task.widgetInstanceId,
						),
					);
				}
			} else if (!finished.has(task.taskId)) {
				finished.add(task.taskId);
				sessions.get(task.taskId)?.close();
				for (const data of progress.get(task.taskId)?.values() ?? [])
					showProgressToast({
						...data,
						done: true,
						success: task.status === "completed",
					});
				progress.delete(task.taskId);
				if (task.chat && task.status !== "cancelled") {
					const messageId = task.chat.responseMessage.id;
					void chatDb.sessions
						.get(task.streamId)
						.then((session) => {
							if (disposed || session?.appId !== task.appId) return;
							recordChatActivity({
								target: {
									kind: "app",
									appId: task.appId,
									eventId: task.eventId,
									sessionId: task.streamId,
								},
								messageId,
								title: task.title,
								preview: task.preview || task.error || "Reply ready",
							});
						})
						.catch((error) =>
							console.warn("Chat activity could not be recorded", error),
						);
				}
			}
		}
	};
	const unsubscribe = client.subscribe(update);
	const unsubscribeSurfaces = subscribeExecutionSurfaces(update);
	const unsubscribeWidgets = subscribeMicroWidgetBridges(update);
	document.addEventListener("visibilitychange", update);
	window.addEventListener("focus", update);
	update();
	return () => {
		disposed = true;
		unsubscribe();
		unsubscribeSurfaces();
		unsubscribeWidgets();
		document.removeEventListener("visibilitychange", update);
		window.removeEventListener("focus", update);
		for (const session of sessions.values()) session.close();
		for (const owned of progress.values())
			for (const id of owned.keys()) {
				toast.dismiss(id);
				getActiveProgressToasts().delete(id);
			}
	};
}
