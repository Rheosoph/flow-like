import { invoke } from "@tauri-apps/api/core";
import type { IPage, IPageState, PageListItem } from "@tm9657/flow-like-ui";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class PageState implements IPageState {
	constructor(private readonly backend: TauriBackend) {}

	private async pushPageToServer(appId: string, page: IPage): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) return;

		await fetcher(
			this.backend.profile,
			`apps/${appId}/pages/${page.id}`,
			{
				method: "PUT",
				body: JSON.stringify({ page }),
			},
			this.backend.auth,
		);
	}

	private async fetchRemotePage(
		appId: string,
		pageId: string,
	): Promise<IPage | null> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) return null;

		try {
			return await fetcher<IPage>(
				this.backend.profile,
				`apps/${appId}/pages/${pageId}`,
				{ method: "GET" },
				this.backend.auth,
			);
		} catch {
			return null;
		}
	}

	async getPages(appId: string, boardId?: string): Promise<PageListItem[]> {
		const localPages = await invoke<PageListItem[]>("get_pages", {
			appId,
			boardId,
		});

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) {
			return localPages;
		}

		try {
			const url = boardId
				? `apps/${appId}/pages?board_id=${boardId}`
				: `apps/${appId}/pages`;
			const remotePages = await fetcher<PageListItem[]>(
				this.backend.profile,
				url,
				{ method: "GET" },
				this.backend.auth,
			);

			const localMap = new Map(localPages.map((p) => [p.pageId, p]));
			const remoteIds = new Set(remotePages.map((p) => p.pageId));
			const result: PageListItem[] = [];

			for (const rp of remotePages) {
				result.push(localMap.get(rp.pageId) ?? rp);
			}

			for (const lp of localPages) {
				if (!remoteIds.has(lp.pageId)) {
					result.push(lp);
				}
			}

			const syncTask = (async () => {
				for (const remotePage of remotePages) {
					if (!localMap.has(remotePage.pageId)) {
						try {
							const fullPage = await this.fetchRemotePage(
								appId,
								remotePage.pageId,
							);
							if (fullPage) {
								await invoke("update_page", { appId, page: fullPage });
							}
						} catch {
							// Individual page sync failure is non-critical
						}
					}
				}
			})();
			this.backend.backgroundTaskHandler(syncTask);

			return result;
		} catch {
			return localPages;
		}
	}

	async getPage(
		appId: string,
		pageId: string,
		boardId?: string,
	): Promise<IPage> {
		let localPage: IPage | null = null;
		try {
			localPage = await invoke<IPage>("get_page", {
				appId,
				pageId,
				boardId,
			});
		} catch {
			const remotePage = await this.fetchRemotePage(appId, pageId);
			if (remotePage) {
				await invoke("update_page", { appId, page: remotePage }).catch(
					() => {},
				);
				return remotePage;
			}
			throw new Error(`Page not found: ${pageId}`);
		}

		const syncTask = (async () => {
			const remotePage = await this.fetchRemotePage(appId, pageId);
			if (!remotePage) return;

			const remoteUpdated = new Date(remotePage.updatedAt ?? 0).getTime();
			const localUpdated = new Date(localPage!.updatedAt ?? 0).getTime();

			if (remoteUpdated > localUpdated) {
				const merged = {
					...remotePage,
					boardId: remotePage.boardId || localPage!.boardId,
				};
				await invoke("update_page", { appId, page: merged });
			}
		})();
		this.backend.backgroundTaskHandler(syncTask);

		return localPage;
	}

	async createPage(
		appId: string,
		pageId: string,
		name: string,
		route: string,
		boardId: string,
		title?: string,
	): Promise<IPage> {
		const page = await invoke<IPage>("create_page", {
			appId,
			pageId,
			name,
			route,
			boardId,
			title,
		});

		try {
			await this.pushPageToServer(appId, page);
		} catch (error) {
			console.error("Failed to sync page creation to server:", error);
		}

		return page;
	}

	async updatePage(appId: string, page: IPage): Promise<void> {
		await invoke("update_page", { appId, page });

		try {
			await this.pushPageToServer(appId, page);
		} catch (error) {
			console.error("Failed to sync page update to server:", error);
		}
	}

	async deletePage(
		appId: string,
		pageId: string,
		boardId: string,
	): Promise<void> {
		await invoke("delete_page", { appId, pageId, boardId });

		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline && this.backend.profile && this.backend.auth) {
			try {
				await fetcher(
					this.backend.profile,
					`apps/${appId}/pages/${pageId}?board_id=${boardId}`,
					{ method: "DELETE" },
					this.backend.auth,
				);
			} catch (error) {
				console.error("Failed to sync page deletion to server:", error);
			}
		}
	}

	async getOpenPages(): Promise<[string, string, string][]> {
		return invoke<[string, string, string][]>("get_open_pages");
	}

	async closePage(pageId: string): Promise<void> {
		return invoke("close_page", { pageId });
	}
}
