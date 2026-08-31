import {
	type IPageBootstrap,
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

	private pageUrl(
		appId: string,
		pageId: string,
		boardId?: string,
		version?: [number, number, number],
	): string {
		const query = new URLSearchParams();
		if (boardId) query.set("board_id", boardId);
		if (version) query.set("version", version.join("_"));
		const params = query.size > 0 ? `?${query.toString()}` : "";
		return `apps/${appId}/pages/${pageId}${params}`;
	}

	async getPage(
		appId: string,
		pageId: string,
		boardId?: string,
		version?: [number, number, number],
	): Promise<IPage> {
		if (!version) {
			return apiGet<IPage>(
				this.pageUrl(appId, pageId, boardId),
				this.backend.auth,
			);
		}

		try {
			return await apiGet<IPage>(
				this.pageUrl(appId, pageId, boardId, version),
				this.backend.auth,
			);
		} catch (error) {
			// Board versions published before pages were snapshotted have no page of
			// their own. The current page is the only thing left to show, and it beats
			// failing an interface that used to render.
			console.warn(
				`[WebPageState] Version ${version.join(".")} of page ${pageId} is unavailable; serving the current page instead:`,
				error,
			);
			return apiGet<IPage>(
				this.pageUrl(appId, pageId, boardId),
				this.backend.auth,
			);
		}
	}

	async getPageBootstrap(
		appId: string,
		route?: string,
		eventId?: string,
	): Promise<IPageBootstrap> {
		const params = new URLSearchParams();
		if (route !== undefined) params.set("route", route);
		if (eventId !== undefined) params.set("eventId", eventId);
		const query = params.size > 0 ? `?${params.toString()}` : "";
		return apiGet<IPageBootstrap>(
			`apps/${appId}/pages/bootstrap${query}`,
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
