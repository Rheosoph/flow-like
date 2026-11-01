import { processChatEvents } from "../../components/interfaces/chat-default/event-processor";
import type {
	IAttachment,
	IMessage,
	IPlanStep,
} from "../../components/interfaces/chat-default/chat-db";
import { Response } from "../../lib/llm/response";
import { IRole } from "../../lib/schema/llm/history";
import { createAgentDebugStreamRecorder } from "./agent-debug-report";
import {
	applyStreamEvent,
	createStreamAccumulator,
	orderedSteps,
} from "./copilot-stream-steps";
import { useGlobalChatStore } from "./global-chat-store";
import { foldA2UIServerMessage } from "../../components/a2ui/fold-surfaces";
import type { A2UIServerMessage, Surface } from "../../components/a2ui/types";

/** Render native execution output inside its owning turn. No tools execute here. */
export function createNativeStreamPresentation(runId: string) {
	const specialists = new Map<
		string,
		{
			parser: ReturnType<typeof createAgentDebugStreamRecorder>;
			acc: ReturnType<typeof createStreamAccumulator>;
		}
	>();
	const chats = new Map<
		string,
		{
			message: IMessage;
			messages: IMessage[];
			intermediate: Response;
			attachments: Map<string, IAttachment>;
			surfaces: Map<string, Surface>;
			done: boolean;
			surfaceId?: string;
		}
	>();
	const publishSteps = (prefix: string, steps: IPlanStep[]) => {
		const store = useGlobalChatStore.getState();
		const run = store.runs[runId];
		if (!run) return;
		const current = run.subPlanSteps;
		const at = current.findIndex((step) => step.id.startsWith(prefix));
		const others = current.filter((step) => !step.id.startsWith(prefix));
		const mine = steps.map(({ content_offset, ...step }) => ({
			...step,
			id: `${prefix}${step.id}`,
			title: `↳ ${step.title}`,
		}));
		others.splice(at < 0 ? others.length : at, 0, ...mine);
		store.setSubPlanSteps(runId, others);
	};
	return (chunk: string): boolean => {
		let frame: Record<string, any>;
		try {
			frame = JSON.parse(chunk);
		} catch {
			return false;
		}
		if (
			!frame ||
			!["native_app_chat_event", "native_specialist_event"].includes(frame.type)
		)
			return false;
		const store = useGlobalChatStore.getState();
		if (!store.runs[runId]) return true;
		if (frame.type === "native_specialist_event") {
			if (
				typeof frame.requestId !== "string" ||
				typeof frame.frame !== "string"
			)
				return true;
			let specialist = specialists.get(frame.requestId);
			if (!specialist) {
				specialist = {
					acc: createStreamAccumulator(),
					parser: createAgentDebugStreamRecorder({
						scope: "nested",
						requestId: frame.requestId,
						parentRequestId: frame.parentRequestId,
						record: (event) =>
							useGlobalChatStore.getState().recordDebugEvent(runId, event),
					}),
				};
				specialists.set(frame.requestId, specialist);
			}
			for (const event of specialist.parser.push(frame.frame)) {
				if (event.type !== "text") applyStreamEvent(specialist.acc, event);
			}
			if (frame.stage === "finished") {
				for (const [id, step] of specialist.acc.steps) {
					if (step.status === "progress" || step.status === "planned")
						specialist.acc.steps.set(id, {
							...step,
							status: frame.status === "error" ? "failed" : "done",
						});
				}
			}
			publishSteps(`native:${frame.requestId}:`, orderedSteps(specialist.acc));
			store.addSubUsageStats(runId, specialist.acc.usageStats);
			return true;
		}
		const origin = frame.origin;
		if (
			!origin ||
			![origin.app_id, origin.board_id, origin.event_id, origin.chat_id].every(
				(id) => typeof id === "string",
			)
		)
			return true;
		let chat = chats.get(origin.chat_id);
		if (!chat) {
			const message: IMessage = {
				id: `native:${origin.chat_id}`,
				appId: origin.app_id,
				sessionId: origin.chat_id,
				inner: { role: IRole.Assistant, content: "" },
				files: [],
				tools: [],
				actions: [],
				timestamp: Date.now(),
			};
			chat = {
				message,
				messages: [message],
				intermediate: Response.default(),
				attachments: new Map(),
				surfaces: new Map(),
				done: false,
			};
			chats.set(origin.chat_id, chat);
		}
		const result = processChatEvents([frame.event], {
			responseMessage: chat.message,
			responseMessages: chat.messages,
			intermediateResponse: chat.intermediate,
			attachments: chat.attachments,
			tmpLocalState: null,
			tmpGlobalState: null,
			done: chat.done,
			appId: origin.app_id,
			eventId: origin.event_id,
			sessionId: origin.chat_id,
		});
		chat.message = result.responseMessage;
		chat.messages = result.responseMessages;
		chat.intermediate = result.intermediateResponse;
		chat.attachments = result.attachments;
		chat.done = result.done;
		const widgetOrigin = {
			appId: origin.app_id,
			boardId: origin.board_id,
			eventId: origin.event_id,
		};
		store.addPendingAppRef(runId, origin.app_id);
		store.addSubWidgets(
			runId,
			chat.messages
				.flatMap((message) => message.widgets ?? [])
				.map((widget) => ({ ...widget, origin: widgetOrigin })),
		);
		if (chat.message.main_widget) {
			store.setSubMainWidget(runId, {
				...chat.message.main_widget,
				widget: chat.message.main_widget.widget
					? { ...chat.message.main_widget.widget, origin: widgetOrigin }
					: null,
			});
		}
		store.addSubAttachments(
			runId,
			chat.messages.flatMap((message) => message.files ?? []),
		);
		store.addSubUsageStats(
			runId,
			chat.messages.flatMap((message) => message.usage_stats ?? []),
		);
		publishSteps(
			`native:${origin.chat_id}:`,
			chat.messages.flatMap((message) =>
				(message.plan_steps ?? []).map((step) => ({
					...step,
					id: `${message.id}:${step.id}`,
				})),
			),
		);
		if (result.interactions?.length) store.addInteractions(result.interactions);
		if (frame.event?.event_type === "a2ui" && frame.event.payload) {
			chat.surfaces = foldA2UIServerMessage(
				chat.surfaces,
				frame.event.payload as A2UIServerMessage,
			);
			if (chat.surfaces.size) {
				const surfaces = [...chat.surfaces.values()];
				if (chat.surfaceId) {
					const id = chat.surfaceId;
					useGlobalChatStore.setState((state) => ({
						inlineAppSurfaces: state.inlineAppSurfaces.map((surface) =>
							surface.id === id ? { ...surface, surfaces } : surface,
						),
					}));
				} else {
					store.addInlineAppSurface(
						{ appId: origin.app_id, name: origin.app_id, surfaces },
						runId,
					);
					chat.surfaceId = useGlobalChatStore
						.getState()
						.inlineAppSurfaces.at(-1)?.id;
				}
			} else if (chat.surfaceId) {
				store.removeInlineAppSurface(chat.surfaceId);
				chat.surfaceId = undefined;
			}
		}
		return true;
	};
}
