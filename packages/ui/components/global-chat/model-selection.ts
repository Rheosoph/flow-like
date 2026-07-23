export interface SelectableModel {
	id: string;
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
