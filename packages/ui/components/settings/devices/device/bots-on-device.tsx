"use client";

import { useTranslation } from "@flow-like/locales";
import { Bot, Plus } from "lucide-react";
import { useMemo, useState } from "react";
import {
	botPresence,
	useBotPresence,
} from "../../../../hooks/use-bot-presence";
import { useBots } from "../../../../hooks/use-bots";
import { useBackend } from "../../../../state/backend-state";
import type { BotPresenceState } from "../../../../state/backend-state/bot-state";
import type { IBot } from "../../../../state/backend-state/bot-state";
import { useBotPresenceText } from "../../../bots-tab/bot-labels";
import {
	BOTS_ROUTE,
	dockSelectionQuery,
} from "../../../bots-tab/dock-selection";
import { BotEditorDialog } from "../../../bots-tab/editor";
import { BotAvatar } from "../../../bots/bot-avatar";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import {
	PresenceGlyph,
	type PresenceGlyphKind,
} from "../primitives/presence-glyph";
import { useHostLink } from "../routing/use-devices-route";
import type { DevicePage } from "./use-device-page";

const GLYPH: Record<BotPresenceState, PresenceGlyphKind> = {
	online: "online",
	offline: "offline",
	unknown: "never",
};

export const botHref = (botId: string) =>
	`${BOTS_ROUTE}?${dockSelectionQuery({ type: "bot", botId }, "")}`;

function BotRow({
	bot,
	state,
}: Readonly<{ bot: IBot; state: BotPresenceState }>) {
	const presenceText = useBotPresenceText();
	const hostLink = useHostLink();
	return (
		<li
			data-bot-on-device={bot.id}
			className="flex items-center gap-3 border-b border-border px-4 py-2.5 last:border-b-0"
		>
			<BotAvatar appearance={bot.appearance} size={32} decorative />
			<a
				{...hostLink(botHref(bot.id))}
				className="min-w-0 flex-1 truncate font-medium underline decoration-border-strong underline-offset-2 hover:decoration-current"
			>
				{bot.name}
			</a>
			<span
				data-bot-presence={state}
				className="inline-flex items-center gap-1.5 text-xs text-muted-foreground"
			>
				<PresenceGlyph kind={GLYPH[state]} decorative />
				{presenceText(state)}
			</span>
		</li>
	);
}

/** The person's bots that live on this device, with whether each one answers. */
export function BotsOnDevice({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const available = Boolean(useBackend().botState);
	const [creating, setCreating] = useState(false);
	const bots = useBots();
	const reports = useBotPresence(available).data;
	const here = useMemo(
		() =>
			(bots.data ?? []).filter(
				(bot) =>
					!bot.local &&
					!bot.system &&
					bot.location === "device" &&
					bot.device_id === page.deviceId,
			),
		[bots.data, page.deviceId],
	);
	if (!available) return null;
	return (
		<>
			<Block
				id="device-bots"
				icon={Bot}
				title={t("device.bots.title", "Bots on this device")}
				count={here.length}
				flush={here.length > 0}
				tools={
					<DvButton size="sm" icon={Plus} onClick={() => setCreating(true)}>
						{t("device.bots.create", "Create a bot on this device")}
					</DvButton>
				}
			>
				{here.length > 0 ? (
					<ul className="flex flex-col">
						{here.map((bot) => (
							<BotRow
								key={bot.id}
								bot={bot}
								state={botPresence(bot, reports)?.state ?? "unknown"}
							/>
						))}
					</ul>
				) : (
					<p className="text-ui text-muted-foreground">
						{bots.isLoading
							? t("device.bots.loading", "Loading bots…")
							: t(
									"device.bots.empty",
									"No bots live on {{device}} yet. Choose it as the host when you create a bot.",
									{ device: page.name },
								)}
					</p>
				)}
			</Block>
			<BotEditorDialog
				open={creating}
				onOpenChange={setCreating}
				initialDeviceId={page.deviceId}
			/>
		</>
	);
}
