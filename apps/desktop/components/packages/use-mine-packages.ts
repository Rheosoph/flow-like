"use client";

import { useBackend } from "@flow-like/flow-like-ui";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import type { DeveloperProject } from "@flow-like/flow-like-ui/lib/schema/developer";
import type { PackageManifest } from "@flow-like/flow-like-ui/lib/schema/wasm";
import { useTranslation } from "@flow-like/locales";
import {
	type UseQueryResult,
	useMutation,
	useQueries,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	addProjectFolder,
	checkStaleness,
	inspectLint,
	listProjects,
	pickProjectFolder,
	readManifest,
	removeProject,
} from "./local-projects";
import {
	type MineLintCounts,
	type MineProjectInput,
	type MineRegistryInput,
	deriveMine,
	idsToLookUp,
} from "./mine-model";

export type MineRegistryStatus =
	| "ready"
	| "loading"
	| "signed-out"
	| "unavailable";

const MAINTAINER_LIMIT = 100;
const STALENESS_INTERVAL_MS = 5000;
const REGISTRY_STALE_MS = 60_000;

export const mineQueryKeys = {
	projects: ["developer-projects"] as const,
	manifest: (path: string) => ["developer-manifest", path] as const,
	lint: (path: string) => ["developer-lint", path] as const,
	staleness: ["developer-staleness"] as const,
	/** Always fetched with disabled packages; `deriveMine` hides the ones without a checkout. */
	maintained: (user?: string) => ["mine-registry-maintained", user] as const,
	lookup: (ids: readonly string[]) => ["mine-registry-lookup", ...ids] as const,
};

export interface UseMinePackagesOptions {
	/** Also list disabled (soft-deleted) packages that have no checkout here. */
	includeDisabled?: boolean;
}

const NO_PROJECTS: DeveloperProject[] = [];

type ManifestResults = UseQueryResult<PackageManifest | null>[];
type LintResults = UseQueryResult<MineLintCounts>[];

function combineByPath<T>(
	paths: readonly string[],
	results: readonly { data?: T | null; isPending: boolean }[],
) {
	const byPath = new Map<string, T>();
	results.forEach((result, index) => {
		if (result.data) byPath.set(paths[index], result.data);
	});
	return { byPath, pending: results.some((result) => result.isPending) };
}

export function useMinePackages({
	includeDisabled = false,
}: UseMinePackagesOptions = {}) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const auth = useAuth();
	const queryClient = useQueryClient();
	const signedIn = Boolean(auth.user?.access_token);
	const [inspectPaths, setInspectPaths] = useState<ReadonlySet<string>>(
		() => new Set(),
	);

	const projects = useQuery({
		queryKey: mineQueryKeys.projects,
		queryFn: listProjects,
	});
	const projectList = projects.data ?? NO_PROJECTS;
	const projectPaths = useMemo(
		() => projectList.map((project) => project.path),
		[projectList],
	);

	const combineManifests = useCallback(
		(results: ManifestResults) =>
			combineByPath<PackageManifest>(projectPaths, results),
		[projectPaths],
	);
	const manifests = useQueries({
		queries: projectPaths.map((path) => ({
			queryKey: mineQueryKeys.manifest(path),
			queryFn: () => readManifest(path),
			staleTime: REGISTRY_STALE_MS,
		})),
		combine: combineManifests,
	});

	const lintPaths = useMemo(
		() => projectPaths.filter((path) => inspectPaths.has(path)),
		[projectPaths, inspectPaths],
	);
	const combineLint = useCallback(
		(results: LintResults) => combineByPath<MineLintCounts>(lintPaths, results),
		[lintPaths],
	);
	const lint = useQueries({
		queries: lintPaths.map((path) => ({
			queryKey: mineQueryKeys.lint(path),
			queryFn: () => inspectLint(path),
			staleTime: Number.POSITIVE_INFINITY,
			refetchOnWindowFocus: false,
			retry: false,
		})),
		combine: combineLint,
	});

	const staleness = useQuery({
		queryKey: mineQueryKeys.staleness,
		queryFn: checkStaleness,
		refetchInterval: STALENESS_INTERVAL_MS,
		retry: false,
	});

	const inputs = useMemo<MineProjectInput[]>(() => {
		const stale = staleness.data ?? [];
		const stalePaths = new Set(stale.map((info) => info.project_path));
		const staleIds = new Set(stale.map((info) => info.package_id));
		return projectList.map((project) => {
			const manifest = manifests.byPath.get(project.path) ?? null;
			return {
				project,
				manifest,
				lint: lint.byPath.get(project.path) ?? null,
				stale:
					stalePaths.has(project.path) ||
					(manifest ? staleIds.has(manifest.id) : false),
			};
		});
	}, [projectList, manifests.byPath, lint.byPath, staleness.data]);

	const maintained = useQuery({
		queryKey: mineQueryKeys.maintained(auth.user?.profile?.sub),
		queryFn: () =>
			backend.registryState.getOwnedPackages({
				access: "maintainer",
				limit: MAINTAINER_LIMIT,
				includeDisabled: true,
			}),
		enabled: signedIn,
		staleTime: REGISTRY_STALE_MS,
	});

	const lookupIds = useMemo(
		() =>
			manifests.pending
				? []
				: idsToLookUp(inputs, maintained.data?.packages ?? []),
		[manifests.pending, inputs, maintained.data],
	);
	const lookup = useQuery({
		queryKey: mineQueryKeys.lookup(lookupIds),
		queryFn: () =>
			backend.registryState.searchPackages({
				ids: lookupIds,
				includeOwn: true,
				limit: lookupIds.length,
			}),
		enabled: signedIn && maintained.isSuccess && lookupIds.length > 0,
		staleTime: REGISTRY_STALE_MS,
		retry: false,
	});

	const registryStatus: MineRegistryStatus = !signedIn
		? "signed-out"
		: maintained.isError || lookup.isError
			? "unavailable"
			: maintained.isPending || (lookupIds.length > 0 && lookup.isPending)
				? "loading"
				: "ready";

	const registry = useMemo<MineRegistryInput | null>(
		() =>
			signedIn && maintained.data
				? {
						maintained: maintained.data.packages ?? [],
						lookedUp: lookup.data?.packages ?? [],
					}
				: null,
		[signedIn, maintained.data, lookup.data],
	);

	const model = useMemo(
		() => deriveMine(inputs, registry, { includeDisabled }),
		[inputs, registry, includeDisabled],
	);

	const requestInspection = useCallback((path: string) => {
		setInspectPaths((current) =>
			current.has(path) ? current : new Set(current).add(path),
		);
	}, []);

	const invalidateProject = useCallback(
		(path: string) => {
			queryClient.invalidateQueries({ queryKey: mineQueryKeys.lint(path) });
			queryClient.invalidateQueries({ queryKey: mineQueryKeys.staleness });
		},
		[queryClient],
	);

	const refresh = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: mineQueryKeys.projects });
		queryClient.invalidateQueries({ queryKey: ["developer-manifest"] });
		queryClient.invalidateQueries({ queryKey: ["developer-lint"] });
		queryClient.invalidateQueries({ queryKey: ["mine-registry-maintained"] });
		queryClient.invalidateQueries({ queryKey: ["mine-registry-lookup"] });
	}, [queryClient]);

	const { refetch: refetchMaintained } = maintained;
	const { refetch: refetchLookup } = lookup;
	const retryRegistry = useCallback(() => {
		void refetchMaintained();
		if (lookupIds.length > 0) void refetchLookup();
	}, [refetchMaintained, refetchLookup, lookupIds.length]);

	const addFolder = useMutation({
		mutationFn: async (path?: string) => {
			const target = path ?? (await pickProjectFolder());
			return target ? addProjectFolder(target) : null;
		},
		onSuccess: (project) => {
			if (!project) return;
			toast.success(t("addedName", "Added {{name}}", { name: project.name }));
			queryClient.invalidateQueries({ queryKey: mineQueryKeys.projects });
		},
		onError: (error) => toast.error(getErrorMessage(error)),
	});

	const remove = useMutation({
		mutationFn: async (projectIds: string[]) => {
			for (const projectId of projectIds) await removeProject(projectId);
		},
		onSuccess: () => {
			toast.success(
				t(
					"removedFromListFilesStayOnDisk",
					"Removed from list. Files stay on disk.",
				),
			);
			queryClient.invalidateQueries({ queryKey: mineQueryKeys.projects });
		},
		onError: (error) => toast.error(getErrorMessage(error)),
	});

	return {
		model,
		isLoading: projects.isPending,
		/** Checkouts are grouped by manifest id, so an entry's id is unknown until its manifest loads. */
		manifestsPending: manifests.pending,
		error: projects.error,
		hasProjects: projectList.length > 0,
		registryStatus,
		requestInspection,
		invalidateProject,
		refresh,
		retryRegistry,
		isRetryingRegistry: maintained.isFetching || lookup.isFetching,
		isRefreshing: projects.isFetching || maintained.isFetching,
		addFolder: addFolder.mutate,
		isAddingFolder: addFolder.isPending,
		removeProject: remove.mutate,
	};
}

export type MinePackages = ReturnType<typeof useMinePackages>;
