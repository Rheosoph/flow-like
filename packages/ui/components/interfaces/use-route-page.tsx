"use client";

import { usePathname, useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { pathUseUrl, readUseRoutePath } from "../../lib/use-route-url";
import { UsePageContent, type UsePageContentProps } from "./use-page-content";

export function UseRoutePage({
	eventConfig,
	notFound,
}: Pick<UsePageContentProps, "eventConfig" | "notFound">) {
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const query = searchParams.toString();
	const [ready, setReady] = useState(false);
	useEffect(() => {
		try {
			const current = new URL(window.location.href);
			const canonical = pathUseUrl(current);
			if (
				canonical !== current.pathname + current.search + current.hash ||
				pathname !== current.pathname ||
				query !== current.searchParams.toString()
			) {
				// Development rewrites can inject their path parameter into Next's
				// search params. The browser URL owns the app's actual query data.
				window.history.replaceState(null, "", canonical);
				return;
			}
		} catch {
			// Invalid path encodings render the existing not-found screen below.
		}
		setReady(true);
	}, [pathname, query]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: a different app or query can replace the fragment target without changing the path.
	useEffect(() => {
		if (
			!pathname ||
			!window.location.hash ||
			window.history.state?.flowLikeUseScroll === false
		)
			return;
		let id: string;
		try {
			id = decodeURIComponent(window.location.hash.slice(1));
		} catch {
			return;
		}
		// Page content arrives from the API after the route changes. Wait for a
		// fragment target instead of losing anchor scrolling while it loads.
		const scrollToTarget = () => {
			const target =
				document.getElementById(id) ?? document.getElementsByName(id)[0];
			if (!target) return false;
			target.scrollIntoView();
			return true;
		};
		if (scrollToTarget()) return;
		const observer = new MutationObserver(() => {
			if (scrollToTarget()) observer.disconnect();
		});
		observer.observe(document.body, { childList: true, subtree: true });
		const timeout = window.setTimeout(() => observer.disconnect(), 10_000);
		return () => {
			observer.disconnect();
			window.clearTimeout(timeout);
		};
	}, [pathname, query]);
	// The exported HTML is shared by every app path. Resolve the browser URL
	// after hydration so an initial deep link cannot fetch the default route first.
	if (!ready) return null;
	let routePath: string | undefined;
	try {
		routePath = readUseRoutePath(window.location.pathname);
	} catch {
		return notFound ?? null;
	}
	return (
		<UsePageContent
			eventConfig={eventConfig}
			routePath={routePath}
			notFound={notFound}
		/>
	);
}
