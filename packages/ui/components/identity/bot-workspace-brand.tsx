"use client";

import { useTranslation } from "@flow-like/locales";
import { BotIcon, MessageSquareIcon, SettingsIcon } from "lucide-react";
import { useBot } from "../../hooks/use-bots";
import type { IApp } from "../../lib/schema/app/app";
import { cn } from "../../lib/utils";
import type { IBot } from "../../state/backend-state/bot-state";
import { BOT_LOCATION_ICONS, useBotLocationText } from "../bots-tab/bot-labels";
import { dockRoute } from "../bots-tab/dock-selection";
import { BotAvatar } from "../bots/bot-avatar";
import { Button } from "../ui/button";

/** The bot behind a workspace app, once it has loaded; `undefined` for every other app. */
export function useWorkspaceBot(
	app: Pick<IApp, "bot_id"> | null | undefined,
): IBot | undefined {
	const botId = app?.bot_id ?? undefined;
	return useBot(botId).data;
}

/** The bot's own look, marked with a small bot badge so it never reads as a person. */
export function BotWorkspaceAvatar({
	bot,
	size = 20,
	badge = true,
	animated = false,
	className,
}: Readonly<{
	bot: IBot;
	size?: number;
	badge?: boolean;
	animated?: boolean;
	className?: string;
}>) {
	const badgeSize = Math.max(10, Math.round(size * 0.4));
	return (
		<span
			data-slot="bot-workspace-avatar"
			className={cn("relative inline-flex shrink-0", className)}
			style={{ width: size, height: size }}
		>
			<BotAvatar
				appearance={bot.appearance}
				size={size}
				animated={animated}
				decorative
			/>
			{badge && (
				<span
					data-slot="bot-workspace-badge"
					className="absolute -right-0.5 -bottom-0.5 inline-flex items-center justify-center rounded-full border border-background bg-primary text-primary-foreground"
					style={{ width: badgeSize, height: badgeSize }}
				>
					<BotIcon
						style={{ width: badgeSize * 0.65, height: badgeSize * 0.65 }}
					/>
				</span>
			)}
		</span>
	);
}

/** Top of the app settings sidebar for a bot's workspace: who it belongs to, where it runs, and the two ways into the bot. */
export function BotWorkspaceBlock({
	bot,
	onNavigate,
}: Readonly<{ bot: IBot; onNavigate: (href: string) => void }>) {
	const { t } = useTranslation("bots");
	const locationText = useBotLocationText();
	const place = bot.local
		? {
				label: t("local.location.label", "This computer"),
				description: t(
					"local.location.description",
					"Runs in the desktop app on this computer while the app is open",
				),
			}
		: locationText(bot.location);
	const LocationIcon = BOT_LOCATION_ICONS[bot.location];
	const selection = { type: "bot", botId: bot.id } as const;
	const name = bot.name ?? t("workspace.brand.unnamed", "Bot");

	return (
		<section
			data-slot="bot-workspace-block"
			aria-label={t("workspace.brand.label", "Bot workspace")}
			className="mt-3 mb-1 flex flex-col gap-3 rounded-lg border bg-muted/30 p-3"
		>
			<div className="flex min-w-0 items-center gap-3">
				<BotWorkspaceAvatar bot={bot} size={48} animated />
				<div className="min-w-0 flex-1">
					<p className="truncate text-sm font-semibold" title={name}>
						{name}
					</p>
					<p className="text-xs text-muted-foreground">
						{t("workspace.brand.label", "Bot workspace")}
					</p>
					<p
						className="mt-0.5 flex min-w-0 items-center gap-1 text-xs text-muted-foreground"
						title={place.description}
					>
						<LocationIcon className="size-3 shrink-0" aria-hidden />
						<span className="truncate">{place.label}</span>
					</p>
				</div>
			</div>
			<div className="flex flex-col gap-1.5">
				<Button
					size="sm"
					className="justify-start gap-2"
					onClick={() => onNavigate(dockRoute(selection))}
				>
					<MessageSquareIcon className="size-3.5" />
					{t("workspace.brand.openChat", "Open chat")}
				</Button>
				<Button
					size="sm"
					variant="outline"
					className="justify-start gap-2"
					onClick={() => onNavigate(dockRoute(selection, "details"))}
				>
					<SettingsIcon className="size-3.5" />
					{t("workspace.brand.settings", "Bot settings")}
				</Button>
			</div>
		</section>
	);
}
