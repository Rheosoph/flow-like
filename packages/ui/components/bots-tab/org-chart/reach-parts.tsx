"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ClockIcon,
	EyeOffIcon,
	LockIcon,
	type LucideIcon,
	MessageCircleIcon,
	MessagesSquareIcon,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useUserIdentity } from "../../../hooks/use-user-lookup";
import {
	activeTeamOf,
	classificationOf,
	isGatedInRoom,
	reachOf,
	rootTeamId,
} from "../../../lib/bot-teams/org-model";
import { parseDataClassification } from "../../../lib/data-classification";
import { cn } from "../../../lib/utils";
import {
	type IOrgBot,
	type ITeam,
	MAX_CHAT_NOTE_CHARS,
	type TalkReach,
} from "../../../state/backend-state/bot-team-state";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Textarea } from "../../ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { useOrgChart } from "./org-chart-context";

type PillReach = Exclude<TalkReach, "own">;

const PILL_ICON: Partial<Record<PillReach, LucideIcon>> = {
	team_room: MessagesSquareIcon,
	lead: MessageCircleIcon,
	chat: MessageCircleIcon,
	requested: ClockIcon,
	classified: LockIcon,
};

const PILL_CLASS: Record<PillReach, string> = {
	team_room: "border-border bg-card text-muted-foreground hover:bg-muted",
	lead: "border-primary/40 bg-primary/10 text-primary hover:bg-primary/20",
	chat: "border-border bg-card text-foreground hover:bg-muted",
	request: "border-dashed border-border bg-card text-foreground hover:bg-muted",
	requested: "border-warning-line bg-warning-bg text-warning",
	declined: "border-border bg-muted text-muted-foreground",
	classified: "border-border bg-muted text-muted-foreground",
	unavailable: "border-dashed border-border text-muted-foreground",
};

export interface BotReachInfo {
	reach: TalkReach;
	label: string;
	tip: string;
	icon: LucideIcon | undefined;
	actionable: boolean;
	pending: boolean;
	requesting: boolean;
	run: () => void;
	closeRequest: () => void;
}

function useOpenThread() {
	const { actions, onOpenConversation } = useOrgChart();
	const open = useCallback(
		(teamId: string) =>
			actions.openThread.mutate(teamId, {
				onSuccess: (thread) => onOpenConversation(thread.conversation_id),
				onError: (error) => toast.error(error.message),
			}),
		[actions.openThread, onOpenConversation],
	);
	return { open, pending: actions.openThread.isPending };
}

export function useBotReach(bot: IOrgBot): BotReachInfo {
	const { t } = useTranslation("bots");
	const { org, me, nameOf, onOpenConversation } = useOrgChart();
	const openThread = useOpenThread();
	const owner = useUserIdentity(bot.owner_id);
	const [requesting, setRequesting] = useState(false);
	const reach = useMemo(() => reachOf(org, me, bot.id), [org, me, bot.id]);
	const team = activeTeamOf(org, bot.id);
	const name = nameOf(bot);
	const level = parseDataClassification(bot.classification) ?? "internal";
	const yours =
		me && team
			? classificationOf(org, rootTeamId(org, team.id) ?? team.id, me)
			: "internal";
	const conversationId = reach.conversation_id ?? null;
	const leadNeedsThread =
		reach.reach === "lead" && !conversationId && Boolean(team);

	const run = useCallback(() => {
		if (conversationId) return onOpenConversation(conversationId);
		if (reach.reach === "request") return setRequesting(true);
		if (leadNeedsThread && team) openThread.open(team.id);
	}, [
		conversationId,
		reach.reach,
		leadNeedsThread,
		team,
		openThread.open,
		onOpenConversation,
	]);

	const copy: Record<PillReach, { label: string; tip: string }> = {
		team_room: {
			label: t("teams.reach.teamRoom", "Team room"),
			tip: t(
				"teams.reach.teamRoomTip",
				"Talk to {{bot}} in the {{team}} team room",
				{ bot: name, team: team?.name ?? "" },
			),
		},
		lead: {
			label: t("teams.reach.talk", "Talk"),
			tip: t(
				"teams.reach.talkTip",
				"Talk to {{bot}}, the lead of {{team}}. Its turns are on {{owner}}.",
				{ bot: name, team: team?.name ?? "", owner: owner.label },
			),
		},
		chat: {
			label: t("teams.reach.chat", "Chat"),
			tip: t("teams.reach.chatTip", "Open your chat with {{bot}}", {
				bot: name,
			}),
		},
		request: {
			label: t("teams.reach.request", "Ask to chat"),
			tip: t(
				"teams.reach.requestTip",
				"Ask {{owner}} to let you chat with {{bot}}",
				{ owner: owner.label, bot: name },
			),
		},
		requested: {
			label: t("teams.reach.waiting", "Waiting"),
			tip: t("teams.reach.waitingTip", "Waiting for {{owner}}", {
				owner: owner.label,
			}),
		},
		declined: {
			label: t("teams.reach.declined", "Declined"),
			tip: t("teams.reach.declinedTip", "Ask again from {{date}}", {
				date: reach.retry_at
					? new Date(reach.retry_at).toLocaleDateString()
					: "",
			}),
		},
		classified: {
			label: t("teams.reach.classified", "Classified"),
			tip: t(
				"teams.reach.classifiedTip",
				"{{bot}} is {{level}}, above your classification ({{yours}})",
				{ bot: name, level, yours },
			),
		},
		unavailable: {
			label: t("teams.reach.joining", "Joining"),
			tip: t(
				"teams.reach.joiningTip",
				"{{bot}} has not joined yet: its owner has to accept first",
				{ bot: name },
			),
		},
	};
	const entry = reach.reach === "own" ? undefined : copy[reach.reach];
	return {
		reach: reach.reach,
		label: entry?.label ?? "",
		tip: entry?.tip ?? "",
		icon: reach.reach === "own" ? undefined : PILL_ICON[reach.reach],
		actionable:
			Boolean(conversationId) || reach.reach === "request" || leadNeedsThread,
		pending: openThread.pending,
		requesting,
		run,
		closeRequest: () => setRequesting(false),
	};
}

export function ReachPill({ bot }: Readonly<{ bot: IOrgBot }>) {
	const info = useBotReach(bot);
	if (info.reach === "own") return null;
	const Icon = info.icon;
	const className = cn(
		"inline-flex shrink-0 items-center gap-1 rounded-full border px-1.5 text-[10px] leading-4 font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring",
		PILL_CLASS[info.reach],
	);
	const content = (
		<>
			{Icon && <Icon className="size-2.5" aria-hidden />}
			{info.label}
		</>
	);
	return (
		<>
			<Tooltip>
				<TooltipTrigger asChild>
					{info.actionable ? (
						<button
							type="button"
							aria-label={info.tip}
							disabled={info.pending}
							onClick={info.run}
							className={className}
						>
							{content}
						</button>
					) : (
						<span role="img" aria-label={info.tip} className={className}>
							{content}
						</span>
					)}
				</TooltipTrigger>
				<TooltipContent>{info.tip}</TooltipContent>
			</Tooltip>
			{info.requesting && (
				<ChatRequestDialog bot={bot} onClose={info.closeRequest} />
			)}
		</>
	);
}

export function GatedMark({
	team,
	bot,
}: Readonly<{ team: ITeam; bot: IOrgBot }>) {
	const { t } = useTranslation("bots");
	if (!isGatedInRoom(team, bot.id)) return null;
	const level = parseDataClassification(bot.classification) ?? "internal";
	const tip = t(
		"teams.reach.gatedTip",
		"Doesn't answer in the team room: someone there isn't classified for {{level}}",
		{ level },
	);
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span
					role="img"
					aria-label={tip}
					className="inline-flex shrink-0 text-muted-foreground"
				>
					<EyeOffIcon className="size-3" aria-hidden />
				</span>
			</TooltipTrigger>
			<TooltipContent>{tip}</TooltipContent>
		</Tooltip>
	);
}

export function ChatRequestDialog({
	bot,
	onClose,
}: Readonly<{ bot: IOrgBot; onClose: () => void }>) {
	const { t } = useTranslation("bots");
	const { nameOf, actions } = useOrgChart();
	const owner = useUserIdentity(bot.owner_id);
	const [note, setNote] = useState("");
	const send = () =>
		actions.requestChat.mutate(
			{ botId: bot.id, note: note.trim() || null },
			{
				onSuccess: () => {
					toast.success(
						t("teams.reach.requestSent", "Request sent to {{owner}}", {
							owner: owner.label,
						}),
					);
					onClose();
				},
				onError: (error) => toast.error(error.message),
			},
		);
	return (
		<Dialog open onOpenChange={(open) => !open && onClose()}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>
						{t("teams.reach.requestTitle", "Ask to chat with {{bot}}", {
							bot: nameOf(bot),
						})}
					</DialogTitle>
					<DialogDescription>
						{t(
							"teams.reach.requestBody",
							"{{owner}} decides and sits in the chat to watch. Say what you need.",
							{ owner: owner.label },
						)}
					</DialogDescription>
				</DialogHeader>
				<div className="flex flex-col gap-1">
					<Textarea
						value={note}
						rows={4}
						maxLength={MAX_CHAT_NOTE_CHARS}
						aria-label={t("teams.reach.noteLabel", "Note")}
						placeholder={t(
							"teams.reach.notePlaceholder",
							"Optional: what would you like to talk about?",
						)}
						onChange={(event) => setNote(event.target.value)}
					/>
					<span className="self-end text-[11px] text-muted-foreground tabular-nums">
						{note.length}/{MAX_CHAT_NOTE_CHARS}
					</span>
				</div>
				<DialogFooter>
					<Button variant="ghost" onClick={onClose}>
						{t("teams.org.cancel", "Cancel")}
					</Button>
					<Button disabled={actions.requestChat.isPending} onClick={send}>
						{t("teams.reach.sendRequest", "Send request")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

export function ReachAction({ bot }: Readonly<{ bot: IOrgBot }>) {
	const info = useBotReach(bot);
	if (info.reach === "own") return null;
	const Icon = info.icon;
	return (
		<div className="flex flex-col gap-1.5">
			{info.actionable ? (
				<Button
					size="sm"
					className="self-start"
					disabled={info.pending}
					onClick={info.run}
				>
					{Icon && <Icon aria-hidden />}
					{info.label}
				</Button>
			) : (
				<span
					className={cn(
						"inline-flex items-center gap-1 self-start rounded-full border px-2 py-0.5 text-xs font-medium",
						PILL_CLASS[info.reach],
					)}
				>
					{Icon && <Icon className="size-3" aria-hidden />}
					{info.label}
				</span>
			)}
			<p className="text-xs text-muted-foreground">{info.tip}</p>
			{info.requesting && (
				<ChatRequestDialog bot={bot} onClose={info.closeRequest} />
			)}
		</div>
	);
}

export function TeamReachLine({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { org, me, botOf, onOpenConversation } = useOrgChart();
	const openThread = useOpenThread();
	const lead = team.bots.find(
		(entry) => entry.bot_id === team.lead_bot_id && entry.status === "active",
	);
	const leadBot = lead ? botOf(lead.bot_id) : undefined;
	const thread = org.threads?.find((entry) => entry.team_id === team.id);
	const reach = leadBot ? reachOf(org, me, leadBot.id).reach : null;
	const inTeam = team.people.some(
		(person) => person.user_id === me && person.status === "active",
	);
	const talk = () => {
		if (inTeam && team.conversation_id) {
			return onOpenConversation(team.conversation_id);
		}
		if (thread) return onOpenConversation(thread.conversation_id);
		openThread.open(team.id);
	};
	if (!leadBot) {
		return (
			<p className="px-1 text-[11px] text-muted-foreground">
				{t(
					"teams.reach.noLead",
					"No lead: this team can't be contacted directly",
				)}
			</p>
		);
	}
	if (reach === "classified") {
		const level = parseDataClassification(leadBot.classification) ?? "internal";
		return (
			<p className="flex items-center gap-1 px-1 text-[11px] text-muted-foreground">
				<LockIcon className="size-3 shrink-0" aria-hidden />
				{t("teams.reach.leadClassified", "Lead classified {{level}}", {
					level,
				})}
			</p>
		);
	}
	if (reach === "unavailable") return null;
	return (
		<div className="px-1">
			<Button
				variant="outline"
				size="sm"
				className="h-6 gap-1 rounded-full px-2 text-[11px]"
				disabled={openThread.pending}
				onClick={talk}
			>
				<MessageCircleIcon className="size-3" aria-hidden />
				{t("teams.reach.talkToTeam", "Talk to {{team}}", { team: team.name })}
			</Button>
		</div>
	);
}
