"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { cn } from "../../../lib/utils";
import type { BotEditorStep } from "./bot-editor-steps";

export function useBotStepLabels(): Record<BotEditorStep, string> {
	const { t } = useTranslation("bots");
	return useMemo(
		() => ({
			describe: t("editor.steps.describe"),
			look: t("editor.steps.look"),
			abilities: t("editor.steps.abilities"),
			brain: t("editor.steps.brain"),
			location: t("editor.steps.location"),
			safety: t("editor.steps.safety"),
			secrets: t("editor.steps.secrets"),
		}),
		[t],
	);
}

/** Numbered sections like the look studio's, with a dot on steps that still need something. */
export function BotStepNav({
	steps,
	current,
	flagged,
	onSelect,
}: Readonly<{
	steps: readonly BotEditorStep[];
	current: BotEditorStep;
	flagged: ReadonlySet<BotEditorStep>;
	onSelect: (step: BotEditorStep) => void;
}>) {
	const { t } = useTranslation("bots");
	const labels = useBotStepLabels();
	return (
		<nav
			aria-label={t("editor.stepsLabel")}
			className="flex gap-1 overflow-x-auto rounded-xl border border-border bg-muted p-1 [scrollbar-width:none]"
		>
			{steps.map((step, index) => {
				const active = step === current;
				return (
					<button
						key={step}
						type="button"
						aria-current={active ? "step" : undefined}
						onClick={() => onSelect(step)}
						className={cn(
							"flex min-h-10 shrink-0 grow basis-0 items-center justify-center gap-2 rounded-lg px-3 text-[0.8125rem] font-medium whitespace-nowrap text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
							active && "bg-background text-foreground shadow-sm",
						)}
					>
						<span
							aria-hidden
							className={cn(
								"grid size-[18px] shrink-0 place-items-center rounded-full bg-border text-[0.625rem] tabular-nums",
								active && "bg-primary text-primary-foreground",
							)}
						>
							{index + 1}
						</span>
						{labels[step]}
						{flagged.has(step) && (
							<span
								className="size-1.5 shrink-0 rounded-full bg-warning-solid"
								title={t("editor.stepNeedsAttention")}
							>
								<span className="sr-only">
									{t("editor.stepNeedsAttention")}
								</span>
							</span>
						)}
					</button>
				);
			})}
		</nav>
	);
}
