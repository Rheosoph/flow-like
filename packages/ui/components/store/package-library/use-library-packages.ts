"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type QueryClient,
	useInfiniteQuery,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { getErrorMessage } from "../../../lib/error-message";
import { asArray } from "../../../lib/response-shape";
import {
	useAuthStatusStore,
	useBackend,
	useBackendReady,
} from "../../../state/backend-state";
import {
	type LibraryAuthState,
	libraryCounts,
	libraryIdsToLookUp,
	mergeLibrary,
	resolveLibrarySignedIn,
} from "./library-model";

export type LibraryRegistryStatus =
	| "loading"
	| "signed-out"
	| "error"
	| "ready";
export type LibraryAction = "install" | "update" | "uninstall";

/** Structural slice of `react-oidc-context`'s `AuthContextProps`. */
export type LibraryAuth =
	| (LibraryAuthState & {
			readonly user?: {
				readonly access_token?: string;
				readonly profile?: { readonly sub?: string };
			} | null;
			readonly signinRedirect?: (args?: {
				url_state?: string;
			}) => Promise<void>;
	  })
	| null
	| undefined;

/** Divides by every grid column count (2, 3, 4), so a full page never leaves a ragged last row. */
export const LIBRARY_PAGE_SIZE = 48;
/** `MAX_SEARCH_IDS` in `packages/api/src/routes/registry/search.rs`. */
const MAX_LOOKUP_IDS = 100;
const REGISTRY_STALE_MS = 60_000;

export const libraryQueryKeys = {
	library: (user?: string) => ["registry-library", user] as const,
	lookup: (user: string | undefined, ids: readonly string[]) =>
		["registry-library-lookup", user ?? "anon", ...ids] as const,
	installed: ["installed-packages"] as const,
	updates: ["available-updates"] as const,
};

function invalidateMachine(queryClient: QueryClient, packageId?: string) {
	queryClient.invalidateQueries({ queryKey: libraryQueryKeys.installed });
	queryClient.invalidateQueries({ queryKey: libraryQueryKeys.updates });
	if (packageId) {
		queryClient.invalidateQueries({
			queryKey: ["installed-package", packageId],
		});
	}
}

function usePendingActions() {
	const [pending, setPending] = useState<ReadonlyMap<string, LibraryAction>>(
		() => new Map(),
	);
	const start = useCallback((id: string, action: LibraryAction) => {
		setPending((current) => new Map(current).set(id, action));
	}, []);
	const settle = useCallback((id: string) => {
		setPending((current) => {
			const next = new Map(current);
			next.delete(id);
			return next;
		});
	}, []);
	return { pending, start, settle };
}

export function useLibraryPackages({
	auth,
	statusOf,
}: {
	auth: LibraryAuth;
	statusOf?: (id: string) => string | undefined;
}) {
	const { t } = useTranslation("store");
	const backend = useBackend();
	const backendReady = useBackendReady();
	const queryClient = useQueryClient();
	const pushedSignedIn = useAuthStatusStore((state) => state.signedIn);
	const signedIn = resolveLibrarySignedIn(pushedSignedIn, auth);
	const sub = auth?.user?.profile?.sub;
	const { pending, start, settle } = usePendingActions();

	/** Buying or getting a package on the store detail does not touch this key; the tab remounts after it. */
	const library = useInfiniteQuery({
		queryKey: libraryQueryKeys.library(sub),
		initialPageParam: 0,
		queryFn: ({ pageParam }) =>
			backend.registryState.getOwnedPackages({
				access: "library",
				limit: LIBRARY_PAGE_SIZE,
				offset: pageParam,
			}),
		getNextPageParam: (last, pages) => {
			const loaded = pages.reduce(
				(sum, page) => sum + asArray(page?.packages).length,
				0,
			);
			return asArray(last?.packages).length > 0 &&
				loaded < (last?.totalCount ?? 0)
				? loaded
				: undefined;
		},
		enabled: backendReady && signedIn === true,
		staleTime: REGISTRY_STALE_MS,
		refetchOnMount: "always",
	});

	const installed = useQuery({
		queryKey: libraryQueryKeys.installed,
		queryFn: () => backend.registryState.getInstalledPackages(),
		enabled: backendReady,
	});

	const updates = useQuery({
		queryKey: libraryQueryKeys.updates,
		queryFn: () => backend.registryState.checkForUpdates(),
		enabled: backendReady,
	});

	const libraryPackages = useMemo(
		() =>
			signedIn === true
				? asArray(library.data?.pages).flatMap((page) =>
						asArray(page?.packages),
					)
				: [],
		[signedIn, library.data],
	);
	const libraryTotal =
		signedIn === true
			? Math.max(
					library.data?.pages[0]?.totalCount ?? 0,
					libraryPackages.length,
				)
			: 0;
	const installedPackages = useMemo(
		() => asArray(installed.data),
		[installed.data],
	);
	const librarySettled = signedIn !== true || !library.isPending;

	const lookupIds = useMemo(
		() =>
			librarySettled
				? libraryIdsToLookUp(libraryPackages, installedPackages).slice(
						0,
						MAX_LOOKUP_IDS,
					)
				: [],
		[librarySettled, libraryPackages, installedPackages],
	);
	const lookup = useQuery({
		queryKey: libraryQueryKeys.lookup(sub, lookupIds),
		queryFn: () =>
			backend.registryState.searchPackages({
				ids: lookupIds,
				includeOwn: true,
				limit: lookupIds.length,
			}),
		enabled: backendReady && signedIn !== undefined && lookupIds.length > 0,
		staleTime: REGISTRY_STALE_MS,
		retry: false,
	});

	const registryStatus: LibraryRegistryStatus =
		signedIn === undefined
			? "loading"
			: signedIn === false
				? "signed-out"
				: library.isError && !library.data
					? "error"
					: library.isPending
						? "loading"
						: "ready";
	const summariesPending =
		signedIn === undefined ||
		!librarySettled ||
		(lookupIds.length > 0 && lookup.isPending);

	const entries = useMemo(
		() =>
			mergeLibrary({
				library: libraryPackages,
				installed: installedPackages,
				updates: asArray(updates.data),
				lookups: asArray(lookup.data?.packages),
				statusOf,
			}),
		[libraryPackages, installedPackages, updates.data, lookup.data, statusOf],
	);
	const counts = useMemo(() => libraryCounts(entries), [entries]);
	const names = useMemo(
		() => new Map(entries.map((entry) => [entry.id, entry.name])),
		[entries],
	);
	const nameOf = (id: string) => names.get(id) ?? id;

	const install = useMutation({
		mutationFn: ({ id, version }: { id: string; version?: string }) =>
			backend.registryState.installPackage(id, version),
		onMutate: ({ id }) => start(id, "install"),
		onSuccess: (_, { id }) => {
			toast.success(
				t("libraryPackageInstalled", "{{name}} installed", {
					name: nameOf(id),
				}),
			);
			invalidateMachine(queryClient, id);
		},
		onError: (error, { id }) =>
			toast.error(
				t(
					"libraryPackageInstallFailed",
					"Couldn't install {{name}}: {{message}}",
					{
						name: nameOf(id),
						message: getErrorMessage(error),
					},
				),
			),
		onSettled: (_, __, { id }) => settle(id),
	});

	const update = useMutation({
		mutationFn: ({ id, version }: { id: string; version: string }) =>
			backend.registryState.updatePackage(id, version),
		onMutate: ({ id }) => start(id, "update"),
		onSuccess: (_, { id, version }) => {
			toast.success(
				t("libraryPackageUpdated", "{{name}} updated to v{{version}}", {
					name: nameOf(id),
					version,
				}),
			);
			invalidateMachine(queryClient, id);
		},
		onError: (error, { id }) =>
			toast.error(
				t(
					"libraryPackageUpdateFailed",
					"Couldn't update {{name}}: {{message}}",
					{
						name: nameOf(id),
						message: getErrorMessage(error),
					},
				),
			),
		onSettled: (_, __, { id }) => settle(id),
	});

	const uninstall = useMutation({
		mutationFn: (id: string) => backend.registryState.uninstallPackage(id),
		onMutate: (id) => start(id, "uninstall"),
		onSuccess: (_, id) => {
			toast.success(
				t("libraryPackageUninstalled", "{{name}} uninstalled", {
					name: nameOf(id),
				}),
			);
			invalidateMachine(queryClient, id);
			queryClient.invalidateQueries({ queryKey: ["registry-library"] });
		},
		onError: (error, id) =>
			toast.error(
				t(
					"libraryPackageUninstallFailed",
					"Couldn't uninstall {{name}}: {{message}}",
					{ name: nameOf(id), message: getErrorMessage(error) },
				),
			),
		onSettled: (_, __, id) => settle(id),
	});

	const refresh = useCallback(() => {
		invalidateMachine(queryClient);
		queryClient.invalidateQueries({ queryKey: ["registry-library"] });
		queryClient.invalidateQueries({ queryKey: ["registry-library-lookup"] });
	}, [queryClient]);

	const loadMore = useCallback(() => {
		if (library.hasNextPage && !library.isFetchingNextPage) {
			void library.fetchNextPage();
		}
	}, [library.hasNextPage, library.isFetchingNextPage, library.fetchNextPage]);

	return {
		entries,
		counts,
		registryStatus,
		summariesPending,
		/** Library rows the registry has, loaded or not; entries also hold installed packages outside it. */
		libraryTotal,
		libraryLoaded: libraryPackages.length,
		hasMore: signedIn === true && !!library.hasNextPage,
		loadingMore: library.isFetchingNextPage,
		loadMoreFailed: library.isFetchNextPageError,
		loadMore,
		isLoading: installed.isPending,
		installedError: installed.error,
		retryInstalled: installed.refetch,
		retryRegistry: library.refetch,
		refresh,
		isRefreshing:
			library.isFetching ||
			installed.isFetching ||
			updates.isFetching ||
			lookup.isFetching,
		pending,
		install: install.mutate,
		update: update.mutate,
		uninstall: uninstall.mutate,
	};
}

export type LibraryPackagesState = ReturnType<typeof useLibraryPackages>;
