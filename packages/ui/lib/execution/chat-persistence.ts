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
import type { IChatMessageError } from "../flowpilot/chat-error";
import type { ExecutionTask } from "./types";

export interface ExecutionChatSeed {
	responseMessage: IMessage;
	tmpLocalState?: ILocalChatState | null;
	tmpGlobalState?: IGlobalState | null;
	/** In the sender's language: the runtime saves a detached reply without translations or a tab. */
	detachedNotice?: Pick<IChatMessageError, "title" | "message">;
}

const turns = new Map<string, IChatTurn>();

/** Why a saved reply ends where it does. */
function replyCard(task: ExecutionTask): IChatMessageError | undefined {
	if (task.status === "cancelled")
		return {
			kind: "cancelled",
			title: "Stopped",
			message: task.error || "You stopped this run.",
			retryable: true,
		};
	if (task.status === "failed")
		return {
			kind: "generic",
			title: "Run interrupted",
			message: task.error || "The run ended before its reply was complete.",
			retryable: true,
		};
	const notice = task.detached ? task.chat?.detachedNotice : undefined;
	// The neutral hourglass card: the run did not fail, it is still going.
	return notice && { kind: "rate-limit", ...notice, retryable: false };
}

/**
 * The runtime writes one ordered transcript, regardless of how many tabs render it. A detached
 * run's reply is saved as incomplete; a stop saves it once more, so only a final save drops its fold.
 */
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
	const settled = final || task.detached === true;
	// The last save of a detached run comes later and folds nothing new: its state must not
	// overwrite what a newer run in the conversation wrote meanwhile.
	const writesState = !(final && task.detached);
	const messages = structuredClone(turn.responseMessages);
	if (settled) {
		for (const message of messages) finalizePlanSteps(message);
		const last = messages.at(-1);
		const card = replyCard(task);
		if (last && card) last.error = card;
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
				if (settled && isReactionOnlyResponse(message, task.events)) {
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
					...(!settled ? { streaming: true } : {}),
				});
				if (!settled) await chatDb.drafts.put(row);
			}
			await persistWorkflowReactions(task.events, {
				appId: task.appId,
				sessionId,
				triggerMessageId: seed.responseMessage.trigger_message_id,
			});
			if (writesState && live.tmpLocalState)
				await chatDb.localStage.put(live.tmpLocalState);
			if (writesState && live.tmpGlobalState)
				await chatDb.globalState.put(live.tmpGlobalState);
			await chatDb.sessions.update(sessionId, { updatedAt: Date.now() });
			if (settled) {
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
