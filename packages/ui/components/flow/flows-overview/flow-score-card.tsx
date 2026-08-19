"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ChevronDownIcon,
	ChevronRightIcon,
	ExternalLinkIcon,
	FileTextIcon,
	ShieldIcon,
	Trash2Icon,
} from "lucide-react";
import {
	LOW_COVERAGE_RATIO,
	SCORE_CATEGORIES,
} from "../../../lib/board-metrics";
import { cn } from "../../../lib/utils";
import type { PageListItem } from "../../../state/backend-state/page-state";
import type { SurfaceRunHealth } from "../../settings/dashboard/use-project-runs";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
} from "../../ui/alert-dialog";
import { Button } from "../../ui/button";
import { Card } from "../../ui/card";
import { Collapsible, CollapsibleContent } from "../../ui/collapsible";
import { FlowCardBindings, FlowCardEntryPoints } from "./flow-card-bindings";
import type { IFlowRow } from "./flows-overview-model";
import {
	BAND_FILL,
	BAND_TEXT,
	BAND_TINT,
	DIMENSION_LABEL,
	DIMENSION_SHORT,
	bandOf,
} from "./flows-overview-tokens";

/** How many pages get named on the collapsed card before the rest become "+N". */
const PAGES_ON_FACE = 2;

function ScoreShield({ row }: Readonly<{ row: IFlowRow }>) {
	return (
		<span className="relative grid size-9 shrink-0 place-items-center">
			<ShieldIcon
				className={cn(
					"absolute inset-0 size-9 opacity-25",
					BAND_TEXT[row.band],
				)}
				strokeWidth={1.5}
			/>
			<span
				className={cn(
					"relative font-mono text-[13px] font-bold tabular-nums",
					BAND_TEXT[row.band],
				)}
			>
				{row.worst ?? "–"}
			</span>
		</span>
	);
}

/**
 * Six dimensions as one row so the profile reads as a shape. The weakest cell is
 * tinted — that is the number the card's headline names.
 */
function ScoreMeter({ row }: Readonly<{ row: IFlowRow }>) {
	const { t } = useTranslation("flow");
	return (
		<div className="mt-3 grid grid-cols-6 gap-1 px-3">
			{SCORE_CATEGORIES.map((category) => {
				const value = row.scores?.[category];
				const cellBand = bandOf(value);
				const isWorst = row.worstDimension === category;
				return (
					<div
						key={category}
						className={cn(
							"rounded-md px-1 pb-1 pt-1.5 text-center",
							isWorst && BAND_TINT[cellBand],
						)}
						title={`${t(category, DIMENSION_LABEL[category])}: ${value ?? t("notScored", "Not scored")}`}
					>
						<span
							className={cn(
								"block font-mono text-[13px] font-semibold leading-none tabular-nums",
								BAND_TEXT[cellBand],
							)}
						>
							{value ?? "—"}
						</span>
						<span className="mt-1.5 block h-1 w-full overflow-hidden rounded-full bg-muted">
							<span
								className={cn("block h-full rounded-full", BAND_FILL[cellBand])}
								style={{ width: `${((value ?? 0) / 10) * 100}%` }}
							/>
						</span>
						<span className="mt-1 block text-[9px] font-medium uppercase tracking-wider text-muted-foreground/70">
							{DIMENSION_SHORT[category]}
						</span>
					</div>
				);
			})}
		</div>
	);
}

function MetaLine({ row }: Readonly<{ row: IFlowRow }>) {
	const { t } = useTranslation("flow");
	const lowCoverage =
		row.coverage.nodeCount > 0 && row.coverage.ratio < LOW_COVERAGE_RATIO;
	const parts: { id: string; node: React.ReactNode }[] = [
		{
			id: "version",
			node: <span className="font-mono tabular-nums">{row.versionLabel}</span>,
		},
		{ id: "stage", node: row.board.stage },
		{
			id: "bindings",
			node: row.bindings.length ? (
				<span>
					{t("countBindings", {
						defaultValue_one: "1 binding",
						defaultValue_other: "{{count}} bindings",
						count: row.bindings.length,
					})}
				</span>
			) : (
				<span className="text-amber-600 dark:text-amber-400">
					{t("nothingBound", "nothing bound")}
				</span>
			),
		},
		{
			id: "coverage",
			node: row.scores ? (
				<span
					className={cn(
						"font-mono tabular-nums",
						lowCoverage && "text-amber-600 dark:text-amber-400",
					)}
				>
					{lowCoverage ? "only " : ""}
					{`${row.coverage.scoredNodeCount}/${row.coverage.nodeCount} scored`}
				</span>
			) : (
				<span className="font-mono tabular-nums">
					{t("countNodes", {
						defaultValue_one: "{{count}} Node",
						defaultValue_other: "{{count}} Nodes",
						count: row.nodeTotal,
					})}
				</span>
			),
		},
	];

	return (
		<div className="mt-2.5 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 px-3 text-[11px] text-muted-foreground">
			{parts.map((part, index) => (
				<span key={part.id} className="flex items-center gap-1.5">
					{index > 0 ? (
						<span aria-hidden className="text-muted-foreground/40">
							·
						</span>
					) : null}
					{part.node}
				</span>
			))}
		</div>
	);
}

function PageChip({
	page,
	route,
	href,
}: Readonly<{ page: PageListItem; route?: string; href?: string }>) {
	const { t } = useTranslation("flow");
	// The board lists the page but no copy of its content is reachable, so its name and
	// route are unknown too — saying so beats rendering a chip that opens onto nothing.
	const unavailable = page.unavailable === true;
	const content = (
		<>
			<FileTextIcon className="size-2.5 shrink-0 opacity-70" />
			<span className="truncate text-[10px] font-medium text-foreground/80">
				{page.name}
			</span>
			<span
				className={cn(
					"shrink-0 font-mono text-[9px]",
					route && !unavailable
						? "text-muted-foreground/70"
						: "text-amber-600 dark:text-amber-400",
				)}
			>
				{unavailable ? "unavailable" : (route ?? "no route")}
			</span>
		</>
	);
	const className = cn(
		"inline-flex min-w-0 max-w-[60%] items-center gap-1 rounded border px-1.5 py-0.5 transition-colors",
		route && !unavailable
			? `border-border/60 bg-muted/40 hover:border-border`
			: `border-dashed border-amber-500/50 bg-amber-500/5`,
	);
	const title = unavailable
		? t(
				"nameThisPagesContentCouldNotBeLoadedOnThisDevice",
				"{{name}} — this page's content could not be loaded on this device",
				{ name: page.name },
			)
		: route
			? t("nameServedAtRoute", "{{name}} — served at {{route}}", {
					name: page.name,
					route,
				})
			: t(
					"nameNoRouteEventPointsAtThisPage",
					"{{name}} — no route event points at this page",
					{ name: page.name },
				);

	if (!href) {
		return (
			<span className={className} title={title}>
				{content}
			</span>
		);
	}
	return (
		<a
			className={className}
			href={href}
			title={title}
			onClick={(event) => event.stopPropagation()}
		>
			{content}
		</a>
	);
}

/**
 * Fixed height so a flow with no pages lines up with one that has two. Pages are
 * children of the board, so they are named here rather than counted.
 */
function PageRow({
	row,
	routeByPage,
	pageHref,
}: Readonly<{
	row: IFlowRow;
	routeByPage: Map<string, string>;
	pageHref?: (pageId: string, boardId: string) => string;
}>) {
	const { t } = useTranslation("flow");
	if (row.pages.length === 0) {
		return (
			<div className="mt-2 flex h-6 items-center px-3">
				<span className="text-[10px] italic text-muted-foreground/50">
					{t("noPagesConfigured", "No pages configured")}
				</span>
			</div>
		);
	}
	const shown = row.pages.slice(0, PAGES_ON_FACE);
	const rest = row.pages.length - shown.length;
	return (
		<div className="mt-2 flex h-6 items-center gap-1.5 overflow-hidden px-3">
			{shown.map((page) => (
				<PageChip
					key={page.pageId}
					page={page}
					route={routeByPage.get(page.pageId)}
					href={pageHref?.(page.pageId, row.board.id)}
				/>
			))}
			{rest > 0 ? (
				<span className="shrink-0 font-mono text-[10px] tabular-nums text-muted-foreground/60">{`+${rest}`}</span>
			) : null}
		</div>
	);
}

function CauseList({ row }: Readonly<{ row: IFlowRow }>) {
	const { t } = useTranslation("flow");
	if (row.causes.length === 0) {
		return (
			<p className="text-[11px] leading-relaxed text-muted-foreground">
				{t(
					"noNodeScoresBelow4InAnyCategoryTheLowestScoreIsSimplyTheLowestOfAHealthySet",
					"No node scores below 4 in any category — the lowest score is simply the lowest of a healthy set.",
				)}
			</p>
		);
	}
	return (
		<ul className="flex flex-col gap-1">
			{row.causes.map((cause) => {
				const band = bandOf(cause.score);
				return (
					<li
						key={`${cause.node}-${cause.category}`}
						className="flex items-center gap-2 text-[11px]"
					>
						<span
							className={cn(
								"grid size-5 shrink-0 place-items-center rounded font-mono text-[10px] font-bold tabular-nums",
								BAND_TINT[band],
								BAND_TEXT[band],
							)}
						>
							{cause.score}
						</span>
						<span className="truncate font-medium text-foreground/90">
							{cause.friendlyName}
						</span>
						<span className="truncate font-mono text-[10px] text-muted-foreground/70">
							{cause.node}
						</span>
						<span className="ml-auto shrink-0 text-[10px] text-muted-foreground">{`${t(cause.category, DIMENSION_LABEL[cause.category])} · ×${cause.count}`}</span>
					</li>
				);
			})}
		</ul>
	);
}

function CompositionChips({ row }: Readonly<{ row: IFlowRow }>) {
	const { t } = useTranslation("flow");
	const chips: string[] = [
		t("countNodes", {
			defaultValue_one: "{{count}} Node",
			defaultValue_other: "{{count}} Nodes",
			count: row.nodeTotal,
		}),
		t("countConnections", {
			defaultValue_one: "{{count}} connection",
			defaultValue_other: "{{count}} connections",
			count: row.connections,
		}),
		t("countEntryPoints", {
			defaultValue_one: "1 entry point",
			defaultValue_other: "{{count}} entry points",
			count: row.entryPoints.length,
		}),
		[
			t("countVariables", {
				defaultValue_one: "{{count}} variable",
				defaultValue_other: "{{count}} variables",
				count: row.variables.total,
			}),
			row.variables.secret
				? t("countSecretVariables", {
						defaultValue_one: "{{count}} secret",
						defaultValue_other: "{{count}} secrets",
						count: row.variables.secret,
					})
				: null,
		]
			.filter(Boolean)
			.join(" · "),
	];
	if (row.layers.total > 0)
		chips.push(
			t("countLayers", {
				defaultValue_one: "{{count}} Layer",
				defaultValue_other: "{{count}} Layers",
				count: row.layers.total,
			}),
		);
	return (
		<div className="flex flex-wrap gap-1">
			{chips.map((chip) => (
				<span
					key={chip}
					className="inline-flex items-center gap-1 rounded border border-border/60 bg-muted/40 px-1.5 py-0.5 font-mono text-[10px] tabular-nums text-muted-foreground"
				>
					{chip}
				</span>
			))}
			{row.wasm.packageIds.map((packageId) => (
				<span
					key={packageId}
					className="inline-flex items-center gap-1 rounded border border-border/60 bg-muted/40 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
				>
					{packageId}
				</span>
			))}
			{row.wasm.permissions.map((permission) => (
				<span
					key={permission}
					className="inline-flex items-center gap-1 rounded border border-amber-500/40 bg-amber-500/5 px-1.5 py-0.5 font-mono text-[10px] text-amber-600 dark:text-amber-400"
				>
					{permission}
				</span>
			))}
		</div>
	);
}

function BlockTitle({
	children,
	note,
}: Readonly<{ children: string; note?: string }>) {
	return (
		<p className="flex items-baseline gap-2">
			<span className="text-[10px] font-semibold uppercase tracking-[0.1em] text-muted-foreground/70">
				{children}
			</span>
			{note ? (
				<span className="text-[10px] italic text-muted-foreground/50">
					{note}
				</span>
			) : null}
		</p>
	);
}

export interface FlowScoreCardProps {
	row: IFlowRow;
	expanded: boolean;
	highlighted: boolean;
	routeByPage: Map<string, string>;
	healthByEvent: Map<string, SurfaceRunHealth>;
	boardHref?: string;
	pageHref?: (pageId: string, boardId: string) => string;
	eventsHref?: string;
	onToggle: (boardId: string) => void;
	onOpenBoard: (boardId: string) => void;
	onDeleteBoard: (boardId: string) => Promise<void>;
	registerCard?: (boardId: string, element: HTMLDivElement | null) => void;
}

export function FlowScoreCard({
	row,
	expanded,
	highlighted,
	routeByPage,
	healthByEvent,
	boardHref,
	pageHref,
	eventsHref,
	onToggle,
	onOpenBoard,
	onDeleteBoard,
	registerCard,
}: Readonly<FlowScoreCardProps>) {
	const { t } = useTranslation("flow");
	const weakest = row.worstDimension
		? t(row.worstDimension, DIMENSION_LABEL[row.worstDimension])
		: undefined;
	const expanderLabel = expanded
		? t("hideDetail", "Hide detail")
		: weakest && row.worst !== undefined
			? t("whyValIsWorst", "Why {{val}} is {{worst}}", {
					val: weakest.toLowerCase(),
					worst: row.worst,
				})
			: t("bindingsAndPages", "Bindings and pages");

	return (
		<Card
			ref={(element) => registerCard?.(row.board.id, element)}
			className={cn(
				"gap-0 overflow-hidden border-border/60 bg-card/80 py-0 backdrop-blur-sm transition-colors",
				"dark:border-white/10 dark:bg-muted/40",
				"hover:border-border",
				highlighted && "ring-2 ring-primary/60",
			)}
		>
			<Collapsible open={expanded}>
				<div className="flex items-start gap-2.5 px-3 pt-3">
					<ScoreShield row={row} />
					<div className="min-w-0 flex-1">
						<p className="truncate text-sm font-semibold leading-tight">
							{row.board.name}
						</p>
						<p className="mt-0.5 truncate text-[11px] text-muted-foreground">
							{weakest ? (
								<>
									<span className={cn("font-medium", BAND_TEXT[row.band])}>
										{weakest}
									</span>{" "}
									{t("isItsWeakestOfSix", "is its weakest of six")}
								</>
							) : (
								t("noneOfCountNodesDeclaresAScore", {
									defaultValue_one: "Its only Node does not declare a score",
									defaultValue_other:
										"None of its {{count}} Nodes declares a score",
									count: row.nodeTotal,
								})
							)}
						</p>
					</div>
					<div className="flex shrink-0 items-center gap-0.5">
						<Button
							variant="ghost"
							size="icon"
							className="size-7"
							title={t("openName", "Open {{name}}", { name: row.board.name })}
							data-href={boardHref}
							data-title={row.board.name}
							onClick={() => onOpenBoard(row.board.id)}
						>
							<ExternalLinkIcon className="size-3.5" />
							<span className="sr-only">
								{t("openName", "Open {{name}}", { name: row.board.name })}
							</span>
						</Button>
						<AlertDialog>
							<AlertDialogTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="size-7 text-muted-foreground hover:text-destructive"
									title={t("deleteName", "Delete {{name}}", {
										name: row.board.name,
									})}
								>
									<Trash2Icon className="size-3.5" />
									<span className="sr-only">
										{t("deleteName", "Delete {{name}}", {
											name: row.board.name,
										})}
									</span>
								</Button>
							</AlertDialogTrigger>
							<AlertDialogContent>
								<AlertDialogHeader>
									<AlertDialogTitle>
										Delete &ldquo;{row.board.name}&rdquo;?
									</AlertDialogTitle>
									<AlertDialogDescription>
										{t("thisRemovesTheFlowAndCountPagesInsideIt", {
											defaultValue_one:
												"This removes the Flow and the page inside it.",
											defaultValue_other:
												"This removes the Flow and the {{count}} pages inside it.",
											count: row.pages.length,
										})}{" "}
										{row.bindings.length > 0
											? t("countBoundEventsWillStopFiring", {
													defaultValue_one:
														"One event bound to it will stop firing.",
													defaultValue_other:
														"{{count}} events bound to it will stop firing.",
													count: row.bindings.length,
												})
											: ""}{" "}
										{t("thisCannotBeUndone", "This cannot be undone.")}
									</AlertDialogDescription>
								</AlertDialogHeader>
								<AlertDialogFooter>
									<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
									<AlertDialogAction
										className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
										onClick={() => {
											void onDeleteBoard(row.board.id);
										}}
									>
										{t("deleteFlow", "Delete flow")}
									</AlertDialogAction>
								</AlertDialogFooter>
							</AlertDialogContent>
						</AlertDialog>
					</div>
				</div>

				<ScoreMeter row={row} />
				<MetaLine row={row} />
				<PageRow row={row} routeByPage={routeByPage} pageHref={pageHref} />

				<button
					type="button"
					aria-expanded={expanded}
					onClick={() => onToggle(row.board.id)}
					className="mt-2 flex w-full items-center gap-1.5 border-t border-border/50 bg-muted/20 px-3 py-2 text-[11px] text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground"
				>
					{expanded ? (
						<ChevronDownIcon className="size-3.5 shrink-0" />
					) : (
						<ChevronRightIcon className="size-3.5 shrink-0" />
					)}
					<span className="truncate">{expanderLabel}</span>
					{!expanded && row.causes.length > 0 ? (
						<span className="ml-auto shrink-0 font-mono text-[10px] tabular-nums text-muted-foreground/60">
							{t("countCauses", {
								defaultValue_one: "1 cause",
								defaultValue_other: "{{count}} causes",
								count: row.causes.length,
							})}
						</span>
					) : null}
				</button>

				<CollapsibleContent>
					<div className="flex flex-col gap-3.5 border-t border-border/50 bg-muted/10 px-3 py-3">
						{row.scores ? (
							<div className="flex flex-col gap-2">
								<BlockTitle
									note={t(
										"lowestNodeDeterminesTheScore",
										"Lowest Node determines the score",
									)}
								>
									{weakest
										? t("whyValIsWorst", "Why {{val}} is {{worst}}", {
												val: weakest.toLowerCase(),
												worst: row.worst,
											})
										: t("score", "Score")}
								</BlockTitle>
								<CauseList row={row} />
								<p className="text-[11px] leading-relaxed text-muted-foreground">
									{t(
										"onlyScoredNodeCountOfNodeCountNodesDeclareAScoreTheRestAreSkipped",
										"Only {{scoredNodeCount}} of {{nodeCount}} Nodes declare a score; the rest are skipped.",
										{
											scoredNodeCount: row.coverage.scoredNodeCount,
											nodeCount: row.coverage.nodeCount,
										},
									)}
									{row.wasm.packageIds.length > 0
										? ` ${t("scoresFromExternalPackagesAreSelfdeclaredByTheirManifestOnlyPermissionsAreEnforced", "Scores from external packages are self-declared by their manifest — only permissions are enforced.")}`
										: ""}
								</p>
							</div>
						) : null}

						<div className="flex flex-col gap-2">
							<BlockTitle note={t("joinedFromEvents", "Joined from Events")}>
								{t("bindings", "Bindings")}
							</BlockTitle>
							<FlowCardBindings
								row={row}
								healthByEvent={healthByEvent}
								eventsHref={eventsHref}
							/>
						</div>

						<div className="flex flex-col gap-2">
							<BlockTitle note="owned by this flow">
								{t("entryPoints", "Entry points")}
							</BlockTitle>
							{row.entryPoints.length > 0 ? (
								<FlowCardEntryPoints row={row} />
							) : (
								<p className="text-[11px] text-muted-foreground">
									{t(
										"noNodeInThisFlowIsMarkedAsAStartSoARunHasNowhereToBegin",
										"No node in this flow is marked as a start, so a run has nowhere to begin.",
									)}
								</p>
							)}
						</div>

						<div className="flex flex-col gap-2">
							<BlockTitle>{t("composition", "Composition")}</BlockTitle>
							<CompositionChips row={row} />
						</div>
					</div>
				</CollapsibleContent>
			</Collapsible>
		</Card>
	);
}
