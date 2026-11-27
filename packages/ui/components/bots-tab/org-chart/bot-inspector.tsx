"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowDownIcon, MessageSquareIcon, UsersIcon } from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { useTeamBriefing } from "../../../hooks/use-bot-teams";
import { useUserIdentity } from "../../../hooks/use-user-lookup";
import {
	activeTeamOf,
	leadOfferFor,
	rootTeamId,
	teamById,
} from "../../../lib/bot-teams/org-model";
import { parseDataClassification } from "../../../lib/data-classification";
import type {
	IOrgBot,
	ITeam,
} from "../../../state/backend-state/bot-team-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Button } from "../../ui/button";
import { DataClassificationBadge } from "../../ui/data-classification";
import { Skeleton } from "../../ui/skeleton";
import { Chip, InspectorSection } from "./inspector-parts";
import { BotIoSection } from "./io-inspector";
import { useOrgChart } from "./org-chart-context";
import {
	ASK_GROUP_ORDER,
	type AskGroupKey,
	briefingLineCount,
	canTakeOut,
	groupAsks,
	leadChain,
	moveTargets,
	parseBriefing,
	previewBriefing,
} from "./org-chart-model";
import { AskBadge, LeadStar, OrgBotAvatar } from "./org-chart-parts";
import { ReachAction } from "./reach-parts";

const PREVIEW_LINES = 8;
const LEAD_TURNS_PER_PERSON_A_DAY = 20;

function BotChip({ bot }: Readonly<{ bot: IOrgBot }>) {
	const { select, nameOf } = useOrgChart();
	return (
		<Chip onClick={() => select({ kind: "bot", id: bot.id })}>
			<OrgBotAvatar bot={bot} size={20} />
			<span className="truncate">{nameOf(bot)}</span>
		</Chip>
	);
}

function ReportingLine({ botId }: Readonly<{ botId: string }>) {
	const { t } = useTranslation("bots");
	const { org, select, nameOf } = useOrgChart();
	const chain = leadChain(org, botId);
	return (
		<InspectorSection title={t("teams.org.reportingLine", "Reporting line")}>
			<ol className="flex flex-col">
				{chain.map((bot, index) => (
					<li key={bot.id} className="flex flex-col">
						{index > 0 && (
							<ArrowDownIcon
								className="my-0.5 ml-3 size-3 text-muted-foreground"
								aria-hidden
							/>
						)}
						<button
							type="button"
							onClick={() => select({ kind: "bot", id: bot.id })}
							className="flex items-center gap-2 rounded-lg px-1 py-1 text-left text-sm outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
						>
							<OrgBotAvatar bot={bot} size={24} />
							<span className="truncate">{nameOf(bot)}</span>
							{index > 0 && <LeadStar />}
						</button>
					</li>
				))}
			</ol>
			{chain.length === 1 && (
				<p className="text-xs text-muted-foreground">
					{t(
						"teams.org.noLead",
						"Nobody above this bot yet: its team has no lead.",
					)}
				</p>
			)}
		</InspectorSection>
	);
}

function AskLists({ bot }: Readonly<{ bot: IOrgBot }>) {
	const { t } = useTranslation("bots");
	const { org, nameOf } = useOrgChart();
	const groups = useMemo(() => groupAsks(org, bot.id), [org, bot.id]);
	const hints: Record<AskGroupKey, string> = {
		direct: t("teams.org.ask.directHint", "Asks go through without a card."),
		ok: t("teams.org.ask.okHint", "Its owner allows the first question."),
		via: t("teams.org.ask.viaHint", "Ask their team's lead, who hands it on."),
		blocked: t("teams.org.ask.blockedHint", "Classified higher than this bot."),
		later: t("teams.org.ask.laterHint", "Not accepted by their owner yet."),
	};
	return (
		<InspectorSection
			title={t("teams.org.whoCanAsk", "Who {{name}} can ask", {
				name: nameOf(bot),
			})}
		>
			{ASK_GROUP_ORDER.filter((key) => groups[key].length > 0).map((key) => (
				<div key={key} className="flex flex-col gap-1.5">
					<div className="flex items-center gap-2">
						<AskBadge status={key} />
						<span className="text-xs text-muted-foreground">{hints[key]}</span>
					</div>
					<ul className="flex flex-wrap gap-1.5">
						{groups[key].map((other) => (
							<li key={other.id} className="max-w-full">
								<BotChip bot={other} />
							</li>
						))}
					</ul>
				</div>
			))}
			{groups.none.length > 0 && (
				<p className="text-xs text-muted-foreground">
					{t(
						"teams.org.outOfReach",
						"{{count}} more bots are out of reach: their team's lead doesn't take questions from this bot.",
						{ count: groups.none.length },
					)}
				</p>
			)}
		</InspectorSection>
	);
}

function BriefingBlock({ botId }: Readonly<{ botId: string }>) {
	const { t } = useTranslation("bots");
	const briefing = useTeamBriefing(botId);
	const [expanded, setExpanded] = useState(false);
	const sections = useMemo(
		() => parseBriefing(briefing.data?.text ?? ""),
		[briefing.data?.text],
	);
	const total = briefingLineCount(sections);
	const shown = expanded ? sections : previewBriefing(sections, PREVIEW_LINES);
	return (
		<InspectorSection
			title={t("teams.org.reads", "What it reads")}
			hint={t(
				"teams.org.readsHint",
				"The exact text this bot gets about its team before every turn.",
			)}
		>
			{briefing.isLoading ? (
				<Skeleton className="h-20 w-full" />
			) : sections.length === 0 ? (
				<p className="text-xs text-muted-foreground">
					{t("teams.org.readsEmpty", "Nothing: this bot is not in a team.")}
				</p>
			) : (
				<div className="flex flex-col gap-2 rounded-lg border bg-muted/40 p-3">
					{shown.map((section, index) => (
						<div key={`${section.heading ?? ""}-${index}`}>
							{section.heading && (
								<h4 className="mb-1 text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
									{section.heading}
								</h4>
							)}
							<ul className="flex flex-col gap-0.5 text-xs">
								{section.lines.map((line, lineIndex) =>
									line.kind === "item" ? (
										<li
											key={`${lineIndex}-${line.text}`}
											className="list-inside list-disc"
											style={{ paddingInlineStart: `${line.depth * 0.875}rem` }}
										>
											{line.text}
										</li>
									) : (
										<li key={`${lineIndex}-${line.text}`} className="list-none">
											{line.text}
										</li>
									),
								)}
							</ul>
						</div>
					))}
					{total > PREVIEW_LINES && (
						<Button
							variant="link"
							size="sm"
							className="h-auto self-start p-0 text-xs"
							onClick={() => setExpanded((value) => !value)}
						>
							{expanded
								? t("teams.org.showLess", "Show less")
								: t("teams.org.showAll", "Show all")}
						</Button>
					)}
				</div>
			)}
		</InspectorSection>
	);
}

function MoveTo({ bot }: Readonly<{ bot: IOrgBot }>) {
	const { t } = useTranslation("bots");
	const { org, me, placeBot } = useOrgChart();
	const targets = moveTargets(org, bot, me);
	if (targets.length === 0) return null;
	const own = bot.owner_id === me;
	return (
		<InspectorSection
			title={t("teams.org.moveTo", "Move to")}
			hint={
				own
					? t("teams.org.moveToOwn", "Your bot moves right away.")
					: t(
							"teams.org.moveToOther",
							"You invite it; its owner accepts before it moves.",
						)
			}
		>
			<ul className="flex flex-wrap gap-1.5">
				{targets.map(({ team }) => (
					<li key={team.id}>
						<Button
							variant="outline"
							size="sm"
							className="h-7 rounded-full text-xs"
							onClick={() => placeBot(team, bot)}
						>
							{team.name}
						</Button>
					</li>
				))}
			</ul>
		</InspectorSection>
	);
}

function LeadOfferCard({ team, bot }: Readonly<{ team: ITeam; bot: IOrgBot }>) {
	const { t } = useTranslation("bots");
	const { org, nameOf, actions } = useOrgChart();
	const orgName = teamById(org, rootTeamId(org, team.id) ?? team.id)?.name;
	const args = { teamId: team.id, botId: bot.id };
	const fail = { onError: (error: Error) => toast.error(error.message) };
	return (
		<section className="flex flex-col gap-3 rounded-lg border border-primary/40 bg-primary/5 p-3">
			<p className="text-sm">
				{t(
					"teams.reach.leadOffer",
					"{{team}} asks {{bot}} to lead it. Everyone in {{org}} can then talk to it, and its turns are on you (up to {{limit}} per person a day).",
					{
						team: team.name,
						bot: nameOf(bot),
						org: orgName ?? team.name,
						limit: LEAD_TURNS_PER_PERSON_A_DAY,
					},
				)}
			</p>
			<div className="flex gap-2">
				<Button
					size="sm"
					disabled={actions.acceptLead.isPending}
					onClick={() => actions.acceptLead.mutate(args, fail)}
				>
					{t("teams.reach.acceptLead", "Accept")}
				</Button>
				<Button
					size="sm"
					variant="outline"
					disabled={actions.declineLead.isPending}
					onClick={() => actions.declineLead.mutate(args, fail)}
				>
					{t("teams.reach.declineLead", "Decline")}
				</Button>
			</div>
		</section>
	);
}

function StepDown({ team, bot }: Readonly<{ team: ITeam; bot: IOrgBot }>) {
	const { t } = useTranslation("bots");
	const { nameOf, actions } = useOrgChart();
	const [open, setOpen] = useState(false);
	return (
		<>
			<Button
				size="sm"
				variant="outline"
				className="self-start"
				onClick={() => setOpen(true)}
			>
				{t("teams.reach.stepDown", "Step down as lead")}
			</Button>
			<AlertDialog open={open} onOpenChange={setOpen}>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t(
								"teams.reach.stepDownTitle",
								"Step down as lead of {{team}}?",
								{
									team: team.name,
								},
							)}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"teams.reach.stepDownBody",
								"{{bot}} stays in the team, but nobody can talk to {{team}} directly until another bot leads it.",
								{ bot: nameOf(bot), team: team.name },
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("teams.org.cancel", "Cancel")}
						</AlertDialogCancel>
						<AlertDialogAction
							onClick={() =>
								actions.resignLead.mutate(
									{ teamId: team.id, botId: bot.id },
									{ onError: (error) => toast.error(error.message) },
								)
							}
						>
							{t("teams.reach.stepDownConfirm", "Step down")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}

export function BotInspector({ bot }: Readonly<{ bot: IOrgBot }>) {
	const { t } = useTranslation("bots");
	const { org, me, nameOf, onOpenChat, onOpenConversation, actions, select } =
		useOrgChart();
	const owner = useUserIdentity(bot.owner_id);
	const team = activeTeamOf(org, bot.id);
	const own = bot.owner_id === me;
	const offerTeam = own
		? org.teams.find((entry) => leadOfferFor(entry)?.bot_id === bot.id)
		: undefined;
	const classification =
		parseDataClassification(bot.classification) ?? "internal";

	const takeOut = () => {
		if (!team) return;
		actions.removeBot.mutate(
			{ teamId: team.id, botId: bot.id },
			{
				onSuccess: () => select({ kind: "bot", id: bot.id }),
				onError: (error) => toast.error(error.message),
			},
		);
	};

	return (
		<div className="flex flex-col gap-5">
			<header className="flex items-center gap-3">
				<OrgBotAvatar bot={bot} size={56} />
				<div className="flex min-w-0 flex-1 flex-col gap-1">
					<h2 className="truncate text-base font-semibold">{nameOf(bot)}</h2>
					<p className="truncate text-xs text-muted-foreground">
						{team
							? t("teams.org.botOwnerTeam", "{{owner}}'s bot · {{team}}", {
									owner: owner.label,
									team: team.name,
								})
							: t(
									"teams.org.botOwnerNoTeam",
									"{{owner}}'s bot · not in a team",
									{
										owner: owner.label,
									},
								)}
					</p>
					<DataClassificationBadge
						classification={classification}
						className="self-start"
					/>
				</div>
			</header>
			{bot.description && (
				<p className="text-sm text-muted-foreground">{bot.description}</p>
			)}
			<div className="flex flex-wrap gap-2">
				{own ? (
					<Button size="sm" onClick={() => onOpenChat(bot.id)}>
						<MessageSquareIcon aria-hidden />
						{t("teams.org.openChat", "Open chat")}
					</Button>
				) : (
					team?.conversation_id && (
						<Button
							size="sm"
							onClick={() => onOpenConversation(team.conversation_id as string)}
						>
							<UsersIcon aria-hidden />
							{t("teams.org.openRoom", "Open team room")}
						</Button>
					)
				)}
				{own && team?.conversation_id && (
					<Button
						size="sm"
						variant="outline"
						onClick={() => onOpenConversation(team.conversation_id as string)}
					>
						<UsersIcon aria-hidden />
						{t("teams.org.openRoom", "Open team room")}
					</Button>
				)}
				{team && canTakeOut(team, bot, me) && (
					<Button
						size="sm"
						variant="ghost"
						disabled={actions.removeBot.isPending}
						onClick={takeOut}
					>
						{t("teams.org.takeOut", "Take out of team")}
					</Button>
				)}
			</div>
			<ReachAction bot={bot} />
			{offerTeam && <LeadOfferCard team={offerTeam} bot={bot} />}
			{own && team?.lead_bot_id === bot.id && (
				<StepDown team={team} bot={bot} />
			)}
			<BotIoSection bot={bot} />
			{team && <ReportingLine botId={bot.id} />}
			{team && <AskLists bot={bot} />}
			{team && <BriefingBlock botId={bot.id} />}
			<MoveTo bot={bot} />
		</div>
	);
}
