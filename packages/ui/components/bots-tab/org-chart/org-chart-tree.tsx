"use client";

import { useTranslation } from "@flow-like/locales";
import { PlusIcon } from "lucide-react";
import { Fragment, useState } from "react";
import {
	type OrgTeamNode,
	isTeamAdmin,
} from "../../../lib/bot-teams/org-model";
import { cn } from "../../../lib/utils";
import type { ITeam } from "../../../state/backend-state/bot-team-state";
import { AppDeck } from "./app-deck";
import { useOrgChart } from "./org-chart-context";
import { teamHasPorts } from "./org-chart-io";
import {
	benchBots,
	dropAction,
	isWaitingForConsent,
	toOrgBot,
} from "./org-chart-model";
import { OrgBotAvatar, TeamTile } from "./org-chart-parts";
import { InputRail, OutputRail, RailHeads } from "./org-chart-ports";
import {
	AddBotRow,
	BotRow,
	NewTeamRow,
	TeamRows,
	teamEntries,
} from "./org-chart-rows";
import { TeamReachLine } from "./reach-parts";
import { TeamPeopleStack } from "./team-people";

export function TeamHeader({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { selection, select, highlight } = useOrgChart();
	const selected = selection?.kind === "team" && selection.id === team.id;
	const waiting = isWaitingForConsent(team);
	return (
		<div className="flex w-full items-center gap-1">
			<button
				type="button"
				onClick={() => select({ kind: "team", id: team.id })}
				aria-pressed={selected}
				className={cn(
					"flex min-w-0 flex-1 items-center gap-2 rounded-lg px-1 py-1 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring",
					selected && "bg-primary/10 ring-2 ring-primary ring-inset",
					!selected && highlight.teamIds.has(team.id) && "bg-primary/10",
				)}
			>
				<TeamTile teamId={team.id} name={team.name} imageUrl={team.image_url} />
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
			<TeamPeopleStack team={team} />
		</div>
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

/**
 * A "+" on the box's bottom edge, where the line to its sub-teams starts, that opens New team with
 * this team as its parent. Shown on hover, focus or selection, and always on touch screens.
 */
function NewSubteamHandle({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { me, selection, setDialog } = useOrgChart();
	if (!isTeamAdmin(team, me)) return null;
	const selected = selection?.kind === "team" && selection.id === team.id;
	const label = t("teams.org.newTeamInsideNamed", "New team inside {{team}}", {
		team: team.name,
	});
	return (
		<button
			type="button"
			aria-label={label}
			title={label}
			onClick={() => setDialog({ kind: "newTeam", parentId: team.id })}
			className={cn(
				"absolute bottom-0 left-1/2 flex size-6 -translate-x-1/2 translate-y-1/2 items-center justify-center rounded-full border border-dashed bg-card text-muted-foreground opacity-0 shadow-xs transition-opacity outline-none group-hover/team:opacity-100 hover:border-solid hover:border-primary hover:text-primary focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:opacity-100",
				selected && "opacity-100",
			)}
		>
			<PlusIcon className="size-3.5" aria-hidden />
		</button>
	);
}

function boxTone(highlighted: boolean, over: boolean) {
	return cn(
		"rounded-xl border bg-card shadow-xs transition-colors",
		highlighted && "border-primary",
		over && "border-dashed border-primary bg-primary/5",
	);
}

/**
 * The team box between its rails: ways in (mail, webhooks) on the left, sent mail on the right,
 * each wired to its bot's row. Both rails share one width so the reporting lines stay centered.
 */
function RailedTeamBox({ team }: Readonly<{ team: ITeam }>) {
	const { highlight, botOf, selectedBotId } = useOrgChart();
	const { over, handlers } = useTeamDrop(team);
	const { lead, others } = teamEntries(team, botOf);
	const entries = lead ? [lead, ...others] : others;
	const lastRow = entries.length + 2;
	return (
		<section
			aria-label={team.name}
			{...handlers}
			className="group/team relative grid shrink-0 grid-cols-[13rem_16rem_13rem] gap-y-0.5"
		>
			<div
				aria-hidden
				className={cn(
					"col-start-2",
					boxTone(highlight.teamIds.has(team.id), over),
				)}
				style={{ gridRow: `1 / ${lastRow + 1}` }}
			/>
			<RailHeads team={team} />
			<div className="col-start-2 row-start-1 min-w-0 px-2 pt-2 pb-0.5">
				<TeamHeader team={team} />
				<TeamReachLine team={team} />
			</div>
			{entries.map((entry, index) => {
				const bot = botOf(entry.bot_id);
				const active =
					entry.status === "active" &&
					(selectedBotId === entry.bot_id ||
						highlight.botIds.has(entry.bot_id));
				const row = { gridRow: index + 2 };
				const live = entry.status === "active" ? bot : undefined;
				return (
					<Fragment key={entry.bot_id}>
						<InputRail bot={live} active={active} style={row} />
						<div
							className="col-start-2 grid min-w-0 grid-cols-[minmax(0,1fr)] px-2"
							style={row}
						>
							<BotRow
								team={team}
								entry={entry}
								isLead={entry === lead}
								branch={entry !== lead}
							/>
						</div>
						<OutputRail bot={live} active={active} style={row} />
					</Fragment>
				);
			})}
			<div className="col-start-2 px-2 pb-2" style={{ gridRow: lastRow }}>
				<AddBotRow team={team} branch />
			</div>
			<NewSubteamHandle team={team} />
		</section>
	);
}

export function TeamBox({
	team,
	className = "w-64 shrink-0",
	layout = "rails",
}: Readonly<{
	team: ITeam;
	className?: string;
	/** `rails` puts ways in and out beside the box; `inline` lists them under each bot. */
	layout?: "rails" | "inline";
}>) {
	const { highlight, showIo, botOf } = useOrgChart();
	const { over, handlers } = useTeamDrop(team);
	if (showIo && layout === "rails" && teamHasPorts(team, botOf)) {
		return <RailedTeamBox team={team} />;
	}
	return (
		<section
			aria-label={team.name}
			{...handlers}
			className={cn(
				"group/team relative p-2",
				boxTone(highlight.teamIds.has(team.id), over),
				className,
			)}
		>
			<TeamHeader team={team} />
			<TeamReachLine team={team} />
			<div className="mt-1">
				<TeamRows team={team} inlinePorts={showIo && layout === "inline"}>
					{layout === "inline" && <NewTeamRow team={team} />}
				</TeamRows>
			</div>
			{layout === "rails" && <NewSubteamHandle team={team} />}
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
			<AppDeck team={node.team} trunk={node.children.length > 0} />
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
