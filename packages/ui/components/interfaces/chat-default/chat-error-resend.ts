import type { IMessage } from "./chat-db";

function textOf(message: IMessage): string {
	if (message.source_text) return message.source_text;
	const content = message.inner.content;
	if (typeof content === "string") return content;
	return content
		.filter((part) => part.type === "text" && typeof part.text === "string")
		.map((part) => part.text)
		.join("");
}

/**
 * The question a failed answer belongs to, as text to send again: the nearest ask of yours before
 * it. Nothing comes back when something was said after the failure (the person went on, or it was
 * sent again already), or when the ask carried files or only a reaction, which a resend of the
 * text would lose.
 */
export function askToResend(
	messages: readonly IMessage[],
	answerId: string,
): string | null {
	const at = messages.findIndex((message) => message.id === answerId);
	if (at < 0) return null;
	if (messages.slice(at + 1).some((message) => message.inner.role === "user")) {
		return null;
	}
	for (let index = at - 1; index >= 0; index--) {
		const ask = messages[index];
		if (ask.inner.role !== "user") continue;
		const text = textOf(ask).trim();
		const plain =
			ask.files.length === 0 && ask.interaction?.type !== "reaction";
		return plain && text ? text : null;
	}
	return null;
}
