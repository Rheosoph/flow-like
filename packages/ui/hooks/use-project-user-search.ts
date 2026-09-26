"use client";

import {
	type InfiniteData,
	useInfiniteQuery,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { getApiOrigin } from "../lib/api-url";
import {
	createProjectUserSearch,
	mergeProjectUserResults,
} from "../lib/project-user-search";
import { asArray } from "../lib/response-shape";
import { useBackend } from "../state/backend-state";
import type {
	IProjectContactsPage,
	IUserLookup,
} from "../state/backend-state/types";

const CACHE_TIME = 5 * 60 * 1000;

export type ProjectUserSearch = ReturnType<typeof useProjectUserSearch>;

/**
 * Without an `appId` there is no project to draw colleagues from or to exclude
 * members of, so only the directory is searched.
 */
export function useProjectUserSearch(
	appId: string | undefined,
	query: string,
	open: boolean,
) {
	const backend = useBackend();
	const auth = useAuth();
	const queryClient = useQueryClient();
	const source = backend.userState;
	const scope = [
		getApiOrigin(backend.profile),
		backend.profile?.id ?? "",
		auth.user?.profile.sub ?? "local",
		appId ?? "",
	];
	const contactsKey = ["projectInviteContacts", ...scope];
	const directoryKey = ["projectInviteDirectory", ...scope];
	const trimmed = query.trim().replace(/\s+/g, " ");
	const [debounced, setDebounced] = useState("");
	useEffect(() => {
		if (!open) {
			setDebounced("");
			return;
		}
		const timer = setTimeout(() => setDebounced(trimmed), 250);
		return () => clearTimeout(timer);
	}, [trimmed, open]);

	const contacts = useInfiniteQuery({
		queryKey: contactsKey,
		queryFn: ({ pageParam }) =>
			source.getProjectContacts(appId as string, pageParam),
		initialPageParam: undefined as string | undefined,
		getNextPageParam: (page) => page?.next_cursor ?? undefined,
		enabled: open && !!appId,
		staleTime: CACHE_TIME,
		gcTime: CACHE_TIME,
		retry: 1,
	});
	const { hasNextPage, isFetching, isError, fetchNextPage } = contacts;
	useEffect(() => {
		if (open && hasNextPage && !isFetching && !isError) {
			void fetchNextPage();
		}
	}, [open, hasNextPage, isFetching, isError, fetchNextPage]);

	// Rebuild only when a contact page changes, never for each keystroke.
	const index = useMemo(
		() =>
			createProjectUserSearch(
				asArray(contacts.data?.pages).flatMap((p) => asArray(p?.users)),
			),
		[contacts.data],
	);
	const localResults = useMemo(() => index.search(trimmed), [index, trimmed]);
	const canSearchDirectory =
		[...trimmed].length >= 2 && [...trimmed].length <= 200;
	const isDebouncing = trimmed !== debounced;
	const directory = useQuery({
		queryKey: [...directoryKey, debounced],
		queryFn: () => source.searchUsers(debounced, appId),
		enabled: open && canSearchDirectory && !isDebouncing,
		staleTime: CACHE_TIME,
		gcTime: CACHE_TIME,
		retry: 1,
	});
	// A response for earlier input must never leave an actionable stale result.
	const remoteResults =
		canSearchDirectory && !isDebouncing ? directory.data : undefined;
	const results = useMemo(
		() =>
			mergeProjectUserResults(localResults, asArray(remoteResults), trimmed),
		[localResults, remoteResults, trimmed],
	);

	return {
		results,
		canSearchDirectory,
		isSearchingDirectory:
			open && canSearchDirectory && (isDebouncing || directory.isFetching),
		directoryError:
			canSearchDirectory && !isDebouncing && directory.isError
				? directory.error
				: null,
		contactsError: contacts.isError ? contacts.error : null,
		isLoadingContacts:
			open &&
			(contacts.isFetching || (contacts.hasNextPage && !contacts.isError)),
		retryDirectory: () => directory.refetch(),
		retryContacts: () =>
			contacts.isFetchNextPageError
				? contacts.fetchNextPage()
				: contacts.refetch(),
		invalidate: (invitedId: string) => {
			queryClient.setQueryData<InfiniteData<IProjectContactsPage>>(
				contactsKey,
				(data) =>
					data && {
						...data,
						pages: asArray(data.pages).map((page) => ({
							...page,
							users: asArray(page?.users).filter(
								(user) => user.id !== invitedId,
							),
						})),
					},
			);
			queryClient.setQueriesData<IUserLookup[]>(
				{ queryKey: directoryKey },
				(data) => data && asArray(data).filter((user) => user.id !== invitedId),
			);
			return Promise.all([
				queryClient.invalidateQueries({ queryKey: contactsKey }),
				queryClient.invalidateQueries({ queryKey: directoryKey }),
			]);
		},
	};
}
