"use client";

import { usePathname, useSearchParams } from "next/navigation";
import { useCallback } from "react";
import { useClientRouter } from "../../lib/client-navigation";
import { type BotsView, botsViewQuery, parseBotsView } from "./dock-selection";

/** Chats or the org chart, mirrored into `?view=org`. */
export function useBotsView() {
	const router = useClientRouter();
	const pathname = usePathname();
	const params = useSearchParams();
	const view = parseBotsView(params);
	const setView = useCallback(
		(next: BotsView) => {
			const query = botsViewQuery(next, window.location.search);
			router.replace(
				`${pathname}${query ? `?${query}` : ""}${window.location.hash}`,
				{ scroll: false },
			);
		},
		[pathname, router],
	);
	return { view, setView };
}
