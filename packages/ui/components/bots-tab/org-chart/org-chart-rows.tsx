"use client";

import { useTranslation } from "@flow-like/locales";
import { PlusIcon } from "lucide-react";
import type { ReactNode } from "react";
import { isTeamMember } from "../../../lib/bot-teams/org-model";
import { cn } from "../../../lib/utils";
import type {
	ITeam,
	ITeamBot,
} from "../../../state/backend-state/bot-team-state";
import { useOrgChart } from "./org-chart-context";
import { askBadgeStatus } from "./org-chart-model";
import {
	AskBadge,
	LeadStar,
	OrgBotAvatar,
	PersonName,
} from "./org-chart-parts";

export function BotRow({
	team,
	entry,
	isLead = false,
	branch = false,
}: Readonly<{
	team: ITeam;
	entry: ITeamBot;
	isLead?: boolean;
	branch?: boolean;
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
		<div className={cn("relative", branch && "pl-4")}>
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
					"flex w-full items-center gap-2 rounded-lg px-2 text-left outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring",
					isLead ? "py-2" : "py-1.5",
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
				</span>
				{ask && <AskBadge status={ask} />}
			</button>
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
		<div className={cn("relative", branch && "pl-4")}>
			<button
				type="button"
				onClick={() => setDialog({ kind: "addBot", teamId: team.id })}
				className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
			>
				<span className="flex size-6 items-center justify-center rounded-full border border-dashed">
					<PlusIcon className="size-3" aria-hidden />
				</span>
				{t("teams.org.addBot", "Add bot")}
			</button>
		</div>
	);
}

export function TeamRows({
	team,
	children,
}: Readonly<{ team: ITeam; children?: ReactNode }>) {
	const { botOf } = useOrgChart();
	const lead = team.bots.find(
		(entry) => entry.bot_id === team.lead_bot_id && entry.status === "active",
	);
	const others = team.bots
		.filter((entry) => entry !== lead && botOf(entry.bot_id))
		.sort(
			(a, b) => Number(a.status === "pending") - Number(b.status === "pending"),
		);
	return (
		<div className="flex flex-col gap-0.5">
			{lead && <BotRow team={team} entry={lead} isLead />}
			{others.map((entry) => (
				<BotRow key={entry.bot_id} team={team} entry={entry} branch />
			))}
			<AddBotRow team={team} branch />
			{children}
		</div>
	);
}
