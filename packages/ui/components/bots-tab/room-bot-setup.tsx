"use client";

import { useTranslation } from "@flow-like/locales";
import { BotIcon } from "lucide-react";
import type { IBotRoomLimits } from "../../state/backend-state/inbox-state";
import type { PickedPrincipal } from "./principal-picker";
import { draftFromLimits, limitsFromDraft } from "./room-limits-model";
import { RoomMemorySettings } from "./room-memory-settings";

export function ownRoomBots(picked: readonly PickedPrincipal[]) {
	return picked.filter(
		(principal) => principal.kind === "bot" && principal.mode !== "invite",
	);
}

export function selectedBotLimits(
	picked: readonly PickedPrincipal[],
	limits: Readonly<Record<string, IBotRoomLimits>>,
): Record<string, IBotRoomLimits> {
	return Object.fromEntries(
		ownRoomBots(picked).map((bot) => [
			bot.id,
			limits[bot.id] ?? limitsFromDraft(draftFromLimits(null)) ?? {},
		]),
	);
}

/** The same memory choices are reviewed before an owned bot is added or a room is created. */
export function RoomBotSetup({
	bots,
	limits,
	disabled,
	onChange,
}: Readonly<{
	bots: readonly PickedPrincipal[];
	limits: Readonly<Record<string, IBotRoomLimits>>;
	disabled?: boolean;
	onChange: (id: string, limits: IBotRoomLimits) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<div className="flex flex-col gap-5">
			<div className="space-y-1">
				<h3 className="text-sm font-semibold">
					{t("roomLimits.setup.title", "Set up your bots")}
				</h3>
				<p className="text-xs leading-relaxed text-muted-foreground">
					{t(
						"roomLimits.setup.hint",
						"Each bot starts with this group's memory only. Choose any knowledge it can bring in or share, then add it to the group.",
					)}
				</p>
			</div>
			{bots.map((bot) => {
				const draft = draftFromLimits(limits[bot.id]);
				return (
					<fieldset
						key={bot.id}
						className="space-y-2"
						aria-label={t(
							"roomLimits.setup.named",
							"Memory settings for {{bot}}",
							{ bot: bot.label },
						)}
					>
						<div className="flex items-center gap-2 px-1 text-sm font-medium">
							<BotIcon className="size-4 text-muted-foreground" aria-hidden />
							<span className="truncate">{bot.label}</span>
						</div>
						<RoomMemorySettings
							value={draft}
							disabled={disabled}
							onChange={(memory) =>
								onChange(
									bot.id,
									limitsFromDraft({ ...draft, ...memory }, limits[bot.id]) ??
										{},
								)
							}
						/>
					</fieldset>
				);
			})}
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(
					"roomLimits.setup.later",
					"You can change these choices later in the group's bot settings.",
				)}
			</p>
		</div>
	);
}
