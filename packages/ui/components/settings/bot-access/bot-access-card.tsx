"use client";

import { useTranslation } from "@flow-like/locales";
import { BotIcon, ShieldIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke } from "../../../hooks";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { useBackend } from "../../../state/backend-state";
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

export interface BotAccessCardProps {
	localApp: IApp;
	canEdit: boolean;
	onChanged?: () => void | Promise<void>;
}

/**
 * Whether members' bots may use the app. `PATCH /apps/{id}/settings/bots` is an Owner route,
 * like visibility and forking; a local app keeps the setting in its manifest. A bot's workspace
 * always admits its bot, so it has no such setting.
 */
export function BotAccessCard({
	localApp,
	canEdit,
	onChanged,
}: Readonly<BotAccessCardProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const permissions = useAppPermissions(localApp.id);
	const canManage = canEdit && permissions.can(RolePermissions.Owner);
	const stored = !localApp.refuse_bots;
	const [allowed, setAllowed] = useState(stored);
	const [pending, setPending] = useState(false);

	useEffect(() => {
		setAllowed(stored);
	}, [stored]);

	const handleToggle = useCallback(
		async (next: boolean) => {
			if (next === allowed || pending) return;
			setPending(true);
			setAllowed(next);
			try {
				await backend.appState.changeAppBotAccess(localApp.id, !next);
				await invalidate(backend.appState.getApp, [localApp.id]);
				await invalidate(backend.appState.getApps, []);
				await onChanged?.();
			} catch (error) {
				setAllowed(!next);
				toast.error(
					t("botAccess.saveFailed", {
						message: error instanceof Error ? error.message : String(error),
					}),
				);
			} finally {
				setPending(false);
			}
		},
		[allowed, backend.appState, invalidate, localApp.id, onChanged, pending, t],
	);

	if (localApp.bot_id) return null;
	const inputId = `allow-bots-${localApp.id}`;

	return (
		<Card>
			<CardHeader className="space-y-1">
				<CardTitle className="flex items-center gap-2">
					<BotIcon className="w-4 h-4" />
					{t("botAccess.title")}
				</CardTitle>
				<CardDescription>{t("botAccess.description")}</CardDescription>
			</CardHeader>
			<CardContent>
				<div className="flex items-center justify-between rounded-md border p-4">
					<div className="space-y-1 pr-4">
						<Label htmlFor={inputId} className="text-sm font-medium">
							{t("botAccess.allow")}
						</Label>
						<p className="text-xs text-muted-foreground">
							{allowed ? t("botAccess.allowed") : t("botAccess.refused")}
						</p>
						{!canManage && (
							<p className="text-xs text-muted-foreground flex items-center gap-1">
								<ShieldIcon className="w-3 h-3" />
								{t("botAccess.ownerOnly")}
							</p>
						)}
					</div>
					<Switch
						id={inputId}
						checked={allowed}
						disabled={!canManage || pending}
						onCheckedChange={handleToggle}
					/>
				</div>
			</CardContent>
		</Card>
	);
}
