"use client";

import { usePathname, useSearchParams } from "next/navigation";
import { useCallback } from "react";
import { useClientRouter } from "../../lib/client-navigation";
import { isMoveRequested, withoutMoveRequest } from "./dock-selection";

/**
 * `?move=local` (the desktop's sign-in toast links here) asks for the move review. Dismissing it
 * drops the parameter and keeps the rest, so the dock stays where it is.
 */
export function useMoveRequest() {
	const router = useClientRouter();
	const pathname = usePathname();
	const requested = isMoveRequested(useSearchParams());

	const dismiss = useCallback(() => {
		const live = window.location.search;
		if (!isMoveRequested(new URLSearchParams(live))) return;
		const query = withoutMoveRequest(live);
		router.replace(query ? `${pathname}?${query}` : pathname, {
			scroll: false,
		});
	}, [pathname, router]);

	return { requested, dismiss };
}
