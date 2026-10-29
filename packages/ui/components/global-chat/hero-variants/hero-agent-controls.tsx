"use client";

import { useMemo } from "react";
import { ProviderModelReasoningPicker } from "../../flowpilot/provider-model-reasoning-picker";
import {
	agentThinkingOptions,
	chatThinkingLevel,
	chatThinkingOptions,
} from "../../thinking/thinking-options";
import {
	ThinkingPicker,
	useThinkingLabels,
} from "../../thinking/thinking-picker";
import { useAgentSelection } from "./use-agent-selection";

/**
 * The bubble composer's agent controls: pick the FlowPilot provider (Profile /
 * GitHub Copilot / Codex / Claude Code, whichever are available), its model and
 * the thinking level. All write to the shared global-chat store — the choice
 * carries into /chat and is remembered.
 */
export function HeroAgentControls() {
	const {
		provider,
		setProvider,
		availableProviders,
		models,
		selectedModelId,
		setSelectedModelId,
		selectedModel,
		reasoningEffort,
		setReasoningEffort,
		isAgent,
		connecting,
		connected,
		diagnostic,
		retry,
		statusText,
	} = useAgentSelection();
	const thinkingLabels = useThinkingLabels();
	const thinkingOptions = useMemo(
		() =>
			isAgent
				? agentThinkingOptions(selectedModel, thinkingLabels)
				: chatThinkingOptions(thinkingLabels),
		[isAgent, selectedModel, thinkingLabels],
	);

	return (
		<>
			<ProviderModelReasoningPicker
				provider={provider}
				providers={availableProviders}
				models={models.map((model) => ({
					id: model.id,
					label: model.name,
					isFree: model.isFree,
					deviceId: model.deviceId,
				}))}
				selectedModelId={selectedModelId}
				onProviderChange={setProvider}
				onModelChange={setSelectedModelId}
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
			<ThinkingPicker
				value={isAgent ? reasoningEffort : chatThinkingLevel(reasoningEffort)}
				options={thinkingOptions}
				onChange={setReasoningEffort}
				side="bottom"
				contentClassName="z-200"
			/>
		</>
	);
}
