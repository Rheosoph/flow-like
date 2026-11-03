import type { IBot, IConversation } from "@flow-like/flow-like-ui";

/** An id in both lists is shown once, as the local copy every call about it is routed to. */
function withoutLocal<T extends { id: string }>(
	remote: readonly T[],
	local: readonly T[],
): T[] {
	const localIds = new Set(local.map((record) => record.id));
	return remote.filter((record) => !localIds.has(record.id));
}

/**
 * Assistant or Flux as this computer made them, with nothing to move: never edited here and
 * in no chat (either moves `updated_at`).
 */
function isUntouchedBuiltIn(bot: IBot): boolean {
	return Boolean(bot.system) && bot.created_at === bot.updated_at;
}

/** This computer's bots that hold something for an account: all but untouched built-ins. */
export function botsToMove(local: readonly IBot[]): IBot[] {
	return local.filter((bot) => !isUntouchedBuiltIn(bot));
}

/**
 * Account bots in the account's order, then the bots on this computer. Assistant and Flux of
 * this computer give way to the account's own unless they hold something to move.
 */
export function mergeBots(
	remote: readonly IBot[],
	local: readonly IBot[],
): IBot[] {
	const accountBuiltIns = new Set(remote.map((bot) => bot.system));
	const shown = local.filter(
		(bot) => !(isUntouchedBuiltIn(bot) && accountBuiltIns.has(bot.system)),
	);
	return [...withoutLocal(remote, shown), ...shown];
}

function activityTime(conversation: IConversation): number {
	const time = Date.parse(conversation.last_activity_at);
	return Number.isNaN(time) ? 0 : time;
}

/** Newest activity first across the account and this computer. */
export function mergeConversations(
	remote: readonly IConversation[],
	local: readonly IConversation[],
): IConversation[] {
	return [...withoutLocal(remote, local), ...local].sort(
		(a, b) => activityTime(b) - activityTime(a),
	);
}
