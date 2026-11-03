"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useMemo } from "react";
import type {
	IConversation,
	IInboxEntry,
} from "../../state/backend-state/inbox-state";
import { approvalResponseOf } from "./approval-entry";
import {
	activityTimeKind,
	conversationComposition,
	directPartnerId,
	isSelfPrincipal,
	otherMemberIds,
} from "./crew-model";
import { inboxMessageContent } from "./fold-entries";
import type { CrewDirectory } from "./use-crew-directory";

const MAX_TITLE_NAMES = 3;

/** Titles, previews and timestamps for conversation cards and headers, in the reader's language. */
export function useConversationText(
	directory: CrewDirectory,
	me?: string | null,
) {
	const { t, i18n } = useTranslation("bots");
	const locale = i18n.language;

	const title = useCallback(
		(conversation: IConversation): string => {
			const partner = directPartnerId(conversation, me);
			if (partner) return directory(partner).name;
			const custom = conversation.title?.trim();
			if (custom) return custom;
			const others = otherMemberIds(conversation, me);
			if (others.length === 0) return t("crew.rooms.justYou", "Just you");
			const names = others
				.slice(0, MAX_TITLE_NAMES)
				.map((id) => directory(id).name)
				.join(", ");
			const more = others.length - MAX_TITLE_NAMES;
			return more > 0
				? t("crew.rooms.namesAndMore", "{{names}} and {{count}} more", {
						names,
						count: more,
					})
				: names;
		},
		[directory, me, t],
	);

	const entryText = useCallback(
		(entry: IInboxEntry | null): string => {
			if (!entry) return t("crew.rooms.noMessages", "No messages yet");
			const answer = approvalResponseOf(entry)?.decision;
			if (answer) {
				return {
					once: t("crew.approval.answeredOnce", "Allowed once"),
					always: t("crew.approval.answeredAlways", "Always allowed"),
					deny: t("crew.approval.answeredDeny", "Denied"),
				}[answer];
			}
			const content = inboxMessageContent(entry);
			if (content.kind === "sealed")
				return t("crew.messages.encrypted", "Encrypted message");
			if (content.kind === "unsupported")
				return t("crew.messages.unsupported", "Message this app can't show");
			return content.text.replace(/\s+/g, " ").trim();
		},
		[t],
	);

	const senderLabel = useCallback(
		(senderId: string): string =>
			isSelfPrincipal(senderId, me)
				? t("crew.you", "You")
				: directory(senderId).name,
		[directory, me, t],
	);

	const composition = useCallback(
		(conversation: IConversation): string => {
			const { people, bots } = conversationComposition(conversation);
			const peopleText = t("crew.rooms.people", "{{count}} people", {
				count: people,
				defaultValue_one: "{{count}} person",
			});
			if (bots === 0) return peopleText;
			return t("crew.rooms.composition", "{{people}} · {{bots}}", {
				people: peopleText,
				bots: t("crew.rooms.bots", "{{count}} bots", {
					count: bots,
					defaultValue_one: "{{count}} bot",
				}),
			});
		},
		[t],
	);

	const formats = useMemo(
		() => ({
			time: new Intl.DateTimeFormat(locale, {
				hour: "numeric",
				minute: "2-digit",
			}),
			weekday: new Intl.DateTimeFormat(locale, { weekday: "short" }),
			date: new Intl.DateTimeFormat(locale, { day: "numeric", month: "short" }),
		}),
		[locale],
	);

	const activity = useCallback(
		(iso: string, now = new Date()): string => {
			const time = Date.parse(iso);
			switch (activityTimeKind(time, now)) {
				case "today":
					return formats.time.format(time);
				case "yesterday":
					return t("crew.time.yesterday", "Yesterday");
				case "week":
					return formats.weekday.format(time);
				case "older":
					return formats.date.format(time);
				default:
					return "";
			}
		},
		[formats, t],
	);

	return { title, entryText, senderLabel, composition, activity, formats };
}

export type ConversationText = ReturnType<typeof useConversationText>;
