import type { IMessage } from "./chat-db";

/** Keep replies after their questions even when restored timestamps differ or a turn is steered. */
export function sortChatMessages(messages: readonly IMessage[]): IMessage[] {
	const byId = new Map(messages.map((message) => [message.id, message]));
	const questionTimes = new Map<string, number>();
	const anchor = (
		reply: IMessage | undefined,
		question: IMessage | undefined,
	) => {
		if (
			!reply ||
			!question ||
			reply.inner.role !== "assistant" ||
			question.inner.role !== "user" ||
			reply.sessionId !== question.sessionId
		)
			return;
		questionTimes.set(
			reply.id,
			Math.max(
				questionTimes.get(reply.id) ?? Number.NEGATIVE_INFINITY,
				question.timestamp,
			),
		);
	};
	for (const message of messages) {
		const questionId = message.run_context?.user_message_id;
		if (questionId) anchor(message, byId.get(questionId));
		if (message.steered_run_id)
			anchor(byId.get(message.steered_run_id), message);
	}

	// Derive a sort key instead of comparing linked pairs specially, which can make
	// comparisons contradict one another. Recorded timestamps stay unchanged.
	return messages
		.map((message) => {
			const questionTime = questionTimes.get(message.id);
			return {
				message,
				timestamp: Math.max(
					message.timestamp,
					questionTime ?? Number.NEGATIVE_INFINITY,
				),
				afterQuestion:
					questionTime !== undefined && questionTime >= message.timestamp,
			};
		})
		.sort(
			(a, b) =>
				a.timestamp - b.timestamp ||
				Number(a.afterQuestion) - Number(b.afterQuestion),
		)
		.map(({ message }) => message);
}
