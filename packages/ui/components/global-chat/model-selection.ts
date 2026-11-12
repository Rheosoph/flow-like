import type { GlobalChatAgentSelection } from "../../state/global-chat/global-chat-store";
import {
	flowPilotModelIdForProvider,
	normalizeAIProvider,
} from "../flowpilot/types";

export interface SelectableModel {
	id: string;
}

/**
 * The selection a run's nested assistants use: the profile model its bot turn names, else the
 * run's own.
 */
export function assistantSelection(
	selection: GlobalChatAgentSelection,
	assistantModelId?: string,
): GlobalChatAgentSelection {
	return assistantModelId
		? { ...selection, provider: "bits", selectedModelId: assistantModelId }
		: selection;
}

/** An assistant's error result names the model it ran on, so the bot need not guess which one failed. */
export function withAssistantModel(
	result: unknown,
	selection: GlobalChatAgentSelection,
): unknown {
	if (!result || typeof result !== "object" || Array.isArray(result)) {
		return result;
	}
	const record = result as Record<string, unknown>;
	if (record.status !== "error" || record.model_id) return result;
	const modelId = flowPilotModelIdForProvider(
		normalizeAIProvider(selection.provider),
		selection.selectedModelId,
	);
	return modelId ? { ...record, model_id: modelId } : result;
}

interface ResolveModelSelectionOptions<TModel extends SelectableModel> {
	models: readonly TModel[];
	selectedModelId: string;
	rememberedModelId: string | null;
	/**
	 * Static/offline catalogs may initialize an empty selection, but they must not
	 * replace a non-empty selection. Another mounted surface may already have
	 * loaded the authoritative catalog and selected one of its models.
	 */
	canReplaceInvalidSelection: boolean;
}

/**
 * Returns the model id that should replace the current selection, or `null` when
 * the current surface should leave the shared selection alone.
 */
export function resolveModelSelection<TModel extends SelectableModel>({
	models,
	selectedModelId,
	rememberedModelId,
	canReplaceInvalidSelection,
}: ResolveModelSelectionOptions<TModel>): string | null {
	if (models.length === 0) return null;

	if (
		rememberedModelId &&
		rememberedModelId !== selectedModelId &&
		models.some((model) => model.id === rememberedModelId)
	) {
		return rememberedModelId;
	}

	if (!selectedModelId) return models[0].id;
	if (
		canReplaceInvalidSelection &&
		!models.some((model) => model.id === selectedModelId)
	) {
		return models[0].id;
	}

	return null;
}

/** The selected model's display name, or null while nothing is selected or the catalog lacks it. */
export function selectedModelLabel(
	models: readonly { id: string; name?: string | null }[],
	selectedModelId: string,
): string | null {
	if (!selectedModelId) return null;
	const model = models.find((item) => item.id === selectedModelId);
	return model ? model.name || model.id : null;
}
