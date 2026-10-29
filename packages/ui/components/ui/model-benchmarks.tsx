"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	BookOpenIcon,
	BrainIcon,
	CodeIcon,
	DollarSignIcon,
	ExternalLinkIcon,
	TimerIcon,
	WrenchIcon,
	ZapIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { Badge } from "./badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

interface IntelligenceEstimate {
	score: number;
	low: number;
	high: number;
	confidence: "low" | "medium";
	reasoning: string;
	sources: string[];
	updated_at: string;
}

interface ReportedBenchmark {
	name: string;
	score: number;
	unit: string;
	source: string;
	scope: string;
	notes: string;
	direction: "higher" | "lower";
}

interface AAEvaluations {
	artificial_analysis_intelligence_index?: number | null;
	intelligence_estimate?: IntelligenceEstimate | null;
	reported_benchmarks?: ReportedBenchmark[] | null;
	artificial_analysis_coding_index?: number | null;
	artificial_analysis_math_index?: number | null;
	mmlu_pro?: number | null;
	gpqa?: number | null;
	hle?: number | null;
	livecodebench?: number | null;
	scicode?: number | null;
	math_500?: number | null;
	aime?: number | null;
	aime_25?: number | null;
	ifbench?: number | null;
	lcr?: number | null;
	terminalbench_hard?: number | null;
	terminalbench_v2_1?: number | null;
	terminalbench_v4_0?: number | null;
	tau2?: number | null;
	tau_banking?: number | null;
	automationbench_aa?: number | null;
	artificial_analysis_omniscience_accuracy?: number | null;
	artificial_analysis_omniscience_hallucination_rate?: number | null;
	writing_benchmark_estimated_win_chance?: number | null;
	writing_benchmark_creativity_estimate?: number | null;
	writing_benchmark_match_type?: "direct" | "alias" | "similarity" | null;
	writing_benchmark_source_url?: string | null;
	writing_benchmark_source_model_name?: string | null;
	writing_benchmark_version?: string | null;
}

interface AAPricing {
	price_1m_blended_3_to_1?: number | null;
	price_1m_input_tokens?: number | null;
	price_1m_output_tokens?: number | null;
}

export interface IModelEvaluation {
	slug: string;
	name: string;
	release_date?: string | null;
	creator_name: string;
	creator_slug: string;
	evaluations?: AAEvaluations | null;
	pricing?: AAPricing | null;
	median_output_tokens_per_second?: number | null;
	median_time_to_first_token_seconds?: number | null;
	median_time_to_first_answer_token?: number | null;
}

interface IndexCardProps {
	label: string;
	value: number;
	icon: ReactNode;
	color: string;
	bgColor: string;
}

function IndexCard({
	label,
	value,
	icon,
	color,
	bgColor,
}: Readonly<IndexCardProps>) {
	return (
		<div
			className={`flex flex-col items-center justify-center rounded-lg border p-3 min-w-0 ${bgColor}`}
		>
			<div className={`mb-1 ${color}`}>{icon}</div>
			<span className={`text-xl font-bold tabular-nums ${color}`}>
				{value.toFixed(1)}
			</span>
			<span className="text-[10px] text-muted-foreground mt-0.5 text-center leading-tight">
				{label}
			</span>
		</div>
	);
}

function barColor(ratio: number): string {
	if (ratio >= 0.7) return "bg-emerald-500";
	if (ratio >= 0.4) return "bg-amber-500";
	return "bg-red-500";
}

function barTextColor(ratio: number): string {
	if (ratio >= 0.7) return "text-emerald-600 dark:text-emerald-400";
	if (ratio >= 0.4) return "text-amber-600 dark:text-amber-400";
	return "text-red-600 dark:text-red-400";
}

interface BenchmarkBarProps {
	label: string;
	description: string;
	value: number;
}

function BenchmarkBar({
	label,
	description,
	value,
}: Readonly<BenchmarkBarProps>) {
	const pct = Math.min(value * 100, 100);
	const ratio = value;

	return (
		<div className="space-y-1">
			<div className="flex items-center justify-between text-xs">
				<Tooltip>
					<TooltipTrigger asChild>
						<span className="font-medium cursor-help">{label}</span>
					</TooltipTrigger>
					<TooltipContent side="top" className="max-w-52">
						<p className="text-xs">{description}</p>
					</TooltipContent>
				</Tooltip>
				<span className={`font-semibold tabular-nums ${barTextColor(ratio)}`}>
					{pct.toFixed(1)}%
				</span>
			</div>
			<div className="h-2 w-full rounded-full bg-muted overflow-hidden">
				<div
					className={`h-full rounded-full transition-all ${barColor(ratio)}`}
					style={{ width: `${pct}%` }}
				/>
			</div>
		</div>
	);
}

interface BenchmarkDef {
	key: keyof AAEvaluations;
	label: string;
	description: string;
	category: "reasoning" | "coding" | "knowledge" | "tools";
	invert?: boolean;
}

const BENCHMARKS: BenchmarkDef[] = [
	{
		key: "gpqa",
		label: "GPQA Diamond",
		description: "Graduate-level scientific reasoning",
		category: "reasoning",
	},
	{
		key: "hle",
		label: "HLE",
		description: "Humanity's Last Exam",
		category: "reasoning",
	},
	{
		key: "ifbench",
		label: "IFBench",
		description: "Instruction-following benchmark",
		category: "reasoning",
	},
	{
		key: "math_500",
		label: "MATH 500",
		description: "Competition-level mathematics",
		category: "reasoning",
	},
	{
		key: "aime",
		label: "AIME 2024",
		description: "American Invitational Mathematics Exam",
		category: "reasoning",
	},
	{
		key: "aime_25",
		label: "AIME 2025",
		description: "American Invitational Mathematics Exam 2025",
		category: "reasoning",
	},
	{
		key: "scicode",
		label: "SciCode",
		description: "Python programming for scientific computing",
		category: "coding",
	},
	{
		key: "terminalbench_v4_0",
		label: "Terminal-Bench 4.0",
		description: "Agentic coding and terminal use",
		category: "coding",
	},
	{
		key: "terminalbench_v2_1",
		label: "Terminal-Bench 2.1",
		description: "Agentic coding and terminal use",
		category: "coding",
	},
	{
		key: "terminalbench_hard",
		label: "Terminal-Bench Hard",
		description: "Agentic coding & terminal use",
		category: "coding",
	},
	{
		key: "livecodebench",
		label: "LiveCodeBench",
		description: "Live competitive programming benchmark",
		category: "coding",
	},
	{
		key: "tau2",
		label: "TAU-2",
		description: "Tool use & agentic behavior",
		category: "tools",
	},
	{
		key: "tau_banking",
		label: "TAU-Banking",
		description: "Tool use in banking customer-service tasks",
		category: "tools",
	},
	{
		key: "automationbench_aa",
		label: "AutomationBench-AA",
		description: "Multi-step workflows using business software APIs",
		category: "tools",
	},
	{
		key: "mmlu_pro",
		label: "MMLU Pro",
		description: "Massive Multitask Language Understanding (professional)",
		category: "knowledge",
	},
	{
		key: "lcr",
		label: "AA-LCR",
		description: "Long-context reasoning",
		category: "knowledge",
	},
	{
		key: "artificial_analysis_omniscience_accuracy",
		label: "AA-Omniscience Accuracy",
		description: "Accuracy on factual knowledge questions",
		category: "knowledge",
	},
	{
		key: "artificial_analysis_omniscience_hallucination_rate",
		label: "AA-Omniscience Non-Hallucination Rate",
		description:
			"One minus the hallucination rate on factual knowledge questions",
		category: "knowledge",
		invert: true,
	},
];

const CATEGORY_META: Record<
	string,
	{ icon: ReactNode; color: string; bgColor: string }
> = {
	Reasoning: {
		icon: <BrainIcon className="h-3.5 w-3.5" />,
		color: "text-violet-600 dark:text-violet-400",
		bgColor: "bg-violet-500/5",
	},
	Coding: {
		icon: <CodeIcon className="h-3.5 w-3.5" />,
		color: "text-blue-600 dark:text-blue-400",
		bgColor: "bg-blue-500/5",
	},
	"Tool use": {
		icon: <WrenchIcon className="h-3.5 w-3.5" />,
		color: "text-cyan-600 dark:text-cyan-400",
		bgColor: "bg-cyan-500/5",
	},
	Knowledge: {
		icon: <BookOpenIcon className="h-3.5 w-3.5" />,
		color: "text-amber-600 dark:text-amber-400",
		bgColor: "bg-amber-500/5",
	},
};

function PerfRow({
	icon,
	label,
	value,
	valueColor,
}: Readonly<{
	icon: ReactNode;
	label: string;
	value: string;
	valueColor?: string;
}>) {
	return (
		<div className="flex items-center justify-between text-xs py-2 border-b last:border-b-0 border-border/50">
			<span className="flex items-center gap-1.5 text-muted-foreground">
				{icon}
				{label}
			</span>
			<span className={`font-semibold tabular-nums ${valueColor ?? ""}`}>
				{value}
			</span>
		</div>
	);
}

function speedColor(tokPerSec: number): string {
	if (tokPerSec >= 100) return "text-emerald-600 dark:text-emerald-400";
	if (tokPerSec >= 40) return "text-amber-600 dark:text-amber-400";
	return "text-red-600 dark:text-red-400";
}

function priceColor(price: number): string {
	if (price <= 0) return "text-emerald-600 dark:text-emerald-400";
	if (price <= 1) return "text-emerald-600 dark:text-emerald-400";
	if (price <= 5) return "text-amber-600 dark:text-amber-400";
	return "text-red-600 dark:text-red-400";
}

function intelligenceBadgeStyle(idx: number): {
	bg: string;
	text: string;
	border: string;
} {
	if (idx >= 30)
		return {
			bg: "bg-emerald-500/10",
			text: "text-emerald-700 dark:text-emerald-400",
			border: "border-emerald-500/30",
		};
	if (idx >= 18)
		return {
			bg: "bg-blue-500/10",
			text: "text-blue-700 dark:text-blue-400",
			border: "border-blue-500/30",
		};
	if (idx >= 10)
		return {
			bg: "bg-amber-500/10",
			text: "text-amber-700 dark:text-amber-400",
			border: "border-amber-500/30",
		};
	return {
		bg: "bg-zinc-500/10",
		text: "text-zinc-600 dark:text-zinc-400",
		border: "border-zinc-500/30",
	};
}

function formatPrice(price: number | null | undefined): string {
	if (price == null) return "N/A";
	if (price === 0) return "Free";
	if (price < 0.01) return `$${price.toFixed(4)}`;
	return `$${price.toFixed(2)}`;
}

function formatSpeed(val: number | null | undefined): string {
	if (val == null) return "N/A";
	return i18next.t("valToks", "{{val}} tok/s", { val: val.toFixed(1) });
}

function formatLatency(val: number | null | undefined): string {
	if (val == null) return "N/A";
	if (val < 1) return `${(val * 1000).toFixed(0)}ms`;
	return `${val.toFixed(2)}s`;
}

function isScore(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function intelligenceEstimate(value: unknown): IntelligenceEstimate | null {
	if (!value || typeof value !== "object") return null;
	const estimate = value as Partial<IntelligenceEstimate>;
	if (
		!isScore(estimate.score) ||
		!isScore(estimate.low) ||
		!isScore(estimate.high) ||
		estimate.high > 100 ||
		estimate.low > estimate.score ||
		estimate.score > estimate.high ||
		!["low", "medium"].includes(estimate.confidence ?? "") ||
		typeof estimate.reasoning !== "string" ||
		!estimate.reasoning.trim() ||
		!Array.isArray(estimate.sources) ||
		!estimate.sources.every((source) => typeof source === "string") ||
		typeof estimate.updated_at !== "string" ||
		!Number.isFinite(Date.parse(estimate.updated_at))
	) {
		return null;
	}
	return estimate as IntelligenceEstimate;
}

function benchmarkSourceUrl(value: string | null | undefined): string | null {
	if (!value) return null;
	try {
		const url = new URL(value);
		return ["https:", "http:"].includes(url.protocol) ? url.href : null;
	} catch {
		return null;
	}
}

function estimateSourceUrls(sources: string[]): string[] {
	return [
		...new Set(
			sources.flatMap((source) => {
				const url = httpsSourceUrl(source);
				return url ? [url] : [];
			}),
		),
	];
}

function httpsSourceUrl(value: unknown): string | null {
	if (typeof value !== "string") return null;
	const source = benchmarkSourceUrl(value);
	if (!source) return null;
	const url = new URL(source);
	return url.protocol === "https:" && !url.username && !url.password
		? url.href
		: null;
}

function reportedBenchmarks(value: unknown): ReportedBenchmark[] {
	if (!Array.isArray(value)) return [];
	return value.flatMap((entry) => {
		if (!entry || typeof entry !== "object" || Array.isArray(entry)) return [];
		const benchmark = entry as Partial<ReportedBenchmark>;
		const source = httpsSourceUrl(benchmark.source);
		if (
			typeof benchmark.score !== "number" ||
			!Number.isFinite(benchmark.score) ||
			!source ||
			![benchmark.name, benchmark.unit, benchmark.scope, benchmark.notes].every(
				(field) => typeof field === "string" && field.trim(),
			) ||
			!["higher", "lower"].includes(benchmark.direction ?? "")
		) {
			return [];
		}
		return [{ ...(benchmark as ReportedBenchmark), source }];
	});
}

function PublisherBenchmarks({
	benchmarks,
}: Readonly<{ benchmarks: ReportedBenchmark[] }>) {
	const { t } = useTranslation("common");
	return (
		<div className="rounded-lg border p-3 space-y-3">
			<h5 className="text-xs font-medium">
				{t("publisherBenchmarks", "Publisher benchmarks")}
			</h5>
			{benchmarks.map((benchmark) => (
				<div
					key={`${benchmark.source}:${benchmark.name}:${benchmark.unit}`}
					className="space-y-1.5 border-t pt-3 text-xs"
				>
					<div className="flex items-start justify-between gap-3">
						<span className="font-medium">{benchmark.name}</span>
						<span className="font-semibold tabular-nums text-right">
							{Number(benchmark.score.toFixed(4)).toString()} {benchmark.unit}
						</span>
					</div>
					<p className="text-[10px] text-muted-foreground">
						{benchmark.direction === "lower"
							? t("lowerIsBetter", "Lower is better")
							: t("higherIsBetter", "Higher is better")}
						{" · "}
						{t("benchmarkScope", "Scope:")} {benchmark.scope}
					</p>
					<p className="text-muted-foreground">{benchmark.notes}</p>
					<a
						href={benchmark.source}
						target="_blank"
						rel="noopener noreferrer"
						className="text-[10px] underline hover:text-foreground transition-colors"
					>
						{t("benchmarkSource", "Source:")}{" "}
						{new URL(benchmark.source).hostname}
					</a>
				</div>
			))}
		</div>
	);
}

function EstimateDetails({
	estimate,
	showSources = false,
}: Readonly<{ estimate: IntelligenceEstimate; showSources?: boolean }>) {
	const { t } = useTranslation("common");
	const sources = showSources ? estimateSourceUrls(estimate.sources) : [];
	return (
		<div className="space-y-1.5 text-xs">
			<p>
				{t("estimatedIntelligenceRange", "Range: {{low}} to {{high}}", {
					low: estimate.low.toFixed(1),
					high: estimate.high.toFixed(1),
				})}
				{" · "}
				{estimate.confidence === "medium"
					? t("mediumConfidence", "Medium confidence")
					: t("lowConfidence", "Low confidence")}
			</p>
			<p>{estimate.reasoning}</p>
			{showSources && (
				<p className="text-[10px] text-muted-foreground">
					{t("intelligenceEstimateDate", "Estimated {{date}}", {
						date: new Date(estimate.updated_at).toISOString().slice(0, 10),
					})}
				</p>
			)}
			{sources.length > 0 && (
				<div className="flex flex-wrap gap-x-2 gap-y-1 text-[10px]">
					<span className="text-muted-foreground">
						{t("estimateSources", "Estimate sources:")}
					</span>
					{sources.map((url) => (
						<a
							key={url}
							href={url}
							target="_blank"
							rel="noopener noreferrer"
							className="underline hover:text-foreground transition-colors"
						>
							{new URL(url).hostname}
						</a>
					))}
				</div>
			)}
		</div>
	);
}

export function ModelBenchmarks({
	evaluation,
}: Readonly<{ evaluation: IModelEvaluation }>) {
	const { t } = useTranslation("common");
	const evals = evaluation.evaluations as AAEvaluations | null | undefined;
	const pricing = evaluation.pricing as AAPricing | null | undefined;
	if (!evals) return null;

	const intelligenceIndex = evals.artificial_analysis_intelligence_index;
	const codingIndex = evals.artificial_analysis_coding_index;
	const mathIndex = evals.artificial_analysis_math_index;
	const estimate = isScore(intelligenceIndex)
		? null
		: intelligenceEstimate(evals.intelligence_estimate);

	const hasAAIndices = [intelligenceIndex, codingIndex, mathIndex].some(
		isScore,
	);
	const hasIndices = hasAAIndices || estimate !== null;

	const availableBenchmarks = BENCHMARKS.filter((b) => {
		const val = evals[b.key];
		return isScore(val) && val <= 1;
	});

	const reasoningBenchmarks = availableBenchmarks.filter(
		(b) => b.category === "reasoning",
	);
	const codingBenchmarks = availableBenchmarks.filter(
		(b) => b.category === "coding",
	);
	const knowledgeBenchmarks = availableBenchmarks.filter(
		(b) => b.category === "knowledge",
	);
	const toolBenchmarks = availableBenchmarks.filter(
		(b) => b.category === "tools",
	);
	const writingWinChance = evals.writing_benchmark_estimated_win_chance;
	const hasWritingBenchmark =
		evals.writing_benchmark_match_type === "direct" &&
		isScore(writingWinChance) &&
		writingWinChance <= 1;
	const writingSource = hasWritingBenchmark
		? benchmarkSourceUrl(evals.writing_benchmark_source_url)
		: null;
	const hasAABenchmarks = hasAAIndices || availableBenchmarks.length > 0;
	const publisherBenchmarks = reportedBenchmarks(evals.reported_benchmarks);

	if (
		!hasAABenchmarks &&
		!hasWritingBenchmark &&
		!estimate &&
		publisherBenchmarks.length === 0
	) {
		return null;
	}

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between">
				<h4 className="text-sm font-medium">{t("benchmarks", "Benchmarks")}</h4>
				{hasAABenchmarks &&
					!evaluation.slug.startsWith("flow-like-estimate-") && (
						<a
							href={`https://artificialanalysis.ai/models/${encodeURIComponent(evaluation.slug)}`}
							target="_blank"
							rel="noopener noreferrer"
							className="text-[10px] text-muted-foreground hover:text-foreground transition-colors flex items-center gap-1"
						>
							{t("artificialAnalysis", "Artificial Analysis")}
							<ExternalLinkIcon className="h-2.5 w-2.5" />
						</a>
					)}
			</div>

			{hasIndices && (
				<div className="grid grid-cols-3 gap-2">
					{isScore(intelligenceIndex) && (
						<IndexCard
							label="Intelligence"
							value={intelligenceIndex}
							icon={<BrainIcon className="h-4 w-4" />}
							color="text-emerald-600 dark:text-emerald-400"
							bgColor="bg-emerald-500/5 border-emerald-500/20"
						/>
					)}
					{estimate && (
						<IndexCard
							label={t("estimatedIntelligence", "Estimated intelligence")}
							value={estimate.score}
							icon={<BrainIcon className="h-4 w-4" />}
							color="text-amber-600 dark:text-amber-400"
							bgColor="bg-amber-500/5 border-amber-500/20 border-dashed"
						/>
					)}
					{isScore(codingIndex) && (
						<IndexCard
							label="Coding"
							value={codingIndex}
							icon={<CodeIcon className="h-4 w-4" />}
							color="text-blue-600 dark:text-blue-400"
							bgColor="bg-blue-500/5 border-blue-500/20"
						/>
					)}
					{isScore(mathIndex) && (
						<IndexCard
							label="Math"
							value={mathIndex}
							icon={<span className="text-sm font-bold">∑</span>}
							color="text-violet-600 dark:text-violet-400"
							bgColor="bg-violet-500/5 border-violet-500/20"
						/>
					)}
				</div>
			)}
			{estimate && (
				<div className="rounded-lg border border-dashed p-3 bg-amber-500/5">
					<EstimateDetails estimate={estimate} showSources />
				</div>
			)}

			{reasoningBenchmarks.length > 0 && (
				<BenchmarkCategory
					label="Reasoning"
					benchmarks={reasoningBenchmarks}
					evals={evals}
				/>
			)}
			{codingBenchmarks.length > 0 && (
				<BenchmarkCategory
					label="Coding"
					benchmarks={codingBenchmarks}
					evals={evals}
				/>
			)}
			{knowledgeBenchmarks.length > 0 && (
				<BenchmarkCategory
					label="Knowledge"
					benchmarks={knowledgeBenchmarks}
					evals={evals}
				/>
			)}
			{toolBenchmarks.length > 0 && (
				<BenchmarkCategory
					label="Tool use"
					benchmarks={toolBenchmarks}
					evals={evals}
				/>
			)}
			{hasWritingBenchmark && (
				<div className="rounded-lg border p-3 space-y-3 bg-rose-500/5">
					<BenchmarkBar
						label="Creative writing win chance"
						description="Estimated chance of winning a comparison against another model in the tested writing leaderboard. Results depend on the models included."
						value={writingWinChance}
					/>
					<p className="text-[10px] text-muted-foreground">
						{t(
							"writingLeaderboardComparison",
							"Relative to the models in the tested writing leaderboard.",
						)}
					</p>
					{evals.writing_benchmark_source_model_name && (
						<p className="text-[10px] text-muted-foreground">
							{t("evaluatedWritingModel", "Evaluated model:")}{" "}
							{evals.writing_benchmark_source_model_name}
						</p>
					)}
				</div>
			)}
			{publisherBenchmarks.length > 0 && (
				<PublisherBenchmarks benchmarks={publisherBenchmarks} />
			)}

			{(pricing || evaluation.median_output_tokens_per_second) && (
				<div className="rounded-lg border bg-card p-3 space-y-0">
					<h5 className="text-xs font-medium mb-1 flex items-center gap-1.5">
						{t("performancePricing", "Performance & Pricing")}
					</h5>
					{evaluation.median_output_tokens_per_second != null && (
						<PerfRow
							icon={<ZapIcon className="h-3 w-3" />}
							label={t("outputSpeed", "Output Speed")}
							value={formatSpeed(evaluation.median_output_tokens_per_second)}
							valueColor={speedColor(
								evaluation.median_output_tokens_per_second,
							)}
						/>
					)}
					{evaluation.median_time_to_first_token_seconds != null && (
						<PerfRow
							icon={<TimerIcon className="h-3 w-3" />}
							label={t("timeToFirstToken", "Time to First Token")}
							value={formatLatency(
								evaluation.median_time_to_first_token_seconds,
							)}
						/>
					)}
					{evaluation.median_time_to_first_answer_token != null && (
						<PerfRow
							icon={<TimerIcon className="h-3 w-3" />}
							label={t("timeToFirstAnswer", "Time to First Answer")}
							value={formatLatency(
								evaluation.median_time_to_first_answer_token,
							)}
						/>
					)}
					{pricing?.price_1m_input_tokens != null && (
						<PerfRow
							icon={<DollarSignIcon className="h-3 w-3" />}
							label={t("input1mTokens", "Input (1M tokens)")}
							value={formatPrice(pricing.price_1m_input_tokens)}
							valueColor={priceColor(pricing.price_1m_input_tokens)}
						/>
					)}
					{pricing?.price_1m_output_tokens != null && (
						<PerfRow
							icon={<DollarSignIcon className="h-3 w-3" />}
							label={t("output1mTokens", "Output (1M tokens)")}
							value={formatPrice(pricing.price_1m_output_tokens)}
							valueColor={priceColor(pricing.price_1m_output_tokens)}
						/>
					)}
					{pricing?.price_1m_blended_3_to_1 != null && (
						<PerfRow
							icon={<DollarSignIcon className="h-3 w-3" />}
							label={t("blended31", "Blended (3:1)")}
							value={formatPrice(pricing.price_1m_blended_3_to_1)}
							valueColor={priceColor(pricing.price_1m_blended_3_to_1)}
						/>
					)}
				</div>
			)}

			{(hasAABenchmarks || writingSource) && (
				<p className="text-[10px] text-muted-foreground text-center leading-relaxed">
					{t("benchmarkSources", "Benchmark sources:")}{" "}
					{hasAABenchmarks && (
						<a
							href="https://artificialanalysis.ai/"
							target="_blank"
							rel="noopener noreferrer"
							className="underline hover:text-foreground transition-colors"
						>
							{t("artificialAnalysis", "Artificial Analysis")}
						</a>
					)}
					{hasAABenchmarks && writingSource && " · "}
					{writingSource && (
						<a
							href={writingSource}
							target="_blank"
							rel="noopener noreferrer"
							className="underline hover:text-foreground transition-colors"
						>
							{t("creativeWritingBenchmark", "Creative writing benchmark")}
						</a>
					)}
				</p>
			)}
		</div>
	);
}

function BenchmarkCategory({
	label,
	benchmarks,
	evals,
}: Readonly<{
	label: string;
	benchmarks: BenchmarkDef[];
	evals: AAEvaluations;
}>) {
	const meta = CATEGORY_META[label] ?? CATEGORY_META.Knowledge;

	return (
		<div className={`rounded-lg border p-3 space-y-3 ${meta.bgColor}`}>
			<h5
				className={`text-xs font-medium flex items-center gap-1.5 ${meta.color}`}
			>
				{meta.icon}
				{label}
			</h5>
			{benchmarks.map((b) => {
				const val = evals[b.key] as number;
				return (
					<BenchmarkBar
						key={b.key}
						label={b.label}
						description={b.description}
						value={b.invert ? 1 - val : val}
					/>
				);
			})}
		</div>
	);
}

export function IntelligenceIndexBadge({
	evaluation,
}: Readonly<{ evaluation?: IModelEvaluation | null }>) {
	const { t } = useTranslation("common");
	if (!evaluation?.evaluations) return null;
	const evals = evaluation.evaluations as AAEvaluations;
	const measuredIndex = evals.artificial_analysis_intelligence_index;
	const estimate = isScore(measuredIndex)
		? null
		: intelligenceEstimate(evals.intelligence_estimate);
	const idx = isScore(measuredIndex) ? measuredIndex : estimate?.score;
	if (idx === undefined) return null;

	const style = intelligenceBadgeStyle(idx);
	const label = estimate
		? t("estimatedIntelligence", "Estimated intelligence")
		: t(
				"artificialAnalysisIntelligenceIndex",
				"Artificial Analysis Intelligence Index",
			);

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Badge
					variant="outline"
					aria-label={`${label}: ${idx.toFixed(1)}`}
					className={`text-[10px] px-1.5 py-0 h-5 tabular-nums gap-0.5 ${style.bg} ${style.text} ${style.border}`}
				>
					<BrainIcon className="h-2.5 w-2.5" />
					{estimate && t("estimatedScorePrefix", "Est.")}
					{estimate && " "}
					{idx.toFixed(1)}
				</Badge>
			</TooltipTrigger>
			<TooltipContent
				side="top"
				className={estimate ? "max-w-80 space-y-1.5" : undefined}
			>
				<p className="text-xs">{label}</p>
				{estimate && <EstimateDetails estimate={estimate} />}
			</TooltipContent>
		</Tooltip>
	);
}
