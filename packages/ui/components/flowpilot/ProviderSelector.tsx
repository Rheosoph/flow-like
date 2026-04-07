"use client";

import { GithubIcon, LayersIcon, ServerIcon } from "lucide-react";
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

import type { AIProvider, CopilotAuthStatus, CopilotModel } from "./types";

interface ProviderSelectorProps {
	provider: AIProvider;
	onProviderChange: (provider: AIProvider) => void;
	copilotModels: CopilotModel[];
	copilotAuthStatus: CopilotAuthStatus | null;
	copilotRunning: boolean;
	copilotConnecting: boolean;
	onStartCopilot: (serverUrl?: string) => Promise<void>;
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

	const handleProviderChange = useCallback(
		async (newProvider: AIProvider) => {
			if (newProvider === "copilot" && !copilotRunning) {
				if (copilotUnavailable) {
					return;
				}

				try {
					await onStartCopilot();
					onProviderChange(newProvider);
				} catch {
					// Error will be handled by the hook
				}
			} else if (newProvider === "bits" && copilotRunning) {
				// Optionally stop Copilot when switching away
				// For now, keep it running in background
				onProviderChange(newProvider);
			} else {
				onProviderChange(newProvider);
			}
		},
		[copilotRunning, copilotUnavailable, onStartCopilot, onProviderChange],
	);

	return (
		<div className={cn("flex items-center gap-1.5", className)}>
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant={provider === "bits" ? "secondary" : "ghost"}
							size="sm"
							className={cn(
								"h-7 px-2 text-xs gap-1.5 rounded-lg transition-all",
								provider === "bits" &&
									"bg-accent border border-primary/20 shadow-sm",
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

				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant={provider === "copilot" ? "secondary" : "ghost"}
							size="sm"
							className={cn(
								"h-7 px-2 text-xs gap-1.5 rounded-lg transition-all",
								provider === "copilot" &&
									"bg-accent border border-primary/20 shadow-sm",
								copilotConnecting && "animate-pulse",
							)}
							onClick={() => handleProviderChange("copilot")}
							disabled={disabled || copilotConnecting || copilotUnavailable}
						>
							<GithubIcon className="w-3.5 h-3.5" />
							<span className="hidden sm:inline">Copilot</span>
							{copilotRunning && provider === "copilot" && (
								<span className="relative flex h-1.5 w-1.5">
									<span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75" />
									<span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-green-500" />
								</span>
							)}
						</Button>
					</TooltipTrigger>
					<TooltipContent side="bottom" className="text-xs max-w-[200px]">
						{copilotRunning ? (
							<div>
								<div className="font-medium">GitHub Copilot Connected</div>
								{copilotAuthStatus?.authenticated &&
									copilotAuthStatus.login && (
										<div className="text-muted-foreground mt-0.5">
											Signed in as {copilotAuthStatus.login}
										</div>
									)}
							</div>
						) : copilotUnavailable ? (
							"GitHub Copilot is currently desktop-only in FlowPilot"
						) : isTauriEnv ? (
							"Use GitHub Copilot (local)"
						) : (
							"GitHub Copilot is unavailable in this environment"
						)}
					</TooltipContent>
				</Tooltip>

				{/* Disconnect button when Copilot is running */}
				{copilotRunning && provider === "copilot" && (
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-6 w-6 text-muted-foreground hover:text-destructive"
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
	const models = provider === "copilot" ? copilotModels : bitsModels;

	if (models.length === 0) {
		return (
			<div
				className={cn(
					"h-8 px-3 text-xs flex items-center text-muted-foreground rounded-lg border border-border/30 bg-background/60",
					className,
				)}
			>
				{provider === "copilot"
					? "Loading Copilot models..."
					: "No models available"}
			</div>
		);
	}

	return (
		<Select value={selectedModelId} onValueChange={onModelChange}>
			<SelectTrigger
				className={cn(
					"h-8 text-xs bg-background/60 backdrop-blur-sm border-border/30 hover:border-primary/30 transition-all duration-200 rounded-lg focus:ring-2 focus:ring-primary/20",
					className,
				)}
				disabled={disabled}
			>
				<SelectValue placeholder="Select Model" />
			</SelectTrigger>
			<SelectContent className="rounded-lg z-150">
				{models.map((model) => {
					const displayName =
						provider === "copilot"
							? (model as CopilotModel).name || (model as CopilotModel).id
							: (model as any).meta?.en?.name ||
								(model as any).friendly_name ||
								model.id;

					return (
						<SelectItem
							key={model.id}
							value={model.id}
							className="text-xs rounded-md"
						>
							{displayName}
						</SelectItem>
					);
				})}
			</SelectContent>
		</Select>
	);
});
