"use client";

import {
	BookOpenIcon,
	BrainIcon,
	CodeIcon,
	DollarSignIcon,
	ExternalLinkIcon,
	TimerIcon,
	ZapIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { Badge } from "./badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

interface AAEvaluations {
	artificial_analysis_intelligence_index?: number | null;
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
	tau2?: number | null;
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
	category: "reasoning" | "coding" | "knowledge";
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
		category: "coding",
	},
	{
		key: "mmlu_pro",
		label: "MMLU Pro",
		description: "Massive Multitask Language Understanding (professional)",
		category: "knowledge",
	},
	{
		key: "lcr",
		label: "LCR",
		description: "Long context recall",
		category: "knowledge",
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
	return `${val.toFixed(1)} tok/s`;
}

function formatLatency(val: number | null | undefined): string {
	if (val == null) return "N/A";
	if (val < 1) return `${(val * 1000).toFixed(0)}ms`;
	return `${val.toFixed(2)}s`;
}

export function ModelBenchmarks({
	evaluation,
}: Readonly<{ evaluation: IModelEvaluation }>) {
	const evals = evaluation.evaluations as AAEvaluations | null | undefined;
	const pricing = evaluation.pricing as AAPricing | null | undefined;
	if (!evals) return null;

	const intelligenceIndex = evals.artificial_analysis_intelligence_index;
	const codingIndex = evals.artificial_analysis_coding_index;
	const mathIndex = evals.artificial_analysis_math_index;

	const hasIndices = [intelligenceIndex, codingIndex, mathIndex].some(
		(v) => v != null && v > 0,
	);

	const availableBenchmarks = BENCHMARKS.filter((b) => {
		const val = evals[b.key];
		return val != null && (val as number) > 0;
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

	if (!hasIndices && availableBenchmarks.length === 0) return null;

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between">
				<h4 className="text-sm font-medium">Benchmarks</h4>
				<a
					href={`https://artificialanalysis.ai/leaderboards/models?model=${evaluation.slug}`}
					target="_blank"
					rel="noopener noreferrer"
					className="text-[10px] text-muted-foreground hover:text-foreground transition-colors flex items-center gap-1"
				>
					Artificial Analysis
					<ExternalLinkIcon className="h-2.5 w-2.5" />
				</a>
			</div>

			{hasIndices && (
				<div className="grid grid-cols-3 gap-2">
					{intelligenceIndex != null && intelligenceIndex > 0 && (
						<IndexCard
							label="Intelligence"
							value={intelligenceIndex}
							icon={<BrainIcon className="h-4 w-4" />}
							color="text-emerald-600 dark:text-emerald-400"
							bgColor="bg-emerald-500/5 border-emerald-500/20"
						/>
					)}
					{codingIndex != null && codingIndex > 0 && (
						<IndexCard
							label="Coding"
							value={codingIndex}
							icon={<CodeIcon className="h-4 w-4" />}
							color="text-blue-600 dark:text-blue-400"
							bgColor="bg-blue-500/5 border-blue-500/20"
						/>
					)}
					{mathIndex != null && mathIndex > 0 && (
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

			{(pricing || evaluation.median_output_tokens_per_second) && (
				<div className="rounded-lg border bg-card p-3 space-y-0">
					<h5 className="text-xs font-medium mb-1 flex items-center gap-1.5">
						Performance & Pricing
					</h5>
					{evaluation.median_output_tokens_per_second != null && (
						<PerfRow
							icon={<ZapIcon className="h-3 w-3" />}
							label="Output Speed"
							value={formatSpeed(evaluation.median_output_tokens_per_second)}
							valueColor={speedColor(
								evaluation.median_output_tokens_per_second,
							)}
						/>
					)}
					{evaluation.median_time_to_first_token_seconds != null && (
						<PerfRow
							icon={<TimerIcon className="h-3 w-3" />}
							label="Time to First Token"
							value={formatLatency(
								evaluation.median_time_to_first_token_seconds,
							)}
						/>
					)}
					{evaluation.median_time_to_first_answer_token != null && (
						<PerfRow
							icon={<TimerIcon className="h-3 w-3" />}
							label="Time to First Answer"
							value={formatLatency(
								evaluation.median_time_to_first_answer_token,
							)}
						/>
					)}
					{pricing?.price_1m_input_tokens != null && (
						<PerfRow
							icon={<DollarSignIcon className="h-3 w-3" />}
							label="Input (1M tokens)"
							value={formatPrice(pricing.price_1m_input_tokens)}
							valueColor={priceColor(pricing.price_1m_input_tokens)}
						/>
					)}
					{pricing?.price_1m_output_tokens != null && (
						<PerfRow
							icon={<DollarSignIcon className="h-3 w-3" />}
							label="Output (1M tokens)"
							value={formatPrice(pricing.price_1m_output_tokens)}
							valueColor={priceColor(pricing.price_1m_output_tokens)}
						/>
					)}
					{pricing?.price_1m_blended_3_to_1 != null && (
						<PerfRow
							icon={<DollarSignIcon className="h-3 w-3" />}
							label="Blended (3:1)"
							value={formatPrice(pricing.price_1m_blended_3_to_1)}
							valueColor={priceColor(pricing.price_1m_blended_3_to_1)}
						/>
					)}
				</div>
			)}

			<p className="text-[10px] text-muted-foreground text-center leading-relaxed">
				Benchmark data provided by{" "}
				<a
					href="https://artificialanalysis.ai/"
					target="_blank"
					rel="noopener noreferrer"
					className="underline hover:text-foreground transition-colors"
				>
					Artificial Analysis
				</a>
			</p>
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
						value={val}
					/>
				);
			})}
		</div>
	);
}

export function IntelligenceIndexBadge({
	evaluation,
}: Readonly<{ evaluation?: IModelEvaluation | null }>) {
	if (!evaluation?.evaluations) return null;
	const evals = evaluation.evaluations as AAEvaluations;
	const idx = evals.artificial_analysis_intelligence_index;
	if (idx == null || idx <= 0) return null;

	const style = intelligenceBadgeStyle(idx);

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Badge
					variant="outline"
					className={`text-[10px] px-1.5 py-0 h-5 tabular-nums gap-0.5 ${style.bg} ${style.text} ${style.border}`}
				>
					<BrainIcon className="h-2.5 w-2.5" />
					{idx.toFixed(1)}
				</Badge>
			</TooltipTrigger>
			<TooltipContent side="top">
				<p className="text-xs">Artificial Analysis Intelligence Index</p>
			</TooltipContent>
		</Tooltip>
	);
}
