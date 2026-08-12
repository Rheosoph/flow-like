import type {
	IBoard,
	IMetadata,
	ITemplatePreview,
	ITemplateSearchHit,
	ITemplateSearchQuery,
	ITemplateState,
	IVersionType,
} from "@flow-like/flow-like-ui";
import { type WebBackendRef, apiDelete, apiGet, apiPut } from "./api-utils";

export class WebTemplateState implements ITemplateState {
	constructor(private readonly backend: WebBackendRef) {}

	async getTemplates(
		appId?: string,
		language?: string,
	): Promise<[string, string, IMetadata | undefined][]> {
		const params = new URLSearchParams();
		if (language) params.set("language", language);

		try {
			const endpoint = appId
				? `apps/${appId}/templates?${params}`
				: `user/templates?${params}`;
			return await apiGet<[string, string, IMetadata | undefined][]>(
				endpoint,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async searchTemplates(
		query: ITemplateSearchQuery,
	): Promise<ITemplateSearchHit[]> {
		const params = new URLSearchParams();
		params.set("query", query.query);
		if (query.language) params.set("language", query.language);
		if (query.category) params.set("category", query.category);
		if (query.tag) params.set("tag", query.tag);
		if (query.forkable_only) params.set("forkable_only", "true");
		if (query.limit !== undefined) params.set("limit", String(query.limit));
		if (query.offset !== undefined) params.set("offset", String(query.offset));

		try {
			return await apiGet<ITemplateSearchHit[]>(
				`apps/templates/search?${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async getTemplatePreview(
		appId: string,
		templateId: string,
	): Promise<ITemplatePreview> {
		return apiGet<ITemplatePreview>(
			`apps/${appId}/templates/${templateId}/preview`,
			this.backend.auth,
		);
	}

	async getTemplate(
		appId: string,
		templateId: string,
		version?: [number, number, number],
	): Promise<IBoard> {
		const params = version ? `?version=${version.join("_")}` : "";
		return apiGet<IBoard>(
			`apps/${appId}/templates/${templateId}${params}`,
			this.backend.auth,
		);
	}

	async upsertTemplate(
		appId: string,
		boardId: string,
		templateId?: string,
		boardVersion?: [number, number, number],
		versionType?: IVersionType,
	): Promise<[string, [number, number, number]]> {
		return apiPut<[string, [number, number, number]]>(
			`apps/${appId}/templates/${templateId ?? "new"}`,
			{
				board_id: boardId,
				board_version: boardVersion,
				version_type: versionType,
			},
			this.backend.auth,
		);
	}

	async deleteTemplate(appId: string, templateId: string): Promise<void> {
		await apiDelete(`apps/${appId}/templates/${templateId}`, this.backend.auth);
	}

	async getTemplateMeta(
		appId: string,
		templateId: string,
		language?: string,
	): Promise<IMetadata> {
		return apiGet<IMetadata>(
			`apps/${appId}/meta?language=${language ?? "en"}&template_id=${templateId}`,
			this.backend.auth,
		);
	}

	async pushTemplateMeta(
		appId: string,
		templateId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/meta?language=${language ?? "en"}&template_id=${templateId}`,
			metadata,
			this.backend.auth,
		);
	}
}
