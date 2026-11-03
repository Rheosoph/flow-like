import { LOCAL_OWNER_ID } from "../../state/backend-state/bot-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { isApprovalCard } from "./approval-entry";

/**
 * `account`: signed in. `local`: signed out on a host that keeps bots on this device, so `local`
 * is you. `guest`: signed out anywhere else; only Assistant and Flux.
 */
export type CrewMode = "account" | "local" | "guest";

/** The reader: their account's sub, or `local`, who owns this device's bots and chats. */
export function isSelfPrincipal(
	principalId: string | null | undefined,
	me?: string | null,
): boolean {
	if (!principalId) return false;
	return principalId === me || principalId === LOCAL_OWNER_ID;
}

export function otherMemberIds(
	conversation: Pick<IConversation, "members">,
	me?: string | null,
): string[] {
	return conversation.members
		.map((member) => member.principal_id)
		.filter((id) => !isSelfPrincipal(id, me));
}

/** The one other principal of a direct conversation. */
export function directPartnerId(
	conversation: Pick<IConversation, "kind" | "members">,
	me?: string | null,
): string | undefined {
	if (conversation.kind !== "direct") return undefined;
	return otherMemberIds(conversation, me)[0];
}

export function isBotMember(
	conversation: Pick<IConversation, "members">,
	principalId: string,
	myBotIds: ReadonlySet<string>,
): boolean {
	return (
		myBotIds.has(principalId) ||
		conversation.members.some(
			(member) => member.principal_id === principalId && member.kind === "bot",
		)
	);
}

/** Active members including the reader, split into people and bots. */
export function conversationComposition(
	conversation: Pick<IConversation, "members">,
): { people: number; bots: number } {
	const bots = conversation.members.filter(
		(member) => member.kind === "bot",
	).length;
	return { people: conversation.members.length - bots, bots };
}

/**
 * Conversations waiting on the reader: unread, and the newest message came from a bot. A bot's
 * approval card stays here, read or not, until an answer becomes the newest message.
 */
export function selectNeedsYou(
	conversations: readonly IConversation[],
	options: {
		me?: string | null;
		myBotIds: ReadonlySet<string>;
		openConversationId?: string | null;
	},
): IConversation[] {
	return conversations.filter((conversation) => {
		const sender = conversation.last_entry?.sender_id;
		return (
			(conversation.unread_count > 0 ||
				isApprovalCard(conversation.last_entry)) &&
			conversation.id !== options.openConversationId &&
			Boolean(sender) &&
			!isSelfPrincipal(sender, options.me) &&
			isBotMember(conversation, sender as string, options.myBotIds)
		);
	});
}

export function splitConversations(conversations: readonly IConversation[]): {
	rooms: IConversation[];
	direct: IConversation[];
} {
	return {
		rooms: conversations.filter(
			(conversation) => conversation.kind === "group",
		),
		direct: conversations.filter(
			(conversation) => conversation.kind === "direct",
		),
	};
}

export function findDirectConversation(
	conversations: readonly IConversation[] | undefined,
	principalId: string,
	me?: string | null,
): IConversation | undefined {
	return conversations?.find(
		(conversation) => directPartnerId(conversation, me) === principalId,
	);
}

function normalize(value: string): string {
	return value.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();
}

/** Every word of the query appears in at least one field. */
export function matchesSearch(
	query: string,
	fields: readonly (string | null | undefined)[],
): boolean {
	const words = normalize(query).split(/\s+/).filter(Boolean);
	if (words.length === 0) return true;
	const haystack = normalize(fields.filter(Boolean).join(" "));
	return words.every((word) => haystack.includes(word));
}

export type CrewTranslate = (
	key: string,
	options: Record<string, unknown>,
) => string;

/** "2 things need you · 4 bots"; the bot count is left out while it is unknown. */
export function crewSummaryLine(
	t: CrewTranslate,
	counts: { needs: number; bots: number | null },
): string {
	const needs =
		counts.needs > 0
			? t("crew.summary.needsYou", {
					ns: "bots",
					count: counts.needs,
					defaultValue: "{{count}} things need you",
					defaultValue_one: "{{count}} thing needs you",
				})
			: t("crew.summary.nothingNeedsYou", {
					ns: "bots",
					defaultValue: "Nothing needs you",
				});
	if (counts.bots === null) return needs;
	const bots = t("crew.summary.bots", {
		ns: "bots",
		count: counts.bots,
		defaultValue: "{{count}} bots",
		defaultValue_one: "{{count}} bot",
	});
	return t("crew.summary.line", {
		ns: "bots",
		needs,
		bots,
		defaultValue: "{{needs}} · {{bots}}",
	});
}

export type ActivityTimeKind = "today" | "yesterday" | "week" | "older";

function startOfDay(date: Date): number {
	return new Date(
		date.getFullYear(),
		date.getMonth(),
		date.getDate(),
	).getTime();
}

/** Which way to print a timestamp in a list: a clock time today, a weekday this week, else a date. */
export function activityTimeKind(
	time: number,
	now: Date,
): ActivityTimeKind | null {
	if (!Number.isFinite(time)) return null;
	const days = Math.round(
		(startOfDay(now) - startOfDay(new Date(time))) / 86_400_000,
	);
	if (days <= 0) return "today";
	if (days === 1) return "yesterday";
	if (days < 7) return "week";
	return "older";
}
