"use client";

import { useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo } from "react";
import { isUnconfirmedRestore } from "../../../lib/query-persister";
import { isRecord } from "../../../lib/response-shape";
import {
	type InstalledPackage,
	PackageStatus,
	type RegistryEntry,
} from "../../../lib/schema/wasm";
import { useAuthStatusStore, useBackend } from "../../../state/backend-state";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import { useHubProfile } from "./use-workspace-data";
import {
	type WorkspaceAuthState,
	type WorkspaceRemoteSource,
	type WorkspaceRemoteStatus,
	remoteStatusOf,
	workspaceAuthState,
} from "./workspace-model";

export type RegistryPackageAuth =
	| {
			readonly isLoading?: boolean;
			readonly activeNavigator?: string;
			readonly user?: {
				readonly access_token?: string;
				readonly expired?: boolean;
				readonly profile?: { readonly sub?: string };
			} | null;
			readonly signinRedirect?: (args?: {
				url_state?: string;
			}) => Promise<unknown>;
	  }
	| null
	| undefined;

/** Every caller-dependent entry carries the user; `["registry-package", id]` still invalidates them all. */
export function registryPackageKey(id: string, sub?: string | null) {
	return ["registry-package", id, sub ?? "anon"] as const;
}

export interface RegistryPackageResult {
	entry: RegistryEntry | undefined;
	source: WorkspaceRemoteSource | undefined;
	/** `loading` while the entry is a restored copy the server has not confirmed yet. */
	status: WorkspaceRemoteStatus;
	authState: WorkspaceAuthState;
	isLoading: boolean;
	isFetching: boolean;
	error: unknown;
	retry: () => void;
}

/** The detail view dereferences `manifest` and `versions` unguarded. */
function usableEntry(data: unknown): RegistryEntry | undefined {
	return isRecord(data) &&
		isRecord(data.manifest) &&
		Array.isArray(data.versions)
		? (data as unknown as RegistryEntry)
		: undefined;
}

function entryFromInstalled(local: InstalledPackage): RegistryEntry {
	return {
		id: local.id,
		manifest: local.manifest,
		nodes: [],
		versions: [
			{
				version: local.version,
				wasmHash: "",
				wasmSize: 0,
				publishedAt: local.installedAt,
				yanked: false,
			},
		],
		status: PackageStatus.Active,
		downloadCount: 0,
		createdAt: local.installedAt,
		updatedAt: local.installedAt,
		source: local.source,
		verified: false,
		price: 0,
		visibility: "local",
	};
}

/**
 * The registry entry as the signed-in caller sees it, falling back to the
 * installed copy when the registry has none. The entry is cached under the
 * stored user even while their session is unconfirmed or expired, so a renewal
 * keeps the last confirmed answer; only a confirmed session (or a signed-out
 * caller, anonymously) is ever sent, and an entry restored from a previous
 * session reports `loading` until the server answers again.
 */
export function useRegistryPackage(
	packageId: string | null | undefined,
	fetcher: GenericFetcher,
	auth?: RegistryPackageAuth,
	{ localFallback = true }: { localFallback?: boolean } = {},
): RegistryPackageResult {
	const backend = useBackend();
	const profile = useHubProfile();
	const signedIn = useAuthStatusStore((state) => state.signedIn);
	const authState = workspaceAuthState(signedIn, auth);
	const canFetch = authState === "signed_in" || authState === "signed_out";
	const id = packageId ?? "";
	const hubProfile = profile.data?.hub_profile;

	const remote = useQuery({
		queryKey: registryPackageKey(id, auth?.user?.profile?.sub),
		queryFn: () => {
			if (!hubProfile) throw new Error("Profile not loaded");
			if (!canFetch || auth?.user?.expired === true) {
				throw new Error(
					`Session for registry package ${id} is ${authState}; sign in again to load it`,
				);
			}
			return fetcher<RegistryEntry>(
				hubProfile,
				`registry/package/${encodeURIComponent(id)}`,
				{ method: "GET" },
				authState === "signed_in" ? auth : undefined,
			);
		},
		enabled: !!id && !!hubProfile && canFetch,
		retry: false,
	});

	const unconfirmed =
		remote.status === "success" && isUnconfirmedRestore(remote.data);
	const canConfirm = unconfirmed && canFetch;
	const { refetch: refetchRemote } = remote;
	useEffect(() => {
		if (canConfirm) void refetchRemote({ cancelRefetch: false });
	}, [canConfirm, refetchRemote]);

	const remoteEntry = usableEntry(remote.data);
	const hasConfirmedData = !!remoteEntry && !isUnconfirmedRestore(remote.data);
	const remoteSettled = remote.status !== "pending" || authState === "expired";

	const installed = useQuery({
		queryKey: ["local-package-fallback", id],
		queryFn: () => backend.registryState.getPackage(id),
		enabled: localFallback && !!id && remoteSettled && !remoteEntry,
	});

	const localEntry = useMemo(
		() =>
			localFallback && !remoteEntry && installed.data
				? entryFromInstalled(installed.data)
				: undefined,
		[localFallback, remoteEntry, installed.data],
	);

	const status: WorkspaceRemoteStatus =
		profile.error && !hubProfile
			? "error"
			: unconfirmed
				? "loading"
				: remoteStatusOf({
						status: remote.status,
						fetchStatus: remote.fetchStatus,
						error: remote.error,
						hasConfirmedData,
					});
	const entry = remoteEntry ?? localEntry;
	const source: WorkspaceRemoteSource | undefined = remoteEntry
		? "registry"
		: localEntry
			? "local"
			: undefined;

	const { refetch: refetchProfile } = profile;
	const { refetch: refetchInstalled } = installed;
	const retry = useCallback(() => {
		void refetchProfile();
		void refetchRemote();
		if (localFallback) void refetchInstalled();
	}, [refetchProfile, refetchRemote, refetchInstalled, localFallback]);

	return {
		entry,
		source,
		status,
		authState,
		isLoading:
			!!id &&
			!entry &&
			(profile.isLoading ||
				(authState !== "expired" &&
					(status === "idle" || status === "loading")) ||
				(localFallback && installed.isLoading)),
		isFetching: remote.isFetching,
		error: remote.error ?? profile.error ?? null,
		retry,
	};
}
