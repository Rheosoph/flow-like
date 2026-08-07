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

/** A fresh native install can know the page's board id before that board exists locally. */
export function isNativePageBoardUnavailableError(error: unknown): boolean {
	const message = nativeErrorMessage(error)?.trim();
	return Boolean(
		message?.startsWith("Failed to open board '") &&
			message.includes(" while looking up page '"),
	);
}

/**
 * The board lists the page, but its payload cannot be read on this device.
 * A board synced from remote carries page ids, never the page files themselves,
 * so every device that never opened the app's flow configuration hits this on
 * its first read. The server holds the authoritative payload in that case.
 */
export function isNativePageContentUnavailableError(error: unknown): boolean {
	const message = nativeErrorMessage(error)?.trim();
	return Boolean(
		message?.startsWith("Failed to load page '") &&
			message.includes(" from board '"),
	);
}

/**
 * Both sides report the page payload's own revision, so equal timestamps mean equal content.
 * A cached page with no revision predates that contract and is refreshed once; a listing entry
 * without one carries no evidence of a change and is left alone.
 */
export function isCachedPageOutdated(
	cached: PageListItem | undefined,
	remote: PageListItem,
): boolean {
	if (!cached) return true;
	// An unreadable local payload is worth replacing whatever the revisions claim.
	if (cached.unavailable) return true;
	if (!remote.updatedAt) return false;
	if (!cached.updatedAt) return true;

	const remoteUpdated = new Date(remote.updatedAt).getTime();
	const cachedUpdated = new Date(cached.updatedAt).getTime();
	if (Number.isNaN(remoteUpdated) || Number.isNaN(cachedUpdated)) return false;

	return remoteUpdated > cachedUpdated;
}

export class PageState implements IPageState {
	constructor(private readonly backend: TauriBackend) {}

	private async getNativePage(
		appId: string,
		pageId: string,
		boardId?: string,
		version?: [number, number, number],
	): Promise<IPage> {
		try {
			return await invoke<IPage>("get_page", {
				appId,
				pageId,
				boardId,
				version,
			});
		} catch (localError) {
			if (!boardId || !isNativePageBoardUnavailableError(localError)) {
				throw localError;
			}

			try {
				// Native get_page opens the board manifest for the requested view. Ensure
				// that exact local storage view exists before retrying the lookup.
				await this.backend.boardState.getBoard(appId, boardId, version, true);
			} catch {
				// Preserve the authoritative native storage failure when repair itself
				// is unavailable (offline, unauthenticated, or a real storage error).
				throw localError;
			}

			return invoke<IPage>("get_page", {
				appId,
				pageId,
				boardId,
				version,
			});
		}
	}

	/**
	 * `update_page` rejects a page without a board id, so a cache write that drops it
	 * would fail silently and force every later read back onto the network.
	 */
	private async cacheRemotePage(
		appId: string,
		remotePage: IPage,
		boardId?: string,
	): Promise<IPage> {
		const page = remotePage.boardId
			? remotePage
			: { ...remotePage, boardId: boardId };
		await invoke("update_page", { appId, page }).catch(() => {});
		return page;
	}

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
		version?: [number, number, number],
	): Promise<IPage | null> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) return null;

		const query = new URLSearchParams();
		if (boardId) query.set("board_id", boardId);
		if (version) query.set("version", version.join("_"));
		const params = query.size > 0 ? `?${query.toString()}` : "";
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
				const local = localMap.get(rp.pageId);
				// A page renamed on another device stays renamed: the server row is
				// authoritative for listing metadata, the local entry only fills in
				// what the listing does not carry. An unreadable local file is not worth
				// flagging while the server can still serve the page — the sync below
				// repairs it.
				result.push(
					local
						? {
								...local,
								...rp,
								boardId: rp.boardId ?? local.boardId,
								unavailable: false,
							}
						: rp,
				);
			}

			for (const lp of localPages) {
				if (!remoteIds.has(lp.pageId)) {
					result.push(lp);
				}
			}

			const outdated = remotePages.filter((remotePage) =>
				isCachedPageOutdated(localMap.get(remotePage.pageId), remotePage),
			);

			const syncTask = (async () => {
				for (const remotePage of outdated) {
					try {
						const fullPage = await this.fetchRemotePage(
							appId,
							remotePage.pageId,
							remotePage.boardId,
						);
						if (fullPage) {
							await invoke("update_page", {
								appId,
								page: fullPage.boardId
									? fullPage
									: { ...fullPage, boardId: remotePage.boardId },
							});
						}
					} catch {
						// Individual page sync failure is non-critical
					}
				}
			})();
			this.backend.backgroundTaskHandler(syncTask);

			return result;
		} catch {
			return localPages;
		}
	}

	/**
	 * A pinned board version resolves against the published snapshot, which is immutable:
	 * whatever answers first is correct, and nothing is written back to the current page
	 * file. With no snapshot reachable — the common offline case — the current page is the
	 * last state this device can honestly show, which beats failing the interface.
	 */
	private async getVersionedPage(
		appId: string,
		pageId: string,
		version: [number, number, number],
		boardId?: string,
	): Promise<IPage> {
		let versionError: unknown;
		try {
			return await this.getNativePage(appId, pageId, boardId, version);
		} catch (error) {
			versionError = error;
		}

		try {
			const remotePage = await this.fetchRemotePage(
				appId,
				pageId,
				boardId,
				version,
			);
			if (remotePage) return remotePage;
		} catch (error) {
			versionError = error;
		}

		const currentPage = await this.getNativePage(appId, pageId, boardId).catch(
			() => null,
		);
		if (currentPage) {
			console.warn(
				`[PageState] Version ${version.join(".")} of page ${pageId} is unavailable; serving the current page instead:`,
				versionError,
			);
			return currentPage;
		}

		throw versionError;
	}

	async getPage(
		appId: string,
		pageId: string,
		boardId?: string,
		version?: [number, number, number],
	): Promise<IPage> {
		if (version) {
			return this.getVersionedPage(appId, pageId, version, boardId);
		}

		let localPage: IPage | null = null;
		try {
			localPage = await this.getNativePage(appId, pageId, boardId);
		} catch (localError) {
			const nativeMiss = isNativePageNotFoundError(localError);
			const contentUnavailable =
				isNativePageContentUnavailableError(localError);
			if (!nativeMiss && !contentUnavailable) {
				throw localError;
			}

			// A page the board knows about but this device cannot read is a normal
			// state on a device that only ever synced the board manifest. Remote is
			// the authority for the payload; the native failure is only preserved
			// when the server cannot answer.
			let remotePage: IPage | null = null;
			try {
				remotePage = await this.fetchRemotePage(appId, pageId, boardId);
			} catch (remoteError) {
				if (nativeMiss) throw remoteError;
				throw localError;
			}

			if (remotePage) {
				return this.cacheRemotePage(appId, remotePage, boardId);
			}
			if (nativeMiss) throw new Error(`Page not found: ${pageId}`);
			throw localError;
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
