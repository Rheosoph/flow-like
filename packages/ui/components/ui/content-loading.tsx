"use client";

import { useTranslation } from "@flow-like/locales";
import { Skeleton } from "./skeleton";

/** A route fallback that leaves the surrounding navigation available. */
export function ContentLoadingSkeleton() {
	const { t } = useTranslation("common");

	return (
		<div
			data-slot="content-loading"
			className="flex min-h-0 w-full flex-1 flex-col gap-6 overflow-hidden p-6"
		>
			<output className="sr-only">{t("loadingPage", "Loading page...")}</output>
			<div aria-hidden="true" className="space-y-6">
				<div className="space-y-3">
					<Skeleton className="h-7 w-48 max-w-full bg-muted motion-reduce:animate-none" />
					<Skeleton className="h-4 w-72 max-w-full bg-muted motion-reduce:animate-none" />
				</div>
				<div className="space-y-4 rounded-xl border border-border/60 p-5">
					<Skeleton className="h-5 w-1/3 bg-muted motion-reduce:animate-none" />
					<Skeleton className="h-4 w-full bg-muted motion-reduce:animate-none" />
					<Skeleton className="h-4 w-5/6 bg-muted motion-reduce:animate-none" />
					<Skeleton className="h-4 w-2/3 bg-muted motion-reduce:animate-none" />
				</div>
			</div>
		</div>
	);
}
