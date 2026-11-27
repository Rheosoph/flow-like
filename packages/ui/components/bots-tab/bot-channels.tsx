"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import {
	ChevronDownIcon,
	Loader2Icon,
	MessageCircleIcon,
	UnlinkIcon,
	UsersIcon,
} from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import {
	botKeys,
	useBotChannelLinks,
	useBotTeams,
	useDeleteBotChannelLink,
} from "../../hooks/use-bots";
import { ApiResponseError } from "../../lib/api-error";
import { formatRelativeTime } from "../../lib/date";
import { getErrorMessage } from "../../lib/error-message";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type {
	BotChannelProvider,
	IBotChannelLink,
} from "../../state/backend-state/bot-channels-state";
import type { IBot } from "../../state/backend-state/bot-state";
import { type TeamsAdapter, TeamsSetupForm } from "../interfaces/configs/teams";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { BotDiscordChannel } from "./bot-discord-channel";

const UNSUPPORTED_STATUSES = new Set([403, 404, 501]);

/** Whether the Details tab offers outside chats for this bot. */
export function offersChannels(bot: IBot): boolean {
	return !bot.local && bot.system !== "flux";
}

function useBotTeamsAdapter(botId: string): TeamsAdapter {
	const state = useBackend().botState;
	const queryClient = useQueryClient();
	const setupFn = state?.setupTeams?.bind(state);
	const disconnectFn = state?.disconnectTeams?.bind(state);
	const packageFn = state?.getTeamsPackage?.bind(state);
	const rotateFn = state?.rotateTeamsSecret?.bind(state);
	const accessFn = state?.getTeamsAccess?.bind(state);
	const accessKey = useMemo(() => botKeys.teamsAccess(botId), [botId]);
	return useMemo(
		() => ({
			scope: `bot/${botId}`,
			setup: setupFn && ((input) => setupFn(botId, input)),
			disconnect: disconnectFn && (() => disconnectFn(botId)),
			package: packageFn && (() => packageFn(botId)),
			rotate: rotateFn && (() => rotateFn(botId)),
			access: accessFn && (() => accessFn(botId)),
			accessKey,
			refreshConnection: () =>
				queryClient.invalidateQueries({ queryKey: botKeys.teams(botId) }),
			refreshAccess: () =>
				queryClient.invalidateQueries({ queryKey: accessKey }),
		}),
		[
			botId,
			setupFn,
			disconnectFn,
			packageFn,
			rotateFn,
			accessFn,
			accessKey,
			queryClient,
		],
	);
}

function TeamsChannel({ botId }: Readonly<{ botId: string }>) {
	const { t } = useTranslation("bots");
	const connection = useBotTeams(botId);
	const adapter = useBotTeamsAdapter(botId);
	const [open, setOpen] = useState(false);
	const error = connection.error;
	if (
		error instanceof ApiResponseError &&
		UNSUPPORTED_STATUSES.has(error.status)
	) {
		return null;
	}
	const saved = connection.data;
	const connected = saved?.configured === true && saved.status === "ready";
	return (
		<div
			className="flex flex-col gap-3 rounded-lg border p-3"
			data-slot="bot-channel-teams"
		>
			<div className="flex items-center justify-between gap-2">
				<div className="flex min-w-0 items-center gap-2">
					<UsersIcon className="size-4 shrink-0" aria-hidden />
					<span className="truncate text-sm font-medium">
						{t("panel.channels.teams.title", "Microsoft Teams")}
					</span>
					{saved && (
						<Badge variant={connected ? "default" : "secondary"}>
							{connected
								? t("panel.channels.teams.connected", "Connected")
								: t("panel.channels.teams.notConnected", "Not connected")}
						</Badge>
					)}
				</div>
				<Button
					type="button"
					variant="ghost"
					size="sm"
					aria-expanded={open}
					disabled={!saved}
					onClick={() => setOpen((value) => !value)}
				>
					{connection.isPending ? (
						<Loader2Icon className="animate-spin" aria-hidden />
					) : (
						<ChevronDownIcon
							className={cn("transition-transform", open && "rotate-180")}
							aria-hidden
						/>
					)}
					{open
						? t("panel.channels.teams.hide", "Hide")
						: t("panel.channels.teams.setup", "Set up")}
				</Button>
			</div>
			{error && (
				<p role="alert" className="text-xs text-destructive">
					{getErrorMessage(error)}
				</p>
			)}
			{open && saved && (
				<TeamsSetupForm
					adapter={adapter}
					isEditing
					saved={saved}
					refreshing={connection.isFetching}
					onRefresh={() => connection.refetch()}
					intro={t(
						"panel.channels.teams.intro",
						"Add this bot to Microsoft Teams. Chats you add it to appear below; the server answers, so your desktop can be closed.",
					)}
				/>
			)}
		</div>
	);
}

const PROVIDER_ICONS: Record<BotChannelProvider, typeof UsersIcon> = {
	teams: UsersIcon,
	discord: MessageCircleIcon,
};

function useProviderLabel() {
	const { t } = useTranslation("bots");
	return (provider: BotChannelProvider) =>
		provider === "teams"
			? t("panel.channels.providers.teams", "Microsoft Teams")
			: t("panel.channels.providers.discord", "Discord");
}

function ChannelLinkRow({
	link,
	onOpen,
	onUnlink,
}: Readonly<{
	link: IBotChannelLink;
	onOpen?: (conversationId: string) => void;
	onUnlink: (link: IBotChannelLink) => void;
}>) {
	const { t } = useTranslation("bots");
	const providerLabel = useProviderLabel();
	const Icon = PROVIDER_ICONS[link.provider] ?? MessageCircleIcon;
	const lastSeen = formatRelativeTime(link.last_seen_at, "long", "");
	return (
		<li className="flex items-center gap-2 rounded-lg border px-3 py-2">
			<Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
			<button
				type="button"
				disabled={!onOpen}
				className="flex min-w-0 flex-1 flex-col text-start enabled:hover:underline"
				onClick={() => onOpen?.(link.conversation_id)}
			>
				<span className="truncate text-sm">{link.chat_title}</span>
				<span className="truncate text-xs text-muted-foreground">
					{lastSeen
						? t(
								"panel.channels.links.lastSeen",
								"{{provider}}, last seen {{when}}",
								{
									provider: providerLabel(link.provider),
									when: lastSeen,
								},
							)
						: providerLabel(link.provider)}
				</span>
			</button>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="size-8 shrink-0"
				aria-label={t("panel.channels.links.unlink", "Unlink {{title}}", {
					title: link.chat_title,
				})}
				onClick={() => onUnlink(link)}
			>
				<UnlinkIcon aria-hidden />
			</Button>
		</li>
	);
}

function ChannelLinks({
	botId,
	onOpen,
}: Readonly<{ botId: string; onOpen?: (conversationId: string) => void }>) {
	const { t } = useTranslation("bots");
	const links = useBotChannelLinks(botId);
	const remove = useDeleteBotChannelLink();
	const [pending, setPending] = useState<IBotChannelLink | null>(null);
	const items = links.data ?? [];
	if (links.isError) return null;
	return (
		<div className="flex flex-col gap-2" data-slot="bot-channel-links">
			<h4 className="text-xs text-muted-foreground">
				{t("panel.channels.links.title", "Linked chats")}
			</h4>
			{items.length === 0 ? (
				<p className="text-xs text-muted-foreground">
					{links.isPending
						? t("panel.channels.links.loading", "Loading...")
						: t(
								"panel.channels.links.empty",
								"No chats yet. Each chat the bot is added to opens its own room here.",
							)}
				</p>
			) : (
				<ul className="flex flex-col gap-1.5">
					{items.map((link) => (
						<ChannelLinkRow
							key={link.id}
							link={link}
							onOpen={onOpen}
							onUnlink={setPending}
						/>
					))}
				</ul>
			)}
			<AlertDialog
				open={pending !== null}
				onOpenChange={(open) => {
					if (!open) setPending(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("panel.channels.links.unlinkTitle", "Unlink this chat?")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"panel.channels.links.unlinkBody",
								"The room stays in your list. The bot no longer answers in {{title}} until someone writes to it again, which opens a new room.",
								{ title: pending?.chat_title ?? "" },
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("panel.channels.links.cancel", "Cancel")}
						</AlertDialogCancel>
						<AlertDialogAction
							onClick={() => {
								if (!pending) return;
								remove.mutate(
									{ botId, linkId: pending.id },
									{ onError: (error) => toast.error(getErrorMessage(error)) },
								);
								setPending(null);
							}}
						>
							{t("panel.channels.links.unlinkConfirm", "Unlink")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}

/** Microsoft Teams, Discord and the chats outside people reached the bot from. */
export function BotChannels({
	bot,
	onOpenConversation,
}: Readonly<{
	bot: IBot;
	onOpenConversation?: (conversationId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const backend = useBackend();
	const hasTeams = Boolean(backend.botState?.getTeams);
	const hasLinks = Boolean(backend.botState?.listChannelLinks);
	if (!hasTeams && !hasLinks && !backend.botDiscordState) return null;
	return (
		<section className="flex flex-col gap-3" data-slot="bot-channels">
			<h3 className="text-xs text-muted-foreground">
				{t("panel.channels.title", "Channels")}
			</h3>
			<p className="text-xs text-muted-foreground">
				{t(
					"panel.channels.guestNote",
					"People in these chats are guests. The bot cannot use your memory or your apps there, and tool approvals come to you.",
				)}
			</p>
			{hasTeams && <TeamsChannel botId={bot.id} />}
			{backend.botDiscordState ? (
				<BotDiscordChannel botId={bot.id} />
			) : (
				<p className="text-xs text-muted-foreground">
					{t(
						"panel.channels.discord.desktopOnly",
						"Discord works from the desktop app.",
					)}
				</p>
			)}
			{hasLinks && <ChannelLinks botId={bot.id} onOpen={onOpenConversation} />}
		</section>
	);
}
