"use client";

import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";
import { userLookupQueryOptions } from "../../../../hooks/use-user-lookup";
import { userDisplayName } from "../../../../lib/user-display";
import { useBackend } from "../../../../state/backend-state";

export type PersonNames = (userId: string) => string | undefined;

/**
 * Display names of accounts from the people directory, in one batched lookup
 * that stays silent when it fails. A name that isn't known (still loading, no
 * answer, an account without a name) is undefined: the caller says "One
 * person" or "the owner", never the account id (R3).
 */
export function usePersonNames(userIds: readonly string[]): PersonNames {
	const backend = useBackend();
	const key = JSON.stringify([...new Set(userIds)].sort());
	const ids = useMemo(() => JSON.parse(key) as string[], [key]);
	const results = useQueries({
		queries: ids.map((id) => ({
			...userLookupQueryOptions(backend.userState, id),
			retry: false,
		})),
	});
	const names = results.map((result) =>
		result.data ? userDisplayName(result.data, "") : "",
	);
	const signature = names.join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: `signature` stands for the looked-up names
	return useMemo(() => {
		const byId = new Map(ids.map((id, index) => [id, names[index]]));
		return (userId: string) => byId.get(userId) || undefined;
	}, [ids, signature]);
}
