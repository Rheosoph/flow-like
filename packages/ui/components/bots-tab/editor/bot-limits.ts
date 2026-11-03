/** The bot would run on the server and the plan's bots there are used up. */
export const BOT_SERVER_LIMIT_REACHED = "BOT_SERVER_LIMIT_REACHED";

const ERROR_TAG = /^[^[]*\[[A-Z][A-Z0-9_]*(?:;[^\]]*)?\]\s*/;

/**
 * What the hub says about the plan's allowance, the part before the ways out it suggests after a
 * `;`: the plan, its limit and how many bots are used. Empty when it said nothing.
 */
export function serverLimitFact(message: string): string {
	const text = message.replace(ERROR_TAG, "");
	const [fact = ""] = text.split(";");
	return fact.trim().replace(/[.!]+$/, "");
}
