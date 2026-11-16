"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useId } from "react";
import {
	COST_ASSUMPTIONS,
	type CostExampleKey,
	type ModelPrice,
	type PriceTier,
	costExamples,
	formatUsd,
} from "../../lib/bit/model-pricing";
import type { IBitTypes } from "../../lib/schema/bit/bit";
import { cn } from "../../lib/utils";
import { useModelTypeNoun } from "./model-rating";

export interface PriceText {
	main: string;
	sub: string;
	free: boolean;
}

/** A price as one short figure plus the unit it is charged in. */
export function usePriceText() {
	const { t } = useTranslation("common");
	return useCallback(
		(price: ModelPrice, size?: string): PriceText => {
			const paid = (main: string, sub: string) => ({
				main: price.reference && price.kind !== "unlisted" ? `≈ ${main}` : main,
				sub: price.reference
					? `OpenRouter · ${sub || t("providerReference", "provider reference")}`
					: sub,
				free: false,
			});
			switch (price.kind) {
				case "tokens":
					return paid(
						`${formatUsd(price.input)} / ${formatUsd(price.output)}`,
						t("pricePerMillionTokensInOut", "per 1M tokens, in / out"),
					);
				case "image":
					return paid(
						formatUsd(price.perImage),
						t("pricePerImage", "per image"),
					);
				case "video":
					return paid(formatUsd(price.perClip), t("pricePerClip", "per clip"));
				case "speech":
					return paid(
						formatUsd(price.perMillion),
						price.unit === "characters"
							? t("pricePerMillionCharacters", "per 1M characters")
							: t("pricePerMillionTextBytes", "per 1M bytes of text"),
					);
				case "transcription":
					return paid(
						formatUsd(price.perMinute),
						t("pricePerAudioMinute", "per audio minute"),
					);
				case "embedding":
					return paid(
						formatUsd(price.perMillion),
						price.unit === "tokens"
							? t("pricePerMillionTokens", "per 1M tokens")
							: t("pricePerMillionBytes", "per 1M bytes"),
					);
				case "free":
					return {
						main: t("priceFree", "Free"),
						sub:
							price.where === "flows"
								? t("priceRunsInFlows", "runs inside your flows")
								: size
									? t(
											"priceRunsOnDeviceSize",
											"runs on this device · {{size}}",
											{
												size,
											},
										)
									: t("priceRunsOnDevice", "runs on this device"),
						free: true,
					};
				case "ownKey":
					return paid(
						t("priceOwnKey", "Your API key"),
						t("priceBilledByProvider", "billed by the provider"),
					);
				default:
					return paid(t("priceUnlisted", "Price not listed"), "");
			}
		},
		[t],
	);
}

/** $ to $$$$: where a paid model's price sits among models of its type. */
export function PriceTierMark({
	tier,
	type,
	className,
}: Readonly<{ tier: PriceTier; type: IBitTypes; className?: string }>) {
	const { t } = useTranslation("common");
	const noun = useModelTypeNoun();
	const label = t(
		"priceLevelAmong",
		"Price level {{tier}} of 4 among {{type}}",
		{
			tier,
			type: noun(type),
		},
	);
	return (
		<span
			role="img"
			aria-label={label}
			title={label}
			className={cn("shrink-0 font-mono text-[11px] tracking-[1px]", className)}
		>
			{[1, 2, 3, 4].map((step) => (
				<span
					key={step}
					className={
						step <= tier ? "text-foreground/80" : "text-muted-foreground/35"
					}
				>
					$
				</span>
			))}
		</span>
	);
}

function useCostLabel() {
	const { t } = useTranslation("common");
	return useCallback(
		(key: CostExampleKey): string => {
			switch (key) {
				case "chatReply":
					return t("costChatReply", "One chat reply");
				case "longDocument":
					return t("costLongDocument", "Summarize a 50-page document");
				case "thousandReplies":
					return t("costThousandReplies", "1,000 chat replies");
				case "tenImages":
					return t("costTenImages", "10 images");
				case "hundredImages":
					return t("costHundredImages", "100 images");
				case "thousandImages":
					return t("costThousandImages", "1,000 images");
				case "shortClip":
					return t("costOneClip", "One clip");
				case "longClip":
					return t("costTenClips", "10 clips");
				case "hundredClips":
					return t("costHundredClips", "100 clips");
				case "speechMinute":
					return t("costSpeechMinute", "One minute of speech");
				case "speechHour":
					return t("costSpeechHour", "One hour of narration");
				case "speechHundredHours":
					return t("costSpeechHundredHours", "100 hours of narration");
				case "audioHour":
					return t("costAudioHour", "One hour of audio");
				case "audioTenHours":
					return t("costAudioTenHours", "10 hours of audio");
				case "audioHundredHours":
					return t("costAudioHundredHours", "100 hours of audio");
				case "thousandPages":
					return t("costThousandPages", "1,000 pages");
				case "hundredThousandPages":
					return t("costHundredThousandPages", "100,000 pages");
			}
		},
		[t],
	);
}

interface PriceRow {
	label: string;
	value: string;
	unit: string;
	free?: boolean;
}

/** The pricing block of the detail view: rates, how they compare, and what typical jobs cost. */
export function ModelPricePanel({
	price,
	tier,
	type,
	size,
}: Readonly<{
	price: ModelPrice;
	tier?: PriceTier;
	type: IBitTypes;
	size?: string;
}>) {
	const { t } = useTranslation("common");
	const headingId = useId();
	const noun = useModelTypeNoun();
	const costLabel = useCostLabel();
	const examples = costExamples(price);

	const rows: PriceRow[] = (() => {
		switch (price.kind) {
			case "tokens":
				return [
					{
						label: t("priceInput", "Input"),
						value: formatUsd(price.input),
						unit: t("pricePerMillionTokensUnit", "/ 1M tokens"),
					},
					{
						label: t("priceOutput", "Output"),
						value: formatUsd(price.output),
						unit: t("pricePerMillionTokensUnit", "/ 1M tokens"),
					},
					...(price.request > 0
						? [
								{
									label: t("pricePerRequestLabel", "Per request"),
									value: formatUsd(price.request),
									unit: "",
								},
							]
						: []),
				];
			case "image":
				return [
					{
						label: t("pricePerImageLabel", "Per image"),
						value: formatUsd(price.perImage),
						unit: "",
					},
				];
			case "video":
				return [
					{
						label: t("pricePerClipLabel", "Per clip"),
						value: formatUsd(price.perClip),
						unit: "",
					},
				];
			case "speech":
				return [
					{
						label: t("priceSpeech", "Speech"),
						value: formatUsd(price.perMillion),
						unit:
							price.unit === "characters"
								? t("pricePerMillionCharactersUnit", "/ 1M characters")
								: t("pricePerMillionBytesUnit", "/ 1M bytes"),
					},
				];
			case "transcription":
				return [
					{
						label: t("priceAudio", "Audio"),
						value: formatUsd(price.perMinute),
						unit: t("pricePerMinuteUnit", "/ minute"),
					},
				];
			case "embedding":
				return [
					{
						label: t("priceInput", "Input"),
						value: formatUsd(price.perMillion),
						unit:
							price.unit === "tokens"
								? t("pricePerMillionTokensUnit", "/ 1M tokens")
								: t("pricePerMillionBytesUnit", "/ 1M bytes"),
					},
				];
			case "free":
				return [
					{
						label: t("priceUsage", "Usage"),
						value: t("priceFree", "Free"),
						unit: "",
						free: true,
					},
					...(size
						? [
								{
									label: t("priceDownload", "Download"),
									value: size,
									unit: t("priceOnce", "once"),
								},
							]
						: []),
				];
			case "ownKey":
				return [
					{
						label: t("priceUsage", "Usage"),
						value: t("priceOwnKey", "Your API key"),
						unit: "",
					},
				];
			default:
				return [
					{
						label: t("priceUsage", "Usage"),
						value: t("priceNotListedShort", "Not listed"),
						unit: "",
					},
				];
		}
	})();

	const notes = (() => {
		if (price.reference)
			return [
				price.reference.basis,
				...(price.reference.configuration
					? [price.reference.configuration]
					: []),
				...price.reference.caveats,
				...(price.kind === "tokens"
					? [
							t(
								"priceReferenceTokenAssumptions",
								"Each reply uses {{replyInput}} input and {{replyOutput}} output tokens. Each document uses {{documentInput}} input and {{documentOutput}} output tokens.",
								{
									replyInput:
										COST_ASSUMPTIONS.replyInputTokens.toLocaleString(),
									replyOutput:
										COST_ASSUMPTIONS.replyOutputTokens.toLocaleString(),
									documentInput:
										COST_ASSUMPTIONS.documentInputTokens.toLocaleString(),
									documentOutput:
										COST_ASSUMPTIONS.documentOutputTokens.toLocaleString(),
								},
							),
						]
					: []),
				...(price.kind === "embedding"
					? [
							t(
								"priceReferenceEmbeddingAssumptions",
								"Each page uses {{amount}} {{unit}} in one request.",
								{
									amount: (price.unit === "tokens"
										? COST_ASSUMPTIONS.tokensPerPage
										: COST_ASSUMPTIONS.bytesPerPage
									).toLocaleString(),
									unit: price.unit,
								},
							),
						]
					: []),
				...(price.kind === "speech"
					? [
							price.unit === "bytes"
								? t(
										"priceSpeechByteAssumption",
										"Speech examples assume 900 ASCII text bytes per minute. Other characters can use more bytes.",
									)
								: t(
										"priceSpeechCharacterAssumption",
										"Speech examples assume 900 input characters per minute.",
									),
						]
					: []),
			];
		switch (price.kind) {
			case "free":
				return price.where === "flows"
					? [
							t(
								"priceNoteFlows",
								"Runs inside the flow runtime and counts as normal run time.",
							),
						]
					: [
							t(
								"priceNoteDevice",
								"Runs on this device after a one-time download. Nothing is sent to a provider.",
							),
						];
			case "ownKey":
				return [
					t(
						"priceNoteOwnKey",
						"Calls go to the provider on your own key and are billed by them.",
					),
				];
			case "unlisted":
				return [
					t(
						"priceNoteUnlisted",
						"The catalog lists no price for this model yet.",
					),
				];
			default:
				return [
					t(
						"priceNoteListPrice",
						"List price in USD, metered against your plan’s usage.",
					),
					...(price.kind === "tokens" && price.fromBenchmark
						? [
								t(
									"priceNoteFromBenchmark",
									"Taken from the benchmark listing because the catalog sets no price.",
								),
							]
						: []),
					...(price.kind === "tokens"
						? [
								t(
									"priceNoteReplyAssumption",
									"A reply here is {{input}} tokens in and {{output}} out.",
									{
										input: COST_ASSUMPTIONS.replyInputTokens.toLocaleString(),
										output: COST_ASSUMPTIONS.replyOutputTokens.toLocaleString(),
									},
								),
							]
						: []),
				];
		}
	})();

	const tierName = tier
		? [
				"",
				t("priceTierLow", "Low cost"),
				t("priceTierMid", "Mid-range"),
				t("priceTierHigh", "Above average"),
				t("priceTierPremium", "Premium"),
			][tier]
		: "";

	return (
		<section
			aria-labelledby={headingId}
			className="space-y-3 rounded-xl border bg-muted/40 p-4"
		>
			<div className="flex items-center justify-between gap-2">
				<h3 id={headingId} className="text-[13px] font-semibold">
					{price.reference
						? t("providerPriceReference", "Provider price reference")
						: t("pricing", "Pricing")}
				</h3>
				{tier && <PriceTierMark tier={tier} type={type} className="text-xs" />}
			</div>
			{rows.map((row) => (
				<div
					key={row.label}
					className="flex flex-wrap items-baseline justify-between gap-x-2"
				>
					<span className="text-[12.5px] text-muted-foreground">
						{row.label}
					</span>
					<span>
						<span
							className={cn(
								"font-mono text-lg font-semibold tracking-tight tabular-nums",
								row.free && "text-emerald-600 dark:text-emerald-400",
							)}
						>
							{row.value}
						</span>
						{row.unit && (
							<span className="text-[11.5px] text-muted-foreground">
								{" "}
								{row.unit}
							</span>
						)}
					</span>
				</div>
			))}
			{tier && (
				<p className="-mt-1 text-xs text-muted-foreground">
					{t("ratingTierAmong", "{{tier}} among {{type}}", {
						tier: tierName,
						type: noun(type),
					})}
				</p>
			)}
			{examples.length > 0 && (
				<div className="space-y-1.5 border-t border-dashed pt-3">
					<p className="text-[11px] font-semibold uppercase tracking-[0.07em] text-muted-foreground">
						{price.reference
							? t("exampleProviderCosts", "Example provider costs")
							: t("priceWhatItCosts", "What it costs")}
					</p>
					{examples.map((example) => (
						<div
							key={example.key}
							className="flex justify-between gap-2 text-[12.5px]"
						>
							<span className="text-foreground/80">
								{costLabel(example.key)}
							</span>
							<span className="shrink-0 whitespace-nowrap font-mono tabular-nums">
								≈ {formatUsd(example.usd)}
							</span>
						</div>
					))}
				</div>
			)}
			{notes.map((note) => (
				<p
					key={note}
					className="text-[11.5px] leading-relaxed text-muted-foreground"
				>
					{note}
				</p>
			))}
			{price.reference && (
				<div className="space-y-1 border-t border-dashed pt-2 text-[11.5px] text-muted-foreground">
					<p>
						{t("priceReferenceCheckedAt", "Prices checked {{date}}.", {
							date: new Date(price.reference.checkedAt)
								.toISOString()
								.slice(0, 10),
						})}
					</p>
					{price.reference.providers.length > 1 && (
						<details>
							<summary className="cursor-pointer">
								{t(
									"viewProviderPriceOptions",
									"View {{count}} provider price options",
									{ count: price.reference.providers.length },
								)}
							</summary>
							<div className="mt-2 max-h-56 space-y-1 overflow-y-auto">
								{price.reference.providers.map((provider, index) => (
									<p key={`${provider.name}-${index}`}>
										{provider.name}:{" "}
										{provider.price.kind === "tokens"
											? `${formatUsd(provider.price.input)} / ${formatUsd(provider.price.output)} per 1M tokens`
											: provider.price.kind === "transcription"
												? `${formatUsd(provider.price.perMinute)} per audio minute`
												: provider.price.kind === "speech" ||
														provider.price.kind === "embedding"
													? `${formatUsd(provider.price.perMillion)} per 1M ${provider.price.unit}`
													: provider.price.kind === "image"
														? `${formatUsd(provider.price.perImage)} per example image`
														: ""}
									</p>
								))}
							</div>
						</details>
					)}
					<a
						href={price.reference.sources.at(-1)}
						target="_blank"
						rel="noopener noreferrer"
						className="underline underline-offset-2"
					>
						{t("viewProviderPricingSource", "View provider pricing source")}
					</a>
				</div>
			)}
		</section>
	);
}
