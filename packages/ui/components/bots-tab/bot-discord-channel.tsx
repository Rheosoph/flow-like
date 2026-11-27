"use client";

import { useTranslation } from "@flow-like/locales";
import { ExternalLinkIcon, Loader2Icon, MessageCircleIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import {
	useBotDiscord,
	useRemoveBotDiscord,
	useSaveBotDiscord,
} from "../../hooks/use-bots";
import { getErrorMessage } from "../../lib/error-message";
import type {
	IBotDiscordSettings,
	IBotDiscordView,
} from "../../state/backend-state/bot-channels-state";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Switch } from "../ui/switch";
import { Textarea } from "../ui/textarea";

const DEFAULT_SETTINGS: IBotDiscordSettings = {
	enabled: false,
	respond_to_mentions: true,
	respond_to_dms: false,
	channel_allow: [],
};

const STATUS_VARIANTS = {
	stopped: "secondary",
	connecting: "secondary",
	connected: "default",
	error: "destructive",
} as const satisfies Record<IBotDiscordView["status"], string>;

export function parseChannelAllow(text: string): string[] {
	return [...new Set(text.split(/[\s,]+/).filter(Boolean))];
}

function DiscordStatus({ view }: Readonly<{ view: IBotDiscordView }>) {
	const { t } = useTranslation("bots");
	const labels: Record<IBotDiscordView["status"], string> = {
		stopped: t("panel.channels.discord.status.stopped", "Stopped"),
		connecting: t("panel.channels.discord.status.connecting", "Connecting"),
		connected: t("panel.channels.discord.status.connected", "Connected"),
		error: t("panel.channels.discord.status.error", "Error"),
	};
	return (
		<div className="flex flex-wrap items-center gap-2">
			<Badge variant={STATUS_VARIANTS[view.status]}>
				{view.status === "connecting" && (
					<Loader2Icon className="animate-spin" aria-hidden />
				)}
				{labels[view.status]}
			</Badge>
			{view.bot_user && (
				<span className="text-xs text-muted-foreground">
					{view.bot_user.name}
				</span>
			)}
			{view.status === "error" && view.error && (
				<span role="alert" className="text-xs text-destructive">
					{view.error}
				</span>
			)}
		</div>
	);
}

function SettingSwitch({
	label,
	checked,
	onChange,
}: Readonly<{
	label: string;
	checked: boolean;
	onChange: (checked: boolean) => void;
}>) {
	return (
		<Label className="flex items-center justify-between gap-3 text-sm font-normal">
			{label}
			<Switch checked={checked} onCheckedChange={onChange} />
		</Label>
	);
}

function DiscordHelp() {
	const { t } = useTranslation("bots");
	return (
		<ol className="list-decimal space-y-0.5 ps-4 text-xs text-muted-foreground">
			<li>
				{t(
					"panel.channels.discord.helpCreate",
					"Create an application in the Discord Developer Portal and add a bot.",
				)}
			</li>
			<li>
				{t(
					"panel.channels.discord.helpIntent",
					"Under Bot, turn on the Message Content intent.",
				)}
			</li>
			<li>
				{t(
					"panel.channels.discord.helpToken",
					"Copy the bot token and paste it here.",
				)}
			</li>
			<li>
				{t(
					"panel.channels.discord.helpInvite",
					"Save, then use the invite link to add the bot to your server.",
				)}
			</li>
		</ol>
	);
}

/** Discord relay of one bot; the desktop app holds the connection and the token. */
export function BotDiscordChannel({ botId }: Readonly<{ botId: string }>) {
	const { t } = useTranslation("bots");
	const discord = useBotDiscord(botId);
	const save = useSaveBotDiscord();
	const remove = useRemoveBotDiscord();
	const view = discord.data ?? null;
	const [settings, setSettings] =
		useState<IBotDiscordSettings>(DEFAULT_SETTINGS);
	const [allowText, setAllowText] = useState("");
	const [token, setToken] = useState("");

	const savedAllow = (view?.channel_allow ?? []).join("\n");
	useEffect(() => {
		if (discord.isPending) return;
		setSettings({
			enabled: view?.enabled ?? DEFAULT_SETTINGS.enabled,
			respond_to_mentions:
				view?.respond_to_mentions ?? DEFAULT_SETTINGS.respond_to_mentions,
			respond_to_dms: view?.respond_to_dms ?? DEFAULT_SETTINGS.respond_to_dms,
			channel_allow: [],
		});
		setAllowText(savedAllow);
	}, [
		discord.isPending,
		view?.enabled,
		view?.respond_to_mentions,
		view?.respond_to_dms,
		savedAllow,
	]);

	const patch = (change: Partial<IBotDiscordSettings>) =>
		setSettings((previous) => ({ ...previous, ...change }));
	const hasToken = view?.has_token === true;
	const canSave = !save.isPending && (hasToken || token.trim().length > 0);
	const submit = () =>
		save.mutate(
			{
				botId,
				input: {
					...settings,
					channel_allow: parseChannelAllow(allowText),
					token: token.trim() || null,
				},
			},
			{
				onSuccess: () => {
					setToken("");
					toast.success(t("panel.channels.discord.saved", "Discord saved"));
				},
				onError: (error) => toast.error(getErrorMessage(error)),
			},
		);
	const disconnect = () =>
		remove.mutate(
			{ botId },
			{
				onSuccess: () =>
					toast.success(
						t("panel.channels.discord.removed", "Discord disconnected"),
					),
				onError: (error) => toast.error(getErrorMessage(error)),
			},
		);

	return (
		<div
			className="flex flex-col gap-3 rounded-lg border p-3"
			data-slot="bot-channel-discord"
		>
			<div className="flex items-center gap-2">
				<MessageCircleIcon className="size-4 shrink-0" aria-hidden />
				<span className="text-sm font-medium">
					{t("panel.channels.discord.title", "Discord")}
				</span>
			</div>
			<p className="text-xs text-muted-foreground">
				{t(
					"panel.channels.discord.whileOpen",
					"Works while this desktop app is open. The token stays on this computer.",
				)}
			</p>
			<DiscordHelp />
			{view && <DiscordStatus view={view} />}
			<div className="flex flex-col gap-1.5">
				<Label htmlFor={`discord-token-${botId}`} className="text-xs">
					{t("panel.channels.discord.token", "Bot token")}
				</Label>
				<Input
					id={`discord-token-${botId}`}
					type="password"
					autoComplete="off"
					value={token}
					onChange={(event) => setToken(event.target.value)}
					placeholder={
						hasToken
							? t("panel.channels.discord.tokenSaved", "Saved")
							: t("panel.channels.discord.tokenPlaceholder", "Paste the token")
					}
				/>
			</div>
			<SettingSwitch
				label={t("panel.channels.discord.enabled", "Enabled")}
				checked={settings.enabled}
				onChange={(enabled) => patch({ enabled })}
			/>
			<SettingSwitch
				label={t("panel.channels.discord.mentions", "Answer when mentioned")}
				checked={settings.respond_to_mentions}
				onChange={(respond_to_mentions) => patch({ respond_to_mentions })}
			/>
			<SettingSwitch
				label={t("panel.channels.discord.dms", "Answer direct messages")}
				checked={settings.respond_to_dms}
				onChange={(respond_to_dms) => patch({ respond_to_dms })}
			/>
			<div className="flex flex-col gap-1.5">
				<Label htmlFor={`discord-allow-${botId}`} className="text-xs">
					{t("panel.channels.discord.allow", "Only these channels")}
				</Label>
				<Textarea
					id={`discord-allow-${botId}`}
					rows={2}
					value={allowText}
					onChange={(event) => setAllowText(event.target.value)}
					placeholder={t(
						"panel.channels.discord.allowPlaceholder",
						"Channel IDs, one per line. Empty allows every channel the bot can see.",
					)}
				/>
			</div>
			<div className="flex flex-wrap items-center gap-2">
				<Button type="button" size="sm" disabled={!canSave} onClick={submit}>
					{save.isPending && (
						<Loader2Icon className="animate-spin" aria-hidden />
					)}
					{t("panel.channels.discord.save", "Save")}
				</Button>
				{view?.invite_url && (
					<Button asChild type="button" size="sm" variant="outline">
						<a href={view.invite_url} target="_blank" rel="noreferrer">
							{t("panel.channels.discord.invite", "Invite to a server")}
							<ExternalLinkIcon aria-hidden />
						</a>
					</Button>
				)}
				{view && (
					<Button
						type="button"
						size="sm"
						variant="ghost"
						disabled={remove.isPending}
						onClick={disconnect}
					>
						{t("panel.channels.discord.disconnect", "Disconnect")}
					</Button>
				)}
			</div>
		</div>
	);
}
