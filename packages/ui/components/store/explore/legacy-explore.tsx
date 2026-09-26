"use client";

import { Loader2 } from "lucide-react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect } from "react";
import { useDeveloperMode } from "../../../hooks/use-developer-mode";
import type { IEventMapping } from "../../interfaces/interfaces";
import { ExploreAppsPage } from "../explore-apps-page";
import { legacyAppsExploreTarget } from "./explore-href";
import { legacyFallbackTarget } from "./legacy-explore-model";

function Redirecting() {
	return (
		<div className="flex flex-1 items-center justify-center p-10">
			<Loader2
				aria-hidden="true"
				className="size-5 animate-spin text-muted-foreground"
			/>
		</div>
	);
}

/**
 * A hub without the Explore endpoints: `?type=packages` in developer mode goes to the old package list; everything
 * else gets the old Explore apps page, after the URL is rewritten into the query it reads, so a filtered link keeps
 * its category, query and sort. Decided on every render, so developer mode resolving late still redirects.
 */
export function LegacyExploreFallback({
	eventConfig,
}: Readonly<{ eventConfig?: IEventMapping }>) {
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const { developerMode } = useDeveloperMode();
	const target = legacyFallbackTarget({
		developerMode,
		pathname,
		searchParams: new URLSearchParams(searchParams.toString()),
	});
	useEffect(() => {
		if (target) router.replace(target, { scroll: false });
	}, [router, target]);
	if (target) return <Redirecting />;
	return <ExploreAppsPage eventConfig={eventConfig} />;
}

function AppsRedirect() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const target = legacyAppsExploreTarget(searchParams);
	useEffect(() => {
		router.replace(target);
	}, [router, target]);
	return <Redirecting />;
}

/** `/store/explore/apps`: unfiltered links land on the curated landing, filtered ones on Browse. */
export function ExploreAppsRedirect() {
	return (
		<Suspense fallback={<Redirecting />}>
			<AppsRedirect />
		</Suspense>
	);
}
