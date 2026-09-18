import { type QueryClient, useQuery } from "@tanstack/react-query";
import {
	type AppPackageWidget,
	listAppPackageWidgets,
} from "../lib/package-widgets";
import { type IBackendState, useBackend } from "../state/backend-state";

function appPackageWidgetsKey(appId: string) {
	return ["app-package-widgets", appId] as const;
}

function appPackageWidgetsQuery(backend: IBackendState, appId: string) {
	return {
		queryKey: appPackageWidgetsKey(appId),
		queryFn: () =>
			listAppPackageWidgets(
				{
					listPackages: backend.appState.listPackages?.bind(backend.appState),
					getPackage: (packageId) =>
						backend.registryState.getPackage(packageId),
				},
				appId,
			),
	};
}

/**
 * Widgets of the packages added to an app (§6.1), resolved from the installed
 * manifests; empty on hosts without the per-app package listing.
 */
export function useAppPackageWidgets(
	appId: string | undefined,
	options: { enabled?: boolean; staleTime?: number } = {},
) {
	const backend = useBackend();
	return useQuery({
		...appPackageWidgetsQuery(backend, appId ?? ""),
		enabled: Boolean(appId) && (options.enabled ?? true),
		...(options.staleTime === undefined
			? {}
			: { staleTime: options.staleTime }),
	});
}

/** Reads past the cache, e.g. right after a local package rebuild. */
export function fetchAppPackageWidgets(
	queryClient: QueryClient,
	backend: IBackendState,
	appId: string,
): Promise<AppPackageWidget[]> {
	return queryClient.fetchQuery({
		...appPackageWidgetsQuery(backend, appId),
		staleTime: 0,
	});
}

export function invalidateAppPackageWidgets(
	queryClient: QueryClient,
	appId: string,
): Promise<void> {
	return queryClient.invalidateQueries({
		queryKey: appPackageWidgetsKey(appId),
	});
}
