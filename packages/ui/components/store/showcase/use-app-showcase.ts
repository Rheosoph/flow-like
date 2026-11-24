"use client";

import { useInvoke } from "../../../hooks/use-invoke";
import type { IAppShowcase } from "../../../lib/schema/app/showcase";
import { useBackend } from "../../../state/backend-state";

/** The public showcase, or `null` while loading, on failure, or when nothing is shown. */
export function useAppShowcase(appId: string | undefined): IAppShowcase | null {
	const backend = useBackend();
	const query = useInvoke<IAppShowcase, [appId: string]>(
		backend.appState.getShowcase,
		backend.appState,
		[appId ?? ""],
		!!appId,
	);
	return query.isError ? null : (query.data ?? null);
}
