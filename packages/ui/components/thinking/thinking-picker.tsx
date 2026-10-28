"use client";

import { useTranslation } from "@flow-like/locales";
import CheckIcon from "lucide-react/dist/esm/icons/check.js";
import { useId, useMemo, useState } from "react";
import { cn } from "../../lib/utils";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { ThinkingGlyph } from "./thinking-glyph";
import type {
	ThinkingGlyphLevel,
	ThinkingLabels,
	ThinkingOption,
} from "./thinking-options";

export interface ThinkingPickerProps {
	readonly value: string;
	readonly options: readonly ThinkingOption[];
	readonly onChange: (id: string) => void;
	readonly disabled?: boolean;
	readonly className?: string;
	readonly contentClassName?: string;
	readonly side?: "top" | "bottom";
}

export function useThinkingLabels(): ThinkingLabels {
	const { t } = useTranslation("chat");
	return useMemo(() => {
		const descriptions: Record<ThinkingGlyphLevel, string> = {
			0: t(
				"answersRightAwayGoodForQuickLookups",
				"Answers right away. Good for quick lookups.",
			),
			1: t("aShortThinkBeforeItAnswers", "A short think before it answers."),
			2: t(
				"worksThroughTheStepsBeforeItAnswers",
				"Works through the steps before it answers.",
			),
			3: t(
				"takesItsTimeOnHardProblemsSlowerAndUsesMoreTokens",
				"Takes its time on hard problems. Slower, and uses more tokens.",
			),
			4: t(
				"theDeepestPassTheModelOffersSlowest",
				"The deepest pass the model offers. Slowest.",
			),
		};
		return {
			off: t("thinkingOff", "Off"),
			low: t("thinkingLow", "Low"),
			mid: t("thinkingMid", "Mid"),
			high: t("thinkingHigh", "High"),
			extraHigh: t("thinkingExtraHigh", "Extra high"),
			auto: t("thinkingAuto", "Auto"),
			autoDescription: t(
				"usesTheProvidersDefault",
				"Uses the provider's default.",
			),
			autoWithDefault: (level: string) =>
				t(
					"usesTheProvidersDefaultLevel",
					"Uses the provider's default ({{level}}).",
					{
						level,
					},
				),
			describe: (level: ThinkingGlyphLevel) => descriptions[level],
		};
	}, [t]);
}

/**
 * The thinking control shared by the app chat and FlowPilot: an atom icon that shows the
 * level, opening a menu with every level and what it does.
 */
export function ThinkingPicker({
	value,
	options,
	onChange,
	disabled = false,
	className,
	contentClassName,
	side = "top",
}: ThinkingPickerProps) {
	const { t } = useTranslation("chat");
	const [open, setOpen] = useState(false);
	const headingId = useId();
	const selected = options.find((option) => option.id === value) ?? options[0];
	if (!selected) return null;

	const triggerLabel = t("thinkingLevelLabel", "Thinking: {{level}}", {
		level: selected.label,
	});

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					disabled={disabled}
					aria-label={triggerLabel}
					title={triggerLabel}
					className={cn(
						"inline-flex size-8 shrink-0 items-center justify-center rounded-lg outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-primary/40 disabled:pointer-events-none disabled:opacity-50 data-[state=open]:bg-accent",
						className,
					)}
				>
					<ThinkingGlyph level={selected.level} auto={selected.auto} />
				</button>
			</PopoverTrigger>
			<PopoverContent
				side={side}
				align="start"
				sideOffset={8}
				className={cn("w-72 p-1.5", contentClassName)}
			>
				<p
					id={headingId}
					className="px-2 pb-1.5 pt-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground"
				>
					{t("thinkingMenuTitle", "Thinking")}
				</p>
				<fieldset
					aria-labelledby={headingId}
					className="m-0 space-y-0.5 border-0 p-0"
				>
					{options.map((option) => {
						const active = option.id === selected.id;
						return (
							<button
								key={option.id || "auto"}
								type="button"
								aria-pressed={active}
								onClick={() => {
									onChange(option.id);
									setOpen(false);
								}}
								className={cn(
									"fl-think-row flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left outline-none transition-colors hover:bg-accent focus-visible:bg-accent",
									active && "bg-primary/10",
								)}
							>
								<span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted/60">
									<ThinkingGlyph
										level={option.level}
										auto={option.auto}
										size="md"
										idle={!active}
									/>
								</span>
								<span className="min-w-0 flex-1">
									<span className="block text-sm font-medium leading-tight">
										{option.label}
									</span>
									{option.description && (
										<span className="mt-0.5 block text-xs leading-snug text-muted-foreground">
											{option.description}
										</span>
									)}
								</span>
								<CheckIcon
									aria-hidden="true"
									className={cn(
										"size-4 shrink-0 text-primary transition-opacity",
										active ? "opacity-100" : "opacity-0",
									)}
								/>
							</button>
						);
					})}
				</fieldset>
			</PopoverContent>
		</Popover>
	);
}
