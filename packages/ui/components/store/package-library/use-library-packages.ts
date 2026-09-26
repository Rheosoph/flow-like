"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type QueryClient,
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

const LIBRARY_LIMIT = 100;
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
	const library = useQuery({
		queryKey: libraryQueryKeys.library(sub),
		queryFn: () =>
			backend.registryState.getOwnedPackages({
				access: "library",
				limit: LIBRARY_LIMIT,
			}),
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
		() => (signedIn === true ? asArray(library.data?.packages) : []),
		[signedIn, library.data],
	);
	const installedPackages = useMemo(
		() => asArray(installed.data),
		[installed.data],
	);
	const librarySettled = signedIn !== true || !library.isPending;

	const lookupIds = useMemo(
		() =>
			librarySettled
				? libraryIdsToLookUp(libraryPackages, installedPackages)
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
				: library.isError
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

	const install = useMutation({
		mutationFn: ({ id, version }: { id: string; version?: string }) =>
			backend.registryState.installPackage(id, version),
		onMutate: ({ id }) => start(id, "install"),
		onSuccess: (_, { id }) => {
			toast.success(
				t("packageInstalledSuccessfully", "Package installed successfully"),
			);
			invalidateMachine(queryClient, id);
		},
		onError: (error) =>
			toast.error(
				t(
					"failedToInstallPackageMessage",
					"Failed to install package: {{message}}",
					{ message: getErrorMessage(error) },
				),
			),
		onSettled: (_, __, { id }) => settle(id),
	});

	const update = useMutation({
		mutationFn: ({ id, version }: { id: string; version: string }) =>
			backend.registryState.updatePackage(id, version),
		onMutate: ({ id }) => start(id, "update"),
		onSuccess: (_, { id }) => {
			toast.success(
				t("countPackagesUpdated", {
					defaultValue_one: "Package updated",
					defaultValue_other: "{{count}} packages updated",
					count: 1,
				}),
			);
			invalidateMachine(queryClient, id);
		},
		onError: (error) =>
			toast.error(
				t(
					"failedToUpdatePackageMessage",
					"Failed to update package: {{message}}",
					{ message: getErrorMessage(error) },
				),
			),
		onSettled: (_, __, { id }) => settle(id),
	});

	const uninstall = useMutation({
		mutationFn: (id: string) => backend.registryState.uninstallPackage(id),
		onMutate: (id) => start(id, "uninstall"),
		onSuccess: (_, id) => {
			toast.success(
				t("packageUninstalledSuccessfully", "Package uninstalled successfully"),
			);
			invalidateMachine(queryClient, id);
			queryClient.invalidateQueries({ queryKey: ["registry-library"] });
		},
		onError: (error) =>
			toast.error(
				t(
					"failedToUninstallPackageMessage",
					"Failed to uninstall package: {{message}}",
					{ message: getErrorMessage(error) },
				),
			),
		onSettled: (_, __, id) => settle(id),
	});

	const refresh = useCallback(() => {
		invalidateMachine(queryClient);
		queryClient.invalidateQueries({ queryKey: ["registry-library"] });
		queryClient.invalidateQueries({ queryKey: ["registry-library-lookup"] });
	}, [queryClient]);

	return {
		entries,
		counts,
		registryStatus,
		summariesPending,
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
