"use client";

import { type UseQueryResult, useQuery } from "@tanstack/react-query";
import type {
	ExploreSearchQuery,
	ExploreSearchResponse,
} from "./explore-types";
import { exploreRetry, useExploreViewer } from "./use-explore";

export type ExploreSearchParams = Omit<ExploreSearchQuery, "language" | "dev">;

/**
 * Browse results. Previous results stay on screen while only the search params change; a different hub, profile,
 * locale, dev toggle or auth state never shows the other identity's results.
 */
export function useExploreSearch(
	params: ExploreSearchParams,
): UseQueryResult<ExploreSearchResponse, Error> {
	const { backend, apiOrigin, profileId, language, developerMode, signedIn } =
		useExploreViewer();
	const viewerKey = [
		"explore-search",
		apiOrigin,
		profileId,
		signedIn,
		language,
		developerMode,
	] as const;
	return useQuery<ExploreSearchResponse, Error>({
		queryKey: [...viewerKey, params],
		queryFn: () =>
			backend.appState.searchExplore({
				...params,
				language,
				dev: developerMode,
			}),
		enabled: signedIn !== undefined,
		retry: exploreRetry,
		placeholderData: (previous, previousQuery) =>
			viewerKey.every((part, index) => previousQuery?.queryKey[index] === part)
				? previous
				: undefined,
	});
}
