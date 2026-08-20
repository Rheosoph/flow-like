"use client";

import { useTranslation } from "@flow-like/locales";
import { GitForkIcon, ShieldIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import type { IApp } from "../../../types";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Label } from "../../ui/label";
import { Switch } from "../../ui/switch";

export interface AllowForkingSwitcherProps {
	localApp: IApp;
	canEdit: boolean;
	onAllowForkingChange: (appId: string, allow: boolean) => Promise<void>;
	/** Rendered below the toggle, inside the same card — used for the
	 * owner's fork-policy editor so the two controls read as one setting. */
	children?: React.ReactNode;
}

/**
 * Owner-only toggle for the project-level Fork-an-app opt-in. The
 * backend enforces ownership on `PATCH /apps/{app_id}/settings/forking`;
 * this component only renders the control + handles the optimistic
 * update + toast feedback.
 */
export function AllowForkingSwitcher({
	localApp,
	canEdit,
	onAllowForkingChange,
	children,
}: Readonly<AllowForkingSwitcherProps>) {
	const { t } = useTranslation("settings");
	const initial = Boolean(localApp.allow_forking);
	const [allow, setAllow] = useState(initial);
	const [pending, setPending] = useState(false);

	// Mirror externally-driven prop changes (e.g. parent refetched).
	useEffect(() => {
		setAllow(initial);
	}, [initial]);

	const handleToggle = useCallback(
		async (next: boolean) => {
			if (next === allow || pending) return;
			setPending(true);
			const previous = allow;
			setAllow(next);
			try {
				await onAllowForkingChange(localApp.id, next);
				toast.success(next ? "Forking enabled" : "Forking disabled");
			} catch (err) {
				setAllow(previous);
				toast.error(
					err instanceof Error
						? t(
								"couldntUpdateForkingSettingMessage",
								"Couldn't update forking setting: {{message}}",
								{ message: err.message },
							)
						: t(
								"couldntUpdateForkingSetting",
								"Couldn't update forking setting",
							),
				);
			} finally {
				setPending(false);
			}
		},
		[allow, pending, localApp.id, onAllowForkingChange],
	);

	const inputId = `allow-forking-${localApp.id}`;

	return (
		<Card>
			<CardHeader className="flex flex-row items-start justify-between gap-4 space-y-0">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2">
						<GitForkIcon className="w-4 h-4" />
						{t("forking2", "Forking")}
					</CardTitle>
					<CardDescription>
						{t(
							"letOtherUsersWithReadAccessCreateTheirOwnCopyOfThisAppOffByDefaultSecretVariablesOauthBindingsAndRemoteeventTokensAreStrippedOrReplacedWhenAForkIsCreated",
							"Let other users with read access create their own copy of this app. Off by default — secret variables, OAuth bindings and remote-event tokens are stripped or replaced when a fork is created.",
						)}
					</CardDescription>
				</div>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="flex items-center justify-between rounded-md border p-4">
					<div className="space-y-1 pr-4">
						<Label htmlFor={inputId} className="text-sm font-medium">
							{t("allowForking", "Allow forking")}
						</Label>
						<p className="text-xs text-muted-foreground">
							{allow
								? t(
										"membersWithReadAccessCanForkThisApp",
										"Members with read access can fork this app.",
									)
								: t(
										"forkingIsDisabledTheForkButtonIsHiddenForEveryone",
										"Forking is disabled. The Fork button is hidden for everyone.",
									)}
						</p>
						{!canEdit && (
							<p className="text-xs text-muted-foreground flex items-center gap-1">
								<ShieldIcon className="w-3 h-3" />
								{t(
									"onlyTheAppOwnerCanChangeThis",
									"Only the app owner can change this.",
								)}
							</p>
						)}
					</div>
					<Switch
						id={inputId}
						checked={allow}
						disabled={!canEdit || pending}
						onCheckedChange={handleToggle}
					/>
				</div>
				{children}
			</CardContent>
		</Card>
	);
}
