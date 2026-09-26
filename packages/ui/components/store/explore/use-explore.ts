"use client";

import { useTranslation } from "@flow-like/locales";
import { type UseQueryResult, useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useAuth } from "react-oidc-context";
import { create } from "zustand";
import { useDeveloperMode } from "../../../hooks/use-developer-mode";
import { getApiOrigin } from "../../../lib/api-url";
import { isRecord } from "../../../lib/response-shape";
import { useAuthStatusStore, useBackend } from "../../../state/backend-state";
import { isExploreUnsupportedError } from "./explore-model";
import type { ResolvedExplore } from "./explore-types";

/** How long Explore waits for the host's auth state before it asks the hub as a signed-out viewer. */
export const EXPLORE_AUTH_STATE_GRACE_MS = 3_000;

/** App-wide, so an Explore page opened after the wait ran out does not wait again. */
const useAuthStateWait = create<{ stalled: boolean }>(() => ({
	stalled: false,
}));

/**
 * The auth state Explore asks the hub with. Hosts push `signedIn: false` while OIDC is still loading, so a push only
 * counts once loading ends. A host that never pushes (the desktop on a hub without OpenID, or offline with no cached
 * config) leaves it unknown; once the grace period has run out (`stalled`) the viewer counts as signed out.
 */
export function exploreViewerSignedIn({
	signedIn,
	authLoading,
	stalled,
}: {
	signedIn: boolean | undefined;
	authLoading: boolean;
	stalled: boolean;
}): boolean | undefined {
	if (!authLoading && signedIn !== undefined) return signedIn;
	return stalled ? (signedIn ?? false) : undefined;
}

/** Not useSignedIn(): that reads true while the host has not pushed its auth state yet. */
function useViewerSignedIn(): boolean | undefined {
	const pushed = useAuthStatusStore((state) => state.signedIn);
	const authLoading = useAuth()?.isLoading === true;
	const stalled = useAuthStateWait((state) => state.stalled);
	const current = exploreViewerSignedIn({
		signedIn: pushed,
		authLoading,
		stalled,
	});
	// Keeps the last known state while OIDC reloads (silent sign-in, popup), so the page keeps its query instead of blanking.
	const [held, setHeld] = useState(current);
	if (current !== undefined && current !== held) setHeld(current);
	const settled = !authLoading && pushed !== undefined;
	const waiting = current === undefined && held === undefined;

	useEffect(() => {
		if (settled && stalled) {
			useAuthStateWait.setState({ stalled: false });
			return;
		}
		if (!waiting) return;
		const timer = setTimeout(
			() => useAuthStateWait.setState({ stalled: true }),
			EXPLORE_AUTH_STATE_GRACE_MS,
		);
		return () => clearTimeout(timer);
	}, [settled, stalled, waiting]);

	return current ?? held;
}

/** Identity shared by every Explore query: the hub, the profile, the locale, the dev toggle and the auth state. */
export function useExploreViewer() {
	const backend = useBackend();
	const { i18n } = useTranslation();
	const { developerMode } = useDeveloperMode();
	const signedIn = useViewerSignedIn();
	return {
		backend,
		apiOrigin: getApiOrigin(backend.profile),
		profileId: backend.profile?.id ?? "",
		language: i18n.resolvedLanguage ?? i18n.language ?? "en",
		developerMode,
		signedIn,
	};
}

/** One retry for transient failures; a client error (other than 408/429) answers the same way again. */
export function exploreRetry(failureCount: number, error: unknown): boolean {
	if (isExploreUnsupportedError(error)) return false;
	const status = isRecord(error) ? error.status : undefined;
	if (
		typeof status === "number" &&
		status >= 400 &&
		status < 500 &&
		status !== 408 &&
		status !== 429
	) {
		return false;
	}
	return failureCount < 1;
}

export function useExplore(): UseQueryResult<ResolvedExplore, Error> {
	const { backend, apiOrigin, profileId, language, developerMode, signedIn } =
		useExploreViewer();
	return useQuery<ResolvedExplore, Error>({
		queryKey: [
			"explore",
			apiOrigin,
			profileId,
			language,
			developerMode,
			signedIn,
		],
		queryFn: () =>
			backend.appState.getExplore({ language, dev: developerMode }),
		enabled: signedIn !== undefined,
		retry: exploreRetry,
	});
}

/**
 * `undefined` only while pending (including the bounded wait for the auth state), `false` only for a hub without
 * Explore, `true` on success and on every other error, so the page shows its own error, offline or sign-in state.
 */
export function exploreSupportFromQuery({
	status,
	error,
}: {
	status: "pending" | "error" | "success";
	error: unknown;
}): boolean | undefined {
	if (status === "pending") return undefined;
	if (status === "error" && isExploreUnsupportedError(error)) return false;
	return true;
}

export function useExploreSupported(): boolean | undefined {
	const { status, error } = useExplore();
	return exploreSupportFromQuery({ status, error });
}
