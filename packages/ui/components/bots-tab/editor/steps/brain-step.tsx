"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowDown, ArrowUp, Plus, X } from "lucide-react";
import { type ReactNode, useId } from "react";
import { cn } from "../../../../lib/utils";
import {
	BOT_LIMITS,
	type BotLocation,
	type IBotModelPolicy,
} from "../../../../state/backend-state/bot-state";
import { Button } from "../../../ui/button";
import { Label } from "../../../ui/label";
import { Textarea } from "../../../ui/textarea";
import { BotNotice, BotStepPanel } from "../bot-editor-parts";
import { BotModelPicker } from "./model-picker";
import { BotDecisionModelField } from "./decision-model-field";
import { type BotModelOption, useBotModelOptions } from "./use-bot-models";

function moveItem<T>(items: readonly T[], from: number, to: number): T[] {
	const next = [...items];
	const [item] = next.splice(from, 1);
	next.splice(to, 0, item);
	return next;
}

export function BrainStep({
	instructions,
	model,
	location,
	onInstructionsChange,
	onModelChange,
	children,
}: Readonly<{
	instructions: string;
	model: IBotModelPolicy;
	location: BotLocation;
	onInstructionsChange: (instructions: string) => void;
	onModelChange: (model: IBotModelPolicy) => void;
	/** Shown after the models, such as the chat's model settings and a FlowPilot bot's memory model. */
	children?: ReactNode;
}>) {
	const { t } = useTranslation("bots");
	const ids = useId();
	const { options } = useBotModelOptions();
	const length = instructions.trim().length;
	const primaryIsLocal = options.some(
		(option) => option.id === model.primary && option.local,
	);
	return (
		<BotStepPanel
			title={t("editor.brain.title")}
			subtitle={t("editor.brain.subtitle")}
		>
			<div className="flex flex-col gap-1.5">
				<div className="flex items-baseline justify-between gap-3">
					<Label htmlFor={`${ids}-instructions`}>
						{t("editor.brain.instructions")}
					</Label>
					<span
						className={cn(
							"text-[0.6875rem] tabular-nums text-muted-foreground",
							length > BOT_LIMITS.instructions && "text-destructive",
						)}
					>
						{t("editor.brain.instructionsCount", {
							length,
							max: BOT_LIMITS.instructions,
						})}
					</span>
				</div>
				<Textarea
					id={`${ids}-instructions`}
					value={instructions}
					rows={7}
					aria-invalid={length > BOT_LIMITS.instructions || undefined}
					placeholder={t("editor.brain.instructionsPlaceholder")}
					onChange={(event) => onInstructionsChange(event.target.value)}
					className="min-h-36 resize-y rounded-xl leading-relaxed"
				/>
			</div>
			<div className="flex flex-col gap-1.5">
				<span className="text-sm font-medium">{t("editor.brain.primary")}</span>
				<BotModelPicker
					label={t("editor.brain.primary")}
					value={model.primary}
					options={options}
					fallback={{
						label: t("editor.brain.automatic"),
						hint: t("editor.brain.automaticHint"),
					}}
					exclude={model.fallbacks}
					onChange={(primary) =>
						onModelChange({
							...model,
							primary,
							fallbacks: model.fallbacks.filter((id) => id !== primary),
						})
					}
				/>
			</div>
			<FallbackList
				model={model}
				options={options}
				onModelChange={onModelChange}
			/>
			{location === "server" && primaryIsLocal && (
				<BotNotice tone="warning">{t("editor.brain.localOnServer")}</BotNotice>
			)}
			<BotDecisionModelField
				value={model.decision ?? null}
				location={location}
				onChange={(decision) => onModelChange({ ...model, decision })}
			/>
			{children}
		</BotStepPanel>
	);
}

function FallbackList({
	model,
	options,
	onModelChange,
}: Readonly<{
	model: IBotModelPolicy;
	options: readonly BotModelOption[];
	onModelChange: (model: IBotModelPolicy) => void;
}>) {
	const { t } = useTranslation("bots");
	const { fallbacks } = model;
	const taken = [...(model.primary ? [model.primary] : []), ...fallbacks];
	const setFallbacks = (next: string[]) =>
		onModelChange({ ...model, fallbacks: next });
	const full = fallbacks.length >= BOT_LIMITS.fallbacks;
	return (
		<div className="flex flex-col gap-2">
			<div className="flex flex-col gap-1">
				<span className="text-sm font-medium">
					{t("editor.brain.fallbacks")}
				</span>
				<p className="text-xs leading-relaxed text-muted-foreground">
					{t("editor.brain.fallbacksHint")}
				</p>
			</div>
			{fallbacks.length > 0 && (
				<ol className="flex flex-col gap-2">
					{fallbacks.map((id, index) => (
						<li key={id} className="flex items-center gap-2">
							<span className="grid size-6 shrink-0 place-items-center rounded-full bg-muted text-[0.6875rem] font-medium tabular-nums text-muted-foreground">
								{index + 2}
							</span>
							<div className="min-w-0 flex-1">
								<BotModelPicker
									label={t("editor.brain.fallbackLabel", {
										position: index + 1,
									})}
									value={id}
									options={options}
									exclude={taken}
									onChange={(next) =>
										setFallbacks(
											fallbacks.map((existing, position) =>
												position === index && next ? next : existing,
											),
										)
									}
								/>
							</div>
							<Button
								variant="ghost"
								size="icon"
								aria-label={t("editor.brain.moveUp")}
								disabled={index === 0}
								onClick={() =>
									setFallbacks(moveItem(fallbacks, index, index - 1))
								}
							>
								<ArrowUp />
							</Button>
							<Button
								variant="ghost"
								size="icon"
								aria-label={t("editor.brain.moveDown")}
								disabled={index === fallbacks.length - 1}
								onClick={() =>
									setFallbacks(moveItem(fallbacks, index, index + 1))
								}
							>
								<ArrowDown />
							</Button>
							<Button
								variant="ghost"
								size="icon"
								aria-label={t("editor.brain.remove")}
								onClick={() =>
									setFallbacks(
										fallbacks.filter((_, position) => position !== index),
									)
								}
							>
								<X />
							</Button>
						</li>
					))}
				</ol>
			)}
			<div className="flex items-center gap-3">
				<BotModelPicker
					label={t("editor.brain.addFallback")}
					value={null}
					options={options}
					exclude={taken}
					onChange={(id) => id && setFallbacks([...fallbacks, id])}
					trigger={
						<Button variant="outline" size="sm" disabled={full}>
							<Plus />
							{t("editor.brain.addFallback")}
						</Button>
					}
				/>
				{full && (
					<span className="text-xs text-muted-foreground">
						{t("editor.brain.fallbackLimit", { max: BOT_LIMITS.fallbacks })}
					</span>
				)}
			</div>
		</div>
	);
}
