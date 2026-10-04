import { type QueryClient, useQuery } from "@tanstack/react-query";
import {
	type AppPackageWidget,
	listAppPackageWidgets,
} from "../lib/package-widgets";
import { type IBackendState, useBackend } from "../state/backend-state";

function appPackageWidgetsKey(appId: string) {
	return ["app-package-widgets", appId] as const;
}

function appPackageWidgetsQuery(
	backend: IBackendState,
	appId: string,
	options: { strict?: boolean; packageId?: string } = {},
) {
	return {
		queryKey: options.packageId
			? [
					...appPackageWidgetsKey(appId),
					"package",
					options.packageId,
					Boolean(options.strict),
				]
			: options.strict
				? [...appPackageWidgetsKey(appId), "strict"]
				: appPackageWidgetsKey(appId),
		queryFn: () =>
			listAppPackageWidgets(
				{
					listPackages: backend.appState.listPackages?.bind(backend.appState),
					getPackage: (packageId, packageAppId) =>
						backend.registryState.getPackage(packageId, packageAppId),
				},
				appId,
				options,
			),
	};
}

/**
 * Widgets of the packages added to an app (§6.1), resolved from the package
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
export async function fetchAppPackageWidgets(
	queryClient: QueryClient,
	backend: IBackendState,
	appId: string,
	options: { strict?: boolean; packageId?: string } = {},
): Promise<AppPackageWidget[]> {
	const widgets = await queryClient.fetchQuery({
		...appPackageWidgetsQuery(backend, appId, options),
		staleTime: 0,
	});
	if (options.strict && !options.packageId)
		queryClient.setQueryData(appPackageWidgetsKey(appId), widgets);
	return widgets;
}

export function invalidateAppPackageWidgets(
	queryClient: QueryClient,
	appId: string,
): Promise<void> {
	return queryClient.invalidateQueries({
		queryKey: appPackageWidgetsKey(appId),
	});
}
