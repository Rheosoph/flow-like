"use client";

import { useTranslation } from "@flow-like/locales";

/** Non-blocking bar over a rendered page while its onLoad run has produced nothing yet. */
export function PageLoadIndicator() {
	return (
		<div
			aria-hidden="true"
			data-page-load-indicator=""
			className="pointer-events-none sticky top-0 z-50 h-0"
		>
			<div className="relative h-0.5 w-full overflow-hidden bg-primary/15">
				<div className="absolute inset-y-0 left-0 w-1/3 animate-[indeterminate_1.5s_ease-in-out_infinite] bg-primary motion-reduce:w-full motion-reduce:animate-none motion-reduce:opacity-60" />
			</div>
		</div>
	);
}

/**
 * Stays mounted outside any busy region: a live region only announces text that changes after
 * it is registered, so the status is toggled rather than mounted with the loader.
 */
export function PageLoadStatus({ loading }: Readonly<{ loading: boolean }>) {
	const { t } = useTranslation("interfaces");
	return (
		<output className="sr-only">
			{loading ? t("loadingPageData", "Loading page data…") : ""}
		</output>
	);
}
