"use client";

import { AlertCircle, Package, SparklesIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { memo, useMemo } from "react";
import { useInfiniteInvoke } from "../../hooks/use-invoke";
import { IAppSearchSort } from "../../lib/schema/app/app-search-query";
import { useBackend } from "../../state/backend-state";
import { Alert, AlertDescription } from "../ui/alert";
import { AppCard } from "../ui/app-card";
import { Skeleton } from "../ui/skeleton";

export const StoreRecommendations = memo(function StoreRecommendations() {
	const backend = useBackend();
	const router = useRouter();

	const {
		data: apps,
		isLoading,
		error,
	} = useInfiniteInvoke(backend.appState.searchApps, backend.appState, [
		undefined,
		undefined,
		undefined,
		undefined,
		undefined,
		IAppSearchSort.BestRated,
		undefined,
	]);

	const combinedApps = useMemo(() => {
		if (!apps) return [];
		return apps.pages.flat();
	}, [apps]);

	if (!combinedApps.length && !isLoading) return null;

	return (
		<section className="space-y-4">
			<h2 className="text-sm font-medium text-muted-foreground/60 uppercase tracking-wider flex items-center gap-2">
				<SparklesIcon className="w-4 h-4" />
				You might also like
			</h2>

			{error && (
				<Alert className="border-destructive/20 bg-destructive/5">
					<AlertCircle className="h-4 w-4" />
					<AlertDescription>Failed to load: {error.message}</AlertDescription>
				</Alert>
			)}

			{isLoading ? (
				<div className="flex gap-4 overflow-hidden">
					{[...Array(4)].map((_, i) => (
						<div key={i} className="shrink-0 w-65 md:w-75 space-y-3">
							<Skeleton className="h-40 w-full rounded-xl" />
							<Skeleton className="h-4 w-3/4 rounded-full" />
							<Skeleton className="h-3 w-1/2 rounded-full" />
						</div>
					))}
				</div>
			) : combinedApps.length === 0 ? (
				<div className="py-12 text-center">
					<Package className="w-10 h-10 mx-auto text-muted-foreground/20 mb-2" />
					<p className="text-sm text-muted-foreground/60">
						No recommendations right now.
					</p>
				</div>
			) : (
				<div className="-mx-6 md:-mx-10">
					<div
						className="flex gap-4 overflow-x-auto px-6 md:px-10 snap-x snap-mandatory pb-4"
						style={{ scrollbarWidth: "none" }}
					>
						{combinedApps.map(([app, metadata]) => (
							<div key={app.id} className="snap-start shrink-0 w-65 md:w-75">
								<AppCard
									app={app}
									variant="extended"
									metadata={metadata}
									className="w-full h-full"
									onClick={() => router.push(`/store?id=${app.id}`)}
									href={`/store?id=${app.id}`}
								/>
							</div>
						))}
					</div>
				</div>
			)}
		</section>
	);
});
