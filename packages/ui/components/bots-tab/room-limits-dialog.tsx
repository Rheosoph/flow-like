"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo, useState } from "react";
import { useBotApps } from "../../hooks/use-bots";
import { apiErrorMessage } from "../../lib/api-error";
import type { IBotRoomLimits } from "../../state/backend-state/inbox-state";
import { Button } from "../ui/button";
import { Checkbox } from "../ui/checkbox";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { RestrictionsEditor, useAppNames } from "./bot-app-restrictions";
import {
	type RoomLimitsDraft,
	draftFromLimits,
	limitsFromDraft,
	toggleApp,
} from "./room-limits-model";

const NO_OPTIONS = { tables: [], events: [], boards: [] };

function RoomLimitsForm({
	botId,
	initial,
	confirmLabel,
	pending,
	error,
	onCancel,
	onConfirm,
}: Readonly<{
	botId: string;
	initial: IBotRoomLimits | null | undefined;
	confirmLabel: string;
	pending: boolean;
	error: unknown;
	onCancel: () => void;
	onConfirm: (limits: IBotRoomLimits | null) => void;
}>) {
	const { t } = useTranslation("bots");
	const [draft, setDraft] = useState<RoomLimitsDraft>(() =>
		draftFromLimits(initial),
	);
	const attached = useBotApps(botId);
	const names = useAppNames();
	const appIds = useMemo(
		() => (attached.data ?? []).map((app) => app.app_id),
		[attached.data],
	);
	return (
		<form
			className="flex min-h-0 flex-col gap-4"
			onSubmit={(event) => {
				event.preventDefault();
				onConfirm(limitsFromDraft(draft, initial));
			}}
		>
			<div className="flex max-h-[50dvh] flex-col gap-4 overflow-y-auto pr-1">
				<label className="flex items-start gap-2 text-sm">
					<Checkbox
						className="mt-0.5"
						checked={draft.mentionedOnly}
						disabled={pending}
						onCheckedChange={(on) =>
							setDraft((current) => ({
								...current,
								mentionedOnly: on === true,
							}))
						}
					/>
					<span className="flex flex-col gap-0.5">
						<span>
							{t("roomLimits.mentionedOnly", "Answer only when mentioned")}
						</span>
						<span className="text-xs text-muted-foreground">
							{t(
								"roomLimits.mentionedOnlyHint",
								"Without this, the bot reads along with messages from people in this room and decides itself whether to answer.",
							)}
						</span>
					</span>
				</label>
				<RestrictionsEditor
					value={draft.restrictions}
					options={NO_OPTIONS}
					disabled={pending}
					permissionsOnly
					onChange={(restrictions) =>
						setDraft((current) => ({ ...current, restrictions }))
					}
				/>
				{appIds.length > 0 && (
					<fieldset className="flex flex-col gap-2">
						<legend className="mb-1 text-xs text-muted-foreground">
							{t("roomLimits.apps", "Projects it may use in this room")}
						</legend>
						{appIds.map((appId) => (
							<label key={appId} className="flex items-center gap-2 text-sm">
								<Checkbox
									checked={draft.apps === null || draft.apps.includes(appId)}
									disabled={pending}
									onCheckedChange={(on) =>
										setDraft((current) =>
											toggleApp(current, appId, appIds, on === true),
										)
									}
								/>
								<span className="truncate">
									{names.get(appId) ||
										t("editor.abilities.appsUntitled", "Untitled app")}
								</span>
							</label>
						))}
					</fieldset>
				)}
			</div>
			{error !== null && error !== undefined && (
				<p role="alert" className="text-sm text-destructive">
					{apiErrorMessage(
						error,
						t("roomLimits.failed", "The room limits couldn't be saved."),
					)}
				</p>
			)}
			<DialogFooter>
				<Button type="button" variant="outline" onClick={onCancel}>
					{t("crew.cancel", "Cancel")}
				</Button>
				<Button type="submit" disabled={pending}>
					{confirmLabel}
				</Button>
			</DialogFooter>
		</form>
	);
}

/**
 * What a bot may do in one room, on top of its own restrictions. The bot's owner sets it when
 * accepting an invitation and can change it any time; it applies from the bot's next call.
 */
export function RoomLimitsDialog({
	open,
	onOpenChange,
	botId,
	botName,
	roomName,
	initial,
	confirmLabel,
	pending = false,
	error,
	onConfirm,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	botId: string;
	botName: string;
	roomName: string;
	initial?: IBotRoomLimits | null;
	confirmLabel: string;
	pending?: boolean;
	error?: unknown;
	onConfirm: (limits: IBotRoomLimits | null) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="flex max-h-[85dvh] flex-col sm:max-w-md">
				<DialogHeader>
					<DialogTitle>
						{t("roomLimits.title", "{{bot}} in {{room}}", {
							bot: botName,
							room: roomName,
						})}
					</DialogTitle>
					<DialogDescription>
						{t(
							"roomLimits.description",
							"Everyone in the room can wake this bot. It may do at most this here, and never more than you allow it elsewhere.",
						)}
					</DialogDescription>
				</DialogHeader>
				{open && (
					<RoomLimitsForm
						botId={botId}
						initial={initial}
						confirmLabel={confirmLabel}
						pending={pending}
						error={error}
						onCancel={() => onOpenChange(false)}
						onConfirm={onConfirm}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}
