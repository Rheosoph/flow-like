"use client";

import ArrowUpRight from "lucide-react/dist/esm/icons/arrow-up-right.js";
import Check from "lucide-react/dist/esm/icons/check.js";
import CircleMinus from "lucide-react/dist/esm/icons/circle-minus.js";
import Loader2 from "lucide-react/dist/esm/icons/loader-2.js";
import Search from "lucide-react/dist/esm/icons/search.js";
import { useState } from "react";
import { cn } from "../../../lib/utils";
import type { IPlanStep, PlanStepStatus } from "./chat-db";
import { type ActivitySource, WebsiteIcon } from "./tool-activity";

const SEARCH_TOOLS = new Set(["internet_search", "web_search", "websearch"]);
const READ_TOOLS = new Set([
	"open_url",
	"web_fetch",
	"webfetch",
	"fetch_url",
	"http_request",
]);

function toolKey(step: IPlanStep) {
	return (
		(step.toolName ?? "")
			.split(/__|[./]/)
			.at(-1)
			?.toLowerCase() ?? ""
	);
}

/** Keep catalog lookups and HTTP writes in their own activity rows. */
export function getResearchSteps(steps: IPlanStep[]): IPlanStep[] {
	const hasResearch = steps.some((step) => {
		const key = toolKey(step);
		return key === "research_agent" || SEARCH_TOOLS.has(key);
	});
	if (!hasResearch) return [];
	return steps.filter((step) => {
		const key = toolKey(step);
		if (
			key === "research_agent" ||
			key === "archive_lookup" ||
			SEARCH_TOOLS.has(key)
		)
			return true;
		if (!READ_TOOLS.has(key)) return false;
		return (
			key !== "http_request" ||
			(step.detail?.kind === "tool_activity" &&
				["GET", "HEAD"].includes(step.detail.method ?? ""))
		);
	});
}

type SourceState =
	| "found"
	| "reading"
	| "checking"
	| "read"
	| "checked"
	| "unavailable"
	| "queued";
interface ResearchSource extends ActivitySource {
	state: SourceState;
}

function collectResearch(steps: IPlanStep[]) {
	const sources = new Map<string, ResearchSource>();
	const queries: { id: string; text: string; status: PlanStepStatus }[] = [];
	for (const step of steps) {
		if (step.detail?.kind !== "tool_activity") continue;
		const detail = step.detail;
		const key = toolKey(step);
		if (SEARCH_TOOLS.has(key)) {
			const texts = detail.queries?.length
				? detail.queries
				: detail.query
					? [detail.query]
					: [];
			for (const [index, text] of texts.entries()) {
				queries.push({ id: `${step.id}:${index}`, text, status: step.status });
			}
		}
		const reading = READ_TOOLS.has(key);
		const archive = key === "archive_lookup";
		const state: SourceState =
			!reading && !archive
				? "found"
				: step.status === "progress"
					? archive
						? "checking"
						: "reading"
					: step.status === "failed"
						? "unavailable"
						: step.status === "planned"
							? "queued"
							: archive || detail.method === "HEAD"
								? "checked"
								: "read";
		for (const source of detail.sources ?? []) {
			const previous = sources.get(source.url);
			sources.set(source.url, {
				...previous,
				...source,
				title: source.title ?? previous?.title,
				linkable: previous?.linkable === false ? false : source.linkable,
				// A later search may rediscover a page we have already opened.
				state: state === "found" && previous ? previous.state : state,
			});
		}
	}
	return { queries, sources: [...sources.values()] };
}

const SOURCE_LABELS: Record<SourceState, string> = {
	found: "Found",
	reading: "Reading",
	checking: "Checking",
	read: "Read",
	checked: "Checked",
	unavailable: "Unavailable",
	queued: "Queued",
};

function SourceCard({ source }: { source: ResearchSource }) {
	const pending = source.state === "reading" || source.state === "checking";
	const className = cn(
		"group/source relative flex min-w-0 flex-col gap-2 rounded-lg border bg-background/70 p-3 transition-colors",
		pending ? "border-primary/35 bg-primary/[0.04]" : "border-border/60",
		source.linkable !== false &&
			"hover:border-primary/35 hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50",
	);
	const content = (
		<>
			<span className="flex min-w-0 items-center gap-2">
				<span className="flex size-7 shrink-0 items-center justify-center rounded-md border border-border/50 bg-background">
					<WebsiteIcon source={source} />
				</span>
				<span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/80">
					{source.hostname.replace(/^www\./, "")}
				</span>
				<span
					className={cn(
						"flex shrink-0 items-center gap-1 text-[10px]",
						pending
							? "text-primary"
							: source.state === "read"
								? "text-emerald-700 dark:text-emerald-400"
								: "text-muted-foreground",
					)}
				>
					{pending && (
						<Loader2
							className="size-3 motion-safe:animate-spin"
							aria-hidden="true"
						/>
					)}
					{source.state === "read" && (
						<Check className="size-3" aria-hidden="true" />
					)}
					{source.state === "unavailable" && (
						<CircleMinus className="size-3" aria-hidden="true" />
					)}
					{SOURCE_LABELS[source.state]}
				</span>
			</span>
			{source.title && (
				<span className="line-clamp-2 text-xs font-medium leading-relaxed text-foreground/90">
					{source.title}
				</span>
			)}
			<span className="flex min-w-0 items-end gap-2">
				<span className="min-w-0 flex-1 break-all font-mono text-[10px] leading-relaxed text-muted-foreground/80">
					{source.url}
				</span>
				{source.linkable !== false && (
					<ArrowUpRight
						className="size-3.5 shrink-0 text-muted-foreground/50 transition-colors group-hover/source:text-primary"
						aria-hidden="true"
					/>
				)}
			</span>
		</>
	);
	return source.linkable === false ? (
		<div
			className={className}
			title={source.url}
			data-research-source={source.url}
		>
			{content}
		</div>
	) : (
		<a
			href={source.url}
			target="_blank"
			rel="noopener noreferrer"
			className={className}
			title={source.url}
			data-research-source={source.url}
		>
			{content}
		</a>
	);
}

export function ResearchActivity({ steps }: { steps: IPlanStep[] }) {
	const [showAllSources, setShowAllSources] = useState(false);
	const { queries, sources } = collectResearch(steps);
	const running = steps.some(
		(step) => step.status === "progress" || step.status === "planned",
	);
	const websites = new Set(sources.map((source) => source.hostname)).size;
	const activeRead = sources.findLast((source) => source.state === "reading");
	const activeQuery = queries.findLast((query) => query.status === "progress");
	const summary = activeRead
		? `Reading ${activeRead.hostname.replace(/^www\./, "")}`
		: activeQuery
			? "Searching the web"
			: running
				? sources.length
					? "Researching sources"
					: "Planning the search"
				: "Research activity";
	return (
		<section
			className="relative my-1 min-w-0 overflow-hidden rounded-xl border border-border/70 bg-gradient-to-br from-primary/[0.04] via-background/70 to-background"
			aria-label="Web research"
			data-research-activity
		>
			{running && (
				<div className="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/70 to-transparent motion-safe:animate-pulse" />
			)}
			<div className="flex items-center gap-3 px-4 pt-4 pb-3">
				<span
					className="relative flex size-9 shrink-0 items-center justify-center rounded-full border border-primary/20 bg-primary/10 text-primary"
					aria-hidden="true"
				>
					{activeRead ? (
						<WebsiteIcon source={activeRead} />
					) : (
						<Search className="size-4" />
					)}
					{running && (
						<span className="absolute -right-0.5 bottom-0 size-2.5 rounded-full border-2 border-background bg-primary motion-safe:animate-pulse" />
					)}
				</span>
				<div className="min-w-0 flex-1">
					<div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">
						Research trail
					</div>
					<output className="mt-0.5 block truncate text-sm font-medium">
						{summary}
					</output>
				</div>
				{running && (
					<span className="flex shrink-0 items-center gap-1.5 rounded-full border border-primary/20 bg-primary/5 px-2 py-1 text-[9px] font-semibold tracking-wider text-primary">
						<span className="size-1.5 rounded-full bg-primary motion-safe:animate-pulse" />
						LIVE
					</span>
				)}
			</div>
			<div className="mx-4 flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-border/60 pb-3 text-[11px] text-muted-foreground">
				<span>
					<strong className="font-semibold tabular-nums text-foreground/85">
						{queries.length}
					</strong>{" "}
					{queries.length === 1 ? "search" : "searches"}
				</span>
				<span
					className="size-0.5 rounded-full bg-muted-foreground/40"
					aria-hidden="true"
				/>
				<span>
					<strong className="font-semibold tabular-nums text-foreground/85">
						{sources.length}
					</strong>{" "}
					{sources.length === 1 ? "page" : "pages"}
				</span>
				<span
					className="size-0.5 rounded-full bg-muted-foreground/40"
					aria-hidden="true"
				/>
				<span>
					<strong className="font-semibold tabular-nums text-foreground/85">
						{websites}
					</strong>{" "}
					{websites === 1 ? "website" : "websites"}
				</span>
			</div>
			{queries.length > 0 && (
				<div className="px-4 pt-3">
					<h4 className="mb-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
						Searches
					</h4>
					<ol
						aria-label="Search queries"
						// biome-ignore lint/a11y/noNoninteractiveTabindex: Makes the scrollable search history keyboard accessible.
						tabIndex={0}
						className="max-h-44 space-y-1 overflow-y-auto overscroll-contain rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
					>
						{queries.map((query, index) => (
							<li
								key={query.id}
								data-research-query
								className="flex items-start gap-2.5 rounded-md bg-muted/30 px-2.5 py-2 text-xs"
							>
								<span className="mt-px shrink-0 font-mono text-[10px] tabular-nums text-muted-foreground/55">
									{String(index + 1).padStart(2, "0")}
								</span>
								<span className="min-w-0 flex-1 break-words leading-relaxed text-foreground/85">
									{query.text}
								</span>
								<span
									className="mt-0.5 shrink-0"
									aria-label={
										query.status === "progress"
											? "Searching"
											: query.status === "failed"
												? "Search failed"
												: query.status === "planned"
													? "Queued"
													: "Search complete"
									}
								>
									{query.status === "progress" ? (
										<Loader2 className="size-3.5 text-primary motion-safe:animate-spin" />
									) : query.status === "failed" ? (
										<CircleMinus className="size-3.5 text-muted-foreground" />
									) : query.status === "done" ? (
										<Check className="size-3.5 text-muted-foreground/70" />
									) : (
										<span className="block size-1.5 rounded-full bg-muted-foreground/40" />
									)}
								</span>
							</li>
						))}
					</ol>
				</div>
			)}
			{sources.length > 0 && (
				<div className="px-4 pt-3 pb-4">
					<div className="mb-2 flex flex-wrap items-center justify-between gap-1 text-[10px] text-muted-foreground">
						<h4 className="text-[10px] font-medium uppercase tracking-wider">
							Sources
						</h4>
						<span>Search results and opened pages</span>
					</div>
					<section
						aria-label="Sources"
						// biome-ignore lint/a11y/noNoninteractiveTabindex: Makes the scrollable source list keyboard accessible.
						tabIndex={0}
						className={cn(
							"grid grid-cols-[repeat(auto-fit,minmax(min(100%,14rem),1fr))] gap-2 overflow-y-auto overscroll-contain rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40",
							!showAllSources && "max-h-80",
						)}
					>
						{sources.map((source) => (
							<SourceCard key={source.url} source={source} />
						))}
					</section>
					{sources.length > 2 && (
						<button
							type="button"
							onClick={() => setShowAllSources(!showAllSources)}
							aria-expanded={showAllSources}
							className="mt-2 w-full rounded-md py-1.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
						>
							{showAllSources
								? "Show compact view"
								: `Show all ${sources.length} pages`}
						</button>
					)}
				</div>
			)}
			{!sources.length && (
				<p className="px-4 py-4 text-xs text-muted-foreground">
					{running
						? "Pages will appear here as the search progresses."
						: "No source pages were returned."}
				</p>
			)}
		</section>
	);
}
