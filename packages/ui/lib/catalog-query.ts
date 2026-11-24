import type { QueryClient } from "@tanstack/react-query";

/** Query key heads under which an app's node catalog is cached. */
const CATALOG_QUERY_KEYS = ["getCatalog", "app-catalog-nodes"] as const;

/**
 * Installing a package or changing a Data Studio ontology invalidates the catalog directly.
 * This window only bounds how long a teammate's change of that kind stays unseen, while a
 * mount inside it reuses the copy in memory instead of asking the hub again.
 */
const CATALOG_STALE_TIME_MS = 2 * 60_000;

export function applyCatalogQueryDefaults(queryClient: QueryClient): void {
	for (const key of CATALOG_QUERY_KEYS) {
		queryClient.setQueryDefaults([key], { staleTime: CATALOG_STALE_TIME_MS });
	}
}
