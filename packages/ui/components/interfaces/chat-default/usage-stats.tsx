"use client";

import {
	ArrowDownIcon,
	ArrowUpIcon,
	BrainIcon,
	ChevronRightIcon,
	ClockIcon,
	CoinsIcon,
	HashIcon,
	LayersIcon,
	RepeatIcon,
	ZapIcon,
} from "lucide-react";
import {
	createContext,
	useCallback,
	useContext,
	useMemo,
	useState,
} from "react";
import { useModelNames } from "../../../hooks/use-model-names";
import { cn, modelLabel } from "../../../lib";
import {
	Badge,
	Separator,
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

// --- Formatting helpers ---

function formatTokenCount(count: number): string {
	if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
	if (count >= 1_000) return `${(count / 1_000).toFixed(1)}k`;
	return count.toString();
}

function formatTokenCountFull(count: number): string {
	return count.toLocaleString();
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

// --- Model name helpers ---

/**
 * Bit id → catalog name, resolved once per sheet and read by every badge below.
 * Providers report the Bit id as their model whenever the model definition has
 * no explicit `model_id`, which is what would otherwise be rendered raw.
 */
const ModelNamesContext = createContext<ReadonlyMap<string, string>>(new Map());

// --- Token intensity ---

type TokenIntensity = "low" | "moderate" | "high" | "very-high";

function getTokenIntensity(tokens: number): TokenIntensity {
	if (tokens < 1_000) return "low";
	if (tokens < 10_000) return "moderate";
	if (tokens < 50_000) return "high";
	return "very-high";
}

function intensityColor(intensity: TokenIntensity): string {
	switch (intensity) {
		case "low":
			return "text-emerald-500";
		case "moderate":
			return "text-amber-500";
		case "high":
			return "text-orange-500";
		case "very-high":
			return "text-red-500";
	}
}

function intensityBg(intensity: TokenIntensity): string {
	switch (intensity) {
		case "low":
			return "bg-emerald-500/15 border-emerald-500/30";
		case "moderate":
			return "bg-amber-500/15 border-amber-500/30";
		case "high":
			return "bg-orange-500/15 border-orange-500/30";
		case "very-high":
			return "bg-red-500/15 border-red-500/30";
	}
}

function intensityLabel(intensity: TokenIntensity): string {
	switch (intensity) {
		case "low":
			return "Low usage";
		case "moderate":
			return "Moderate";
		case "high":
			return "High usage";
		case "very-high":
			return "Very high";
	}
}

// --- Deterministic color for model names ---

function hashString(str: string): number {
	let hash = 0;
	for (let i = 0; i < str.length; i++) {
		hash = (hash << 5) - hash + str.charCodeAt(i);
		hash |= 0;
	}
	return Math.abs(hash);
}

const MODEL_COLORS = [
	"bg-blue-500/20 text-blue-400 border-blue-500/30",
	"bg-violet-500/20 text-violet-400 border-violet-500/30",
	"bg-cyan-500/20 text-cyan-400 border-cyan-500/30",
	"bg-pink-500/20 text-pink-400 border-pink-500/30",
	"bg-teal-500/20 text-teal-400 border-teal-500/30",
	"bg-indigo-500/20 text-indigo-400 border-indigo-500/30",
	"bg-rose-500/20 text-rose-400 border-rose-500/30",
	"bg-sky-500/20 text-sky-400 border-sky-500/30",
];

const MODEL_BAR_COLORS = [
	"bg-blue-500",
	"bg-violet-500",
	"bg-cyan-500",
	"bg-pink-500",
	"bg-teal-500",
	"bg-indigo-500",
	"bg-rose-500",
	"bg-sky-500",
];

function modelColor(model: string): string {
	return MODEL_COLORS[hashString(model) % MODEL_COLORS.length];
}

function modelBarColor(model: string): string {
	return MODEL_BAR_COLORS[hashString(model) % MODEL_BAR_COLORS.length];
}

// --- Aggregation ---

interface AggregatedModelEntry {
	tokens: number;
	promptTokens: number;
	completionTokens: number;
	cost: number | null;
	calls: number;
}

interface AggregatedStats {
	totalTokens: number;
	promptTokens: number;
	completionTokens: number;
	totalCost: number | null;
	totalDuration: number | null;
	byModel: Map<string, AggregatedModelEntry>;
}

function aggregateStats(stats: IChatUsageStat[]): AggregatedStats {
	let totalTokens = 0;
	let promptTokens = 0;
	let completionTokens = 0;
	let totalCost: number | null = null;
	let totalDuration: number | null = null;
	const byModel = new Map<string, AggregatedModelEntry>();

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
				promptTokens: 0,
				completionTokens: 0,
				cost: null,
				calls: 0,
			};
			existing.tokens += call.usage.total_tokens;
			existing.promptTokens += call.usage.prompt_tokens;
			existing.completionTokens += call.usage.completion_tokens;
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
				promptTokens: 0,
				completionTokens: 0,
				cost: null,
				calls: 0,
			};
			existing.tokens += u.total_tokens;
			existing.promptTokens += u.prompt_tokens;
			existing.completionTokens += u.completion_tokens;
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

// --- Token ratio bar ---

function TokenRatioBar({
	prompt,
	completion,
}: { prompt: number; completion: number }) {
	const total = prompt + completion;
	if (total === 0) return null;
	const promptPct = Math.round((prompt / total) * 100);
	const completionPct = 100 - promptPct;

	return (
		<div className="space-y-1.5">
			<div className="flex justify-between text-[11px] text-muted-foreground">
				<span className="flex items-center gap-1">
					<ArrowUpIcon className="w-3 h-3 text-blue-400" />
					Input {formatTokenCount(prompt)}{" "}
					<span className="opacity-60">({promptPct}%)</span>
				</span>
				<span className="flex items-center gap-1">
					Output {formatTokenCount(completion)}{" "}
					<span className="opacity-60">({completionPct}%)</span>
					<ArrowDownIcon className="w-3 h-3 text-emerald-400" />
				</span>
			</div>
			<div className="flex h-2 rounded-full overflow-hidden bg-muted/50">
				<div
					className="bg-blue-500 transition-all"
					style={{ width: `${promptPct}%` }}
				/>
				<div
					className="bg-emerald-500 transition-all"
					style={{ width: `${completionPct}%` }}
				/>
			</div>
		</div>
	);
}

// --- Model badge ---

function ModelBadge({ model }: { model: string }) {
	const names = useContext(ModelNamesContext);
	const { label, resolved, opaque } = modelLabel(model, names);

	const badge = (
		<Badge
			variant="outline"
			className={cn(
				"text-[11px] px-1.5 py-0 h-5 border",
				resolved ? "font-medium" : "font-mono",
				modelColor(model),
			)}
		>
			{opaque && <HashIcon className="w-2.5 h-2.5 mr-0.5 opacity-70" />}
			{label}
		</Badge>
	);

	if (!resolved && !opaque) return badge;

	return (
		<TooltipProvider delayDuration={200}>
			<Tooltip>
				<TooltipTrigger asChild>{badge}</TooltipTrigger>
				<TooltipContent side="top" className="max-w-xs">
					<p className="font-mono text-xs break-all">{model}</p>
					<p className="text-[11px] text-muted-foreground mt-1">
						{resolved
							? "Internal model / deployment ID"
							: "Unresolved model / deployment ID"}
					</p>
				</TooltipContent>
			</Tooltip>
		</TooltipProvider>
	);
}

// --- Step detail ---

function intensityDotColor(intensity: TokenIntensity): string {
	switch (intensity) {
		case "low":
			return "bg-emerald-500 ring-emerald-500/30";
		case "moderate":
			return "bg-amber-500 ring-amber-500/30";
		case "high":
			return "bg-orange-500 ring-orange-500/30";
		case "very-high":
			return "bg-red-500 ring-red-500/30";
	}
}

function intensityLineColor(intensity: TokenIntensity): string {
	switch (intensity) {
		case "low":
			return "bg-emerald-500/30";
		case "moderate":
			return "bg-amber-500/30";
		case "high":
			return "bg-orange-500/30";
		case "very-high":
			return "bg-red-500/30";
	}
}

function StepDetail({
	stat,
	maxTokens,
	index,
	isLast,
}: {
	stat: IChatUsageStat;
	maxTokens: number;
	index: number;
	isLast: boolean;
}) {
	const calls = stat.stats.calls ?? [];
	const intensity = getTokenIntensity(stat.stats.usage.total_tokens);
	const pct =
		maxTokens > 0
			? Math.round((stat.stats.usage.total_tokens / maxTokens) * 100)
			: 0;

	return (
		<div className="flex gap-4">
			{/* Stepper rail */}
			<div className="flex flex-col items-center pt-1">
				<div
					className={cn(
						"w-3 h-3 rounded-full ring-4 shrink-0 z-10",
						intensityDotColor(intensity),
					)}
				/>
				{!isLast && (
					<div
						className={cn("w-0.5 flex-1 mt-1", intensityLineColor(intensity))}
					/>
				)}
			</div>

			{/* Step content */}
			<div className={cn("flex-1 min-w-0 space-y-2.5 pb-6", isLast && "pb-0")}>
				<div className="flex items-center justify-between gap-2">
					<div className="flex items-center gap-2 min-w-0">
						<span className="text-[11px] font-mono text-muted-foreground/60">
							#{index + 1}
						</span>
						<span className="text-sm font-medium truncate">
							{stat.step_name}
						</span>
					</div>
					{stat.stats.duration_ms != null && (
						<span className="text-[11px] text-muted-foreground flex items-center gap-1 shrink-0">
							<ClockIcon className="w-3 h-3" />
							{formatDuration(stat.stats.duration_ms)}
						</span>
					)}
				</div>

				<div className="rounded-lg border bg-muted/20 p-3 space-y-2.5">
					<div className="flex items-center gap-2 flex-wrap">
						<span
							className={cn(
								"text-xs font-medium tabular-nums",
								intensityColor(intensity),
							)}
						>
							{formatTokenCount(stat.stats.usage.total_tokens)} tokens
						</span>
						<span className="text-[11px] text-muted-foreground">
							({formatTokenCount(stat.stats.usage.prompt_tokens)} in /{" "}
							{formatTokenCount(stat.stats.usage.completion_tokens)} out)
						</span>
						{stat.stats.usage.cost != null && (
							<Badge
								variant="outline"
								className="text-[11px] font-mono h-5 px-1.5 py-0"
							>
								<CoinsIcon className="w-2.5 h-2.5 mr-0.5 opacity-70" />
								{formatCost(stat.stats.usage.cost)}
							</Badge>
						)}
					</div>

					{/* Mini usage bar relative to max step */}
					<div className="h-1.5 rounded-full overflow-hidden bg-muted/60">
						<div
							className={cn(
								"h-full rounded-full transition-all",
								intensity === "low" && "bg-emerald-500/70",
								intensity === "moderate" && "bg-amber-500/70",
								intensity === "high" && "bg-orange-500/70",
								intensity === "very-high" && "bg-red-500/70",
							)}
							style={{ width: `${Math.max(pct, 2)}%` }}
						/>
					</div>

					{calls.length > 0 && (
						<div className="space-y-1.5 pt-2 border-t border-border/40">
							{calls.map((call, idx) => (
								<ModelCallRow key={`${call.model}-${idx}`} call={call} />
							))}
						</div>
					)}
					{stat.stats.iterations != null && stat.stats.iterations > 1 && (
						<div className="flex items-center gap-1 text-[11px] text-muted-foreground pt-0.5">
							<RepeatIcon className="w-3 h-3" />
							{stat.stats.iterations} iterations
						</div>
					)}
				</div>
			</div>
		</div>
	);
}

function ModelCallRow({ call }: { call: IModelCallEntry }) {
	const intensity = getTokenIntensity(call.usage.total_tokens);

	return (
		<div className="flex items-center justify-between text-xs py-0.5 gap-2">
			<ModelBadge model={call.model || "unknown"} />
			<div className="flex items-center gap-2 text-muted-foreground shrink-0">
				<span
					className={cn("tabular-nums font-medium", intensityColor(intensity))}
				>
					{formatTokenCount(call.usage.total_tokens)} tok
				</span>
				{call.usage.cost != null && (
					<span className="tabular-nums">{formatCost(call.usage.cost)}</span>
				)}
				{call.duration_ms != null && (
					<span className="tabular-nums">
						{formatDuration(call.duration_ms)}
					</span>
				)}
			</div>
		</div>
	);
}

// --- Model breakdown ---

function ModelBreakdownSection({
	byModel,
	totalTokens,
}: {
	byModel: Map<string, AggregatedModelEntry>;
	totalTokens: number;
}) {
	const entries = Array.from(byModel.entries()).sort(
		(a, b) => b[1].tokens - a[1].tokens,
	);

	if (entries.length === 0) return null;

	return (
		<div className="space-y-3">
			<div className="flex items-center gap-2">
				<BrainIcon className="w-3.5 h-3.5 text-muted-foreground" />
				<span className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
					Models Used
				</span>
			</div>
			<div className="space-y-2.5">
				{entries.map(([model, data]) => {
					const pct =
						totalTokens > 0 ? Math.round((data.tokens / totalTokens) * 100) : 0;
					const intensity = getTokenIntensity(data.tokens);
					return (
						<div key={model} className="space-y-1.5">
							<div className="flex items-center justify-between gap-2">
								<div className="flex items-center gap-2 min-w-0">
									<ModelBadge model={model} />
									<span className="text-[11px] text-muted-foreground">
										{data.calls} call{data.calls !== 1 ? "s" : ""}
									</span>
								</div>
								<span
									className={cn(
										"text-xs font-medium tabular-nums shrink-0",
										intensityColor(intensity),
									)}
								>
									{formatTokenCount(data.tokens)} tok
								</span>
							</div>
							<div className="flex items-center gap-2">
								<div className="flex-1 h-1.5 rounded-full overflow-hidden bg-muted/50">
									<div
										className={cn(
											"h-full rounded-full transition-all",
											modelBarColor(model),
										)}
										style={{ width: `${Math.max(pct, 2)}%` }}
									/>
								</div>
								<span className="text-[11px] text-muted-foreground tabular-nums w-8 text-right">
									{pct}%
								</span>
							</div>
							<div className="flex items-center gap-3 text-[11px] text-muted-foreground">
								<span className="flex items-center gap-0.5">
									<ArrowUpIcon className="w-2.5 h-2.5 text-blue-400" />
									{formatTokenCount(data.promptTokens)} in
								</span>
								<span className="flex items-center gap-0.5">
									<ArrowDownIcon className="w-2.5 h-2.5 text-emerald-400" />
									{formatTokenCount(data.completionTokens)} out
								</span>
								{data.cost != null && (
									<span className="flex items-center gap-0.5">
										<CoinsIcon className="w-2.5 h-2.5" />
										{formatCost(data.cost)}
									</span>
								)}
							</div>
						</div>
					);
				})}
			</div>
		</div>
	);
}

// --- Main component ---

export function UsageStats({
	stats,
	className,
}: { stats: IChatUsageStat[]; className?: string }) {
	const [sheetOpen, setSheetOpen] = useState(false);

	const aggregated = useMemo(() => aggregateStats(stats), [stats]);
	const modelNames = useModelNames(
		useMemo(() => Array.from(aggregated.byModel.keys()), [aggregated.byModel]),
	);

	const handleClick = useCallback(() => {
		setSheetOpen(true);
	}, []);

	const totalIntensity = getTokenIntensity(aggregated.totalTokens);
	const maxStepTokens = Math.max(
		...stats.map((s) => s.stats.usage.total_tokens),
		1,
	);

	return (
		<ModelNamesContext.Provider value={modelNames}>
			<TooltipProvider delayDuration={300}>
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							onClick={handleClick}
							className={cn(
								"inline-flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors py-1 px-2.5 rounded-lg hover:bg-muted/50 border border-transparent hover:border-border/50",
								className,
							)}
						>
							<ZapIcon
								className={cn("w-3 h-3", intensityColor(totalIntensity))}
							/>
							<span className="tabular-nums font-medium">
								{formatTokenCount(aggregated.totalTokens)}
							</span>
							<span className="opacity-60">
								({formatTokenCount(aggregated.promptTokens)} in /{" "}
								{formatTokenCount(aggregated.completionTokens)} out)
							</span>
							{aggregated.totalDuration != null && (
								<>
									<span className="opacity-30">·</span>
									<span className="tabular-nums">
										{formatDuration(aggregated.totalDuration)}
									</span>
								</>
							)}
							{aggregated.totalCost != null && (
								<>
									<span className="opacity-30">·</span>
									<span className="tabular-nums">
										{formatCost(aggregated.totalCost)}
									</span>
								</>
							)}
							<ChevronRightIcon className="w-3 h-3 opacity-40" />
						</button>
					</TooltipTrigger>
					<TooltipContent
						side="top"
						className="max-w-sm border border-border/60 bg-popover/95 p-3 text-popover-foreground shadow-xl [&>svg]:bg-popover/95 [&>svg]:fill-popover/95"
					>
						<div className="space-y-2 text-xs">
							<div className="flex items-center gap-2">
								<ZapIcon
									className={cn("w-3.5 h-3.5", intensityColor(totalIntensity))}
								/>
								<span className="font-medium">
									{formatTokenCountFull(aggregated.totalTokens)} total tokens
								</span>
								<Badge
									variant="outline"
									className={cn(
										"text-[10px] h-4 px-1 py-0 border",
										intensityBg(totalIntensity),
										intensityColor(totalIntensity),
									)}
								>
									{intensityLabel(totalIntensity)}
								</Badge>
							</div>
							<TokenRatioBar
								prompt={aggregated.promptTokens}
								completion={aggregated.completionTokens}
							/>
							{aggregated.byModel.size > 0 && (
								<div className="pt-1 border-t border-border/50 space-y-0.5">
									{Array.from(aggregated.byModel.entries()).map(
										([model, data]) => (
											<div
												key={model}
												className="flex items-center justify-between gap-3"
											>
												<span className="truncate">
													{modelLabel(model, modelNames).label}
												</span>
												<span className="text-muted-foreground tabular-nums shrink-0">
													{data.calls}x · {formatTokenCount(data.tokens)} tok
												</span>
											</div>
										),
									)}
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
				{/* This sheet is opened from inside the FlowPilot panels, which sit very high in the
				    stacking order (embedded board panel z-100, global overlay z-9999). Raise the sheet
				    (and its backdrop) above them so the stats aren't hidden behind the panel. */}
				<SheetContent
					side="right"
					className="z-10000 w-full sm:w-lg lg:w-xl sm:max-w-xl"
					overlayClassName="z-10000"
				>
					<SheetHeader className="px-2 shrink-0">
						<SheetTitle className="flex items-center gap-2">
							<ZapIcon className="w-4 h-4" />
							Model Usage
						</SheetTitle>
						<SheetDescription>
							Detailed breakdown of model invocations for this message
						</SheetDescription>
					</SheetHeader>

					<div className="flex-1 overflow-y-auto space-y-8 px-6 pb-8">
						{/* Summary cards */}
						<div className="grid grid-cols-2 gap-4">
							<div
								className={cn(
									"rounded-xl border p-4 col-span-2",
									intensityBg(totalIntensity),
								)}
							>
								<div className="flex items-center justify-between mb-2">
									<div className="text-xs text-muted-foreground">
										Total Tokens
									</div>
									<Badge
										variant="outline"
										className={cn(
											"text-[10px] h-4 px-1.5 py-0 border",
											intensityBg(totalIntensity),
											intensityColor(totalIntensity),
										)}
									>
										{intensityLabel(totalIntensity)}
									</Badge>
								</div>
								<div
									className={cn(
										"text-2xl font-bold tabular-nums",
										intensityColor(totalIntensity),
									)}
								>
									{formatTokenCount(aggregated.totalTokens)}
								</div>
								<div className="text-[11px] text-muted-foreground mt-0.5 tabular-nums">
									{formatTokenCountFull(aggregated.totalTokens)} tokens
								</div>
								<div className="mt-3">
									<TokenRatioBar
										prompt={aggregated.promptTokens}
										completion={aggregated.completionTokens}
									/>
								</div>
							</div>
							{aggregated.totalDuration != null && (
								<div className="rounded-xl border p-4">
									<div className="flex items-center gap-1.5 text-xs text-muted-foreground mb-1.5">
										<ClockIcon className="w-3.5 h-3.5" />
										Duration
									</div>
									<div className="text-lg font-semibold tabular-nums">
										{formatDuration(aggregated.totalDuration)}
									</div>
								</div>
							)}
							{aggregated.totalCost != null && (
								<div className="rounded-xl border p-4">
									<div className="flex items-center gap-1.5 text-xs text-muted-foreground mb-1.5">
										<CoinsIcon className="w-3.5 h-3.5" />
										Total Cost
									</div>
									<div className="text-lg font-semibold tabular-nums">
										{formatCost(aggregated.totalCost)}
									</div>
								</div>
							)}
							<div className="rounded-xl border p-4">
								<div className="flex items-center gap-1.5 text-xs text-muted-foreground mb-1.5">
									<LayersIcon className="w-3.5 h-3.5" />
									Steps
								</div>
								<div className="text-lg font-semibold">{stats.length}</div>
							</div>
						</div>

						<Separator />

						{/* Model breakdown */}
						<ModelBreakdownSection
							byModel={aggregated.byModel}
							totalTokens={aggregated.totalTokens}
						/>

						<Separator />

						{/* Timeline stepper */}
						<div className="space-y-4">
							<div className="flex items-center gap-2">
								<ClockIcon className="w-3.5 h-3.5 text-muted-foreground" />
								<span className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
									Timeline
								</span>
								<span className="text-[11px] text-muted-foreground/60">
									{stats.length} step{stats.length !== 1 ? "s" : ""}
								</span>
							</div>
							<div className="pl-1">
								{stats.map((stat, idx) => (
									<StepDetail
										key={`${stat.step_name}-${idx}`}
										stat={stat}
										maxTokens={maxStepTokens}
										index={idx}
										isLast={idx === stats.length - 1}
									/>
								))}
							</div>
						</div>

						{/* Bottom spacer for scroll breathing room */}
						<div className="h-4 shrink-0" />
					</div>
				</SheetContent>
			</Sheet>
		</ModelNamesContext.Provider>
	);
}
