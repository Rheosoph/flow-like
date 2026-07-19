"use client";

import {
	BrainCircuitIcon,
	Code2Icon,
	GithubIcon,
	LayersIcon,
	ServerIcon,
	SparklesIcon,
} from "lucide-react";
import { memo, useCallback } from "react";

import { isTauri } from "../../lib/platform";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

import {
	type AIProvider,
	type AgentBackendProvider,
	type CopilotAuthStatus,
	type CopilotModel,
	isAgentBackendProvider,
	normalizeAIProvider,
} from "./types";

interface ProviderSelectorProps {
	provider: AIProvider;
	onProviderChange: (provider: AIProvider) => void;
	copilotModels: CopilotModel[];
	copilotAuthStatus: CopilotAuthStatus | null;
	copilotRunning: boolean;
	copilotConnecting: boolean;
	onStartCopilot: (
		backend?: AgentBackendProvider,
		serverUrl?: string,
	) => Promise<void>;
	onStopCopilot: () => Promise<void>;
	disabled?: boolean;
	className?: string;
}

export const ProviderSelector = memo(function ProviderSelector({
	provider,
	onProviderChange,
	copilotAuthStatus,
	copilotRunning,
	copilotConnecting,
	onStartCopilot,
	onStopCopilot,
	disabled = false,
	className,
}: ProviderSelectorProps) {
	const isTauriEnv = isTauri();
	const copilotUnavailable = !isTauriEnv;
	const normalizedProvider = normalizeAIProvider(provider);

	const handleProviderChange = useCallback(
		async (newProvider: AIProvider) => {
			const normalized = normalizeAIProvider(newProvider);

			onProviderChange(normalized);

			if (isAgentBackendProvider(normalized)) {
				if (copilotUnavailable) return;
				if (copilotRunning && normalized === normalizedProvider) return;
				try {
					await onStartCopilot(normalized);
				} catch {
					// Error will be handled by the hook
				}
			}
		},
		[
			copilotRunning,
			copilotUnavailable,
			normalizedProvider,
			onStartCopilot,
			onProviderChange,
		],
	);

	const agentProviders: Array<{
		id: AgentBackendProvider;
		label: string;
		icon: typeof GithubIcon;
		tooltip: string;
		disabled?: boolean;
	}> = [
		{
			id: "github-copilot",
			label: "Copilot",
			icon: GithubIcon,
			tooltip: "Use GitHub Copilot SDK (local)",
		},
		{
			id: "codex",
			label: "Codex",
			icon: Code2Icon,
			tooltip: "Use a tool-capable Codex backend adapter",
		},
		{
			id: "claude-code",
			label: "Claude Code",
			icon: SparklesIcon,
			tooltip: "Use the Claude Code CLI through the shared FlowPilot MCP tools",
		},
	];

	return (
		<div
			className={cn(
				"flex min-w-0 flex-wrap items-center gap-1 rounded-xl border border-border/40 bg-background/40 p-1 shadow-inner shadow-black/5",
				className,
			)}
		>
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="sm"
						className={cn(
							"h-8 shrink-0 gap-1.5 rounded-lg px-2.5 text-xs font-medium transition-all",
							normalizedProvider === "bits" &&
								"bg-primary text-primary-foreground shadow-sm hover:bg-primary/90 hover:text-primary-foreground",
							normalizedProvider !== "bits" &&
								"text-muted-foreground hover:bg-accent/70 hover:text-foreground",
						)}
						onClick={() => handleProviderChange("bits")}
						disabled={disabled}
					>
						<LayersIcon className="w-3.5 h-3.5" />
						<span className="hidden sm:inline">Bits</span>
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom" className="text-xs">
					Use configured model bits
				</TooltipContent>
			</Tooltip>

			{agentProviders.map((option) => {
				const Icon = option.icon;
				const active = normalizedProvider === option.id;
				const optionDisabled = Boolean(option.disabled);
				return (
					<Tooltip key={option.id}>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="sm"
								className={cn(
									"h-8 shrink-0 gap-1.5 rounded-lg px-2.5 text-xs font-medium transition-all",
									active &&
										"bg-primary text-primary-foreground shadow-sm hover:bg-primary/90 hover:text-primary-foreground",
									!active &&
										"text-muted-foreground hover:bg-accent/70 hover:text-foreground",
									active && copilotConnecting && "animate-pulse",
									optionDisabled &&
										"cursor-not-allowed opacity-45 hover:bg-transparent hover:text-muted-foreground",
								)}
								onClick={() => handleProviderChange(option.id)}
								aria-disabled={optionDisabled}
								disabled={
									disabled ||
									copilotUnavailable ||
									(active && copilotConnecting)
								}
							>
								<Icon className="w-3.5 h-3.5" />
								<span className="hidden sm:inline">{option.label}</span>
								{copilotRunning && active && (
									<span className="relative flex h-1.5 w-1.5">
										<span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75" />
										<span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-green-500" />
									</span>
								)}
							</Button>
						</TooltipTrigger>
						<TooltipContent side="bottom" className="text-xs max-w-56">
							{copilotRunning && active ? (
								<div>
									<div className="font-medium">{option.label} Connected</div>
									{copilotAuthStatus?.authenticated &&
										copilotAuthStatus.login && (
											<div className="text-muted-foreground mt-0.5">
												Signed in as {copilotAuthStatus.login}
											</div>
										)}
									{copilotAuthStatus?.message && (
										<div className="text-muted-foreground mt-0.5">
											{copilotAuthStatus.message}
										</div>
									)}
								</div>
							) : copilotUnavailable ? (
								"Agent SDK backends are currently desktop-only in FlowPilot"
							) : optionDisabled ? (
								option.tooltip
							) : (
								option.tooltip
							)}
						</TooltipContent>
					</Tooltip>
				);
			})}

			{/* Disconnect button when Copilot is running */}
			{copilotRunning && isAgentBackendProvider(normalizedProvider) && (
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className="h-8 w-8 shrink-0 rounded-lg text-muted-foreground hover:text-destructive"
							onClick={onStopCopilot}
							disabled={disabled || copilotConnecting}
						>
							<ServerIcon className="w-3 h-3" />
						</Button>
					</TooltipTrigger>
					<TooltipContent side="bottom" className="text-xs">
						Disconnect Copilot
					</TooltipContent>
				</Tooltip>
			)}
		</div>
	);
});

interface ModelSelectorProps {
	provider: AIProvider;
	// Bits models
	bitsModels: Array<{
		id: string;
		meta?: { en?: { name?: string } };
		friendly_name?: string;
	}>;
	// Copilot models
	copilotModels: CopilotModel[];
	selectedModelId: string;
	onModelChange: (modelId: string) => void;
	disabled?: boolean;
	className?: string;
}

export const ModelSelector = memo(function ModelSelector({
	provider,
	bitsModels,
	copilotModels,
	selectedModelId,
	onModelChange,
	disabled = false,
	className,
}: ModelSelectorProps) {
	const normalizedProvider = normalizeAIProvider(provider);
	const models = normalizedProvider === "bits" ? bitsModels : copilotModels;

	if (models.length === 0) {
		return (
			<div
				className={cn(
					"flex h-8 min-w-0 items-center rounded-lg border border-border/30 bg-background/60 px-3 text-xs text-muted-foreground",
					className,
				)}
			>
				<span className="truncate">
					{normalizedProvider === "github-copilot"
						? "Loading Copilot models..."
						: normalizedProvider !== "bits"
							? "Loading backend models..."
							: "No models available"}
				</span>
			</div>
		);
	}

	return (
		<Select value={selectedModelId} onValueChange={onModelChange}>
			<SelectTrigger
				className={cn(
					"h-8 min-w-0 overflow-hidden rounded-lg border-border/30 bg-background/60 text-xs backdrop-blur-sm transition-all duration-200 hover:border-primary/30 focus:ring-2 focus:ring-primary/20",
					className,
				)}
				disabled={disabled}
			>
				<SelectValue placeholder="Select Model" />
			</SelectTrigger>
			<SelectContent className="rounded-lg z-150">
				{models.map((model) => {
					const bitsModel = model as ModelSelectorProps["bitsModels"][number];
					const displayName =
						normalizedProvider !== "bits"
							? (model as CopilotModel).name || (model as CopilotModel).id
							: bitsModel.meta?.en?.name ||
								bitsModel.friendly_name ||
								bitsModel.id;

					return (
						<SelectItem
							key={model.id}
							value={model.id}
							className="text-xs rounded-md"
						>
							<span className="block truncate">{displayName}</span>
						</SelectItem>
					);
				})}
			</SelectContent>
		</Select>
	);
});

const PROVIDER_DEFAULT_REASONING = "__provider_default__";

interface ReasoningEffortSelectorProps {
	model?: CopilotModel;
	selectedEffort: string;
	onEffortChange: (effort: string) => void;
	disabled?: boolean;
	className?: string;
}

/**
 * Model-aware reasoning control. Every concrete option comes from the active
 * backend's live model catalog; an empty value deliberately defers to that
 * provider's advertised/configured default.
 */
export const ReasoningEffortSelector = memo(function ReasoningEffortSelector({
	model,
	selectedEffort,
	onEffortChange,
	disabled = false,
	className,
}: ReasoningEffortSelectorProps) {
	const efforts = model?.supportedReasoningEfforts ?? [];
	if (efforts.length === 0) return null;

	const defaultName = model?.defaultReasoningEffort
		? (efforts.find((effort) => effort.id === model.defaultReasoningEffort)
				?.name ?? model.defaultReasoningEffort)
		: undefined;
	const defaultLabel = defaultName
		? `Default (${defaultName})`
		: "Default (recommended)";

	return (
		<Select
			value={selectedEffort || PROVIDER_DEFAULT_REASONING}
			onValueChange={(value) =>
				onEffortChange(value === PROVIDER_DEFAULT_REASONING ? "" : value)
			}
		>
			<SelectTrigger
				aria-label="Reasoning effort"
				className={cn(
					"h-8 min-w-0 overflow-hidden rounded-lg border-border/30 bg-background/60 text-xs backdrop-blur-sm transition-all duration-200 hover:border-primary/30 focus:ring-2 focus:ring-primary/20",
					className,
				)}
				disabled={disabled}
			>
				<BrainCircuitIcon className="mr-1.5 size-3.5 shrink-0 text-muted-foreground" />
				<SelectValue placeholder="Reasoning" />
			</SelectTrigger>
			<SelectContent className="z-150 rounded-lg">
				<SelectItem
					value={PROVIDER_DEFAULT_REASONING}
					className="rounded-md text-xs"
				>
					{defaultLabel}
				</SelectItem>
				{efforts.map((effort) => (
					<SelectItem
						key={effort.id}
						value={effort.id}
						className="rounded-md text-xs"
						title={effort.description}
					>
						{effort.name || effort.id}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
});
