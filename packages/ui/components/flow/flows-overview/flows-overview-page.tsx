"use client";

import { useTranslation } from "@flow-like/locales";
import type { UseQueryResult } from "@tanstack/react-query";
import { SearchIcon, WorkflowIcon, XIcon } from "lucide-react";
import { useCallback, useMemo, useRef, useState } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { useSearch } from "../../../hooks/use-search-index";
import {
	type IScoreCategory,
	SCORE_CATEGORIES,
} from "../../../lib/board-metrics";
import type { IBoard } from "../../../lib/schema/flow/board";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import type { IApp } from "../../../types";
import { useProjectRuns } from "../../settings/dashboard/use-project-runs";
import { Button } from "../../ui/button";
import { Card } from "../../ui/card";
import { Input } from "../../ui/input";
import { Skeleton } from "../../ui/skeleton";
import {
	type FlowLibraryBoardCreationState,
	FlowLibraryCreateDialog,
} from "../flow-library";
import { FlowScoreCard } from "./flow-score-card";
import { FlowsExecutionsRail } from "./flows-executions-rail";
import {
	BAND_ORDER,
	type IFlowRow,
	appWideMinimum,
	appWideSecurityGovernance,
	buildFlowRows,
	buildRouteByPage,
	groupIntoBands,
	groupPagesByBoard,
	sortRows,
} from "./flows-overview-model";
import {
	AI_ACT_MAX_POINTS,
	BAND_SQUARE,
	BAND_TEXT,
	DIMENSION_LABEL,
	aiActPoints,
	bandOf,
} from "./flows-overview-tokens";

/** How long a card stays ringed after being reached from the timeline. */
const HIGHLIGHT_MS = 2400;

function DimensionControl({
	rows,
	sortDimension,
	onSort,
}: Readonly<{
	rows: IFlowRow[];
	sortDimension: IScoreCategory | null;
	onSort: (dimension: IScoreCategory | null) => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		<div className="no-scrollbar inline-flex max-w-full items-center gap-0.5 overflow-x-auto rounded-md border bg-muted/40 p-0.5">
			<button
				type="button"
				aria-pressed={sortDimension === null}
				onClick={() => onSort(null)}
				className={cn(
					"inline-flex h-7 shrink-0 items-center gap-1.5 rounded px-2.5 text-xs font-medium transition-colors",
					sortDimension === null
						? "bg-card text-foreground"
						: "text-muted-foreground hover:text-foreground",
				)}
			>
				{t('weakestFirst', 'Weakest first')}
			</button>
			{SCORE_CATEGORIES.map((category) => {
				const minimum = appWideMinimum(rows, category);
				const active = sortDimension === category;
				const categoryLabel = t(category, DIMENSION_LABEL[category]);
				return (
					<button
						key={category}
						type="button"
						aria-pressed={active}
						onClick={() => onSort(active ? null : category)}
						title={t('sortEveryBandByVal', 'Sort every band by {{val}}', { val: categoryLabel })}
						className={cn(
							"inline-flex h-7 shrink-0 items-center gap-1.5 rounded px-2.5 text-xs font-medium transition-colors",
							active
								? "bg-card text-foreground"
								: "text-muted-foreground hover:text-foreground",
						)}
					>
						{categoryLabel}
						{minimum !== undefined ? (
							<span
								className={cn(
									"font-mono text-[11px] tabular-nums",
									BAND_TEXT[bandOf(minimum)],
								)}
							>
								{minimum}
							</span>
						) : null}
					</button>
				);
			})}
		</div>
	);
}

function BandSection({
	band,
	rows,
	children,
}: Readonly<{
	band: (typeof BAND_ORDER)[number];
	rows: IFlowRow[];
	children: React.ReactNode;
}>) {
	const { t } = useTranslation("flow");
	if (rows.length === 0) return null;
	const label = {
		flagged: t('flagged', 'Flagged'),
		watch: t('watch', 'Watch'),
		good: t('good', 'Good'),
		unscored: t('notScored', 'Not scored'),
	}[band];
	const description = {
		flagged: t('weakestDimensionIsUnder4', 'Weakest dimension is under 4'),
		watch: t('weakestDimensionIs4To6', 'Weakest dimension is 4 to 6'),
		good: t('everyDimensionIs7OrBetter', 'Every dimension is 7 or better'),
		unscored: t('noNodeDeclaresAScore', 'No Node declares a score'),
	}[band];
	return (
		<section className="flex flex-col gap-2.5">
			<div className="flex items-baseline gap-2">
				<span
					className={cn(
						"h-2.5 w-2.5 shrink-0 self-center rounded-[3px]",
						BAND_SQUARE[band],
					)}
				/>
				<h3 className="text-[11px] font-semibold uppercase tracking-[0.12em] text-foreground/80">
					{label}
				</h3>
				<span className="font-mono text-[11px] tabular-nums text-muted-foreground">
					{rows.length}
				</span>
				<span className="text-[11px] italic text-muted-foreground/60">
					{description}
				</span>
			</div>
			{children}
		</section>
	);
}

export interface FlowsOverviewPageProps {
	appId: string;
	app?: IApp;
	boards: UseQueryResult<IBoard[]>;
	boardCreation: FlowLibraryBoardCreationState;
	setBoardCreation: React.Dispatch<
		React.SetStateAction<FlowLibraryBoardCreationState>
	>;
	onCreateBoard: () => Promise<void>;
	onOpenBoard: (boardId: string) => Promise<void>;
	onDeleteBoard: (boardId: string) => Promise<void>;
	boardHref?: (boardId: string) => string;
	pageHref?: (pageId: string, boardId: string) => string;
	eventsHref?: string;
}

export function FlowsOverviewPage({
	appId,
	boards,
	boardCreation,
	setBoardCreation,
	onCreateBoard,
	onOpenBoard,
	onDeleteBoard,
	boardHref,
	pageHref,
	eventsHref,
}: Readonly<FlowsOverviewPageProps>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const enabled = appId.length > 0;

	// One app-wide call each, rather than one per card.
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		enabled,
	);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[appId],
		enabled,
	);
	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		enabled,
	);
	const runs = useProjectRuns(enabled ? appId : undefined, boards.data);

	const [query, setQuery] = useState("");
	const [sortDimension, setSortDimension] = useState<IScoreCategory | null>(
		null,
	);
	const [expandedId, setExpandedId] = useState<string | null>(null);
	const [highlightedId, setHighlightedId] = useState<string | null>(null);
	const cardRefs = useRef(new Map<string, HTMLDivElement>());
	const highlightTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

	const eventList = useMemo(() => events.data ?? [], [events.data]);
	const pagesByBoard = useMemo(
		() => groupPagesByBoard(pages.data ?? []),
		[pages.data],
	);
	const routeByPage = useMemo(
		() => buildRouteByPage(eventList, routes.data ?? []),
		[eventList, routes.data],
	);

	const rows = useMemo(
		() => buildFlowRows(boards.data ?? [], eventList, pagesByBoard),
		[boards.data, eventList, pagesByBoard],
	);

	const matchedRows = useSearch(rows, query, {
		fields: ["board.name", "board.description", "board.stage", "versionLabel"],
		extract: (row) =>
			[
				...row.pages.map((page) => page.name),
				...row.bindings.map((event) => event.name),
			].join(" "),
		boost: { "board.name": 3 },
	});

	const bands = useMemo(
		() => groupIntoBands(sortRows(matchedRows, sortDimension)),
		[matchedRows, sortDimension],
	);

	const visibleCount = useMemo(
		() =>
			BAND_ORDER.reduce(
				(total, band) => total + (bands.get(band)?.length ?? 0),
				0,
			),
		[bands],
	);

	const conformity = useMemo(() => appWideSecurityGovernance(rows), [rows]);

	const registerCard = useCallback(
		(boardId: string, element: HTMLDivElement | null) => {
			if (element) cardRefs.current.set(boardId, element);
			else cardRefs.current.delete(boardId);
		},
		[],
	);

	const handleSelectRun = useCallback((boardId: string) => {
		setHighlightedId(boardId);
		cardRefs.current
			.get(boardId)
			?.scrollIntoView({ block: "center", behavior: "smooth" });
		if (highlightTimer.current) clearTimeout(highlightTimer.current);
		highlightTimer.current = setTimeout(
			() => setHighlightedId(null),
			HIGHLIGHT_MS,
		);
	}, []);

	const handleToggle = useCallback((boardId: string) => {
		setExpandedId((current) => (current === boardId ? null : boardId));
	}, []);

	const handleOpen = useCallback(
		(boardId: string) => {
			void onOpenBoard(boardId);
		},
		[onOpenBoard],
	);

	return (
		<div className="flex min-h-0 w-full flex-1 flex-col gap-5">
			<header className="flex flex-col gap-3">
				<div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
					<div className="min-w-0">
						<h1 className="text-xl font-semibold tracking-tight">{t('flows', 'Flows')}</h1>
						<p className="mt-0.5 text-xs text-muted-foreground">
							{t('ratedByThe', 'Rated by the')}{" "}
							<span className="font-medium text-foreground">
								{t('lowestscoringNode', 'lowest-scoring node')}
							</span>{" "}
							{t('inEachCategoryHigherIsBetter', 'in each category — higher is better.')}
						</p>
					</div>
					<div className="flex items-center gap-2">
						<div className="relative w-full sm:w-64">
							<SearchIcon className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground/60" />
							<Input
								value={query}
								onChange={(event) => setQuery(event.target.value)}
								placeholder={t('searchFlowsPagesEvents', 'Search flows, pages, events…')}
								aria-label={t('searchFlows', 'Search flows')}
								className="h-9 border-border/60 bg-muted/40 pl-9 text-sm focus:bg-background"
							/>
							{query ? (
								<button
									type="button"
									onClick={() => setQuery("")}
									aria-label={t('clearSearch', 'Clear search')}
									className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground/60 transition-colors hover:text-foreground"
								>
									<XIcon className="size-3.5" />
								</button>
							) : null}
						</div>
						<FlowLibraryCreateDialog
							boardCreation={boardCreation}
							setBoardCreation={setBoardCreation}
							onCreateBoard={onCreateBoard}
						/>
					</div>
				</div>

				{rows.length > 0 ? (
					<div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
						<DimensionControl
							rows={rows}
							sortDimension={sortDimension}
							onSort={setSortDimension}
						/>
						{conformity !== undefined ? (
							<p className="text-[11px] italic text-muted-foreground/70">
								{t('aiActConformityScalesOn', 'AI Act conformity scales on')}{" "}
								<span className="font-mono not-italic">
									{t('minsecurityGovernance', 'min(security, governance)')}
								</span>{" "}
								{t('weakestFlow', '— weakest flow')}{" "}
								<span
									className={cn(
										"font-mono not-italic",
										BAND_TEXT[bandOf(conformity)],
									)}
								>{`${conformity}/10`}</span>{" "}
								→{" "}
								<span
									className={cn(
										"font-mono not-italic",
										BAND_TEXT[bandOf(conformity)],
									)}
								>
									{aiActPoints(conformity)}/{AI_ACT_MAX_POINTS} pts
								</span>
							</p>
						) : null}
					</div>
				) : null}
			</header>

			<div className="flex min-h-0 flex-1 flex-col gap-6 min-[1120px]:flex-row min-[1120px]:items-start">
				<div className="flex min-w-0 flex-1 flex-col gap-7 pb-6">
					{boards.isLoading ? (
						<div
							className="grid items-start gap-3"
							style={{
								gridTemplateColumns: "repeat(auto-fill, minmax(348px, 1fr))",
							}}
						>
							{[0, 1, 2, 3].map((index) => (
								<Card
									key={`skeleton-${index}`}
									className="gap-3 border-border/60 bg-card/80 p-3"
								>
									<Skeleton className="h-9 w-3/4" />
									<Skeleton className="h-8 w-full" />
									<Skeleton className="h-4 w-1/2" />
								</Card>
							))}
						</div>
					) : rows.length === 0 ? (
						<EmptyFlows
							boardCreation={boardCreation}
							setBoardCreation={setBoardCreation}
						/>
					) : visibleCount === 0 ? (
						<p className="rounded-xl border border-dashed border-border/60 px-6 py-12 text-center text-sm text-muted-foreground">{t('noFlowMatchesLdquoqueryrdquo', 'No flow matches &ldquo;{{query}}&rdquo;.', { query })}</p>
					) : (
						BAND_ORDER.map((band) => {
							const bandRows = bands.get(band) ?? [];
							return (
								<BandSection key={band} band={band} rows={bandRows}>
									<div
										className="grid items-start gap-3"
										style={{
											gridTemplateColumns:
												"repeat(auto-fill, minmax(348px, 1fr))",
										}}
									>
										{bandRows.map((row) => (
											<FlowScoreCard
												key={row.board.id}
												row={row}
												expanded={expandedId === row.board.id}
												highlighted={highlightedId === row.board.id}
												routeByPage={routeByPage}
												healthByEvent={runs.byEvent}
												boardHref={boardHref?.(row.board.id)}
												pageHref={pageHref}
												eventsHref={eventsHref}
												onToggle={handleToggle}
												onOpenBoard={handleOpen}
												onDeleteBoard={onDeleteBoard}
												registerCard={registerCard}
											/>
										))}
									</div>
								</BandSection>
							);
						})
					)}
				</div>

				{rows.length > 0 ? (
					<FlowsExecutionsRail
						runs={runs}
						rows={rows}
						events={eventList}
						onSelectRun={handleSelectRun}
					/>
				) : null}
			</div>
		</div>
	);
}

function EmptyFlows({
	boardCreation,
	setBoardCreation,
}: Readonly<{
	boardCreation: FlowLibraryBoardCreationState;
	setBoardCreation: React.Dispatch<
		React.SetStateAction<FlowLibraryBoardCreationState>
	>;
}>) {
	const { t } = useTranslation("flow");
	return (
		<Card className="items-center justify-center gap-3 border-dashed border-border/60 bg-card/60 py-16">
			<WorkflowIcon className="size-10 text-muted-foreground/40" />
			<div className="text-center">
				<p className="text-base font-semibold">{t('noFlowsYet', 'No flows yet')}</p>
				<p className="mt-1 max-w-md text-sm text-muted-foreground">
					{t('aFlowHoldsTheNodesThatDoTheWorkPlusAnyPagesBuiltOnTopOfItEventsBoundOnTheEventsPageDecideWhatStartsIt', "A flow holds the nodes that do the work, plus any pages built on top of it. Events bound on the Events page decide what starts it.")}
				</p>
			</div>
			<Button
				onClick={() => setBoardCreation({ ...boardCreation, open: true })}
			>
				{t('createYourFirstFlow', 'Create your first flow')}
			</Button>
		</Card>
	);
}
