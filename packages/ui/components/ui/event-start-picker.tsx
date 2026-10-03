"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueries } from "@tanstack/react-query";
import { ChevronDown, FileText, Loader2, Search } from "lucide-react";
import { useId, useState } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import { formatEventTypeLabel } from "../../lib/event-type-label";
import { asArray, isRecord } from "../../lib/response-shape";
import type { IExecutionMode } from "../../lib/schema/flow/board";
import type { BoardVersion } from "../../lib/schema/flow/board-version";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { IEventMapping } from "../interfaces/interfaces";
import { Button } from "./button";
import { Input } from "./input";

export interface EventStartSelection {
	boardId: string;
	nodeId?: string;
	pageId?: string;
	boardVersion?: BoardVersion;
}

export interface EventStartTarget extends EventStartSelection {
	name: string;
	nodeType?: string;
	boardExecutionMode?: IExecutionMode;
}

export interface EventStartPickerProps {
	appId: string;
	eventConfig: IEventMapping;
	selected: EventStartSelection;
	onSelect(target: EventStartTarget): void;
	disabled?: boolean;
}

interface StartRow extends EventStartTarget {
	key: string;
	flowName: string;
	kind: string;
	unavailable?: boolean;
}

function isSelected(row: StartRow, selected: EventStartSelection): boolean {
	return row.pageId
		? row.pageId === selected.pageId
		: !selected.pageId &&
				row.boardId === selected.boardId &&
				row.nodeId === selected.nodeId;
}

export function EventStartPicker({
	appId,
	eventConfig,
	selected,
	onSelect,
	disabled = false,
}: Readonly<EventStartPickerProps>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const id = useId();
	const [search, setSearch] = useState("");
	const [flow, setFlow] = useState("");
	const summaries = useInvoke(
		backend.boardState.getBoardSummaries,
		backend.boardState,
		[appId, ["node_types"]],
		!!appId,
	);
	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId],
		!!appId,
	);
	const flows = asArray(summaries.data);
	// Older summaries have no entry nodes. A pinned flow also needs its exact graph.
	const boardReads = useQueries({
		queries: flows.map((summary) => {
			const version =
				summary.id === selected.boardId ? selected.boardVersion : undefined;
			return {
				queryKey: [
					backend.boardState.getBoard.name || "backendFn",
					appId,
					summary.id,
					...(version ? [version] : []),
				],
				queryFn: () => backend.boardState.getBoard(appId, summary.id, version),
				enabled: !!appId && (!!version || !Array.isArray(summary.entryNodes)),
			};
		}),
	});
	const rows: StartRow[] = [];
	const problems: { key: string; message: string; retry(): void }[] = [];
	if (summaries.isError)
		problems.push({
			key: "flows",
			message: t("eventStartPicker.flowsError", "Could not load flows."),
			retry: () => void summaries.refetch(),
		});
	if (pages.isError)
		problems.push({
			key: "pages",
			message: t("eventStartPicker.pagesError", "Could not load pages."),
			retry: () => void pages.refetch(),
		});
	flows.forEach((summary, index) => {
		const read = boardReads[index];
		const version =
			summary.id === selected.boardId ? selected.boardVersion : undefined;
		const needsGraph = !!version || !Array.isArray(summary.entryNodes);
		if (needsGraph && read.isError)
			problems.push({
				key: summary.id,
				message: t(
					"eventStartPicker.boardError",
					"Could not load start nodes in {{flow}}.",
					{ flow: summary.name },
				),
				retry: () => void read.refetch(),
			});
		const entries = needsGraph
			? isRecord(read.data?.nodes)
				? Object.values(read.data.nodes)
						.filter((node) => node.start)
						.map((node) => ({
							nodeId: node.id,
							nodeType: node.name,
							friendlyName: node.friendly_name,
						}))
				: []
			: (summary.entryNodes ?? []);
		for (const entry of entries) {
			const kind = eventConfig[entry.nodeType]
				? formatEventTypeLabel(entry.nodeType.replace(/^events?_/, ""))
				: t("eventStartPicker.customStart", "Custom start");
			rows.push({
				key: `node:${summary.id}:${entry.nodeId}`,
				boardId: summary.id,
				nodeId: entry.nodeId,
				nodeType: entry.nodeType,
				boardVersion: version,
				boardExecutionMode: needsGraph
					? read.data?.execution_mode
					: summary.executionMode,
				name:
					entry.friendlyName ||
					formatEventTypeLabel(entry.nodeType.replace(/^events?_/, "")),
				flowName: summary.name,
				kind,
			});
		}
	});
	for (const page of asArray(pages.data)) {
		const summary = flows.find((item) => item.id === page.boardId);
		rows.push({
			key: `page:${page.pageId}`,
			boardId: page.boardId ?? "",
			pageId: page.pageId,
			name: page.name,
			flowName: summary?.name ?? t("pages", "Pages"),
			kind: t("page", "Page"),
			boardExecutionMode: summary?.executionMode,
			unavailable: page.unavailable,
		});
	}
	const loading =
		summaries.isLoading ||
		pages.isLoading ||
		boardReads.some((read) => read.isLoading);
	const needle = search.trim().toLocaleLowerCase();
	const filtered = rows.filter(
		(row) =>
			(!flow || row.boardId === flow) &&
			(!needle ||
				`${row.name} ${row.flowName} ${row.kind}`
					.toLocaleLowerCase()
					.includes(needle)),
	);
	const groups = new Map<string, { name: string; rows: StartRow[] }>();
	for (const row of filtered) {
		const group = groups.get(row.boardId) ?? { name: row.flowName, rows: [] };
		group.rows.push(row);
		groups.set(row.boardId, group);
	}
	const chosen = rows.find((row) => isSelected(row, selected));
	const count =
		loading || problems.length
			? t("eventStartPicker.loadedCount", "{{count}} loaded", {
					count: rows.length,
				})
			: needle || flow
				? t("eventStartPicker.filteredCount", "{{shown}} of {{count}}", {
						shown: filtered.length,
						count: rows.length,
					})
				: t("eventStartPicker.totalCount", "{{count}} in {{flows}} flows", {
						count: rows.length,
						flows: flows.length,
					});

	return (
		<div className="space-y-2">
			<div className="flex items-baseline justify-between gap-3">
				<span id={`${id}-label`} className="text-sm font-medium">
					{t("eventStartPicker.title", "Start node")}
				</span>
				<span className="text-xs text-muted-foreground" aria-live="polite">
					{count}
				</span>
			</div>
			<div className="flex gap-2">
				<div className="relative min-w-0 flex-1">
					<Search
						className="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground"
						aria-hidden="true"
					/>
					<Input
						type="search"
						aria-label={t(
							"eventStartPicker.searchPrompt",
							"Search start nodes, pages and flows",
						)}
						placeholder={t(
							"eventStartPicker.searchPrompt",
							"Search start nodes, pages and flows",
						)}
						className="h-10 pl-9"
						value={search}
						onChange={(event) => setSearch(event.target.value)}
						disabled={disabled}
					/>
				</div>
				<div className="relative max-w-[40%]">
					<select
						aria-label={t("eventStartPicker.flowFilter", "Filter by flow")}
						value={flow}
						onChange={(event) => setFlow(event.target.value)}
						disabled={disabled}
						className="h-10 w-full appearance-none truncate rounded-md border border-input bg-background pl-3 pr-8 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
					>
						<option value="">
							{t("eventStartPicker.allFlows", "All flows")}
						</option>
						{flows.map((item) => (
							<option key={item.id} value={item.id}>
								{item.name}
							</option>
						))}
					</select>
					<ChevronDown
						className="pointer-events-none absolute right-2 top-3 size-4 text-muted-foreground"
						aria-hidden="true"
					/>
				</div>
			</div>
			<div
				role="radiogroup"
				aria-labelledby={`${id}-label`}
				aria-busy={loading}
				className="max-h-[220px] min-h-32 overflow-y-auto rounded-md border bg-background p-1"
			>
				{[...groups].map(([key, group]) => (
					<div key={key}>
						<div className="sticky top-0 z-10 flex items-center justify-between bg-background px-3 py-2 text-xs text-muted-foreground">
							<span className="truncate font-medium">{group.name}</span>
							<span>{group.rows.length}</span>
						</div>
						{group.rows.map((row) => {
							const checked = isSelected(row, selected);
							return (
								<label
									key={row.key}
									className={cn(
										"relative flex cursor-pointer items-center gap-3 rounded-md px-3 py-2.5 hover:bg-muted/50 has-focus-visible:ring-2 has-focus-visible:ring-ring",
										checked && "bg-primary/8 hover:bg-primary/10",
										(disabled || row.unavailable) &&
											"cursor-not-allowed opacity-50",
									)}
								>
									<input
										type="radio"
										name={`${id}-start`}
										className="peer sr-only"
										aria-label={`${row.flowName}: ${row.name}`}
										checked={checked}
										disabled={disabled || row.unavailable}
										onChange={() =>
											onSelect({
												boardId: row.boardId,
												nodeId: row.nodeId,
												pageId: row.pageId,
												boardVersion: row.boardVersion,
												name: row.name,
												nodeType: row.nodeType,
												boardExecutionMode: row.boardExecutionMode,
											})
										}
									/>
									<span
										aria-hidden="true"
										className={cn(
											"flex size-4 shrink-0 items-center justify-center rounded-full border border-muted-foreground/50",
											checked && "border-primary",
										)}
									>
										{checked && (
											<span className="size-2 rounded-full bg-primary" />
										)}
									</span>
									<span className="min-w-0 flex-1">
										<span
											className={cn(
												"block truncate text-sm",
												checked && "font-medium",
											)}
										>
											{row.name}
										</span>
										<span className="block text-xs text-muted-foreground">
											{row.unavailable
												? t(
														"eventStartPicker.pageUnavailable",
														"Page unavailable",
													)
												: row.kind}
										</span>
									</span>
									{row.pageId && (
										<FileText
											aria-hidden="true"
											className="size-4 text-muted-foreground"
										/>
									)}
								</label>
							);
						})}
					</div>
				))}
				{loading && (
					<output className="flex items-center justify-center gap-2 px-3 py-4 text-xs text-muted-foreground">
						<Loader2 aria-hidden="true" className="size-4 animate-spin" />
						{t("eventStartPicker.loading", "Loading start nodes and pages…")}
					</output>
				)}
				{!loading && filtered.length === 0 && (
					<div className="px-3 py-6 text-center text-sm text-muted-foreground">
						{problems.length
							? t(
									"eventStartPicker.incomplete",
									"No matches in the available results. Retry the failed reads below.",
								)
							: needle || flow
								? t(
										"eventStartPicker.noMatches",
										"No start node or page matches.",
									)
								: t(
										"eventStartPicker.empty",
										"Add a start node or page to a flow to create an event.",
									)}
						{(needle || flow) && (
							<Button
								type="button"
								variant="link"
								size="sm"
								className="mx-auto block"
								disabled={disabled}
								onClick={() => {
									setSearch("");
									setFlow("");
								}}
							>
								{t("eventStartPicker.clear", "Clear search and filter")}
							</Button>
						)}
					</div>
				)}
			</div>
			{problems.map((problem) => (
				<div
					key={problem.key}
					role="alert"
					className="flex items-center justify-between gap-2 rounded-md border border-destructive/25 bg-destructive/5 px-3 py-2 text-xs"
				>
					<span>{problem.message}</span>
					<Button
						type="button"
						variant="ghost"
						size="sm"
						disabled={disabled}
						onClick={problem.retry}
					>
						{t("retry", "Retry")}
					</Button>
				</div>
			))}
			{chosen && (
				<p className="text-xs text-muted-foreground">
					{t("eventStartPicker.chosen", "Chosen")}{" "}
					<span className="text-foreground">
						{chosen.flowName} <span aria-hidden="true">›</span> {chosen.name}
					</span>
				</p>
			)}
		</div>
	);
}
