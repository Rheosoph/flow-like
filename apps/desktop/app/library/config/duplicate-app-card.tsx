"use client";

import {
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Input,
	Label,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { CopyIcon, Loader2Icon } from "lucide-react";
import { useCallback, useId, useState } from "react";
import { useDuplicateLocalApp } from "../../../lib/use-duplicate-local-app";

interface DuplicateAppCardProps {
	appId: string;
	appName: string;
}

/**
 * Offline → offline copy on this device. The name is edited inline rather
 * than in a dialog because this card already lives inside the settings modal.
 */
export function DuplicateAppCard({
	appId,
	appName,
}: Readonly<DuplicateAppCardProps>) {
	const { t } = useTranslation("common");
	const inputId = useId();
	const { duplicate, isDuplicating } = useDuplicateLocalApp();
	/** `undefined` keeps the suggestion in sync with the app's current name. */
	const [editedName, setEditedName] = useState<string>();
	const name =
		editedName ?? t("nameCopy", "{{name}} (copy)", { name: appName });
	const canDuplicate = name.trim().length > 0 && !isDuplicating;

	const handleDuplicate = useCallback(async () => {
		await duplicate(appId, name.trim());
		setEditedName(undefined);
	}, [appId, duplicate, name]);

	return (
		<Card>
			<CardHeader className="space-y-1">
				<CardTitle className="flex items-center gap-2">
					<CopyIcon className="w-4 h-4" />
					{t("duplicateOnThisDevice", "Duplicate on this device")}
				</CardTitle>
				<CardDescription>
					{t(
						"makeAnIndependentOfflineCopyWithItsFlowsPagesDataAndSecretsTheOriginalStaysUnchanged",
						"Make an independent offline copy with its flows, pages, data and secrets. The original stays unchanged.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="space-y-2">
					<Label htmlFor={inputId}>{t("copyName", "Name of the copy")}</Label>
					<Input
						id={inputId}
						value={name}
						disabled={isDuplicating}
						onChange={(event) => setEditedName(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter" && canDuplicate) handleDuplicate();
						}}
					/>
				</div>
				<p className="text-xs text-muted-foreground">
					{t(
						"theCopyStartsWithAFreshVersionHistorySchedulesWebhooksAndBotsStayOffUntilYouSaveTheirEventsInTheCopy",
						"The copy starts with a fresh version history. Schedules, webhooks and bots stay off until you save their events in the copy.",
					)}
				</p>
				<Button
					onClick={handleDuplicate}
					disabled={!canDuplicate}
					className="gap-2"
				>
					{isDuplicating ? (
						<Loader2Icon className="w-4 h-4 animate-spin" />
					) : (
						<CopyIcon className="w-4 h-4" />
					)}
					{isDuplicating
						? t("duplicating", "Duplicating…")
						: t("duplicate", "Duplicate")}
				</Button>
			</CardContent>
		</Card>
	);
}
