"use client";

import { ProviderModelReasoningPicker } from "../../flowpilot/provider-model-reasoning-picker";
import { useAgentSelection } from "./use-agent-selection";

/**
 * The bubble composer's agent controls: pick the FlowPilot provider (Profile /
 * GitHub Copilot / Codex / Claude Code, whichever are available) and its model.
 * Both write to the shared global-chat store — the choice carries into /chat and
 * is remembered.
 */
export function HeroAgentControls() {
	const {
		provider,
		setProvider,
		availableProviders,
		models,
		selectedModelId,
		setSelectedModelId,
		reasoningEffort,
		setReasoningEffort,
		connecting,
		connected,
		diagnostic,
		retry,
		statusText,
	} = useAgentSelection();

	return (
		<ProviderModelReasoningPicker
			provider={provider}
			providers={availableProviders}
			models={models.map((model) => ({
				id: model.id,
				label: model.name,
				isFree: model.isFree,
				supportedReasoningEfforts: model.supportedReasoningEfforts,
				defaultReasoningEffort: model.defaultReasoningEffort,
			}))}
			selectedModelId={selectedModelId}
			selectedEffort={reasoningEffort}
			onProviderChange={setProvider}
			onModelChange={setSelectedModelId}
			onEffortChange={setReasoningEffort}
			connecting={connecting}
			connected={connected}
			diagnostic={diagnostic}
			onRetry={retry}
			statusText={statusText}
			emptyModelLabel="No models available"
			triggerClassName="hero-bubble-pill hero-bubble-flowpilot h-auto max-w-full border-0 bg-transparent px-0 hover:border-transparent hover:bg-transparent"
			contentClassName="z-200"
			sideOffset={8}
		/>
	);
}
