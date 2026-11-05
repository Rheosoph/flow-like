"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useMemo } from "react";
import type { LocalPrincipalResolver } from "../../hooks/use-user-lookup";
import { type IBot, LOCAL_OWNER_ID } from "../../state/backend-state/bot-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
import type { IUserLookup } from "../../state/backend-state/types";
import { defaultBotCatalog } from "../bots/catalog";

/** A local bot as the directory would describe it. */
export function localBotLookup(bot: IBot): IUserLookup {
	return {
		id: bot.id,
		name: bot.name ?? undefined,
		description: bot.description ?? undefined,
		created_at: bot.created_at,
		bot: {
			owner_id: bot.owner_id,
			system: bot.system,
			appearance: bot.appearance,
			deleted: false,
		},
	};
}

/** Ids that only this device knows: every principal of a local chat except you. */
export function localConversationPrincipals(
	conversations: readonly IConversation[],
): Set<string> {
	const ids = new Set<string>();
	for (const conversation of conversations) {
		if (!conversation.local) continue;
		for (const member of conversation.members) ids.add(member.principal_id);
		if (conversation.last_entry) ids.add(conversation.last_entry.sender_id);
	}
	ids.delete(LOCAL_OWNER_ID);
	return ids;
}

/** The crew's resolver: local bots, and whoever else sits in a chat on this device. */
export function useCrewLocalPrincipals(
	bots: readonly IBot[] | undefined,
	conversations: readonly IConversation[],
): LocalPrincipalResolver {
	const ids = useMemo(
		() => localConversationPrincipals(conversations),
		[conversations],
	);
	const isLocal = useCallback(
		(principalId: string) => ids.has(principalId),
		[ids],
	);
	return useLocalPrincipals(bots, isLocal);
}

/**
 * Local bots from the bot list, and the other ids `isLocal` vouches for: a bot that is gone
 * reads as deleted once the list has loaded, and stays pending until then. Account bots (a bot
 * that moved still sits in local chats under its account id) and everything else are left to the
 * directory.
 */
export function localPrincipalResolver(
	bots: readonly IBot[] | undefined,
	isLocal: (principalId: string) => boolean,
	deletedName: string,
): LocalPrincipalResolver {
	const localBots = new Map<string, IUserLookup>();
	const accountBots = new Set<string>();
	for (const bot of bots ?? []) {
		if (bot.local) localBots.set(bot.id, localBotLookup(bot));
		else accountBots.add(bot.id);
	}
	return (principalId) => {
		const known = localBots.get(principalId);
		if (known) return known;
		if (
			principalId === LOCAL_OWNER_ID ||
			accountBots.has(principalId) ||
			!isLocal(principalId)
		)
			return undefined;
		if (bots === undefined) return null;
		return {
			id: principalId,
			name: deletedName,
			created_at: "",
			bot: {
				owner_id: LOCAL_OWNER_ID,
				system: null,
				appearance: { kind: defaultBotCatalog.defaultKind },
				deleted: true,
			},
		};
	};
}

export function useLocalPrincipals(
	bots: readonly IBot[] | undefined,
	isLocal: (principalId: string) => boolean,
): LocalPrincipalResolver {
	const { t } = useTranslation("bots");
	const deletedName = t("identity.deleted", "Deleted bot");
	return useMemo(
		() => localPrincipalResolver(bots, isLocal, deletedName),
		[bots, deletedName, isLocal],
	);
}
