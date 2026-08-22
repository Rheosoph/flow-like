"use client";

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
	userSecondaryLabel,
} from "../lib/user-display";
import { useBackend } from "../state/backend-state";
import type { IUserLookup } from "../state/backend-state/types";
import type { IUserState } from "../state/backend-state/user-state";
import {
	isLocalUserSub,
	resolveAccountId,
} from "../state/backend-state/user-state";

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

	return useQuery<UserLookupResult, Error>({
		queryKey: ["lookupUserBatched", userId ?? null],
		queryFn: () => batcherFor(source).load(userId as string),
		enabled: Boolean(userId),
		staleTime: USER_LOOKUP_STALE_TIME,
	});
}

export interface UserIdentity {
	/** The resolved record, or `null` once we know there is no account. */
	user: UserLookupResult;
	/** The account id to link to, which is never the local placeholder. */
	accountId?: string;
	/** Best human-readable name, falling back to the stored id. */
	label: string;
	/** Handle or email, when it says something the label does not. */
	subtitle?: string;
	avatarUrl?: string;
	initials: string;
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
	const label = userDisplayName(user, accountId ?? userId ?? "");
	const initials = useMemo(() => userInitials(label, "??"), [label]);

	return {
		user,
		accountId,
		label,
		subtitle: userSecondaryLabel(user),
		avatarUrl: userAvatarUrl(user),
		initials,
		isPending: lookup.isPending && Boolean(userId),
		isResolved: Boolean(user),
		isError: lookup.isError,
	};
}

/** Exposed for tests: the coalescing layer has no React surface of its own. */
export const __testing = { UserLookupBatcher, BATCH_WINDOW_MS };
