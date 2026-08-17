"use client";

import { useTranslation } from "@flow-like/locales";
import { AlertCircle, SparklesIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { memo, useMemo } from "react";
import { useInfiniteInvoke } from "../../hooks/use-invoke";
import { IAppSearchSort } from "../../lib/schema/app/app-search-query";
import { useBackend } from "../../state/backend-state";
import { Alert, AlertDescription } from "../ui/alert";
import { AppCard } from "../ui/app-card";
import { Skeleton } from "../ui/skeleton";

export const StoreRecommendations = memo(function StoreRecommendations({
	excludeAppId,
}: Readonly<{ excludeAppId?: string }>) {
	const { t } = useTranslation("store");
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
		return apps.pages.flat().filter(([app]) => app.id !== excludeAppId);
	}, [apps, excludeAppId]);

	if (!combinedApps.length && !isLoading) return null;

	return (
		<section className="space-y-4">
			<h2 className="text-sm font-medium text-muted-foreground/60 uppercase tracking-wider flex items-center gap-2">
				<SparklesIcon className="w-4 h-4" />
				{t('youMightAlsoLike', 'You might also like')}
			</h2>

			{error && (
				<Alert className="border-destructive/20 bg-destructive/5">
					<AlertCircle className="h-4 w-4" />
					<AlertDescription>{t('failedToLoadMessage', 'Failed to load: {{message}}', { message: error.message })}</AlertDescription>
				</Alert>
			)}

			{isLoading ? (
				<div className="flex gap-4 overflow-hidden">
					{["sk-1", "sk-2", "sk-3", "sk-4"].map((key) => (
						<div key={key} className="shrink-0 w-65 md:w-75 space-y-3">
							<Skeleton className="h-40 w-full rounded-xl" />
							<Skeleton className="h-4 w-3/4 rounded-full" />
							<Skeleton className="h-3 w-1/2 rounded-full" />
						</div>
					))}
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
