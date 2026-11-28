import type {
	IBot,
	ICalendarFeed,
	IUpdateCalendarFeed,
} from "../../state/backend-state/bot-state";

export type FeedFlag = keyof Required<IUpdateCalendarFeed>;

export type FeedTargetId = "apple" | "google" | "outlook" | "microsoft";

export interface FeedTarget {
	id: FeedTargetId;
	href: string;
}

export const FEED_DRAFT: Required<IUpdateCalendarFeed> = {
	include_events: true,
	include_tasks: true,
	include_memories: false,
	include_notes: false,
};

/** The layers a feed can carry for this bot, by the capabilities that feed them; none means no feed. */
export function feedFlags(bot: Pick<IBot, "capabilities">): FeedFlag[] | null {
	const has = (id: string) => (bot.capabilities as string[]).includes(id);
	const flags: FeedFlag[] = [];
	if (has("calendar")) flags.push("include_events");
	if (has("scheduling")) flags.push("include_tasks");
	if (has("memory")) flags.push("include_memories");
	if (flags.length === 0) return null;
	return [...flags, "include_notes"];
}

/** Where each calendar app takes the link; Apple's own handler is the `webcal` link itself. */
export function feedTargets(
	feed: Pick<ICalendarFeed, "url" | "webcal_url">,
	calendarName: string,
): FeedTarget[] {
	const url = encodeURIComponent(feed.url);
	const name = encodeURIComponent(calendarName);
	return [
		{ id: "apple", href: feed.webcal_url },
		{
			id: "google",
			href: `https://calendar.google.com/calendar/r?cid=${encodeURIComponent(feed.webcal_url)}`,
		},
		{
			id: "outlook",
			href: `https://outlook.live.com/calendar/0/addfromweb?url=${url}&name=${name}`,
		},
		{
			id: "microsoft",
			href: `https://outlook.office.com/calendar/0/addfromweb?url=${url}&name=${name}`,
		},
	];
}

/** The title calendar apps show for the feed. */
export function feedCalendarName(botName: string): string {
	return `${botName} · Flow-Like`;
}
