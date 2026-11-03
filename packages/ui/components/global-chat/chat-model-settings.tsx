"use client";

import { useTranslation } from "@flow-like/locales";
import { ProviderModelReasoningPicker } from "../flowpilot/provider-model-reasoning-picker";
import { useAgentSelection } from "./hero-variants/use-agent-selection";

/** The shared provider choice used when the bot does not pin its own model. */
export function ChatModelSettings() {
	const { t } = useTranslation("chat");
	const selection = useAgentSelection();
	return (
		<div className="flex flex-col gap-2 rounded-xl border border-border p-3">
			<span className="text-sm font-medium">
				{t("defaultChatModel", "Default chat model")}
			</span>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(
					"defaultChatModelHint",
					"Used by every bot, Flux included, that has no model of its own. Changes apply immediately.",
				)}
			</p>
			<ProviderModelReasoningPicker
				provider={selection.provider}
				providers={selection.availableProviders}
				models={selection.models.map((model) => ({
					id: model.id,
					label: model.name,
					isFree: model.isFree,
					deviceId: model.deviceId,
				}))}
				selectedModelId={selection.selectedModelId}
				onProviderChange={selection.setProvider}
				onModelChange={selection.setSelectedModelId}
				connecting={selection.connecting}
				connected={selection.connected}
				diagnostic={selection.diagnostic}
				onRetry={selection.retry}
				statusText={selection.statusText}
				triggerClassName="w-full justify-between"
				contentClassName="z-10000"
			/>
		</div>
	);
}
