import {
	type ChatErrorKind,
	type IChatMessageError,
	modelRequiredError,
	modelRequiredFromText,
} from "../../lib/flowpilot/chat-error";

/** The owner's profile has no language model the turn could run on. */
const BOT_TURN_NO_MODEL = "BOT_TURN_NO_MODEL";

const ERROR_KINDS: Record<string, ChatErrorKind> = {
	BOT_TURN_MODEL_ERROR: "server",
	BOT_TURN_TIMEOUT: "network",
	BOT_TURN_QUOTA: "billing",
	BOT_TURN_SETUP: "config",
	BOT_TURN_REPLY_REFUSED: "permission",
};

/**
 * The card of a turn a bot could not answer, from the code its message carries. Where only another
 * model helps, the bot's owner gets the model picker; anyone else just reads why. Replies stored
 * before `BOT_TURN_NO_MODEL` existed say so in their text, under a quota or setup code. An empty
 * `title` leaves the card its default.
 */
export function botTurnError(
	code: string,
	text: string,
	options: { title: string; ownBot: boolean; botId?: string },
): IChatMessageError {
	const { title, ownBot, botId } = options;
	const modelRequired =
		code === BOT_TURN_NO_MODEL
			? modelRequiredError(text, true, { code })
			: modelRequiredFromText(text, false, { code });
	if (modelRequired) {
		const action =
			ownBot && modelRequired.action
				? { ...modelRequired.action, ...(botId ? { botId } : {}) }
				: undefined;
		return { ...modelRequired, title, action };
	}
	return {
		kind: ERROR_KINDS[code] ?? "generic",
		title,
		message: text,
		code,
		retryable: false,
	};
}
