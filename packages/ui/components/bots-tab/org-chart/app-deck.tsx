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
	type BusHalves,
	type DeckEntry,
	canPinTo,
	litBus,
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
		<span className="inline-flex w-max items-center gap-1.5 whitespace-nowrap">
			{count > 0 && (
				<span className="inline-flex h-5 shrink-0 items-center gap-1.5 rounded-full bg-muted px-2 text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
					{t("teams.org.apps.caption", "Apps")}
					<span className="tabular-nums">{count}</span>
				</span>
			)}
			{canPin && (
				<button
					type="button"
					onClick={() => setDialog({ kind: "pinApp", teamId: team.id })}
					className="inline-flex h-5 shrink-0 items-center gap-1 rounded-full border border-dashed border-muted-foreground/40 bg-card pr-2 pl-1.5 text-[11px] font-medium text-muted-foreground transition-colors outline-none hover:border-solid hover:border-primary hover:text-primary focus-visible:ring-2 focus-visible:ring-ring"
				>
					<PlusIcon className="size-3" aria-hidden />
					{t("teams.org.apps.pin", "Pin app")}
				</button>
			)}
		</span>
	);
}

/** The bus wire above one cell, drawn as two halves so each lights on its own. */
function BusWire({
	halves,
	first,
	last,
}: Readonly<{ halves: BusHalves; first: boolean; last: boolean }>) {
	return (
		<>
			{!first && (
				<span
					aria-hidden
					className={cn(
						"absolute top-0 left-0 right-1/2 border-t",
						halves.left ? LIT_WIRE : WIRE,
					)}
				/>
			)}
			{!last && (
				<span
					aria-hidden
					className={cn(
						"absolute top-0 left-1/2 right-0 border-t",
						halves.right ? LIT_WIRE : WIRE,
					)}
				/>
			)}
		</>
	);
}

/** One card hanging from the deck's bus. */
function Hanger({
	entry,
	first,
	last,
	bus,
	lit,
	dim,
}: Readonly<{
	entry: DeckEntry;
	first: boolean;
	last: boolean;
	bus: BusHalves;
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
			<BusWire halves={bus} first={first} last={last} />
			<AppCard entry={entry} lit={lit} dim={dim} />
		</div>
	);
}

/** The bus gap the reporting line runs through when the team has teams below it. */
function TrunkGap({
	bus,
	busRight,
}: Readonly<{ bus: BusHalves; busRight: boolean }>) {
	return (
		<div aria-hidden className="relative w-4 self-stretch">
			<BusWire halves={bus} first={false} last={!busRight} />
		</div>
	);
}

function TrunkLine({
	lit,
	className,
}: Readonly<{ lit: boolean; className: string }>) {
	return (
		<span
			aria-hidden
			className={cn(
				"absolute left-1/2 w-px -translate-x-1/2",
				lit ? "bg-primary" : "bg-border",
				className,
			)}
		/>
	);
}

/**
 * A team's apps hanging under its box on a dotted wire, one card per app. With `trunk`, the
 * team's reporting line runs straight through the middle of the deck to the teams below, lit
 * with `trunkLit`. The wire from the team to a lit card lights along its whole way.
 */
export function AppDeck({
	team,
	trunk,
	trunkLit,
}: Readonly<{ team: ITeam; trunk: boolean; trunkLit: boolean }>) {
	const deck = useDeck(team);
	if (!deck.visible || (deck.total === 0 && !deck.canPin)) return null;
	const { entries } = deck;
	const split = trunk ? Math.ceil(entries.length / 2) : entries.length;
	const left = entries.slice(0, split);
	const right = entries.slice(split);
	const lit = entries.map(deck.isLit);
	const anyLit = lit.some(Boolean);
	const bus = trunk
		? litBus([...lit.slice(0, split), false, ...lit.slice(split)], split + 0.5)
		: litBus(lit, entries.length / 2);
	const busAt = (index: number) =>
		bus[trunk && index >= split ? index + 1 : index];
	const hanger = (index: number, first: boolean, last: boolean) => (
		<Hanger
			key={entries[index].app.id}
			entry={entries[index]}
			first={first}
			last={last}
			bus={busAt(index)}
			lit={lit[index]}
			dim={deck.dimming}
		/>
	);
	return (
		<div className="relative flex min-w-full flex-col items-center">
			{trunk && (
				<>
					<TrunkLine lit={anyLit || trunkLit} className="top-0 h-5.5" />
					<TrunkLine lit={trunkLit} className="top-5.5 bottom-0" />
				</>
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
				<span className="absolute top-px left-[calc(50%+20px)]">
					<DeckCaption team={team} count={deck.total} canPin={deck.canPin} />
				</span>
			</div>
			{entries.length > 0 && (
				<div className="flex items-start">
					{left.map((_, index) =>
						hanger(index, index === 0, !trunk && index === left.length - 1),
					)}
					{trunk && <TrunkGap bus={bus[split]} busRight={right.length > 0} />}
					{right.map((_, index) =>
						hanger(split + index, false, index === right.length - 1),
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
