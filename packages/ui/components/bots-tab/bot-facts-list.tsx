"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	BrainIcon,
	CircleAlertIcon,
	ListTreeIcon,
	MapIcon,
	SearchIcon,
	SearchXIcon,
	XIcon,
} from "lucide-react";
import {
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { getErrorMessage } from "../../lib/error-message";
import { cn } from "../../lib/utils";
import {
	type BotFact,
	forgetBotFact,
	listBotFacts,
} from "../../state/global-chat/bot-memory-facts";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { BotMemoryCard } from "./bot-memory-card";
import { memoryKindStyle, useMemoryKindLabel } from "./bot-memory-kinds";
import { MemoryMapFrame } from "./bot-memory-map-frame";
import {
	type MemoryFormat,
	type MemoryGroup,
	groupMemories,
	memoryFormat,
	memoryPlace,
} from "./bot-memory-model";

type MemoryView = "timeline" | "map";

export function botFactsKey(botId: string) {
	return ["bot-memory", "facts", botId] as const;
}

/** Relative labels ("in 5 minutes") stay true while the pane is open. */
function useMinuteClock() {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const timer = setInterval(() => setNow(Date.now()), 60_000);
		return () => clearInterval(timer);
	}, []);
	return now;
}

function reducedMotion() {
	return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
}

function useGroupLabel(format: MemoryFormat, now: number) {
	const { t } = useTranslation("bots");
	const year = new Date(now).getFullYear();
	return (group: MemoryGroup) => {
		switch (group.key.kind) {
			case "upcoming":
				return t("memory.groups.upcoming", "Upcoming");
			case "today":
				return t("memory.groups.today", "Today");
			case "yesterday":
				return t("memory.groups.yesterday", "Yesterday");
			case "week":
				return t("memory.groups.week", "Last 7 days");
			case "month":
				return format.monthYear(
					group.key.year,
					group.key.month,
					group.key.year !== year,
				);
			default:
				return format.year(group.key.year);
		}
	};
}

function ViewSwitch({
	view,
	places,
	onChange,
}: Readonly<{
	view: MemoryView;
	places: number;
	onChange: (view: MemoryView) => void;
}>) {
	const { t } = useTranslation("bots");
	const options = [
		{
			value: "timeline" as const,
			icon: ListTreeIcon,
			label: t("memory.views.timeline", "Timeline"),
		},
		{
			value: "map" as const,
			icon: MapIcon,
			label: t("memory.views.map", "Map of {{count}} places", {
				count: places,
				defaultValue_one: "Map of {{count}} place",
			}),
		},
	];
	return (
		<fieldset
			aria-label={t("memory.views.label", "Show memories as")}
			className="flex h-9 shrink-0 items-center gap-0.5 rounded-xl border border-border/65 bg-muted/30 p-0.5"
		>
			{options.map(({ value, icon: Icon, label }) => (
				<button
					key={value}
					type="button"
					aria-pressed={view === value}
					aria-label={label}
					title={label}
					onClick={() => onChange(value)}
					className={cn(
						"inline-flex h-full items-center gap-1 rounded-[10px] px-2.5 text-[11px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-ring",
						view === value
							? "bg-background text-foreground shadow-xs"
							: "text-muted-foreground hover:text-foreground",
					)}
				>
					<Icon className="size-3.5" aria-hidden />
					{value === "map" && <span className="tabular-nums">{places}</span>}
				</button>
			))}
		</fieldset>
	);
}

function TimelineSkeleton() {
	const { t } = useTranslation("bots");
	return (
		<output
			className="block pt-4"
			aria-label={t("crew.dock.loading", "Loading…")}
		>
			<span className="sr-only">{t("crew.dock.loading", "Loading…")}</span>
			<div aria-hidden className="mb-3 h-6 w-20 rounded-full bg-muted" />
			<div className="relative flex flex-col gap-3 before:absolute before:top-3 before:bottom-3 before:left-[7px] before:w-px before:bg-border">
				{[0, 1, 2].map((index) => (
					<div key={index} aria-hidden className="relative pl-6">
						<span className="absolute top-[1.4rem] left-[3px] size-[9px] rounded-full bg-muted ring-[3px] ring-(--memory-canvas)" />
						<div className="space-y-3 rounded-2xl border border-border/50 bg-card p-4 motion-safe:animate-pulse">
							<div className="h-6 w-24 rounded-lg bg-muted" />
							<div className="h-3.5 w-full rounded bg-muted" />
							<div className="h-3.5 w-2/3 rounded bg-muted" />
							<div className="h-13 rounded-xl bg-muted/60" />
						</div>
					</div>
				))}
			</div>
		</output>
	);
}

/** Saved facts with the messages they came from. Forgetting a fact keeps its source chat. */
export function BotFactsList({
	botId,
	appId,
	name,
	memoryOff = false,
	onJump,
}: Readonly<{
	botId: string;
	appId: string;
	name: string;
	/** Memory is off for this bot, so an empty list isn't "as you chat". */
	memoryOff?: boolean;
	onJump: (seq: number) => void;
}>) {
	const { t, i18n } = useTranslation("bots");
	const token = useAuth().user?.access_token;
	const queryClient = useQueryClient();
	const headingId = useId();
	const scroller = useRef<HTMLElement>(null);
	const now = useMinuteClock();
	const format = memoryFormat(i18n.language);
	const groupLabel = useGroupLabel(format, now);
	const kindLabel = useMemoryKindLabel();
	const [search, setSearch] = useState("");
	const [selectedKind, setSelectedKind] = useState<string | null>(null);
	const [selectedView, setSelectedView] = useState<MemoryView>("timeline");
	const [located, setLocated] = useState<string | null>(null);
	const owner = { botId, appId, token };
	const facts = useQuery({
		queryKey: botFactsKey(botId),
		queryFn: () => listBotFacts(owner),
		retry: false,
	});
	const forget = useMutation({
		mutationFn: (id: string) => forgetBotFact(owner, id),
		onSuccess: (_, id) =>
			queryClient.setQueryData<BotFact[]>(botFactsKey(botId), (current) =>
				current?.filter((fact) => fact.id !== id),
			),
		onError: (error) => toast.error(getErrorMessage(error)),
	});
	const kinds = useMemo(() => {
		const counts = new Map<string, number>();
		for (const fact of facts.data ?? [])
			counts.set(fact.kind, (counts.get(fact.kind) ?? 0) + 1);
		return [...counts].sort(([a], [b]) => a.localeCompare(b));
	}, [facts.data]);
	const placed = useMemo(
		() =>
			new Set(
				(facts.data ?? [])
					.filter((fact) => memoryPlace(fact.geojson))
					.map((fact) => fact.id),
			),
		[facts.data],
	);
	// Deleting the last fact of a category returns the inspector to all memories.
	const kind = kinds.some(([value]) => value === selectedKind)
		? selectedKind
		: null;
	const view: MemoryView = placed.size > 0 ? selectedView : "timeline";
	const query = search.trim().toLocaleLowerCase();
	const visible = useMemo(
		() =>
			(facts.data ?? []).filter((fact) => {
				if (kind && fact.kind !== kind) return false;
				if (!query) return true;
				return [
					fact.text,
					fact.kind,
					kindLabel(fact.kind),
					...fact.evidence.flatMap((item) => [
						item.quote,
						item.speaker?.name ?? "",
					]),
				]
					.join(" ")
					.toLocaleLowerCase()
					.includes(query);
			}),
		[facts.data, kind, query, kindLabel],
	);
	const shown = useMemo(
		() =>
			view === "map" ? visible.filter((fact) => placed.has(fact.id)) : visible,
		[view, visible, placed],
	);
	const groups = useMemo(
		() => (view === "timeline" ? groupMemories(shown, now) : []),
		[view, shown, now],
	);
	const total = facts.data?.length ?? 0;
	const clearFilters = () => {
		setSearch("");
		setSelectedKind(null);
	};
	const reveal = useCallback((id: string) => {
		setLocated(id);
		scroller.current
			?.querySelector(`[data-memory-id="${CSS.escape(id)}"]`)
			?.scrollIntoView({
				block: "nearest",
				behavior: reducedMotion() ? "auto" : "smooth",
			});
	}, []);
	const card = (fact: BotFact, rail: boolean) => (
		<BotMemoryCard
			key={fact.id}
			fact={fact}
			now={now}
			format={format}
			rail={rail}
			selected={view === "map" && located === fact.id}
			busy={forget.isPending && forget.variables === fact.id}
			disabled={forget.isPending}
			onForget={(id) => forget.mutate(id)}
			onJump={onJump}
			onLocate={view === "map" ? setLocated : undefined}
		/>
	);

	return (
		<section
			ref={scroller}
			aria-labelledby={headingId}
			className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto overscroll-contain"
		>
			<div className="shrink-0 space-y-3 px-4 pt-4 pb-3">
				<div className="flex items-center gap-3">
					<span className="relative flex size-10 shrink-0 items-center justify-center rounded-2xl border border-emerald-600/10 bg-linear-to-br from-emerald-500/15 to-sky-500/10 text-emerald-700 dark:text-emerald-300">
						<BrainIcon className="size-5" strokeWidth={1.5} aria-hidden />
					</span>
					<div className="min-w-0 flex-1">
						<h3 id={headingId} className="text-sm font-semibold tracking-tight">
							{t("memory.library", "Saved memories")}
						</h3>
						<p className="mt-0.5 wrap-anywhere text-xs leading-relaxed text-muted-foreground">
							{t("memory.libraryHint", "What {{name}} keeps in mind.", {
								name,
							})}
						</p>
					</div>
					{facts.isSuccess && (
						<span
							className="flex h-8 min-w-8 shrink-0 items-center justify-center rounded-full border border-border/60 bg-muted/40 px-2 text-xs font-medium tabular-nums"
							aria-label={t("memory.count", "{{count}} memories", {
								count: total,
								defaultValue_one: "{{count}} memory",
							})}
						>
							{total}
						</span>
					)}
				</div>
				{total > 0 && (
					<>
						<div className="flex items-center gap-2">
							<div className="relative min-w-0 flex-1">
								<SearchIcon
									className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
									aria-hidden
								/>
								<Input
									type="search"
									value={search}
									onChange={(event) => setSearch(event.target.value)}
									placeholder={t(
										"memory.searchPlaceholder",
										"Search memories…",
									)}
									aria-label={t("memory.searchPlaceholder", "Search memories…")}
									className="h-9 rounded-xl border-border/65 bg-muted/30 pr-9 pl-9 text-sm shadow-none [&::-webkit-search-cancel-button]:appearance-none"
								/>
								{search && (
									<Button
										type="button"
										variant="ghost"
										size="icon"
										className="absolute top-0.5 right-0.5 size-8 rounded-lg text-muted-foreground"
										aria-label={t("memory.clearSearch", "Clear search")}
										onClick={() => setSearch("")}
									>
										<XIcon className="size-3.5" aria-hidden />
									</Button>
								)}
							</div>
							{placed.size > 0 && (
								<ViewSwitch
									view={view}
									places={placed.size}
									onChange={setSelectedView}
								/>
							)}
						</div>
						<fieldset
							aria-label={t("memory.filter", "Filter memories by category")}
							className="flex flex-wrap gap-1.5"
						>
							<button
								type="button"
								aria-pressed={kind === null}
								onClick={() => setSelectedKind(null)}
								className={cn(
									"inline-flex min-h-7 items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-ring",
									kind === null
										? "border-foreground/15 bg-foreground text-background"
										: "border-border/60 text-muted-foreground hover:bg-muted",
								)}
							>
								{t("memory.all", "All")}
								<span className="tabular-nums opacity-65">{total}</span>
							</button>
							{kinds.map(([value, count]) => {
								const { icon: Icon, color } = memoryKindStyle(value);
								return (
									<button
										key={value}
										type="button"
										aria-pressed={kind === value}
										onClick={() => setSelectedKind(value)}
										className={cn(
											"inline-flex min-h-7 max-w-full items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-ring",
											kind === value
												? cn("border-current/20", color)
												: "border-border/60 text-muted-foreground hover:bg-muted",
										)}
									>
										<Icon className="size-3 shrink-0" aria-hidden />
										<span className="truncate">{kindLabel(value)}</span>
										<span className="tabular-nums opacity-65">{count}</span>
									</button>
								);
							})}
						</fieldset>
					</>
				)}
			</div>
			<div className="grow bg-(--memory-canvas) px-4 pb-6 [--memory-canvas:color-mix(in_oklab,var(--muted)_28%,var(--background))]">
				{facts.isPending ? (
					<TimelineSkeleton />
				) : facts.isError ? (
					<div
						role="alert"
						className="flex flex-col items-center gap-3 rounded-2xl border border-border/60 bg-card px-5 py-8 text-center"
					>
						<CircleAlertIcon
							className="size-6 text-muted-foreground"
							aria-hidden
						/>
						<p className="text-sm font-medium">
							{t("memory.factsFailed", "Couldn't load {{name}}'s facts.", {
								name,
							})}
						</p>
						<p className="max-w-full wrap-anywhere text-xs text-muted-foreground">
							{getErrorMessage(facts.error)}
						</p>
						<Button
							type="button"
							size="sm"
							variant="outline"
							className="rounded-full"
							onClick={() => void facts.refetch()}
						>
							{t("crew.retry", "Try again")}
						</Button>
					</div>
				) : total === 0 ? (
					<MemoryEmpty name={name} memoryOff={memoryOff} />
				) : (
					<>
						{(query || kind) && (
							<output className="block pt-3 text-[11px] text-muted-foreground">
								{t("memory.results", "{{shown}} of {{total}} memories", {
									shown: visible.length,
									total,
								})}
							</output>
						)}
						{shown.length === 0 ? (
							<div className="mt-3 flex flex-col items-center gap-3 rounded-2xl border border-dashed border-border bg-card/50 px-5 py-8 text-center">
								<SearchXIcon
									className="size-6 text-muted-foreground"
									aria-hidden
								/>
								<p className="text-sm font-medium">
									{t("memory.noResults", "No matching memories")}
								</p>
								<p className="text-xs text-muted-foreground">
									{t("memory.noResultsHint", "Try another word or category.")}
								</p>
								<Button
									type="button"
									variant="outline"
									size="sm"
									className="rounded-full"
									onClick={clearFilters}
								>
									{t("memory.resetFilters", "Reset filters")}
								</Button>
							</div>
						) : view === "map" ? (
							<>
								<div className="sticky top-0 z-10 -mx-4 bg-linear-to-b from-(--memory-canvas) from-85% to-transparent px-4 pt-3 pb-4">
									<MemoryMapFrame
										facts={shown}
										selectedId={located}
										onSelect={reveal}
										className="h-60 rounded-2xl border border-border/60 shadow-sm"
									/>
								</div>
								<ul className="flex flex-col gap-3 [&>li]:scroll-mt-68">
									{shown.map((fact) => card(fact, false))}
								</ul>
							</>
						) : (
							<div className="flex flex-col gap-3 pt-1">
								{groups.map((group) => (
									<section
										key={group.id}
										aria-labelledby={`${headingId}-${group.id}`}
									>
										<div className="sticky top-0 z-10 -mx-4 mb-1 bg-(--memory-canvas)/80 px-4 py-2 backdrop-blur-md">
											<h4
												id={`${headingId}-${group.id}`}
												className="inline-flex items-center gap-2 rounded-full border border-border/60 bg-background/90 py-1 pr-2.5 pl-2 text-[11px] font-semibold text-foreground/85 shadow-xs backdrop-blur-sm"
											>
												<span
													aria-hidden
													className={cn(
														"size-1.5 rounded-full",
														group.key.kind === "upcoming"
															? "bg-primary"
															: "bg-foreground/35",
													)}
												/>
												{groupLabel(group)}
												<span className="font-normal tabular-nums text-muted-foreground">
													{group.facts.length}
												</span>
											</h4>
										</div>
										<ol
											className={cn(
												"relative flex flex-col gap-3 before:absolute before:top-4 before:bottom-4 before:left-[7px]",
												group.key.kind === "upcoming"
													? "before:border-l before:border-dashed before:border-border"
													: "before:w-px before:bg-border",
											)}
										>
											{group.facts.map((fact) => card(fact, true))}
										</ol>
									</section>
								))}
							</div>
						)}
					</>
				)}
			</div>
		</section>
	);
}

function MemoryEmpty({
	name,
	memoryOff,
}: Readonly<{ name: string; memoryOff: boolean }>) {
	const { t } = useTranslation("bots");
	return (
		<div className="mt-3 flex flex-col items-center rounded-2xl border border-dashed border-border bg-card/50 px-6 py-10 text-center">
			<span
				aria-hidden
				className="relative mb-5 flex h-16 w-24 items-end justify-center"
			>
				{(["profile", "event", "preference"] as const).map((kind, index) => {
					const { icon: Icon, color } = memoryKindStyle(kind);
					return (
						<span
							key={kind}
							className={cn(
								"absolute flex size-11 items-center justify-center rounded-2xl border border-border/60 bg-card shadow-xs",
								index === 0 && "-translate-x-7 translate-y-1 -rotate-8",
								index === 1 && "z-10 -translate-y-2",
								index === 2 && "translate-x-7 translate-y-1 rotate-8",
							)}
						>
							<span
								className={cn(
									"flex size-7 items-center justify-center rounded-lg",
									color,
								)}
							>
								<Icon className="size-3.5" />
							</span>
						</span>
					);
				})}
			</span>
			<h4 className="text-sm font-medium">
				{t("memory.emptyTitle", "No memories saved yet")}
			</h4>
			<p className="mt-2 max-w-64 text-xs leading-relaxed text-muted-foreground">
				{memoryOff
					? t("memory.factsEmptyOff", "Nothing is saved while memory is off.")
					: t(
							"memory.factsEmpty",
							"{{name}} writes down facts, preferences and decisions here as you chat.",
							{ name },
						)}
			</p>
		</div>
	);
}
