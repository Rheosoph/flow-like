"use client";

import {
	ChevronDownIcon,
	Code2Icon,
	GithubIcon,
	type LucideIcon,
	SparklesIcon,
} from "lucide-react";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuRadioGroup,
	DropdownMenuRadioItem,
	DropdownMenuTrigger,
} from "../../../index";
import type { AIProvider, NormalizedAIProvider } from "../../flowpilot/types";
import { useAgentSelection } from "./use-agent-selection";

const PROVIDER_ICON: Record<NormalizedAIProvider, LucideIcon> = {
	bits: SparklesIcon,
	"github-copilot": GithubIcon,
	codex: Code2Icon,
	"claude-code": SparklesIcon,
};

/**
 * The bubble composer's agent controls: pick the FlowPilot provider (Profile /
 * GitHub Copilot / Codex, whichever are available) and its model. Both write to
 * the shared global-chat store — the choice carries into /chat and is remembered.
 */
export function HeroAgentControls() {
	const {
		provider,
		setProvider,
		availableProviders,
		providerLabel,
		models,
		selectedModelId,
		setSelectedModelId,
		selectedModelName,
		connecting,
	} = useAgentSelection();

	const ProviderIcon = PROVIDER_ICON[provider] ?? SparklesIcon;

	return (
		<>
			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<button
						type="button"
						className="hero-bubble-pill hero-bubble-flowpilot outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
						aria-label={`Provider: ${providerLabel}`}
					>
						<span className="hero-bubble-dot">
							<ProviderIcon className="size-2.5" strokeWidth={2} />
						</span>
						{providerLabel}
						<ChevronDownIcon className="size-3.25" aria-hidden="true" />
					</button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="start" sideOffset={8} className="z-200">
					<DropdownMenuRadioGroup
						value={provider}
						onValueChange={(value) => setProvider(value as AIProvider)}
					>
						{availableProviders.map((option) => {
							const Icon = PROVIDER_ICON[option.id] ?? SparklesIcon;
							return (
								<DropdownMenuRadioItem
									key={option.id}
									value={option.id}
									className="gap-2 text-xs"
								>
									<Icon className="size-3.5" aria-hidden="true" />
									{option.label}
								</DropdownMenuRadioItem>
							);
						})}
					</DropdownMenuRadioGroup>
				</DropdownMenuContent>
			</DropdownMenu>

			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<button
						type="button"
						className="hero-bubble-pill hero-bubble-model outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
						aria-label={`Model: ${selectedModelName}`}
						disabled={models.length === 0 && !connecting}
					>
						<span className="truncate">{selectedModelName}</span>
						<ChevronDownIcon
							className="size-3.25 shrink-0"
							aria-hidden="true"
						/>
					</button>
				</DropdownMenuTrigger>
				<DropdownMenuContent
					align="start"
					sideOffset={8}
					className="z-200 max-h-72 overflow-y-auto"
				>
					{models.length === 0 ? (
						<div className="px-2 py-1.5 text-xs text-muted-foreground">
							{connecting ? "Connecting…" : "No models available"}
						</div>
					) : (
						<DropdownMenuRadioGroup
							value={selectedModelId}
							onValueChange={setSelectedModelId}
						>
							{models.map((model) => (
								<DropdownMenuRadioItem
									key={model.id}
									value={model.id}
									className="text-xs"
								>
									<span className="max-w-52 truncate">{model.name}</span>
								</DropdownMenuRadioItem>
							))}
						</DropdownMenuRadioGroup>
					)}
				</DropdownMenuContent>
			</DropdownMenu>
		</>
	);
}
