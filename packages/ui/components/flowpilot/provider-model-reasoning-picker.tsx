"use client";

import {
	BotIcon,
	BrainCircuitIcon,
	CheckIcon,
	ChevronDownIcon,
	Code2Icon,
	GithubIcon,
	LayersIcon,
	LogOutIcon,
	type LucideIcon,
} from "lucide-react";
import { useState } from "react";

import { cn } from "../../lib/utils";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import type {
	AIProvider,
	CopilotReasoningEffort,
	NormalizedAIProvider,
} from "./types";
import { normalizeAIProvider } from "./types";

export interface ProviderModelPickerProvider {
	id: AIProvider;
	label: string;
	disabled?: boolean;
	title?: string;
}

export interface ProviderModelPickerModel {
	id: string;
	label: string;
	supportedReasoningEfforts?: CopilotReasoningEffort[];
	defaultReasoningEffort?: string;
}

export interface ProviderModelReasoningPickerProps {
	provider: AIProvider;
	providers: ProviderModelPickerProvider[];
	models: ProviderModelPickerModel[];
	selectedModelId: string;
	selectedEffort: string;
	onProviderChange: (provider: AIProvider) => void | Promise<void>;
	onModelChange: (modelId: string) => void;
	onEffortChange: (effort: string) => void;
	disabled?: boolean;
	connecting?: boolean;
	connected?: boolean;
	onDisconnect?: () => void | Promise<void>;
	statusText?: string;
	showProviderSection?: boolean;
	emptyModelLabel?: string;
	triggerClassName?: string;
	contentClassName?: string;
	align?: "start" | "center" | "end";
	sideOffset?: number;
}

const PROVIDER_ICONS: Record<NormalizedAIProvider, LucideIcon> = {
	bits: LayersIcon,
	"github-copilot": GithubIcon,
	codex: Code2Icon,
	"claude-code": BotIcon,
};

export function reasoningEffortLabels(
	model: ProviderModelPickerModel | undefined,
	selectedEffort: string,
) {
	const efforts = model?.supportedReasoningEfforts ?? [];
	const defaultName = model?.defaultReasoningEffort
		? (efforts.find((effort) => effort.id === model.defaultReasoningEffort)
				?.name ?? model.defaultReasoningEffort)
		: undefined;
	const automatic = defaultName
		? `Auto (${defaultName} default)`
		: "Auto (provider default)";
	const selected = selectedEffort
		? (efforts.find((effort) => effort.id === selectedEffort)?.name ??
			selectedEffort)
		: automatic;

	return { automatic, selected, efforts };
}

/**
 * One model-aware picker shared by FlowPilot surfaces. It deliberately owns no
 * backend lifecycle: callers keep their existing start/stop and persistence
 * behavior in `onProviderChange`/`onDisconnect`.
 */
export function ProviderModelReasoningPicker({
	provider,
	providers,
	models,
	selectedModelId,
	selectedEffort,
	onProviderChange,
	onModelChange,
	onEffortChange,
	disabled = false,
	connecting = false,
	connected = false,
	onDisconnect,
	statusText,
	showProviderSection = true,
	emptyModelLabel,
	triggerClassName,
	contentClassName,
	align = "start",
	sideOffset = 6,
}: ProviderModelReasoningPickerProps) {
	const [open, setOpen] = useState(false);
	const normalizedProvider = normalizeAIProvider(provider);
	const currentProvider =
		providers.find(
			(option) => normalizeAIProvider(option.id) === normalizedProvider,
		) ?? providers[0];
	const ProviderIcon = PROVIDER_ICONS[normalizedProvider] ?? BotIcon;
	const selectedModel = models.find((model) => model.id === selectedModelId);
	const reasoning = reasoningEffortLabels(selectedModel, selectedEffort);
	const modelLabel =
		selectedModel?.label ??
		(connecting ? "Connecting…" : emptyModelLabel || "Select model");
	const title = [
		currentProvider?.label,
		modelLabel,
		reasoning.efforts.length > 0 ? reasoning.selected : undefined,
	]
		.filter(Boolean)
		.join(" · ");

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					disabled={disabled}
					aria-label={`Provider and model: ${title}`}
					title={title}
					className={cn(
						"inline-flex h-8 min-w-0 items-center gap-1.5 rounded-lg border border-border/40 bg-background/60 px-2.5 text-xs outline-none transition-colors hover:border-primary/30 hover:bg-accent/40 focus-visible:ring-2 focus-visible:ring-primary/40 disabled:pointer-events-none disabled:opacity-50",
						triggerClassName,
					)}
				>
					<span className="relative shrink-0">
						<ProviderIcon className="size-3.5 text-primary" />
						{connected && (
							<span className="absolute -bottom-0.5 -right-0.5 size-1.5 rounded-full border border-background bg-emerald-500" />
						)}
					</span>
					<span className="min-w-0 max-w-40 truncate font-medium">
						{modelLabel}
					</span>
					{reasoning.efforts.length > 0 && (
						<span data-slot="picker-reasoning" className="contents">
							<span className="text-border" aria-hidden="true">
								·
							</span>
							<BrainCircuitIcon className="size-3.5 shrink-0 text-muted-foreground" />
							<span className="min-w-0 max-w-36 truncate text-muted-foreground">
								{reasoning.selected}
							</span>
						</span>
					)}
					<ChevronDownIcon className="size-3 shrink-0 opacity-50" />
				</button>
			</PopoverTrigger>
			<PopoverContent
				align={align}
				sideOffset={sideOffset}
				className={cn("w-72 p-2", contentClassName)}
			>
				{showProviderSection && providers.length > 0 && (
					<>
						<p className="px-1 pb-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
							Provider
						</p>
						<div className="flex gap-0.5 rounded-lg border border-border/40 bg-muted/30 p-0.5">
							{providers.map((option) => {
								const optionId = normalizeAIProvider(option.id);
								const Icon = PROVIDER_ICONS[optionId] ?? BotIcon;
								const active = optionId === normalizedProvider;
								return (
									<button
										key={option.id}
										type="button"
										title={option.title ?? option.label}
										aria-label={option.label}
										disabled={disabled || option.disabled}
										onClick={() => void onProviderChange(option.id)}
										className={cn(
											"flex h-8 flex-1 items-center justify-center rounded-md outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary/40 disabled:cursor-not-allowed disabled:opacity-40",
											active
												? "bg-linear-to-br from-primary to-purple-600 text-primary-foreground shadow-sm"
												: "text-muted-foreground hover:bg-muted hover:text-foreground",
										)}
									>
										<Icon className="size-4" />
									</button>
								);
							})}
						</div>
					</>
				)}

				<p className="px-1 pb-1.5 pt-2.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
					Model
				</p>
				<div className="max-h-48 space-y-0.5 overflow-y-auto">
					{models.length === 0 ? (
						<p className="px-2 py-4 text-center text-xs text-muted-foreground">
							{connecting
								? "Connecting…"
								: emptyModelLabel || "No models available"}
						</p>
					) : (
						models.map((model) => {
							const active = model.id === selectedModelId;
							return (
								<button
									key={model.id}
									type="button"
									onClick={() => {
										onModelChange(model.id);
										if ((model.supportedReasoningEfforts?.length ?? 0) === 0) {
											setOpen(false);
										}
									}}
									className={cn(
										"flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary/40",
										active ? "bg-primary/10 text-primary" : "hover:bg-muted",
									)}
								>
									<span className="flex-1 truncate">{model.label}</span>
									{active && <CheckIcon className="size-3.5 shrink-0" />}
								</button>
							);
						})
					)}
				</div>

				{reasoning.efforts.length > 0 && (
					<>
						<p className="px-1 pb-1.5 pt-2.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
							Reasoning
						</p>
						<div className="grid grid-cols-2 gap-1">
							<button
								type="button"
								onClick={() => {
									onEffortChange("");
									setOpen(false);
								}}
								className={cn(
									"col-span-2 flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary/40",
									!selectedEffort
										? "bg-primary/10 text-primary"
										: "hover:bg-muted",
								)}
							>
								<BrainCircuitIcon className="size-3.5 shrink-0" />
								<span className="flex-1 truncate">{reasoning.automatic}</span>
								{!selectedEffort && <CheckIcon className="size-3.5 shrink-0" />}
							</button>
							{reasoning.efforts.map((effort) => {
								const active = effort.id === selectedEffort;
								return (
									<button
										key={effort.id}
										type="button"
										title={effort.description}
										onClick={() => {
											onEffortChange(effort.id);
											setOpen(false);
										}}
										className={cn(
											"flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-primary/40",
											active ? "bg-primary/10 text-primary" : "hover:bg-muted",
										)}
									>
										<span className="flex-1 truncate">
											{effort.name || effort.id}
										</span>
										{active && <CheckIcon className="size-3.5 shrink-0" />}
									</button>
								);
							})}
						</div>
					</>
				)}

				{statusText && (
					<p className="mt-2 border-t border-border/40 px-1 pt-2 text-[11px] text-muted-foreground">
						{statusText}
					</p>
				)}
				{connected && onDisconnect && normalizedProvider !== "bits" && (
					<button
						type="button"
						disabled={disabled || connecting}
						onClick={() => {
							setOpen(false);
							void onDisconnect();
						}}
						className="mt-2 flex w-full items-center justify-center gap-1.5 rounded-md border border-border/50 px-2 py-1.5 text-xs text-muted-foreground outline-none transition-colors hover:border-destructive/40 hover:bg-destructive/10 hover:text-destructive focus-visible:ring-2 focus-visible:ring-primary/40 disabled:opacity-50"
					>
						<LogOutIcon className="size-3.5" />
						Disconnect {currentProvider?.label ?? "provider"}
					</button>
				)}
			</PopoverContent>
		</Popover>
	);
}
