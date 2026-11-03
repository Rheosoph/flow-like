"use client";

import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";
import {
	useLookupDisplayName,
	useUserIdentity,
	userLookupQueryOptions,
} from "../../../hooks/use-user-lookup";
import { useBackend } from "../../../state/backend-state";
import type {
	GraphOverlay,
	SubgraphNode,
} from "../../../state/backend-state/graph-state";
import { resolveAccountId } from "../../../state/backend-state/user-state";
import { UserInlineTag } from "../user-identity";
import { nodeCaptionAccountId } from "./graph-user-caption";

export function GraphNodeCaption({
	node,
	overlay,
	className,
	fallback,
}: {
	node: SubgraphNode;
	overlay?: GraphOverlay;
	className?: string;
	fallback?: string;
}) {
	const accountId = nodeCaptionAccountId(node, overlay, fallback !== undefined);
	return accountId ? (
		<UserInlineTag userId={accountId} className={className} />
	) : (
		<span className={className}>{fallback ?? node.caption ?? node.id}</span>
	);
}

/** A plain label for translated sentences that cannot contain an account tag. */
export function useGraphNodeCaption(
	node: SubgraphNode | null,
	overlay?: GraphOverlay,
): string {
	const accountId = nodeCaptionAccountId(node ?? undefined, overlay, false);
	const identity = useUserIdentity(accountId);
	return identity.isResolved
		? identity.label
		: (node?.caption ?? node?.id ?? "");
}

/** Canvas labels share the lookup cache and batch window used by account tags. */
export function useGraphAccountLabels(
	nodes: readonly SubgraphNode[] | undefined,
	overlay: GraphOverlay,
): ReadonlyMap<string, string> {
	const { userState } = useBackend();
	const displayName = useLookupDisplayName();
	const accounts = useMemo(
		() =>
			(nodes ?? []).flatMap((node) => {
				const accountId = nodeCaptionAccountId(node, overlay, false);
				return accountId ? [{ nodeId: node.id, accountId }] : [];
			}),
		[nodes, overlay],
	);
	const ids = useMemo(
		() => [...new Set(accounts.map(({ accountId }) => accountId))],
		[accounts],
	);
	const resolvedKey = useQueries({
		queries: ids.map((id) => userLookupQueryOptions(userState, id)),
		combine: (results) =>
			JSON.stringify(
				results.map((result, index) =>
					result.data
						? displayName(
								result.data,
								resolveAccountId(result.data.id, ids[index]) ?? ids[index],
							)
						: null,
				),
			),
	});
	return useMemo(() => {
		const labels = JSON.parse(resolvedKey) as (string | null)[];
		const byAccount = new Map(ids.map((id, index) => [id, labels[index]]));
		const result = new Map<string, string>();
		for (const { nodeId, accountId } of accounts) {
			const label = byAccount.get(accountId);
			if (label) result.set(nodeId, label);
		}
		return result;
	}, [accounts, ids, resolvedKey]);
}
