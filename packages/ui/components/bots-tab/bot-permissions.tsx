"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2Icon, PlusIcon, XIcon } from "lucide-react";
import { useCallback, useMemo } from "react";
import {
	useBotLabels,
	useBotPermissions,
	useBots,
	useGrantBotPermission,
	useRevokeBotPermission,
} from "../../hooks/use-bots";
import { useConversations } from "../../hooks/use-inbox";
import type { IBot, IBotPermission } from "../../state/backend-state/bot-state";
import { BotAvatar } from "../bots/bot-avatar";
import { exchangesOfBot } from "../global-chat/bot-exchange";
import { BotExchangeList } from "../global-chat/bot-exchange-card";
import { Button } from "../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";

type BotNames = (botId: string) => { name: string; bot?: IBot };

function useBotNames(): BotNames {
	const { t } = useTranslation("bots");
	const labels = useBotLabels();
	const bots = useBots().data;
	const byId = useMemo(
		() => new Map((bots ?? []).map((bot) => [bot.id, bot])),
		[bots],
	);
	return useCallback(
		(botId) => {
			const bot = byId.get(botId);
			return {
				bot,
				name: (bot && labels(bot).name) || t("crew.unknownMember", "Someone"),
			};
		},
		[byId, labels, t],
	);
}

function GrantRow({
	grant,
	otherId,
	names,
	subjectName,
	direction,
	revoking,
	onRevoke,
}: Readonly<{
	grant: IBotPermission;
	otherId: string;
	names: BotNames;
	subjectName: string;
	direction: "granted" | "received";
	revoking: boolean;
	onRevoke: (grant: IBotPermission) => void;
}>) {
	const { t } = useTranslation("bots");
	const other = names(otherId);
	const [from, to] =
		direction === "granted"
			? [subjectName, other.name]
			: [other.name, subjectName];
	const label = t("panel.permissions.revoke", "Stop {{from}} asking {{to}}", {
		from,
		to,
	});
	return (
		<li className="flex items-center gap-2.5 rounded-lg bg-muted/60 py-1.5 pr-1.5 pl-2">
			<span className="flex size-7 shrink-0 items-end justify-center overflow-hidden rounded-md bg-muted">
				<BotAvatar
					appearance={other.bot?.appearance}
					size={28}
					animated={false}
					decorative
				/>
			</span>
			<span className="min-w-0 flex-1 truncate text-sm font-medium">
				{other.name}
			</span>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="size-7 shrink-0 text-muted-foreground"
				disabled={revoking}
				aria-label={label}
				title={label}
				onClick={() => onRevoke(grant)}
			>
				{revoking ? (
					<Loader2Icon className="animate-spin" aria-hidden />
				) : (
					<XIcon aria-hidden />
				)}
			</Button>
		</li>
	);
}

function GrantPicker({
	bot,
	name,
	taken,
	names,
}: Readonly<{
	bot: IBot;
	name: string;
	taken: ReadonlySet<string>;
	names: BotNames;
}>) {
	const { t } = useTranslation("bots");
	const bots = useBots().data;
	const { mutate: grant, isPending } = useGrantBotPermission();
	const candidates = (bots ?? []).filter(
		(other) => other.id !== bot.id && !taken.has(other.id),
	);
	if (candidates.length === 0) return null;
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="self-start"
					disabled={isPending}
				>
					{isPending ? (
						<Loader2Icon className="animate-spin" aria-hidden />
					) : (
						<PlusIcon aria-hidden />
					)}
					{t("panel.permissions.allow", "Allow {{name}} to ask…", { name })}
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="start">
				{candidates.map((other) => (
					<DropdownMenuItem
						key={other.id}
						onSelect={() => grant({ fromBotId: bot.id, toBotId: other.id })}
					>
						<span className="flex size-5 shrink-0 items-end justify-center overflow-hidden rounded bg-muted">
							<BotAvatar
								appearance={other.appearance}
								size={20}
								animated={false}
								decorative
							/>
						</span>
						{names(other.id).name}
					</DropdownMenuItem>
				))}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function GrantList({
	title,
	grants,
	direction,
	names,
	subjectName,
	revokingId,
	onRevoke,
}: Readonly<{
	title: string;
	grants: readonly IBotPermission[];
	direction: "granted" | "received";
	names: BotNames;
	subjectName: string;
	revokingId?: string;
	onRevoke: (grant: IBotPermission) => void;
}>) {
	if (grants.length === 0) return null;
	return (
		<div className="flex flex-col gap-1.5">
			<h4 className="text-xs text-muted-foreground">{title}</h4>
			<ul className="flex flex-col gap-1.5">
				{grants.map((grant) => (
					<GrantRow
						key={grant.id}
						grant={grant}
						otherId={
							direction === "granted" ? grant.to_bot_id : grant.from_bot_id
						}
						names={names}
						subjectName={subjectName}
						direction={direction}
						revoking={revokingId === grant.id}
						onRevoke={onRevoke}
					/>
				))}
			</ul>
		</div>
	);
}

/** The bots this bot may ask without asking you, and the bots that may ask it; each grant can be taken back. */
export function BotPermissions({
	bot,
	name,
}: Readonly<{ bot: IBot; name: string }>) {
	const { t } = useTranslation("bots");
	const permissions = useBotPermissions(bot.id);
	const { mutate: revoke, variables, isPending } = useRevokeBotPermission();
	const names = useBotNames();
	const granted = permissions.data?.granted ?? [];
	const received = permissions.data?.received ?? [];
	if (!permissions.data) return null;
	const revokingId = isPending
		? [...granted, ...received].find(
				(grant) =>
					grant.from_bot_id === variables?.fromBotId &&
					grant.to_bot_id === variables?.toBotId,
			)?.id
		: undefined;
	const onRevoke = (grant: IBotPermission) =>
		revoke({ fromBotId: grant.from_bot_id, toBotId: grant.to_bot_id });
	return (
		<section className="flex flex-col gap-3" data-slot="bot-permissions">
			<h3 className="text-xs text-muted-foreground">
				{t("panel.permissions.title", "Can ask")}
			</h3>
			{granted.length === 0 && received.length === 0 ? (
				<p className="text-sm text-muted-foreground">
					{t(
						"panel.permissions.none",
						"No standing permissions. {{name}} checks with you before asking another bot.",
						{ name },
					)}
				</p>
			) : (
				<>
					<GrantList
						title={t("panel.permissions.granted", "{{name}} can ask", {
							name,
						})}
						grants={granted}
						direction="granted"
						names={names}
						subjectName={name}
						revokingId={revokingId}
						onRevoke={onRevoke}
					/>
					<GrantList
						title={t("panel.permissions.received", "Can ask {{name}}", {
							name,
						})}
						grants={received}
						direction="received"
						names={names}
						subjectName={name}
						revokingId={revokingId}
						onRevoke={onRevoke}
					/>
				</>
			)}
			<GrantPicker
				bot={bot}
				name={name}
				names={names}
				taken={new Set(granted.map((grant) => grant.to_bot_id))}
			/>
		</section>
	);
}

/** The conversations in which this bot talked with another of its owner's bots. */
export function BotExchanges({
	bot,
	onOpen,
}: Readonly<{ bot: IBot; onOpen: (conversationId: string) => void }>) {
	const { t } = useTranslation("bots");
	const conversations = useConversations();
	const names = useBotNames();
	const entries = useMemo(
		() => exchangesOfBot(conversations.data ?? [], bot.id),
		[bot.id, conversations.data],
	);
	if (!conversations.data) return null;
	return (
		<section className="flex flex-col gap-3" data-slot="bot-exchanges">
			<h3 className="text-xs text-muted-foreground">
				{t("exchange.title", "Talked with")}
			</h3>
			<BotExchangeList
				entries={entries}
				nameOf={(botId) => names(botId).name}
				onOpen={onOpen}
			/>
		</section>
	);
}
