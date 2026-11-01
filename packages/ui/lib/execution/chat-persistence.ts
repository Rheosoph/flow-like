import {
	type IGlobalState,
	type ILocalChatState,
	type IMessage,
	chatDb,
	decodeMessageRow,
	encodeMessageRow,
} from "../../components/interfaces/chat-default/chat-db";
import { finalizePlanSteps } from "../../components/interfaces/chat-default/chat-event-fold";
import {
	type IChatTurn,
	createChatTurn,
} from "../../components/interfaces/chat-default/chat-turn-state";
import {
	isReactionOnlyResponse,
	persistWorkflowReactions,
	preserveWorkflowReactionState,
} from "../../components/interfaces/chat-default/workflow-reactions";
import type { ExecutionTask } from "./types";

export interface ExecutionChatSeed {
	responseMessage: IMessage;
	tmpLocalState?: ILocalChatState | null;
	tmpGlobalState?: IGlobalState | null;
}

const turns = new Map<string, IChatTurn>();

/** The runtime writes one ordered transcript, regardless of how many tabs render it. */
export async function persistExecutionChat(
	task: ExecutionTask,
	final: boolean,
): Promise<void> {
	if (!task.chat) return;
	const seed = task.chat;
	const sessionId = seed.responseMessage.sessionId;
	let turn = turns.get(task.taskId);
	if (!turn) {
		turn = createChatTurn({
			...structuredClone(seed),
			tmpLocalState: structuredClone(seed.tmpLocalState),
			tmpGlobalState: structuredClone(seed.tmpGlobalState),
			appId: task.appId,
			eventId: task.eventId,
			sessionId,
			handleRequests: false,
		});
		turns.set(task.taskId, turn);
	}
	turn.catchUp(task.events);
	const messages = structuredClone(turn.responseMessages);
	if (final) {
		for (const message of messages) finalizePlanSteps(message);
		const last = messages.at(-1);
		if (last && (task.status === "failed" || task.status === "cancelled")) {
			last.error = {
				kind: task.status === "cancelled" ? "cancelled" : "generic",
				title: task.status === "cancelled" ? "Stopped" : "Run interrupted",
				message:
					task.error ||
					(task.status === "cancelled"
						? "You stopped this run."
						: "The run ended before its reply was complete."),
				retryable: true,
			};
		}
	}
	const live = turn.live();
	await chatDb.transaction(
		"rw",
		[
			chatDb.sessions,
			chatDb.messages,
			chatDb.drafts,
			chatDb.localStage,
			chatDb.globalState,
		],
		async () => {
			// Deleting a conversation while it runs must not resurrect it on the next checkpoint.
			const session = await chatDb.sessions.get(sessionId);
			if (!session || session.appId !== task.appId) return;
			for (const message of messages) {
				if (final && isReactionOnlyResponse(message, task.events)) {
					await chatDb.messages.delete(message.id);
					await chatDb.drafts.delete(message.id);
					continue;
				}
				const existing = await chatDb.messages.get(message.id);
				const draft = await chatDb.drafts.get(message.id);
				message.rev =
					Math.max(message.rev ?? 0, existing?.rev ?? 0, draft?.rev ?? 0) + 1;
				// Keep user edits made in another tab while the response streams.
				if (existing) {
					const saved = decodeMessageRow(existing);
					message.reactions = saved.reactions;
					message.rating = saved.rating;
					message.ratingSettings = saved.ratingSettings;
				}
				preserveWorkflowReactionState(
					message,
					existing && decodeMessageRow(existing),
					draft && decodeMessageRow(draft),
				);
				const row = encodeMessageRow(message);
				await chatDb.messages.put({
					...row,
					...(!final ? { streaming: true } : {}),
				});
				if (!final) await chatDb.drafts.put(row);
			}
			await persistWorkflowReactions(task.events, {
				appId: task.appId,
				sessionId,
				triggerMessageId: seed.responseMessage.trigger_message_id,
			});
			if (live.tmpLocalState) await chatDb.localStage.put(live.tmpLocalState);
			if (live.tmpGlobalState)
				await chatDb.globalState.put(live.tmpGlobalState);
			await chatDb.sessions.update(sessionId, { updatedAt: Date.now() });
			if (final) {
				const rootId =
					seed.responseMessage.stream_root_id ?? seed.responseMessage.id;
				const drafts = await chatDb.drafts
					.where("sessionId")
					.equals(sessionId)
					.toArray();
				const owned = drafts.filter((row) => {
					const message = decodeMessageRow(row);
					return (message.stream_root_id ?? message.id) === rootId;
				});
				await chatDb.drafts.bulkDelete(owned.map((row) => row.id));
			}
		},
	);
	if (final) turns.delete(task.taskId);
}
