"use client";

import { useTranslation } from "@flow-like/locales";
import { GaugeIcon } from "lucide-react";
import { useCallback, useId, useMemo } from "react";
import {
	AA_INTELLIGENCE_VERSION,
	type ModelRating,
	RATING_FLOORS,
	type RatingTier,
	bitRating,
} from "../../lib/bit/model-rating";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import { cn } from "../../lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

/** One colour per tier, shared by every rating surface so a colour means the same everywhere. */
export const RATING_TONE: Record<
	RatingTier,
	{ chip: string; text: string; fill: string; ring: string; panel: string }
> = {
	1: {
		chip: "border-emerald-500/35 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
		text: "text-emerald-700 dark:text-emerald-400",
		fill: "bg-emerald-500",
		ring: "border-emerald-500",
		panel: "border-emerald-500/30 bg-emerald-500/[0.06]",
	},
	2: {
		chip: "border-blue-500/35 bg-blue-500/10 text-blue-700 dark:text-blue-400",
		text: "text-blue-700 dark:text-blue-400",
		fill: "bg-blue-500",
		ring: "border-blue-500",
		panel: "border-blue-500/30 bg-blue-500/[0.06]",
	},
	3: {
		chip: "border-amber-500/35 bg-amber-500/10 text-amber-700 dark:text-amber-400",
		text: "text-amber-700 dark:text-amber-400",
		fill: "bg-amber-500",
		ring: "border-amber-500",
		panel: "border-amber-500/30 bg-amber-500/[0.06]",
	},
	4: {
		chip: "border-zinc-500/35 bg-zinc-500/10 text-zinc-600 dark:text-zinc-400",
		text: "text-zinc-600 dark:text-zinc-400",
		fill: "bg-zinc-400",
		ring: "border-zinc-400",
		panel: "border-zinc-500/30 bg-zinc-500/[0.06]",
	},
};

/** "chat models", "voice models": the group a rating or price compares against. */
export function useModelTypeNoun() {
	const { t } = useTranslation("common");
	return useCallback(
		(type: IBitTypes): string => {
			switch (type) {
				case IBitTypes.Llm:
				case IBitTypes.Vlm:
					return t("modelTypeChat", "chat models");
				case IBitTypes.ImageGeneration:
					return t("modelTypeImage", "image models");
				case IBitTypes.VideoGeneration:
					return t("modelTypeVideo", "video models");
				case IBitTypes.Tts:
					return t("modelTypeVoice", "voice models");
				case IBitTypes.Stt:
					return t("modelTypeTranscription", "transcription models");
				case IBitTypes.Embedding:
				case IBitTypes.ImageEmbedding:
					return t("modelTypeEmbedding", "embedding models");
				case IBitTypes.SystemOne:
					return t("modelTypeDecision", "decision models");
				case IBitTypes.Forecasting:
					return t("modelTypeForecasting", "forecasting models");
				default:
					return t("modelTypeOther", "models of this type");
			}
		},
		[t],
	);
}

export function useRatingText() {
	const { t } = useTranslation("common");
	const noun = useModelTypeNoun();
	return useCallback(
		(rating: ModelRating, type: IBitTypes) => {
			const tierName = [
				"",
				t("ratingTierTop", "Top tier"),
				t("ratingTierStrong", "Strong"),
				t("ratingTierFair", "Fair"),
				t("ratingTierBasic", "Basic"),
			][rating.tier];
			const range =
				rating.low !== undefined && rating.high !== undefined
					? t("ratingLikelyRange", "Likely between {{low}} and {{high}}", {
							low: Math.round(rating.low),
							high: Math.round(rating.high),
						})
					: "";
			const confidence =
				rating.confidence === "medium"
					? t("mediumConfidence", "Medium confidence")
					: rating.confidence === "low"
						? t("lowConfidence", "Low confidence")
						: "";
			return {
				label:
					rating.kind === "intelligence"
						? rating.estimated
							? t(
									"estimatedRatingIntelligenceVersion",
									"Est. AA v{{version}}",
									{ version: AA_INTELLIGENCE_VERSION },
								)
							: t("ratingIntelligenceVersion", "AA v{{version}}", {
									version: AA_INTELLIGENCE_VERSION,
								})
						: t("ratingQuality", "Quality"),
				heading:
					rating.kind === "quality"
						? t("ratingQualityHeading", "Quality rating")
						: rating.estimated
							? t(
									"estimatedVersionedArtificialAnalysisIntelligenceIndex",
									"Estimated AA v{{version}} Intelligence Index",
									{ version: AA_INTELLIGENCE_VERSION },
								)
							: t(
									"ratingIntelligenceIndexVersion",
									"AA v{{version}} Intelligence Index",
									{ version: AA_INTELLIGENCE_VERSION },
								),
				unit:
					rating.kind === "quality"
						? "/ 100"
						: t("ratingIndexPoints", "points"),
				short: `${rating.estimated ? "≈" : ""}${Math.round(rating.score)}`,
				full:
					rating.kind === "intelligence" && !rating.estimated
						? rating.score.toFixed(1)
						: Math.round(rating.score).toString(),
				tierAmong: t("ratingTierAmong", "{{tier}} among {{type}}", {
					tier: tierName,
					type: noun(type),
				}),
				evidence: rating.estimated
					? [range, confidence].filter(Boolean).join(" · ")
					: t(
							"ratingMeasuredByVersion",
							"Measured by Artificial Analysis (v{{version}})",
							{ version: AA_INTELLIGENCE_VERSION },
						),
			};
		},
		[t, noun],
	);
}

/** The colour-coded score on cards and rows; models without data say so instead of leaving a gap. */
export function ModelRatingBadge({
	rating,
	type,
	className,
}: Readonly<{
	rating: ModelRating | null;
	type: IBitTypes;
	className?: string;
}>) {
	const { t } = useTranslation("common");
	const text = useRatingText();
	if (!rating) {
		return (
			<span
				className={cn(
					"inline-flex h-5.5 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md border border-dashed px-2 text-[11.5px] text-muted-foreground",
					className,
				)}
			>
				<GaugeIcon className="h-3 w-3" aria-hidden="true" />
				{type === IBitTypes.Llm || type === IBitTypes.Vlm
					? t("aaModelNotEvaluated", "Not evaluated in AA v{{version}}", {
							version: AA_INTELLIGENCE_VERSION,
						})
					: t("ratingNotRated", "Not rated")}
			</span>
		);
	}
	const copy = text(rating, type);
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span
					className={cn(
						"inline-flex h-5.5 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md border px-2 text-[11.5px] font-medium",
						RATING_TONE[rating.tier].chip,
						className,
					)}
				>
					<GaugeIcon className="h-3 w-3" aria-hidden="true" />
					{copy.label}
					<span className="font-mono font-semibold tabular-nums">
						{copy.short}
					</span>
				</span>
			</TooltipTrigger>
			<TooltipContent side="top" className="max-w-72 space-y-1 text-xs">
				<p className="font-medium">
					{copy.label} {copy.full} {copy.unit} · {copy.tierAmong}
				</p>
				<p className="text-muted-foreground">{copy.evidence}</p>
			</TooltipContent>
		</Tooltip>
	);
}

/** Mixed asset lists show a badge only for bits with a valid model rating. */
export function BitRatingBadge({
	bit,
	className,
}: Readonly<{ bit: IBit; className?: string }>) {
	const rating = bitRating(bit);
	return rating ? (
		<ModelRatingBadge rating={rating} type={bit.type} className={className} />
	) : null;
}

function httpsSources(sources: string[]): URL[] {
	const seen = new Set<string>();
	return sources.flatMap((source) => {
		try {
			const url = new URL(source);
			if (url.protocol !== "https:" || url.username || url.password) return [];
			if (seen.has(url.href)) return [];
			seen.add(url.href);
			return [url];
		} catch {
			return [];
		}
	});
}

/** The rating with its scale, the likely range and how it was reached. */
export function ModelRatingPanel({
	rating,
	type,
}: Readonly<{ rating: ModelRating; type: IBitTypes }>) {
	const { t } = useTranslation("common");
	const headingId = useId();
	const copy = useRatingText()(rating, type);
	const tone = RATING_TONE[rating.tier];
	const [top, strong, fair] = RATING_FLOORS[rating.kind];
	const at = (value: number) => `${Math.min(100, Math.max(0, value))}%`;
	const bands: { from: number; to: number; tier: RatingTier }[] = [
		{ from: 0, to: fair, tier: 4 },
		{ from: fair, to: strong, tier: 3 },
		{ from: strong, to: top, tier: 2 },
		{ from: top, to: 100, tier: 1 },
	];
	const sources = useMemo(() => httpsSources(rating.sources), [rating.sources]);

	return (
		<section
			aria-labelledby={headingId}
			className={cn("space-y-3 rounded-xl border p-4", tone.panel)}
		>
			<div className="flex flex-wrap items-start gap-x-6 gap-y-3">
				<div className="min-w-36">
					<h3
						id={headingId}
						className="text-xs font-medium text-muted-foreground"
					>
						{copy.heading}
					</h3>
					<p className="mt-1 flex items-baseline gap-1.5">
						<span
							className={cn(
								"font-mono text-3xl font-semibold tracking-tight tabular-nums",
								tone.text,
							)}
						>
							{rating.estimated ? "≈" : ""}
							{copy.full}
						</span>
						<span className="text-xs text-muted-foreground">{copy.unit}</span>
					</p>
					<p className={cn("mt-0.5 text-xs font-semibold", tone.text)}>
						{copy.tierAmong}
					</p>
				</div>
				<div className="min-w-0 flex-[1_1_16rem] space-y-1.5 pt-2">
					<div aria-hidden="true" className="relative h-2.5">
						{bands.map((band) => (
							<span
								key={band.tier}
								className={cn(
									"absolute inset-y-0 opacity-35",
									RATING_TONE[band.tier].fill,
									band.from === 0 && "rounded-l-full",
									band.to === 100 && "rounded-r-full",
								)}
								style={{ left: at(band.from), width: at(band.to - band.from) }}
							/>
						))}
						{rating.low !== undefined && rating.high !== undefined && (
							<span
								className={cn(
									"absolute -inset-y-0.75 rounded-full border-2",
									tone.ring,
								)}
								style={{
									left: at(rating.low),
									width: at(rating.high - rating.low),
								}}
							/>
						)}
						<span
							className="absolute -inset-y-1.5 -ml-px w-0.75 rounded-full bg-foreground"
							style={{ left: at(rating.score) }}
						/>
					</div>
					<div
						aria-hidden="true"
						className="relative h-3.5 font-mono text-[10.5px] text-muted-foreground"
					>
						{[0, fair, strong, top, 100].map((value) => (
							<span
								key={value}
								className={cn(
									"absolute top-0",
									value === 100
										? "-translate-x-full"
										: value > 0 && "-translate-x-1/2",
								)}
								style={{ left: at(value) }}
							>
								{value}
							</span>
						))}
					</div>
					<p className="text-xs text-foreground/80">{copy.evidence}</p>
				</div>
			</div>
			{rating.criteria && (
				<p className="text-[13px] leading-relaxed text-foreground/85">
					<span className="text-muted-foreground">
						{t("ratingHowScored", "How it is scored:")}
					</span>{" "}
					{rating.criteria}
				</p>
			)}
			{rating.reasoning && (
				<p className="text-xs leading-relaxed text-muted-foreground">
					{rating.reasoning}
				</p>
			)}
			{(sources.length > 0 || rating.updatedAt) && (
				<p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
					{rating.updatedAt && (
						<span>
							{t("intelligenceEstimateDate", "Estimated {{date}}", {
								date: rating.updatedAt.slice(0, 10),
							})}
						</span>
					)}
					{sources.map((url) => (
						<a
							key={url.href}
							href={url.href}
							target="_blank"
							rel="noopener noreferrer"
							className="underline underline-offset-2 hover:text-foreground"
						>
							{url.hostname}
						</a>
					))}
				</p>
			)}
		</section>
	);
}
