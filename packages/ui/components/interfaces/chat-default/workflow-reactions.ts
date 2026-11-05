import type { IIntercomEvent } from "../../../lib/schema/events/intercom-event";
import { IContentType } from "../../../lib/schema/llm/history";
import {
	type IMessage,
	chatDb,
	decodeMessageRow,
	encodeMessageRow,
	putChatDrafts,
} from "./chat-db";
import { applyMessageReaction } from "./chat-message-interaction";

/** A workflow may acknowledge a message with a reaction and send no assistant reply. */
export function isReactionOnlyResponse(
	message: IMessage,
	events: readonly Pick<IIntercomEvent, "event_type" | "payload">[],
): boolean {
	if (!events.some((event) => event.event_type === "chat_reaction"))
		return false;
	const content = message.inner.content;
	const hasContent =
		typeof content === "string"
			? Boolean(content.trim())
			: content.some(
					(part) =>
						part.type !== IContentType.Text || Boolean(part.text?.trim()),
				);
	return (
		!hasContent &&
		!message.error &&
		!message.files.length &&
		!message.widgets?.length &&
		!events.some((event) => event.payload?.main_widget != null) &&
		!message.plan_steps?.length
	);
}

/** Body checkpoints may be newer while still carrying an older reaction snapshot. */
export function preserveWorkflowReactionState(
	message: IMessage,
	...saved: (IMessage | undefined)[]
): void {
	let latest = message;
	for (const candidate of saved) {
		if (
			!candidate ||
			candidate.appId !== message.appId ||
			candidate.sessionId !== message.sessionId
		)
			continue;
		const count = candidate.workflowReactionEventIds?.length ?? 0;
		const previousCount = latest.workflowReactionEventIds?.length ?? 0;
		if (
			count > previousCount ||
			(count === previousCount &&
				latest.workflowReactions === undefined &&
				candidate.workflowReactions !== undefined)
		)
			latest = candidate;
	}
	message.workflowReactions = latest.workflowReactions?.slice();
	message.workflowReactionEventIds = latest.workflowReactionEventIds?.slice();
}

/** Keep reactions from another run when saving a newer streaming body. */
export async function putChatDraftsWithReactions(
	messages: readonly IMessage[],
): Promise<void> {
	await chatDb.transaction("rw", chatDb.messages, chatDb.drafts, async () => {
		for (const message of messages) {
			const saved = await chatDb.messages.get(message.id);
			const draft = await chatDb.drafts.get(message.id);
			preserveWorkflowReactionState(
				message,
				saved && decodeMessageRow(saved),
				draft && decodeMessageRow(draft),
			);
			message.rev = Math.max(
				message.rev ?? 0,
				saved?.rev ?? 0,
				draft?.rev ?? 0,
			);
		}
		await putChatDrafts(messages);
	});
}

interface WorkflowReactionTarget {
	appId: string;
	sessionId: string;
	triggerMessageId?: string;
}

/** Apply workflow feedback to saved messages without changing the user's own reactions. */
export async function persistWorkflowReactions(
	events: readonly Pick<
		IIntercomEvent,
		"event_id" | "event_type" | "payload"
	>[],
	target: WorkflowReactionTarget,
): Promise<void> {
	const updates = events.flatMap((event) => {
		if (event.event_type !== "chat_reaction") return [];
		const payload = event.payload;
		if (
			!payload ||
			typeof event.event_id !== "string" ||
			!event.event_id ||
			typeof payload.emoji !== "string" ||
			!payload.emoji.trim() ||
			payload.emoji.trim().length > 64 ||
			(payload.operation !== "add" && payload.operation !== "remove") ||
			(payload.message_id != null && typeof payload.message_id !== "string")
		)
			return [];
		const messageId = payload.message_id?.trim() || target.triggerMessageId;
		return messageId
			? [
					{
						eventId: event.event_id,
						messageId,
						emoji: payload.emoji.trim(),
						operation: payload.operation as "add" | "remove",
					},
				]
			: [];
	});
	if (!updates.length) return;
	await chatDb.transaction(
		"rw",
		[chatDb.sessions, chatDb.messages, chatDb.drafts],
		async () => {
			const session = await chatDb.sessions.get(target.sessionId);
			if (!session || session.appId !== target.appId) return;
			for (const messageId of new Set(
				updates.map((update) => update.messageId),
			)) {
				const rows = await Promise.all([
					chatDb.messages.get(messageId),
					chatDb.drafts.get(messageId),
				]);
				const messages = rows.map((row) => {
					if (!row || row.sessionId !== target.sessionId) return undefined;
					const message = decodeMessageRow(row);
					return message.appId === target.appId ? message : undefined;
				});
				const first = messages.find((message) => message !== undefined);
				if (!first) continue;
				const state = { ...first };
				preserveWorkflowReactionState(state, ...messages);
				let reactions = state.workflowReactions ?? [];
				const applied = new Set(state.workflowReactionEventIds);
				for (const update of updates) {
					if (update.messageId !== messageId || applied.has(update.eventId))
						continue;
					reactions = applyMessageReaction(
						reactions,
						update.emoji,
						update.operation,
					);
					applied.add(update.eventId);
				}
				const eventIds = [...applied];
				for (const [index, row] of rows.entries()) {
					const message = messages[index];
					if (!row || !message) continue;
					if (
						JSON.stringify(reactions) ===
							JSON.stringify(message.workflowReactions ?? []) &&
						JSON.stringify(eventIds) ===
							JSON.stringify(message.workflowReactionEventIds ?? [])
					)
						continue;
					message.workflowReactions = reactions;
					message.workflowReactionEventIds = eventIds;
					message.rev =
						Math.max(message.rev ?? 0, ...rows.map((item) => item?.rev ?? 0)) +
						1;
					const table = index === 0 ? chatDb.messages : chatDb.drafts;
					await table.put({ ...row, ...encodeMessageRow(message) });
				}
			}
		},
	);
}
