import {
	type BotAnswerCase,
	type BotFacts,
	botAnswerCase,
} from "../../../../lib/device-management/bot-config";
import type { DevicesT } from "../primitives/area-context";

/* What a bot answers in groups and servers: one sentence per case of the message rule. */

const ANSWERS = {
	prefix_and_mentions: (t, prefix) =>
		t(
			"devices:deploy.what.botAnswersPrefix",
			"In groups and servers it answers mentions, replies and every message that starts with {{prefix}}.",
			{ prefix },
		),
	prefix_only: (t, prefix) =>
		t(
			"devices:deploy.what.botAnswersPrefixOnly",
			"In groups and servers it answers every message that starts with {{prefix}}.",
			{ prefix },
		),
	mentions: (t) =>
		t(
			"devices:deploy.what.botAnswers",
			"In groups and servers it answers mentions and replies.",
		),
	every: (t) =>
		t(
			"devices:deploy.what.botAnswersEvery",
			"In groups and servers it answers every message: no command prefix is set.",
		),
} satisfies Record<BotAnswerCase, (t: DevicesT, prefix: string) => string>;

/** The sentence the deploy wizard and the Events editor show for a bot. */
export function botAnswersText(
	t: DevicesT,
	bot: Pick<BotFacts, "provider" | "prefix" | "mentions">,
): string {
	return ANSWERS[botAnswerCase(bot)](t, bot.prefix);
}
