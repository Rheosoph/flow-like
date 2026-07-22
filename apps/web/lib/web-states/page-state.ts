import {
	type IPageState,
	normalizePageForPersistence,
} from "@flow-like/flow-like-ui";
import type {
	IPage,
	PageListItem,
} from "@flow-like/flow-like-ui/state/backend-state/page-state";
import { type WebBackendRef, apiDelete, apiGet, apiPut } from "./api-utils";

export class WebPageState implements IPageState {
	constructor(private readonly backend: WebBackendRef) {}

	async getPages(appId: string, boardId?: string): Promise<PageListItem[]> {
		const params = boardId ? `?board_id=${boardId}` : "";
		try {
			return await apiGet<PageListItem[]>(
				`apps/${appId}/pages${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async getPage(
		appId: string,
		pageId: string,
		boardId?: string,
	): Promise<IPage> {
		const params = boardId ? `?board_id=${boardId}` : "";
		return apiGet<IPage>(
			`apps/${appId}/pages/${pageId}${params}`,
			this.backend.auth,
		);
	}

	async createPage(
		appId: string,
		pageId: string,
		name: string,
		route: string,
		boardId: string,
		title?: string,
	): Promise<IPage> {
		const now = new Date().toISOString();
		return apiPut<IPage>(
			`apps/${appId}/pages/${pageId}`,
			{
				page: {
					id: pageId,
					name,
					route,
					boardId,
					title,
					content: [],
					layoutType: "freeform",
					components: [],
					createdAt: now,
					updatedAt: now,
				},
			},
			this.backend.auth,
		);
	}

	async updatePage(appId: string, page: IPage): Promise<void> {
		const normalizedPage = normalizePageForPersistence(page);
		await apiPut(
			`apps/${appId}/pages/${page.id}`,
			{ page: normalizedPage },
			this.backend.auth,
		);
	}

	async deletePage(
		appId: string,
		pageId: string,
		boardId: string,
	): Promise<void> {
		await apiDelete(
			`apps/${appId}/pages/${pageId}?board_id=${boardId}`,
			this.backend.auth,
		);
	}

	async getOpenPages(): Promise<[string, string, string][]> {
		// In web mode, we don't track open pages locally
		return [];
	}

	async closePage(pageId: string): Promise<void> {
		// No-op in web mode
	}
}
