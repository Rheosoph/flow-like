"use client";

import { useTranslation } from "@flow-like/locales";
import { PlusIcon } from "lucide-react";
import type { ReactNode } from "react";
import { isTeamAdmin, isTeamMember } from "../../../lib/bot-teams/org-model";
import { cn } from "../../../lib/utils";
import type {
	ITeam,
	ITeamBot,
} from "../../../state/backend-state/bot-team-state";
import { BotAppMarks } from "./app-card";
import { useOrgChart } from "./org-chart-context";
import { hasPorts } from "./org-chart-io";
import { askBadgeStatus } from "./org-chart-model";
import {
	AskBadge,
	LeadStar,
	OrgBotAvatar,
	PersonName,
} from "./org-chart-parts";
import { CalendarMarks, InlinePorts } from "./org-chart-ports";
import { GatedMark, ReachPill } from "./reach-parts";

export function BotRow({
	team,
	entry,
	isLead = false,
	branch = false,
	inlinePorts = false,
}: Readonly<{
	team: ITeam;
	entry: ITeamBot;
	isLead?: boolean;
	branch?: boolean;
	/** Its mail address and webhooks as chips under its name, for layouts without rails. */
	inlinePorts?: boolean;
}>) {
	const { t } = useTranslation("bots");
	const {
		org,
		botOf,
		nameOf,
		selection,
		select,
		selectedBotId,
		highlight,
		showAsk,
		setDragBotId,
	} = useOrgChart();
	const bot = botOf(entry.bot_id);
	if (!bot) return null;
	const pending = entry.status === "pending";
	const selected = selection?.kind === "bot" && selection.id === bot.id;
	const onChain = !selected && highlight.botIds.has(bot.id);
	const ask = showAsk ? askBadgeStatus(org, selectedBotId, bot.id) : null;
	const name = nameOf(bot);

	return (
		<div className={cn("relative flex items-center", branch && "pl-4")}>
			{branch && (
				<>
					<span
						aria-hidden
						className="absolute top-0 bottom-1/2 left-1.5 w-px bg-border"
					/>
					<span
						aria-hidden
						className="absolute top-1/2 left-1.5 h-px w-2.5 bg-border"
					/>
				</>
			)}
			<button
				type="button"
				draggable={bot.system !== "flux"}
				onDragStart={(event) => {
					event.dataTransfer.setData("text/plain", bot.id);
					event.dataTransfer.effectAllowed = "move";
					setDragBotId(bot.id);
				}}
				onDragEnd={() => setDragBotId(null)}
				onClick={() => select({ kind: "bot", id: bot.id })}
				aria-pressed={selected}
				className={cn(
					"flex min-w-0 flex-1 items-center gap-2 rounded-lg px-2 text-left outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring",
					isLead ? "py-2" : "py-1.5",
					inlinePorts && hasPorts(bot) && "items-start",
					onChain && "bg-primary/10",
					selected && "bg-primary/10 ring-2 ring-primary ring-inset",
					pending && "opacity-60",
				)}
			>
				<OrgBotAvatar bot={bot} size={isLead ? 34 : 24} />
				<span className="flex min-w-0 flex-1 flex-col">
					<span className="flex min-w-0 items-center gap-1">
						<span
							className={cn(
								"truncate font-medium",
								isLead ? "text-sm" : "text-xs",
							)}
						>
							{name}
						</span>
						{isLead && <LeadStar />}
						<CalendarMarks bot={bot} />
					</span>
					{pending ? (
						<span className="truncate text-[11px] text-warning">
							{t("teams.org.waitingFor", "Waiting for")}{" "}
							<PersonName userId={entry.owner_id} />
						</span>
					) : (
						bot.description && (
							<span className="truncate text-[11px] text-muted-foreground">
								{bot.description}
							</span>
						)
					)}
					{inlinePorts && !pending && <InlinePorts bot={bot} />}
				</span>
				{!pending && <BotAppMarks appIds={bot.app_ids} />}
				{ask && <AskBadge status={ask} />}
			</button>
			<div className="flex shrink-0 items-center gap-1 pr-1">
				{!pending && <GatedMark team={team} bot={bot} />}
				<ReachPill bot={bot} />
			</div>
		</div>
	);
}

export function AddBotRow({
	team,
	branch = false,
}: Readonly<{ team: ITeam; branch?: boolean }>) {
	const { t } = useTranslation("bots");
	const { me, setDialog } = useOrgChart();
	if (!isTeamMember(team, me)) return null;
	return (
		<AddRow
			label={t("teams.org.addBot", "Add bot")}
			branch={branch}
			onClick={() => setDialog({ kind: "addBot", teamId: team.id })}
		/>
	);
}

export function NewTeamRow({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { me, setDialog } = useOrgChart();
	if (!isTeamAdmin(team, me)) return null;
	return (
		<AddRow
			label={t("teams.org.newTeamInside", "New team inside")}
			branch
			onClick={() => setDialog({ kind: "newTeam", parentId: team.id })}
		/>
	);
}

function AddRow({
	label,
	branch,
	onClick,
}: Readonly<{ label: string; branch: boolean; onClick: () => void }>) {
	return (
		<div className={cn("relative", branch && "pl-4")}>
			<button
				type="button"
				onClick={onClick}
				className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
			>
				<span className="flex size-6 items-center justify-center rounded-full border border-dashed">
					<PlusIcon className="size-3" aria-hidden />
				</span>
				{label}
			</button>
		</div>
	);
}

/** The team's active lead first, then the bots the viewer can see, pending ones last. */
export function teamEntries(
	team: ITeam,
	botOf: (id: string) => unknown,
): { lead: ITeamBot | undefined; others: ITeamBot[] } {
	const lead = team.bots.find(
		(entry) => entry.bot_id === team.lead_bot_id && entry.status === "active",
	);
	const others = team.bots
		.filter((entry) => entry !== lead && botOf(entry.bot_id))
		.sort(
			(a, b) => Number(a.status === "pending") - Number(b.status === "pending"),
		);
	return { lead, others };
}

export function TeamRows({
	team,
	inlinePorts = false,
	children,
}: Readonly<{ team: ITeam; inlinePorts?: boolean; children?: ReactNode }>) {
	const { botOf } = useOrgChart();
	const { lead, others } = teamEntries(team, botOf);
	return (
		<div className="flex flex-col gap-0.5">
			{lead && (
				<BotRow team={team} entry={lead} isLead inlinePorts={inlinePorts} />
			)}
			{others.map((entry) => (
				<BotRow
					key={entry.bot_id}
					team={team}
					entry={entry}
					branch
					inlinePorts={inlinePorts}
				/>
			))}
			<AddBotRow team={team} branch />
			{children}
		</div>
	);
}
