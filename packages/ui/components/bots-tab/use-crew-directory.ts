"use client";

import { useTranslation } from "@flow-like/locales";
import { type UseQueryResult, useQueries } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { useBotLabels } from "../../hooks/use-bots";
import {
	type UserLookupResult,
	useLocalPrincipalResolver,
	useLookupDisplayName,
	userLookupQueryOptions,
} from "../../hooks/use-user-lookup";
import { useBackend } from "../../state/backend-state";
import { type IBot, LOCAL_OWNER_ID } from "../../state/backend-state/bot-state";
import { isSelfPrincipal } from "./crew-model";

export interface CrewPrincipal {
	id: string;
	name: string;
	isMe: boolean;
	isBot: boolean;
	/** Set for bots: whose bot it is. */
	ownerId?: string;
	/** The pinned voice Bit, shared for call speech. */
	ttsModel?: string | null;
	isPending: boolean;
	isResolved: boolean;
}

export type CrewDirectory = (principalId: string) => CrewPrincipal;

interface LookupState {
	user: UserLookupResult;
	pending: boolean;
}

function combineLookups(
	results: UseQueryResult<UserLookupResult, Error>[],
): LookupState[] {
	return results.map((result) => ({
		user: result.data ?? null,
		pending: result.isPending,
	}));
}

/**
 * Names and bot ownership for the people and bots a view shows, resolved in one batched
 * lookup. The caller's own bots come from their bot list, so renames show at once; `local`
 * (you, signed out) and the local principals the view provides never reach the directory.
 */
export function useCrewDirectory(
	principalIds: readonly string[],
	options: { me?: string | null; bots?: readonly IBot[] },
): CrewDirectory {
	const { t } = useTranslation("bots");
	const backend = useBackend();
	const botLabels = useBotLabels();
	const lookupName = useLookupDisplayName();
	const resolveLocal = useLocalPrincipalResolver();
	const { me, bots } = options;

	const botsById = useMemo(
		() => new Map((bots ?? []).map((bot) => [bot.id, bot])),
		[bots],
	);
	const ids = useMemo(
		() =>
			[...new Set(principalIds)].filter(
				(id) =>
					Boolean(id) &&
					id !== LOCAL_OWNER_ID &&
					!botsById.has(id) &&
					resolveLocal?.(id) === undefined,
			),
		[principalIds, botsById, resolveLocal],
	);
	const lookups = useQueries({
		queries: ids.map((id) => userLookupQueryOptions(backend.userState, id)),
		combine: combineLookups,
	});
	const byId = useMemo(
		() => new Map(ids.map((id, index) => [id, lookups[index]])),
		[ids, lookups],
	);
	const unknown = t("crew.unknownMember", "Someone");
	const you = t("crew.you", "You");

	return useCallback(
		(principalId: string): CrewPrincipal => {
			const isMe = isSelfPrincipal(principalId, me);
			const bot = botsById.get(principalId);
			if (bot) {
				return {
					id: principalId,
					name: botLabels(bot).name || unknown,
					isMe,
					isBot: true,
					ownerId: bot.owner_id,
					ttsModel: bot.tts_model,
					isPending: false,
					isResolved: true,
				};
			}
			if (principalId === LOCAL_OWNER_ID) {
				return {
					id: principalId,
					name: you,
					isMe: true,
					isBot: false,
					isPending: false,
					isResolved: true,
				};
			}
			const local = resolveLocal?.(principalId);
			const lookup = local === undefined ? byId.get(principalId) : undefined;
			const user = local ?? lookup?.user ?? null;
			return {
				id: principalId,
				name: user ? lookupName(user, unknown) : unknown,
				isMe,
				isBot: Boolean(user?.bot) || local === null,
				ownerId: user?.bot?.owner_id,
				ttsModel: user?.bot?.tts_model,
				isPending: local === null || (lookup?.pending ?? false),
				isResolved: Boolean(user),
			};
		},
		[botLabels, botsById, byId, lookupName, me, resolveLocal, unknown, you],
	);
}

/** The first word of a person's name, for "Anna's bot". */
export function firstName(name: string): string {
	return name.trim().split(/\s+/)[0] ?? name;
}
