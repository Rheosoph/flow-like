"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useBackend } from "../state/backend-state";
import type { IInboxState } from "../state/backend-state/inbox-state";
import { INBOX_POLL_MS, inboxKeys } from "./use-inbox";

export const inboxBlockKeys = {
	all: [...inboxKeys.all, "blocks"] as const,
};

function requireInboxState(state: IInboxState | undefined): IInboxState {
	if (!state) throw new Error("Sign in to manage blocked people and bots.");
	return state;
}

export function useInboxBlocks(enabled = true) {
	const state = useBackend().inboxState;
	return useQuery({
		queryKey: inboxBlockKeys.all,
		queryFn: () => requireInboxState(state).listBlocks(),
		enabled: enabled && Boolean(state?.listBlocks),
		refetchInterval: INBOX_POLL_MS.conversations,
		refetchIntervalInBackground: false,
	});
}

export function useBlockPrincipal() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: (principalId: string) =>
			requireInboxState(state).blockPrincipal(principalId),
		onSuccess: () => queryClient.invalidateQueries({ queryKey: inboxKeys.all }),
	});
}

export function useUnblockPrincipal() {
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: (principalId: string) =>
			requireInboxState(state).unblockPrincipal(principalId),
		onSuccess: () => queryClient.invalidateQueries({ queryKey: inboxKeys.all }),
	});
}
