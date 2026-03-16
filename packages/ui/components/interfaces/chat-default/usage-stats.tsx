"use client";

import { ActivityIcon, ChevronRightIcon, CoinsIcon, ZapIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { cn } from "../../../lib";
import {
	Badge,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../ui";
import type { IChatUsageStat, IModelCallEntry } from "./chat-db";

function formatTokenCount(count: number): string {
	if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
	if (count >= 1_000) return `${(count / 1_000).toFixed(1)}k`;
	return count.toString();
}

function formatCost(cost: number): string {
	if (cost < 0.01) return `$${cost.toFixed(4)}`;
	if (cost < 1) return `$${cost.toFixed(3)}`;
	return `$${cost.toFixed(2)}`;
}

function formatDuration(ms: number): string {
	if (ms < 1000) return `${ms}ms`;
	return `${(ms / 1000).toFixed(1)}s`;
}

interface AggregatedStats {
	totalTokens: number;
	promptTokens: number;
	completionTokens: number;
	totalCost: number | null;
	totalDuration: number | null;
	byModel: Map<string, { tokens: number; cost: number | null; calls: number }>;
}

function aggregateStats(stats: IChatUsageStat[]): AggregatedStats {
	let totalTokens = 0;
	let promptTokens = 0;
	let completionTokens = 0;
	let totalCost: number | null = null;
	let totalDuration: number | null = null;
	const byModel = new Map<
		string,
		{ tokens: number; cost: number | null; calls: number }
	>();

	for (const stat of stats) {
		const u = stat.stats.usage;
		totalTokens += u.total_tokens;
		promptTokens += u.prompt_tokens;
		completionTokens += u.completion_tokens;
		if (u.cost != null) {
			totalCost = (totalCost ?? 0) + u.cost;
		}
		if (stat.stats.duration_ms != null) {
			totalDuration = (totalDuration ?? 0) + stat.stats.duration_ms;
		}

		const calls = stat.stats.calls ?? [];
		for (const call of calls) {
			const modelName = call.model || "unknown";
			const existing = byModel.get(modelName) ?? {
				tokens: 0,
				cost: null,
				calls: 0,
			};
			existing.tokens += call.usage.total_tokens;
			if (call.usage.cost != null) {
				existing.cost = (existing.cost ?? 0) + call.usage.cost;
			}
			existing.calls += 1;
			byModel.set(modelName, existing);
		}

		if (calls.length === 0 && stat.stats.model) {
			const modelName = stat.stats.model;
			const existing = byModel.get(modelName) ?? {
				tokens: 0,
				cost: null,
				calls: 0,
			};
			existing.tokens += u.total_tokens;
			if (u.cost != null) {
				existing.cost = (existing.cost ?? 0) + u.cost;
			}
			existing.calls += 1;
			byModel.set(modelName, existing);
		}
	}

	return {
		totalTokens,
		promptTokens,
		completionTokens,
		totalCost,
		totalDuration,
		byModel,
	};
}

function StepDetail({ stat }: { stat: IChatUsageStat }) {
	const calls = stat.stats.calls ?? [];

	return (
		<div className="rounded-lg border bg-muted/30 p-3 space-y-2">
			<div className="flex items-center justify-between">
				<span className="text-sm font-medium">{stat.step_name}</span>
				{stat.stats.duration_ms != null && (
					<span className="text-xs text-muted-foreground">
						{formatDuration(stat.stats.duration_ms)}
					</span>
				)}
			</div>
			<div className="flex gap-3 text-xs text-muted-foreground">
				<span>{formatTokenCount(stat.stats.usage.total_tokens)} tokens</span>
				<span>
					({formatTokenCount(stat.stats.usage.prompt_tokens)} in /{" "}
					{formatTokenCount(stat.stats.usage.completion_tokens)} out)
				</span>
				{stat.stats.usage.cost != null && (
					<span>{formatCost(stat.stats.usage.cost)}</span>
				)}
			</div>
			{calls.length > 0 && (
				<div className="space-y-1 pt-1 border-t border-border/50">
					{calls.map((call, idx) => (
						<ModelCallRow key={`${call.model}-${idx}`} call={call} />
					))}
				</div>
			)}
			{stat.stats.iterations != null && stat.stats.iterations > 1 && (
				<div className="text-xs text-muted-foreground pt-1">
					{stat.stats.iterations} iterations
				</div>
			)}
		</div>
	);
}

function ModelCallRow({ call }: { call: IModelCallEntry }) {
	return (
		<div className="flex items-center justify-between text-xs py-0.5">
			<span className="text-muted-foreground truncate max-w-50">
				{call.model || "unknown"}
			</span>
			<div className="flex gap-2 text-muted-foreground">
				<span>{formatTokenCount(call.usage.total_tokens)} tok</span>
				{call.usage.cost != null && <span>{formatCost(call.usage.cost)}</span>}
				{call.duration_ms != null && (
					<span>{formatDuration(call.duration_ms)}</span>
				)}
			</div>
		</div>
	);
}

function ModelBreakdownTable({
	byModel,
}: {
	byModel: Map<string, { tokens: number; cost: number | null; calls: number }>;
}) {
	const entries = Array.from(byModel.entries()).sort(
		(a, b) => b[1].tokens - a[1].tokens,
	);

	if (entries.length === 0) return null;

	return (
		<div className="space-y-1">
			<div className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
				By Model
			</div>
			{entries.map(([model, data]) => (
				<div
					key={model}
					className="flex items-center justify-between text-sm py-1"
				>
					<span className="truncate max-w-50">{model}</span>
					<div className="flex gap-3 text-muted-foreground text-xs">
						<span>{data.calls}x</span>
						<span>{formatTokenCount(data.tokens)} tok</span>
						{data.cost != null && <span>{formatCost(data.cost)}</span>}
					</div>
				</div>
			))}
		</div>
	);
}

export function UsageStats({
	stats,
	className,
}: { stats: IChatUsageStat[]; className?: string }) {
	const [sheetOpen, setSheetOpen] = useState(false);

	const aggregated = useMemo(() => aggregateStats(stats), [stats]);

	const handleClick = useCallback(() => {
		setSheetOpen(true);
	}, []);

	const inlineParts: string[] = [];
	inlineParts.push(`${formatTokenCount(aggregated.totalTokens)} tokens`);
	if (aggregated.totalCost != null) {
		inlineParts.push(formatCost(aggregated.totalCost));
	}
	if (aggregated.totalDuration != null) {
		inlineParts.push(formatDuration(aggregated.totalDuration));
	}

	return (
		<>
			<TooltipProvider delayDuration={300}>
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							onClick={handleClick}
							className={cn(
								"inline-flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors py-0.5 px-2 rounded-md hover:bg-muted/50",
								className,
							)}
						>
							<ZapIcon className="w-3 h-3" />
							<span>{inlineParts.join(" · ")}</span>
							<ChevronRightIcon className="w-3 h-3 opacity-50" />
						</button>
					</TooltipTrigger>
					<TooltipContent side="top" className="max-w-xs">
						<div className="space-y-1 text-xs">
							<div className="flex items-center gap-1">
								<ActivityIcon className="w-3 h-3" />
								<span>
									{formatTokenCount(aggregated.promptTokens)} in /{" "}
									{formatTokenCount(aggregated.completionTokens)} out
								</span>
							</div>
							{aggregated.totalCost != null && (
								<div className="flex items-center gap-1">
									<CoinsIcon className="w-3 h-3" />
									<span>{formatCost(aggregated.totalCost)}</span>
								</div>
							)}
							<div className="text-muted-foreground">
								{stats.length} step{stats.length !== 1 ? "s" : ""} · Click for
								details
							</div>
						</div>
					</TooltipContent>
				</Tooltip>
			</TooltipProvider>

			<Sheet open={sheetOpen} onOpenChange={setSheetOpen}>
				<SheetContent side="right" className="w-100 sm:w-112.5">
					<SheetHeader>
						<SheetTitle className="flex items-center gap-2">
							<ZapIcon className="w-4 h-4" />
							Model Usage
						</SheetTitle>
						<SheetDescription>
							Detailed breakdown of model invocations for this message
						</SheetDescription>
					</SheetHeader>

					<div className="mt-6 space-y-6 overflow-y-auto max-h-[calc(100vh-12rem)]">
						<div className="grid grid-cols-2 gap-3">
							<div className="rounded-lg border p-3">
								<div className="text-xs text-muted-foreground">
									Total Tokens
								</div>
								<div className="text-lg font-semibold">
									{formatTokenCount(aggregated.totalTokens)}
								</div>
								<div className="text-xs text-muted-foreground">
									{formatTokenCount(aggregated.promptTokens)} in /{" "}
									{formatTokenCount(aggregated.completionTokens)} out
								</div>
							</div>
							{aggregated.totalCost != null && (
								<div className="rounded-lg border p-3">
									<div className="text-xs text-muted-foreground">
										Total Cost
									</div>
									<div className="text-lg font-semibold">
										{formatCost(aggregated.totalCost)}
									</div>
								</div>
							)}
							{aggregated.totalDuration != null && (
								<div className="rounded-lg border p-3">
									<div className="text-xs text-muted-foreground">
										Total Duration
									</div>
									<div className="text-lg font-semibold">
										{formatDuration(aggregated.totalDuration)}
									</div>
								</div>
							)}
							<div className="rounded-lg border p-3">
								<div className="text-xs text-muted-foreground">Steps</div>
								<div className="text-lg font-semibold">{stats.length}</div>
							</div>
						</div>

						<ModelBreakdownTable byModel={aggregated.byModel} />

						<div className="space-y-2">
							<div className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
								Timeline
							</div>
							{stats.map((stat, idx) => (
								<StepDetail key={`${stat.step_name}-${idx}`} stat={stat} />
							))}
						</div>
					</div>
				</SheetContent>
			</Sheet>
		</>
	);
}
