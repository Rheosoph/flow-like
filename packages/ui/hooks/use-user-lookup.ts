"use client";

import { useTranslation } from "@flow-like/locales";
import { queryOptions, useQuery } from "@tanstack/react-query";
import { createContext, useCallback, useContext, useMemo } from "react";
import type { BotAppearance } from "../components/bots/types";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
	userSecondaryLabel,
} from "../lib/user-display";
import { useBackend } from "../state/backend-state";
import type { BotSystemKey } from "../state/backend-state/bot-state";
import type { IUserLookup } from "../state/backend-state/types";
import type { IUserState } from "../state/backend-state/user-state";
import {
	LOCAL_USER_SUB,
	isLocalUserSub,
	resolveAccountId,
} from "../state/backend-state/user-state";
import { useBotLabels } from "./use-bots";

/** A resolved account, or `null` when the directory has no such account. */
export type UserLookupResult = IUserLookup | null;

/** The slice of the backend a batcher needs, so tests can stand one up. */
type LookupSource = Pick<IUserState, "lookupUser" | "lookupUsers">;

/**
 * Long enough for one render pass of a table to queue every id it shows, short
 * enough that a lone hover still resolves without a visible wait.
 */
const BATCH_WINDOW_MS = 20;

interface Waiter {
	resolve: (user: UserLookupResult) => void;
	reject: (error: unknown) => void;
}

/**
 * Collapses the lookups a screenful of cells asks for into one request.
 *
 * A table of 50 rows resolves 50 subs; issued one by one that is 50 round trips
 * the hub answers in a single batch, so ids queued within the same window are
 * sent together and handed back per id.
 */
class UserLookupBatcher {
	private pending = new Map<string, Waiter[]>();
	private timer: ReturnType<typeof setTimeout> | null = null;

	constructor(private readonly source: LookupSource) {}

	load(userId: string): Promise<UserLookupResult> {
		return new Promise<UserLookupResult>((resolve, reject) => {
			const waiters = this.pending.get(userId);
			if (waiters) waiters.push({ resolve, reject });
			else this.pending.set(userId, [{ resolve, reject }]);

			this.timer ??= setTimeout(() => void this.flush(), BATCH_WINDOW_MS);
		});
	}

	private async flush(): Promise<void> {
		this.timer = null;
		const batch = this.pending;
		this.pending = new Map();
		if (batch.size === 0) return;

		// The local placeholder resolves to whoever is signed in rather than to a
		// stored row, so it never goes into the batch the directory answers.
		const subs = [...batch.keys()].filter((id) => !isLocalUserSub(id));

		await Promise.all([
			this.resolveBatch(batch, subs),
			...[...batch.keys()]
				.filter(isLocalUserSub)
				.map((id) => this.resolveSingle(batch, id)),
		]);
	}

	private async resolveBatch(
		batch: Map<string, Waiter[]>,
		subs: string[],
	): Promise<void> {
		if (subs.length === 0) return;

		try {
			const users = await this.source.lookupUsers(subs);
			const bySub = new Map(users.map((user) => [user.id, user]));
			for (const sub of subs) settle(batch, sub, bySub.get(sub) ?? null);
		} catch (error) {
			for (const sub of subs) fail(batch, sub, error);
		}
	}

	private async resolveSingle(
		batch: Map<string, Waiter[]>,
		userId: string,
	): Promise<void> {
		try {
			settle(batch, userId, await this.source.lookupUser(userId));
		} catch (error) {
			fail(batch, userId, error);
		}
	}
}

function settle(
	batch: Map<string, Waiter[]>,
	userId: string,
	user: UserLookupResult,
): void {
	for (const waiter of batch.get(userId) ?? []) waiter.resolve(user);
}

function fail(
	batch: Map<string, Waiter[]>,
	userId: string,
	error: unknown,
): void {
	for (const waiter of batch.get(userId) ?? []) waiter.reject(error);
}

const batchers = new WeakMap<LookupSource, UserLookupBatcher>();

function batcherFor(source: LookupSource): UserLookupBatcher {
	const existing = batchers.get(source);
	if (existing) return existing;
	const created = new UserLookupBatcher(source);
	batchers.set(source, created);
	return created;
}

/** Accounts change rarely; a table paging back and forth should not re-ask. */
const USER_LOOKUP_STALE_TIME = 5 * 60 * 1000;

/** Shared by individual account tags and views that resolve several labels at once. */
export function userLookupQueryOptions(
	source: LookupSource,
	userId?: string | null,
) {
	return queryOptions<UserLookupResult, Error>({
		queryKey: ["lookupUserBatched", userId ?? null],
		queryFn: () => batcherFor(source).load(userId as string),
		enabled: Boolean(userId),
		staleTime: USER_LOOKUP_STALE_TIME,
	});
}

/**
 * Answers for principals that live only on this device (local bots), which the
 * directory has never heard of: a record to show, `null` while it isn't known
 * yet, or `undefined` to ask the directory as usual.
 */
export type LocalPrincipalResolver = (
	principalId: string,
) => IUserLookup | null | undefined;

const LocalPrincipalsContext = createContext<LocalPrincipalResolver | null>(
	null,
);

/** Views that show local bots provide their resolver, so their ids never reach the API. */
export const LocalPrincipalsProvider = LocalPrincipalsContext.Provider;

export function useLocalPrincipalResolver(): LocalPrincipalResolver | null {
	return useContext(LocalPrincipalsContext);
}

/**
 * Resolves one account, coalescing concurrent callers into a single request.
 *
 * Unlike `lookupUser`, an id nobody matches resolves to `null` instead of
 * throwing — for a caller rendering stored data, "no such account" is an answer,
 * not a failure.
 */
export function useUserLookup(userId?: string | null) {
	const backend = useBackend();
	const source = backend.userState;
	const resolveLocal = useLocalPrincipalResolver();
	const local = userId ? resolveLocal?.(userId) : undefined;

	return useQuery({
		...userLookupQueryOptions(source, userId),
		enabled: Boolean(userId) && local === undefined,
		placeholderData: local ?? undefined,
	});
}

/** The person a bot acts for, as far as the directory has answered. */
export interface BotOwnerIdentity {
	label: string;
	/** Handle or email, when it says something the label does not. */
	subtitle?: string;
	/** The owner's account to link to; absent until the directory knows it. */
	accountId?: string;
	avatarUrl?: string;
	initials: string;
	isPending: boolean;
}

/** A bot's sub, resolved. A bot always acts for its owner. */
export interface BotIdentity {
	ownerId: string;
	system: BotSystemKey | null;
	appearance: BotAppearance;
	/** Deleted bots keep their name, so past actions still read as theirs. */
	deleted: boolean;
	/** The bot belongs to whoever is signed in. */
	isOwn: boolean;
	description?: string;
	owner: BotOwnerIdentity;
}

export interface UserIdentity {
	/** The resolved record, or `null` once we know there is no account. */
	user: UserLookupResult;
	/** The account id to link to, which is never the local placeholder. */
	accountId?: string;
	/** Best human-readable name, falling back to the stored id. */
	label: string;
	/** Handle or email, when it says something the label does not; whose bot it is for a bot. */
	subtitle?: string;
	avatarUrl?: string;
	initials: string;
	/** Set when the id is a bot's sub. */
	bot?: BotIdentity;
	isPending: boolean;
	/** Whether the directory answered with an account. */
	isResolved: boolean;
	/**
	 * Whether the directory could not be asked at all. Distinct from `isResolved`
	 * on purpose: "there is no such account" and "we could not find out" look the
	 * same in a cell, and only one of them is a fact.
	 */
	isError: boolean;
}

/**
 * One place that turns a stored account id into everything a surface needs to
 * render it, so a cell, a pill and a hover card describe the same person the
 * same way.
 */
export function useUserIdentity(userId?: string | null): UserIdentity {
	const lookup = useUserLookup(userId);
	const user = lookup.data ?? null;
	const accountId = resolveAccountId(user?.id, userId);
	const bot = useBotIdentity(user);
	const label = bot?.label || userDisplayName(user, accountId ?? userId ?? "");
	const initials = useMemo(() => userInitials(label, "??"), [label]);

	return {
		user,
		accountId,
		label,
		subtitle: bot ? bot.subtitle : userSecondaryLabel(user),
		avatarUrl: bot ? undefined : userAvatarUrl(user),
		initials,
		bot: bot?.identity,
		isPending: lookup.isPending && Boolean(userId),
		isResolved: Boolean(user),
		isError: lookup.isError,
	};
}

/**
 * Names a bot and whose it is. The owner is a second lookup and the signed-in
 * account (the local placeholder) a third; the subtitle waits for both, so a
 * reader's own bot never flashes up as someone else's.
 */
function useBotIdentity(
	user: UserLookupResult,
): { label: string; subtitle?: string; identity: BotIdentity } | undefined {
	const { t } = useTranslation("bots");
	const botLabels = useBotLabels();
	const bot = user?.bot;
	const owner = useUserLookup(bot?.owner_id);
	const self = useUserLookup(bot ? LOCAL_USER_SUB : undefined);
	if (!user || !bot) return undefined;

	const names = botLabels({
		system: bot.system,
		name: user.name,
		description: user.description,
	});
	const ownerAccount = owner.data ?? null;
	const ownerLabel = userDisplayName(
		ownerAccount,
		resolveAccountId(ownerAccount?.id, bot.owner_id) ?? bot.owner_id,
	);
	// A local bot's owner is `local`: whoever uses this device.
	const isOwn =
		isLocalUserSub(bot.owner_id) ||
		(Boolean(bot.owner_id) && self.data?.id === bot.owner_id);

	let subtitle: string | undefined;
	if (bot.deleted) subtitle = t("identity.deleted");
	else if (isOwn) subtitle = t("identity.yourBot");
	else if (!self.isPending && !owner.isPending)
		subtitle = ownerAccount
			? t("identity.botOf", { owner: ownerLabel })
			: t("identity.bot");

	return {
		label: names.name,
		subtitle,
		identity: {
			ownerId: bot.owner_id,
			system: bot.system,
			appearance: bot.appearance,
			deleted: bot.deleted,
			isOwn,
			description: names.description || undefined,
			owner: {
				label: ownerLabel,
				subtitle: userSecondaryLabel(ownerAccount),
				accountId: ownerAccount
					? resolveAccountId(ownerAccount.id, bot.owner_id)
					: undefined,
				avatarUrl: userAvatarUrl(ownerAccount),
				initials: userInitials(ownerLabel, "??"),
				isPending: owner.isPending,
			},
		},
	};
}

/**
 * Names a looked-up account the way `useUserIdentity` does, for views that
 * resolve many ids at once: a built-in bot nobody renamed reads in the reader's
 * language instead of as its id.
 */
export function useLookupDisplayName(): (
	user: IUserLookup,
	fallback: string,
) => string {
	const botLabels = useBotLabels();
	return useCallback(
		(user, fallback) =>
			(user.bot &&
				botLabels({ system: user.bot.system, name: user.name }).name) ||
			userDisplayName(user, fallback),
		[botLabels],
	);
}

/** Exposed for tests: the coalescing layer has no React surface of its own. */
export const __testing = { UserLookupBatcher, BATCH_WINDOW_MS };
