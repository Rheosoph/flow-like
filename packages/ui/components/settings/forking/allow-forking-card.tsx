"use client";

import { useCallback } from "react";
import { useInvalidateInvoke } from "../../../hooks";
import { useBackend } from "../../../state/backend-state";
import type { IApp } from "../../../types";
import { AllowForkingSwitcher } from "./allow-forking-switcher";

export interface AllowForkingCardProps {
	localApp: IApp;
	canEdit: boolean;
	/** Optional callback fired after a successful toggle, e.g. to refetch
	 * the parent's local copy of the app once the cache invalidation has
	 * landed. */
	onChanged?: () => void | Promise<void>;
}

/**
 * Backend-wired wrapper around the pure {@link AllowForkingSwitcher}.
 * Both desktop and web mount this in the app config page so the toggle
 * stays consistent across deployments without each app having to repeat
 * the cache-invalidation glue.
 */
export function AllowForkingCard({
	localApp,
	canEdit,
	onChanged,
}: Readonly<AllowForkingCardProps>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();

	const handleChange = useCallback(
		async (appId: string, allow: boolean) => {
			await backend.appState.changeAppAllowForking(appId, allow);
			await invalidate(backend.appState.getApp, [appId]);
			await invalidate(backend.appState.getApps, []);
			await onChanged?.();
		},
		[backend.appState, invalidate, onChanged],
	);

	return (
		<AllowForkingSwitcher
			localApp={localApp}
			canEdit={canEdit}
			onAllowForkingChange={handleChange}
		/>
	);
}
