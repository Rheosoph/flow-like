import {
	type IPage,
	type IPageState,
	type PageListItem,
	normalizePageForPersistence,
} from "@flow-like/flow-like-ui";
import { invoke } from "@tauri-apps/api/core";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

function nativeErrorMessage(error: unknown): string | undefined {
	if (error instanceof Error) return error.message;
	if (typeof error === "string") return error;
	if (
		error &&
		typeof error === "object" &&
		"error" in error &&
		typeof (error as { error?: unknown }).error === "string"
	) {
		return (error as { error: string }).error;
	}
	return undefined;
}

/**
 * Native page lookup uses these exact messages only when every authoritative board was readable
 * and the requested page was absent. Storage/open/load failures carry contextual messages and
 * must not be downgraded to a create-safe "not found".
 */
export function isNativePageNotFoundError(error: unknown): boolean {
	const message = nativeErrorMessage(error)?.trim();
	return (
		message === "Page not found" ||
		message === "Page not found in specified board"
	);
}

export class PageState implements IPageState {
	constructor(private readonly backend: TauriBackend) {}

	private async pushPageToServer(appId: string, page: IPage): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) return;
		const normalizedPage = normalizePageForPersistence(page);

		await fetcher(
			this.backend.profile,
			`apps/${appId}/pages/${page.id}`,
			{
				method: "PUT",
				body: JSON.stringify({ page: normalizedPage }),
			},
			this.backend.auth,
		);
	}

	private async fetchRemotePage(
		appId: string,
		pageId: string,
		boardId?: string,
	): Promise<IPage | null> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) return null;

		const params = boardId ? `?board_id=${encodeURIComponent(boardId)}` : "";
		return await fetcher<IPage>(
			this.backend.profile,
			`apps/${appId}/pages/${pageId}${params}`,
			{ method: "GET" },
			this.backend.auth,
		);
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
								remotePage.boardId,
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
		} catch (localError) {
			if (!isNativePageNotFoundError(localError)) {
				throw localError;
			}
			const remotePage = await this.fetchRemotePage(appId, pageId, boardId);
			if (remotePage) {
				await invoke("update_page", { appId, page: remotePage }).catch(
					() => {},
				);
				return remotePage;
			}
			throw new Error(`Page not found: ${pageId}`);
		}

		let remotePage: IPage | null;
		try {
			remotePage = await this.fetchRemotePage(appId, pageId, boardId);
		} catch {
			// A valid local page remains usable when remote synchronization is temporarily
			// unavailable. By contrast, the local-miss path above propagates the remote error so
			// callers doing overwrite-safety checks can distinguish 404 from transport/auth failure.
			return localPage;
		}
		if (!remotePage) return localPage;

		const remoteUpdated = new Date(remotePage.updatedAt ?? 0).getTime();
		const localUpdated = new Date(localPage.updatedAt ?? 0).getTime();
		const shouldUseRemote =
			Number.isNaN(localUpdated) ||
			Number.isNaN(remoteUpdated) ||
			remoteUpdated >= localUpdated;

		if (shouldUseRemote) {
			const merged = {
				...remotePage,
				boardId: remotePage.boardId || localPage.boardId,
			};
			await invoke("update_page", { appId, page: merged }).catch(() => {});
			return merged;
		}

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
			throw error;
		}

		return page;
	}

	async updatePage(appId: string, page: IPage): Promise<void> {
		const normalizedPage = normalizePageForPersistence(page);
		await invoke("update_page", { appId, page: normalizedPage });

		try {
			await this.pushPageToServer(appId, normalizedPage);
		} catch (error) {
			console.error("Failed to sync page update to server:", error);
			throw error;
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
