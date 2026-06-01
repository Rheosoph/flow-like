"use client";

import { useCallback } from "react";
import { useInvalidateInvoke } from "../../../hooks";
import { useBackend } from "../../../state/backend-state";
import type { IApp } from "../../../types";
import { AllowForkingSwitcher } from "./allow-forking-switcher";
import { ForkPermissionWarning } from "./fork-permission-warning";

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
		<div className="space-y-0">
			<AllowForkingSwitcher
				localApp={localApp}
				canEdit={canEdit}
				onAllowForkingChange={handleChange}
			/>
			<ForkPermissionWarning
				appId={localApp.id}
				enabled={Boolean(localApp.allow_forking)}
				canEdit={canEdit}
			/>
		</div>
	);
}
