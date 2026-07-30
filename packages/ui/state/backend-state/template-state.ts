import type {
	IBoard,
	IMetadata,
	ITemplatePreview,
	ITemplateSearchHit,
	ITemplateSearchQuery,
	IVersionType,
} from "../../lib";

export interface ITemplateState {
	getTemplates(
		appId?: string,
		language?: string,
		// [appId, templateId, metadata]
	): Promise<[string, string, IMetadata | undefined][]>;
	/**
	 * Store-wide template search across publicly visible apps.
	 * (GET /apps/templates/search)
	 */
	searchTemplates(query: ITemplateSearchQuery): Promise<ITemplateSearchHit[]>;
	/**
	 * A template's structural summary — counts and node types, never its
	 * contents. Readable for any publicly visible app, so a template can be
	 * evaluated before forking or joining.
	 * (GET /apps/{app_id}/templates/{template_id}/preview)
	 */
	getTemplatePreview(
		appId: string,
		templateId: string,
	): Promise<ITemplatePreview>;
	getTemplate(
		appId: string,
		templateId: string,
		version?: [number, number, number],
	): Promise<IBoard>;
	upsertTemplate(
		appId: string,
		boardId: string,
		templateId?: string,
		boardVersion?: [number, number, number],
		versionType?: IVersionType,
	): Promise<[string, [number, number, number]]>;
	deleteTemplate(appId: string, templateId: string): Promise<void>;
	getTemplateMeta(
		appId: string,
		templateId: string,
		language?: string,
	): Promise<IMetadata>;
	pushTemplateMeta(
		appId: string,
		templateId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void>;
}
