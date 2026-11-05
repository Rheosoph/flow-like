"use client";

import { useTranslation } from "@flow-like/locales";
import { Sparkles } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { cn } from "../../../../lib/utils";
import { BotAvatar } from "../../../bots";
import { Button } from "../../../ui/button";
import { Textarea } from "../../../ui/textarea";
import { BOT_CAPABILITY_ICONS } from "../bot-capability-groups";
import {
	useBotCapabilityCopy,
	useBotLocationCopy,
	useBotPresetTexts,
} from "../bot-editor-copy";
import { BotSectionHeading, BotStepPanel, BotTag } from "../bot-editor-parts";
import type { BotPresetMatch } from "../bot-preset-match";
import { BOT_PRESETS, type BotPreset } from "../bot-presets";
import { BOT_LOCATION_ICONS } from "../bot-preview-panel";

const MATCHES_SHOWN = 3;

export function DescribeStep({
	text,
	matches,
	choice,
	onTextChange,
	onPick,
	onScratch,
}: Readonly<{
	text: string;
	matches: readonly BotPresetMatch[];
	/** The preset id that set up the draft, or `BOT_SCRATCH_CHOICE`. */
	choice: string | null;
	onTextChange: (text: string) => void;
	onPick: (preset: BotPreset) => void;
	onScratch: () => void;
}>) {
	const { t } = useTranslation("bots");
	const presetTexts = useBotPresetTexts();
	const [showAll, setShowAll] = useState(false);
	const inputId = useId();
	const inputRef = useRef<HTMLTextAreaElement>(null);
	useEffect(() => inputRef.current?.focus(), []);
	const typing = text.trim().length > 0;
	const ranked = matches.slice(0, MATCHES_SHOWN).map((match) => match.preset);
	const rest = BOT_PRESETS.filter((preset) => !ranked.includes(preset));
	const top = matches[0]?.preset;
	const picked = BOT_PRESETS.find((preset) => preset.id === choice);
	const primary = typing && ranked.length ? ranked : BOT_PRESETS;
	const secondary = typing && ranked.length && showAll ? rest : [];
	const card = (preset: BotPreset) => (
		<BotPresetCard
			key={preset.id}
			preset={preset}
			best={typing && preset === top}
			selected={preset.id === choice}
			onPick={onPick}
		/>
	);

	return (
		<BotStepPanel
			title={t("editor.describe.title")}
			subtitle={t("editor.describe.subtitle")}
		>
			<div className="flex flex-col gap-2">
				<label htmlFor={inputId} className="sr-only">
					{t("editor.describe.title")}
				</label>
				<Textarea
					ref={inputRef}
					id={inputId}
					value={text}
					rows={4}
					placeholder={t("editor.describe.placeholder")}
					onChange={(event) => onTextChange(event.target.value)}
					className="min-h-28 resize-y rounded-xl px-4 py-3 text-base leading-relaxed md:text-sm"
				/>
				<p aria-live="polite" className="text-xs text-muted-foreground">
					{picked
						? t("editor.describe.applied", {
								preset: presetTexts(picked).name,
							})
						: typing && top
							? t("editor.describe.nextUses", {
									preset: presetTexts(top).name,
								})
							: typing
								? t("editor.describe.noMatch")
								: t("editor.describe.hint")}
				</p>
			</div>
			<div className="flex flex-col gap-3">
				<BotSectionHeading
					aside={
						<Button
							variant="link"
							size="sm"
							className="h-auto px-0"
							onClick={onScratch}
						>
							{t("editor.describe.scratch")}
						</Button>
					}
				>
					{typing && ranked.length
						? t("editor.describe.bestMatches")
						: t("editor.describe.presets")}
				</BotSectionHeading>
				<div className="grid gap-3 @xl:grid-cols-2">{primary.map(card)}</div>
				{typing && ranked.length > 0 && rest.length > 0 && (
					<Button
						variant="ghost"
						size="sm"
						className="self-start"
						aria-expanded={showAll}
						onClick={() => setShowAll(!showAll)}
					>
						{showAll
							? t("editor.describe.showFewer")
							: t("editor.describe.showAll")}
					</Button>
				)}
				{secondary.length > 0 && (
					<div className="grid gap-3 @xl:grid-cols-2">
						{secondary.map(card)}
					</div>
				)}
			</div>
		</BotStepPanel>
	);
}

function BotPresetCard({
	preset,
	best,
	selected,
	onPick,
}: Readonly<{
	preset: BotPreset;
	best: boolean;
	selected: boolean;
	onPick: (preset: BotPreset) => void;
}>) {
	const { t } = useTranslation("bots");
	const texts = useBotPresetTexts()(preset);
	const capabilityCopy = useBotCapabilityCopy();
	const locationCopy = useBotLocationCopy();
	const LocationIcon = BOT_LOCATION_ICONS[preset.location];
	return (
		<button
			type="button"
			aria-pressed={selected}
			onClick={() => onPick(preset)}
			className={cn(
				"group flex min-w-0 items-start gap-3 rounded-xl border border-border bg-card p-3 text-left outline-none transition-colors hover:border-primary/50 focus-visible:ring-2 focus-visible:ring-ring",
				best && "border-primary/50 bg-primary/5",
				selected && "border-primary ring-2 ring-primary/25",
			)}
		>
			<span className="shrink-0 rounded-xl bg-muted p-1">
				<BotAvatar
					appearance={preset.appearance}
					size={52}
					pose="center"
					animated={false}
					decorative
				/>
			</span>
			<span className="flex min-w-0 flex-1 flex-col gap-1.5">
				<span className="flex flex-wrap items-center gap-1.5">
					<span className="text-sm font-semibold">{texts.name}</span>
					{selected ? (
						<BotTag className="bg-primary text-primary-foreground">
							{t("editor.describe.selected")}
						</BotTag>
					) : (
						best && (
							<BotTag className="bg-primary/15 text-primary">
								<Sparkles className="size-3" aria-hidden />
								{t("editor.describe.bestMatch")}
							</BotTag>
						)
					)}
				</span>
				<span className="text-xs leading-relaxed text-muted-foreground">
					{texts.description}
				</span>
				<span className="flex flex-wrap gap-1">
					<BotTag>
						<LocationIcon className="size-3" aria-hidden />
						{locationCopy(preset.location).label}
					</BotTag>
					{preset.capabilities.map((capability) => {
						const Icon = BOT_CAPABILITY_ICONS[capability];
						return (
							<BotTag key={capability}>
								<Icon className="size-3" aria-hidden />
								{capabilityCopy(capability).label}
							</BotTag>
						);
					})}
				</span>
			</span>
		</button>
	);
}
