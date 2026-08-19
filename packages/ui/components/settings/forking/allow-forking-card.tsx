"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke, useInvoke } from "../../../hooks";
import type { IForkPolicy } from "../../../lib/schema/app/fork";
import { useBackend } from "../../../state/backend-state";
import type { IApp } from "../../../types";
import { AllowForkingSwitcher } from "./allow-forking-switcher";
import { ForkPermissionWarning } from "./fork-permission-warning";
import { ForkPolicyEditor } from "./fork-policy-editor";

export interface AllowForkingCardProps {
	localApp: IApp;
	canEdit: boolean;
	/** Optional callback fired after a successful toggle, e.g. to refetch
	 * the parent's local copy of the app once the cache invalidation has
	 * landed. */
	onChanged?: () => void | Promise<void>;
}

/**
 * Backend-wired wrapper around the pure {@link AllowForkingSwitcher} and
 * {@link ForkPolicyEditor}. Both desktop and web mount this in the app
 * config page so the toggle stays consistent across deployments without
 * each app having to repeat the cache-invalidation glue.
 *
 * The policy is fetched from `GET /apps/{id}/settings/forking` rather than
 * read off `localApp` — it is deliberately absent from the app proto,
 * because only the server needs it at fork time.
 */
export function AllowForkingCard({
	localApp,
	canEdit,
	onChanged,
}: Readonly<AllowForkingCardProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const allowForking = Boolean(localApp.allow_forking);

	const settings = useInvoke(
		backend.appState.getForkSettings,
		backend.appState,
		[localApp.id],
		canEdit && typeof localApp.id === "string",
	);

	const [policy, setPolicy] = useState<IForkPolicy | undefined>(undefined);
	const [saving, setSaving] = useState(false);

	useEffect(() => {
		if (settings.data?.fork_policy) setPolicy(settings.data.fork_policy);
	}, [settings.data?.fork_policy]);

	const handleChange = useCallback(
		async (appId: string, allow: boolean) => {
			await backend.appState.changeAppAllowForking(appId, allow);
			await invalidate(backend.appState.getApp, [appId]);
			await invalidate(backend.appState.getApps, []);
			await onChanged?.();
		},
		[backend.appState, invalidate, onChanged],
	);

	const handlePolicyChange = useCallback(
		async (next: IForkPolicy) => {
			if (saving) return;
			const previous = policy;
			setPolicy(next);
			setSaving(true);
			try {
				await backend.appState.changeAppForkPolicy(localApp.id, next);
				await invalidate(backend.appState.getForkSettings, [localApp.id]);
			} catch (err) {
				setPolicy(previous);
				toast.error(
					err instanceof Error
						? t(
								"couldntUpdateForkSettingsMessage",
								"Couldn't update fork settings: {{message}}",
								{ message: err.message },
							)
						: t("couldntUpdateForkSettings", "Couldn't update fork settings"),
				);
			} finally {
				setSaving(false);
			}
		},
		[backend.appState, invalidate, localApp.id, policy, saving],
	);

	return (
		<div className="space-y-0">
			<AllowForkingSwitcher
				localApp={localApp}
				canEdit={canEdit}
				onAllowForkingChange={handleChange}
			>
				{canEdit && allowForking && policy && (
					<ForkPolicyEditor
						policy={policy}
						disabled={saving}
						onChange={handlePolicyChange}
					/>
				)}
			</AllowForkingSwitcher>
			{/* Owners wait for their policy before the warning computes which
			    permissions are actually required — otherwise it briefly lists
			    ones an excluded category doesn't need. A failed fetch settles
			    too, falling back to the permissive set. */}
			{(!canEdit || !settings.isPending) && (
				<ForkPermissionWarning
					appId={localApp.id}
					enabled={allowForking}
					canEdit={canEdit}
					policy={policy}
				/>
			)}
		</div>
	);
}
