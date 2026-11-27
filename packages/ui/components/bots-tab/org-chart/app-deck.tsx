"use client";

import { useTranslation } from "@flow-like/locales";
import { PlusIcon } from "lucide-react";
import { cn } from "../../../lib/utils";
import type { ITeam } from "../../../state/backend-state/bot-team-state";
import { Button } from "../../ui/button";
import {
	APP_CARD_WIDTH,
	AppCard,
	OrgAppMark,
	useAppTypeWord,
} from "./app-card";
import { InspectorSection } from "./inspector-parts";
import {
	type DeckEntry,
	canPinTo,
	matchesFilter,
	teamDeck,
} from "./org-chart-apps";
import { useOrgChart } from "./org-chart-context";

const WIRE = "border-dashed border-muted-foreground/40";
const LIT_WIRE = "border-solid border-primary";

function useDeck(team: ITeam) {
	const { org, me, showApps, appFilter, highlight } = useOrgChart();
	const all = teamDeck(org, team);
	return {
		visible: showApps,
		entries: all.filter((entry) => matchesFilter(entry.app, appFilter)),
		total: all.length,
		canPin: canPinTo(team, me),
		isLit: (entry: DeckEntry) => highlight.appIds.has(entry.app.id),
		dimming: highlight.appIds.size > 0,
	};
}

function DeckCaption({
	team,
	count,
	canPin,
}: Readonly<{ team: ITeam; count: number; canPin: boolean }>) {
	const { t } = useTranslation("bots");
	const { setDialog } = useOrgChart();
	return (
		<span className="inline-flex items-center gap-1.5">
			{count > 0 && (
				<span className="inline-flex h-4 items-center gap-1.5 rounded-full bg-muted px-1.5 text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
					{t("teams.org.apps.caption", "Apps")}
					<span className="tabular-nums">{count}</span>
				</span>
			)}
			{canPin && (
				<button
					type="button"
					onClick={() => setDialog({ kind: "pinApp", teamId: team.id })}
					className="inline-flex h-4 items-center gap-0.5 rounded-full border border-dashed border-muted-foreground/40 bg-card px-1.5 text-[10px] font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
				>
					<PlusIcon className="size-2.5" aria-hidden />
					{t("teams.org.apps.pin", "Pin app")}
				</button>
			)}
		</span>
	);
}

/** One card hanging from the deck's bus. */
function Hanger({
	entry,
	first,
	last,
	lit,
	dim,
}: Readonly<{
	entry: DeckEntry;
	first: boolean;
	last: boolean;
	lit: boolean;
	dim: boolean;
}>) {
	return (
		<div className="relative px-1 pt-3">
			<span
				aria-hidden
				className={cn(
					"absolute top-0 left-1/2 h-3 border-l",
					lit ? LIT_WIRE : WIRE,
				)}
			/>
			{!(first && last) && (
				<span
					aria-hidden
					className={cn("absolute top-0 border-t", WIRE)}
					style={{ left: first ? "50%" : 0, right: last ? "50%" : 0 }}
				/>
			)}
			<AppCard entry={entry} lit={lit} dim={dim} />
		</div>
	);
}

/** The bus gap the reporting line runs through when the team has teams below it. */
function TrunkGap({ busRight }: Readonly<{ busRight: boolean }>) {
	return (
		<div aria-hidden className="relative w-4 self-stretch">
			<span
				className={cn("absolute top-0 left-0 border-t", WIRE)}
				style={{ right: busRight ? 0 : "50%" }}
			/>
		</div>
	);
}

/**
 * A team's apps hanging under its box on a dotted wire, one card per app. With `trunk`, the
 * team's reporting line runs straight through the middle of the deck to the teams below.
 */
export function AppDeck({
	team,
	trunk,
}: Readonly<{ team: ITeam; trunk: boolean }>) {
	const deck = useDeck(team);
	if (!deck.visible || (deck.total === 0 && !deck.canPin)) return null;
	const { entries } = deck;
	const split = trunk ? Math.ceil(entries.length / 2) : entries.length;
	const left = entries.slice(0, split);
	const right = entries.slice(split);
	const anyLit = entries.some(deck.isLit);
	const hanger = (entry: DeckEntry, first: boolean, last: boolean) => (
		<Hanger
			key={entry.app.id}
			entry={entry}
			first={first}
			last={last}
			lit={deck.isLit(entry)}
			dim={deck.dimming}
		/>
	);
	return (
		<div className="relative flex min-w-full flex-col items-center">
			{trunk && (
				<span
					aria-hidden
					className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-border"
				/>
			)}
			<div className="relative h-5.5 w-full">
				{!trunk && entries.length > 0 && (
					<span
						aria-hidden
						className={cn(
							"absolute inset-y-0 left-1/2 border-l",
							anyLit ? LIT_WIRE : WIRE,
						)}
					/>
				)}
				<span className="absolute top-0.75 left-[calc(50%+10px)]">
					<DeckCaption team={team} count={deck.total} canPin={deck.canPin} />
				</span>
			</div>
			{entries.length > 0 && (
				<div className="flex items-start">
					{left.map((entry, index) =>
						hanger(entry, index === 0, !trunk && index === left.length - 1),
					)}
					{trunk && <TrunkGap busRight={right.length > 0} />}
					{right.map((entry, index) =>
						hanger(entry, false, index === right.length - 1),
					)}
					{trunk && left.length > right.length && (
						<div aria-hidden style={{ width: APP_CARD_WIDTH + 8 }} />
					)}
				</div>
			)}
		</div>
	);
}

/** The team inspector's list of the team's apps, with the pin action. */
export function TeamAppsSection({ team }: Readonly<{ team: ITeam }>) {
	const { t } = useTranslation("bots");
	const { org, me, setDialog, select, nameOf, botOf } = useOrgChart();
	const typeWord = useAppTypeWord();
	const entries = teamDeck(org, team);
	const canPin = canPinTo(team, me);
	if (entries.length === 0 && !canPin) return null;
	const note = (entry: DeckEntry) => {
		if (entry.pin?.status === "pending") {
			return t("teams.org.apps.pending", "Waiting for approval");
		}
		if (entry.pin) return null;
		const bots = entry.usedBy
			.flatMap((id) => {
				const bot = botOf(id);
				return bot ? [nameOf(bot)] : [];
			})
			.join(", ");
		return t("teams.org.apps.via", "· via {{bots}}", { bots });
	};
	return (
		<InspectorSection
			title={t("teams.org.apps.title", "Apps")}
			action={
				canPin && (
					<Button
						variant="ghost"
						size="sm"
						className="h-6 px-2 text-xs"
						onClick={() => setDialog({ kind: "pinApp", teamId: team.id })}
					>
						<PlusIcon aria-hidden />
						{t("teams.org.apps.pin", "Pin app")}
					</Button>
				)
			}
		>
			{entries.length === 0 ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"teams.org.apps.empty",
						"No apps yet. Pin the apps the team works with; apps its bots use show up here too.",
					)}
				</p>
			) : (
				<ul className="flex flex-col gap-1">
					{entries.map((entry) => (
						<li key={entry.app.id}>
							<button
								type="button"
								onClick={() => select({ kind: "app", id: entry.app.id })}
								className="flex w-full min-w-0 items-center gap-2 rounded-lg text-left outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
							>
								<OrgAppMark app={entry.app} size={24} />
								<span className="flex min-w-0 flex-1 flex-col">
									<span className="truncate text-sm">{entry.app.name}</span>
									<span
										className={cn(
											"truncate text-[11px] text-muted-foreground",
											entry.pin?.status === "pending" && "text-warning",
										)}
									>
										{typeWord(entry.app.app_type)} {note(entry)}
									</span>
								</span>
							</button>
						</li>
					))}
				</ul>
			)}
		</InspectorSection>
	);
}

/** The apps of a team as a wrapped grid under its box, for the narrow outline. */
export function InlineApps({ team }: Readonly<{ team: ITeam }>) {
	const deck = useDeck(team);
	if (!deck.visible || (deck.total === 0 && !deck.canPin)) return null;
	return (
		<div className="mt-2 flex flex-col gap-2">
			<DeckCaption team={team} count={deck.total} canPin={deck.canPin} />
			{deck.entries.length > 0 && (
				<div className="flex flex-wrap gap-2">
					{deck.entries.map((entry) => (
						<AppCard
							key={entry.app.id}
							entry={entry}
							lit={deck.isLit(entry)}
							dim={deck.dimming}
						/>
					))}
				</div>
			)}
		</div>
	);
}
