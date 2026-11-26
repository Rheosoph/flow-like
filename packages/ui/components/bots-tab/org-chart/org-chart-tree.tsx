"use client";

import { useTranslation } from "@flow-like/locales";
import { useState } from "react";
import type { OrgTeamNode } from "../../../lib/bot-teams/org-model";
import { cn } from "../../../lib/utils";
import type { ITeam } from "../../../state/backend-state/bot-team-state";
import { useOrgChart } from "./org-chart-context";
import {
	benchBots,
	dropAction,
	isWaitingForConsent,
	toOrgBot,
} from "./org-chart-model";
import { OrgBotAvatar, TeamTile } from "./org-chart-parts";
import { TeamRows } from "./org-chart-rows";

export function TeamHeader({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { selection, select, highlight } = useOrgChart();
	const selected = selection?.kind === "team" && selection.id === team.id;
	const waiting = isWaitingForConsent(team);
	return (
		<button
			type="button"
			onClick={() => select({ kind: "team", id: team.id })}
			aria-pressed={selected}
			className={cn(
				"flex w-full items-center gap-2 rounded-lg px-1 py-1 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring",
				selected && "bg-primary/10 ring-2 ring-primary ring-inset",
				!selected && highlight.teamIds.has(team.id) && "bg-primary/10",
			)}
		>
			<TeamTile teamId={team.id} name={team.name} />
			<span className="min-w-0 flex-1 truncate text-sm font-semibold">
				{team.name}
			</span>
			{team.direct_asks && (
				<span
					className={cn(
						"shrink-0 rounded-full border px-1.5 text-[10px] leading-4 font-medium",
						waiting
							? "border-warning-line bg-warning-bg text-warning"
							: "border-good-line bg-good-bg text-good",
					)}
				>
					{waiting
						? t("teams.org.waiting", "Waiting")
						: t("teams.org.direct", "Direct")}
				</span>
			)}
		</button>
	);
}

export function useTeamDrop(team: ITeam) {
	const { dragBotId, botOf, me, placeBot, setDragBotId } = useOrgChart();
	const [over, setOver] = useState(false);
	const dragged = dragBotId ? botOf(dragBotId) : undefined;
	const accepts = Boolean(dragged && dropAction(team, dragged, me));
	return {
		over: over && accepts,
		handlers: {
			onDragOver: (event: React.DragEvent) => {
				if (!accepts) return;
				event.preventDefault();
				event.dataTransfer.dropEffect = "move";
				setOver(true);
			},
			onDragLeave: () => setOver(false),
			onDrop: (event: React.DragEvent) => {
				setOver(false);
				if (!accepts || !dragged) return;
				event.preventDefault();
				placeBot(team, dragged);
				setDragBotId(null);
			},
		},
	};
}

export function TeamBox({
	team,
	className = "w-64 shrink-0",
}: Readonly<{ team: ITeam; className?: string }>) {
	const { highlight } = useOrgChart();
	const { over, handlers } = useTeamDrop(team);
	return (
		<section
			aria-label={team.name}
			{...handlers}
			className={cn(
				"rounded-xl border bg-card p-2 shadow-xs transition-colors",
				className,
				highlight.teamIds.has(team.id) && "border-primary",
				over && "border-dashed border-primary bg-primary/5",
			)}
		>
			<TeamHeader team={team} />
			<div className="mt-1">
				<TeamRows team={team} />
			</div>
		</section>
	);
}

function Connector({
	first,
	last,
	active,
}: Readonly<{ first: boolean; last: boolean; active: boolean }>) {
	const tone = active ? "bg-primary" : "bg-border";
	return (
		<>
			<span
				aria-hidden
				className={cn("absolute top-0 left-1/2 h-6 w-px", tone)}
			/>
			{!(first && last) && (
				<span
					aria-hidden
					className={cn("absolute top-0 h-px", tone)}
					style={{ left: first ? "50%" : 0, right: last ? "50%" : 0 }}
				/>
			)}
		</>
	);
}

export function OrgTree({ node }: Readonly<{ node: OrgTeamNode }>) {
	const { highlight } = useOrgChart();
	return (
		<div className="flex flex-col items-center">
			<TeamBox team={node.team} />
			{node.children.length > 0 && (
				<>
					<span
						aria-hidden
						className={cn(
							"h-6 w-px",
							node.children.some((child) =>
								highlight.teamIds.has(child.team.id),
							)
								? "bg-primary"
								: "bg-border",
						)}
					/>
					<div className="flex items-start">
						{node.children.map((child, index) => (
							<div
								key={child.team.id}
								className="relative flex flex-col items-center px-2 pt-6"
							>
								<Connector
									first={index === 0}
									last={index === node.children.length - 1}
									active={highlight.teamIds.has(child.team.id)}
								/>
								<OrgTree node={child} />
							</div>
						))}
					</div>
				</>
			)}
		</div>
	);
}

export function Bench() {
	const { t } = useTranslation("bots");
	const { ownBots, org, me, select, setDragBotId, selection, nameOf } =
		useOrgChart();
	const bench = benchBots(ownBots, org, me);
	return (
		<section
			aria-label={t("teams.org.bench", "Your bots not in a team")}
			className="rounded-xl border border-dashed p-3"
		>
			<h3 className="mb-2 text-xs font-medium text-muted-foreground">
				{t("teams.org.bench", "Your bots not in a team")}
			</h3>
			{bench.length === 0 ? (
				<p className="text-xs text-muted-foreground">
					{t("teams.org.benchEmpty", "Every bot of yours is in a team.")}
				</p>
			) : (
				<div className="flex flex-wrap gap-2">
					{bench.map((bot) => (
						<button
							key={bot.id}
							type="button"
							draggable
							onDragStart={(event) => {
								event.dataTransfer.setData("text/plain", bot.id);
								event.dataTransfer.effectAllowed = "move";
								setDragBotId(bot.id);
							}}
							onDragEnd={() => setDragBotId(null)}
							onClick={() => select({ kind: "bot", id: bot.id })}
							aria-pressed={
								selection?.kind === "bot" && selection.id === bot.id
							}
							className={cn(
								"flex items-center gap-2 rounded-full border bg-card py-1 pr-3 pl-1 text-xs outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring",
								selection?.kind === "bot" &&
									selection.id === bot.id &&
									"border-primary bg-primary/10",
							)}
						>
							<OrgBotAvatar bot={toOrgBot(bot)} size={24} />
							{nameOf(bot)}
						</button>
					))}
				</div>
			)}
		</section>
	);
}
