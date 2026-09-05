import {
	type AppCommentsResponse,
	type IApp,
	type IAppCategory,
	type IAppState,
	IAppVisibility,
	type IBoard,
	IExecutionStage,
	ILogLevel,
	type IMetadata,
	type IPurchaseResponse,
	type UpsertAppCommentRequest,
	type UpsertAppCommentResponse,
	discardOfflineSyncForApp,
	injectDataFunction,
} from "@flow-like/flow-like-ui";
import type { IGroup } from "@flow-like/flow-like-ui";
import {
	type IForkJobView,
	resolveOnlineFork,
} from "@flow-like/flow-like-ui/lib/fork-job";
import type { IAppSearchSort } from "@flow-like/flow-like-ui/lib/schema/app/app-search-query";
import type {
	IBeginOfflineForkBody,
	IBeginOfflineForkResponse,
	IForkPolicy,
	IForkPreviewResponse,
	IForkPreviewTarget,
	IForkSettings,
	IOnlineForkBody,
	IOnlineForkResponse,
} from "@flow-like/flow-like-ui/lib/schema/app/fork";
import {
	mergeMetadataMedia,
	stabilizeMetadata,
	stabilizeMetadataEntries,
} from "@flow-like/flow-like-ui/lib/stable-asset-url";
import type { IMediaItem } from "@flow-like/flow-like-ui/state/backend-state/app-state";
import { createId } from "@paralleldrive/cuid2";
import { invoke } from "@tauri-apps/api/core";
import { dirname, resolve } from "@tauri-apps/api/path";
import { mkdir, open as openFile } from "@tauri-apps/plugin-fs";
import { fetcher, put } from "../../lib/api";
import type { ApiResponseError } from "../../lib/api-error";
import { isMissingResourceError } from "../../lib/api-error";
import { appsDB } from "../../lib/apps-db";
import type { TauriBackend } from "../tauri-provider";

/**
 * Orders app entries by id so the local listing and the remote-merged listing
 * agree. Without it the two produce the same apps in different orders, which
 * reads as a change to the query cache and reshuffles the library on every sync.
 */
function sortAppEntries(
	entries: [IApp, IMetadata | undefined][],
): [IApp, IMetadata | undefined][] {
	return stabilizeMetadataEntries(entries).sort(([a], [b]) =>
		a.id.localeCompare(b.id),
	);
}

export class AppState implements IAppState {
	constructor(private readonly backend: TauriBackend) {}

	private getRemoteAuth() {
		return this.backend.auth?.isAuthenticated ? this.backend.auth : undefined;
	}

	private hasRemoteAccessToken() {
		return Boolean(
			this.backend.auth?.isAuthenticated &&
				this.backend.auth.user?.access_token,
		);
	}

	private async fetchRemoteApp(appId: string): Promise<IApp> {
		if (!this.backend.profile) {
			throw new Error("Profile not set. Cannot get app.");
		}

		const remoteData = await fetcher<IApp>(
			this.backend.profile,
			`apps/${appId}`,
			undefined,
			this.getRemoteAuth(),
		);

		try {
			await invoke("update_app", {
				app: remoteData,
			});
		} catch (error) {
			console.warn("Failed to cache remote app locally:", error);
		}

		try {
			await appsDB.visibility.put({
				visibility: remoteData.visibility ?? IAppVisibility.Private,
				appId: remoteData.id,
			});
		} catch (error) {
			console.warn("Failed to cache remote app visibility:", error);
		}

		return remoteData;
	}

	private async fetchRemoteAppMeta(
		appId: string,
		language?: string,
	): Promise<IMetadata> {
		if (!this.backend.profile) {
			throw new Error("Profile not set. Cannot get app meta.");
		}

		const remoteMeta = stabilizeMetadata(
			await fetcher<IMetadata>(
				this.backend.profile,
				`apps/${appId}/meta?language=${language ?? "en"}`,
				undefined,
				this.getRemoteAuth(),
			),
		);

		try {
			// This mirrors metadata we just read from the server, not a local
			// edit — keep its real `updated_at` rather than stamping the sync
			// time, or "recent" sort would reorder on every background refresh.
			await invoke("push_app_meta", {
				appId,
				metadata: remoteMeta,
				language,
				preserveUpdatedAt: true,
			});
		} catch (error) {
			console.warn("Failed to cache remote app metadata locally:", error);
		}

		return remoteMeta;
	}

	private normalizeAppCommentsResponse(response: {
		comments: Array<{
			id: string;
			text: string;
			rating: number;
			userId?: string;
			user_id?: string;
			userName?: string | null;
			user_name?: string | null;
			userAvatar?: string | null;
			user_avatar?: string | null;
			createdAt?: string;
			created_at?: string;
			updatedAt?: string;
			updated_at?: string;
		}>;
		total: number;
		offset: number;
		limit: number;
	}): AppCommentsResponse {
		return {
			comments: response.comments.map((comment) => ({
				id: comment.id,
				text: comment.text,
				rating: comment.rating,
				userId: comment.userId ?? comment.user_id ?? "",
				userName: comment.userName ?? comment.user_name ?? undefined,
				userAvatar: comment.userAvatar ?? comment.user_avatar ?? undefined,
				createdAt: comment.createdAt ?? comment.created_at ?? "",
				updatedAt: comment.updatedAt ?? comment.updated_at ?? "",
			})),
			total: response.total,
			offset: response.offset,
			limit: response.limit,
		};
	}

	async createApp(
		metadata: IMetadata,
		bits: string[],
		online: boolean,
		template?: IBoard,
	): Promise<IApp> {
		let appId: string | undefined;
		if (online && !this.backend.profile) {
			throw new Error(
				"Cannot create an online project yet — your profile is still loading. Please try again in a moment.",
			);
		}
		if (online && this.backend.profile) {
			const app: IApp = await put(
				this.backend.profile,
				`apps/new`,
				{
					meta: metadata,
					bits: bits,
				},
				this.backend.auth,
			);

			await appsDB.visibility.put({
				visibility: IAppVisibility.Private,
				appId: app.id,
			});

			appId = app.id;
		}

		const app: IApp = await invoke("create_app", {
			metadata: metadata,
			bits: bits,
			id: appId,
		});

		if (appId) {
			await invoke("update_app", {
				app: { ...app, visibility: IAppVisibility.Private },
			});
		}

		if (!online) {
			await appsDB.visibility.put({
				visibility: IAppVisibility.Offline,
				appId: app.id,
			});
		}

		await this.backend.boardState.upsertBoard(
			app.id,
			createId(),
			template?.name ?? "Initial Board",
			template?.description ?? "A blank canvas ready for your ideas",
			template?.log_level ?? ILogLevel.Debug,
			IExecutionStage.Dev,
			template?.execution_mode,
			template,
		);

		return app;
	}

	/**
	 * True when the server no longer holds this app for this user. Deleting an app
	 * that is already gone answers 404, or 403 once its membership row cascaded away
	 * with it. A plain 403 also covers "member, but not the owner", so that case is
	 * settled by re-reading the app: a copy the user can still read is still there.
	 */
	private async isRemoteAppGone(
		appId: string,
		error: unknown,
	): Promise<boolean> {
		if (isMissingResourceError(error)) return true;
		if ((error as Partial<ApiResponseError>)?.status !== 403) return false;
		if (!this.backend.profile) return false;

		try {
			await fetcher(
				this.backend.profile,
				`apps/${appId}`,
				undefined,
				this.getRemoteAuth(),
			);
			return false;
		} catch (probe) {
			const status = (probe as Partial<ApiResponseError>)?.status;
			return status === 403 || status === 404 || status === 410;
		}
	}

	async deleteApp(appId: string): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) {
			await this.wipeLocalApp(appId, "app-deleted");
			return;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Profile, auth or query client not set. Cannot delete app.",
			);
		}

		try {
			await fetcher(
				this.backend.profile,
				`apps/${appId}`,
				{
					method: "DELETE",
				},
				this.backend.auth,
			);
		} catch (error) {
			// An app deleted elsewhere leaves the device holding the only copy, and the
			// server can never accept a delete for it again. Aborting here would strand
			// that copy forever, so a "there is nothing here" answer still clears it.
			if (!(await this.isRemoteAppGone(appId, error))) throw error;
			console.warn(
				`App ${appId} no longer exists on the server, removing the local copy only.`,
			);
		}

		await this.wipeLocalApp(appId, "app-deleted");
	}

	/**
	 * Drop this device's copy of an app it no longer has access to.
	 *
	 * `delete_app` clears the manifest, the project store and the profile
	 * entries, but not the sync outbox — and age-based cleanup never reclaims a
	 * row that still holds commands, so queued board edits for a project that
	 * is gone would stay queued forever.
	 */
	private async wipeLocalApp(appId: string, reason: string): Promise<void> {
		await invoke("delete_app", {
			appId: appId,
		});
		try {
			await discardOfflineSyncForApp(appId, reason);
		} catch (error) {
			console.warn(
				`Failed to clear queued offline edits for app ${appId}:`,
				error,
			);
		}
	}

	async leaveApp(appId: string): Promise<void> {
		if (await this.backend.isLocalOnly(appId)) {
			throw new Error(
				`App ${appId} is local-only. There is no team to leave — delete it instead.`,
			);
		}

		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth not set. Cannot leave app.");
		}

		const sub = this.backend.auth.user?.profile.sub;
		if (!sub) {
			throw new Error("No signed-in user. Cannot leave app.");
		}

		try {
			await fetcher(
				this.backend.profile,
				`apps/${appId}/team/${sub}`,
				{
					method: "DELETE",
				},
				this.backend.auth,
			);
		} catch (error) {
			// A membership that is already gone — revoked elsewhere, or the app
			// itself deleted — leaves the device holding a copy it can no longer
			// sync. Refusing to clean up would strand it, so "there is nothing
			// here" still counts as having left.
			if (!isMissingResourceError(error)) throw error;
			console.warn(
				`Membership for app ${appId} no longer exists, removing the local copy only.`,
			);
		}

		// The same local wipe a delete performs: a copy kept here could never
		// sync again, and the hub now answers 403 to everything it holds.
		await this.wipeLocalApp(appId, "app-left");
	}

	async searchApps(
		id?: string,
		query?: string,
		language?: string,
		category?: IAppCategory,
		author?: string,
		sort?: IAppSearchSort,
		tag?: string,
		offset?: number,
		limit?: number,
	): Promise<[IApp, IMetadata | undefined][]> {
		if (!this.backend.profile) {
			return [];
		}

		const queryParams: Record<string, string> = {};

		if (id) queryParams["id"] = id;
		if (query) queryParams["query"] = query;
		if (language) queryParams["language"] = language;
		if (category) queryParams["category"] = category;
		if (author) queryParams["author"] = author;
		if (sort) queryParams["sort"] = sort;
		if (tag) queryParams["tag"] = tag;
		if (offset) queryParams["offset"] = offset.toString();
		if (limit) queryParams["limit"] = limit.toString();

		const length = Array.from(Object.values(queryParams)).length;
		if (length === 0) {
			return this.getApps();
		}

		try {
			return stabilizeMetadataEntries(
				await fetcher<[IApp, IMetadata | undefined][]>(
					this.backend.profile,
					`apps/search?${new URLSearchParams(queryParams)}`,
					undefined,
					this.backend.auth,
				),
			);
		} catch (error) {
			console.error("Failed to search apps:", error);
			return [];
		}
	}

	async getStoreGroups(offset?: number, limit?: number): Promise<IGroup[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		const params = new URLSearchParams();
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());
		return await fetcher(
			this.backend.profile,
			`store/groups?${params}`,
			undefined,
			this.backend.auth,
		);
	}

	async getStoreGroup(groupId: string): Promise<IGroup> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		return await fetcher(
			this.backend.profile,
			`store/groups/${groupId}`,
			undefined,
			this.backend.auth,
		);
	}

	async getMyGroups(): Promise<IGroup[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		return await fetcher(
			this.backend.profile,
			"user/groups",
			undefined,
			this.backend.auth,
		);
	}

	async getApps(): Promise<[IApp, IMetadata | undefined][]> {
		const localApps = sortAppEntries(
			await invoke<[IApp, IMetadata | undefined][]>("get_apps"),
		);

		if (
			!this?.backend?.queryClient ||
			!this.backend.profile ||
			!this.backend.auth ||
			!this.hasRemoteAccessToken()
		) {
			return localApps;
		}

		const syncRemote = async () => {
			const mergedData = new Map<string, [IApp, IMetadata | undefined]>();

			const remoteData = await fetcher<[IApp, IMetadata | undefined][]>(
				this.backend.profile!,
				"apps",
				undefined,
				this.backend.auth,
			);

			for (const [app, meta] of remoteData) {
				appsDB.visibility
					.put({
						visibility: app.visibility ?? IAppVisibility.Private,
						appId: app.id,
					})
					.catch(() => {});

				const exists = localApps.find(([localApp]) => localApp.id === app.id);
				if (exists) {
					// Keep the local metadata record: it is the copy the rest of the
					// app treats as authoritative, and adopting the remote names and
					// timestamps here reorders the library on every sync.
					//
					// Media cannot come from it. `push_app_meta` pins the media fields
					// to whatever the local record already held — it has to, since
					// those fields name files on disk that only `push_app_media`
					// writes — so the copy below never adopts this app's artwork. A
					// cloud-hosted app therefore reads back with no artwork at all, or
					// with a signature frozen on the day it was first cached. Real
					// local artwork presigns to an unsigned asset:// URL and is kept,
					// so offline apps still skip re-downloading what they already have.
					mergedData.set(app.id, [
						app,
						exists[1] ? mergeMetadataMedia(exists[1], meta) : meta,
					]);
					invoke("update_app", { app }).catch(() => {});
					if (meta)
						invoke("push_app_meta", {
							appId: app.id,
							metadata: meta,
							preserveUpdatedAt: true,
						}).catch(() => {});
					continue;
				}

				mergedData.set(app.id, [app, meta]);

				if (meta) {
					await invoke("create_app", {
						metadata: meta,
						bits: app.bits,
						template: "",
						id: app.id,
					});
					// create_app stamps a brand-new manifest with the current time,
					// which would make every app pulled down for the first time look
					// freshly updated on the next local-first paint. Write the remote
					// record over it so the stored timestamps match the server's.
					await invoke("update_app", { app }).catch(() => {});
				}
			}

			localApps.forEach(([app, meta]) => {
				if (!mergedData.has(app.id)) {
					mergedData.set(app.id, [app, meta]);
				}
			});

			return sortAppEntries(Array.from(mergedData.values()));
		};

		if (localApps.length === 0) {
			try {
				const remoteData = await syncRemote();
				const queryKey = [this.getApps.name || "backendFn"];
				this.backend.queryClient.setQueryData(queryKey, remoteData);
				return remoteData;
			} catch {
				return localApps;
			}
		}

		const promise = injectDataFunction(
			syncRemote,
			this,
			this.backend.queryClient,
			this.getApps,
			[],
			[],
			localApps,
		);

		this.backend.backgroundTaskHandler(promise);
		return localApps;
	}

	async getApp(appId: string): Promise<IApp> {
		let localApp: IApp | undefined;

		try {
			localApp = await invoke("get_app", {
				appId,
			});
		} catch (error) {
			console.warn("Failed to get app from local cache:", error);
		}

		if (localApp) {
			const isOffline =
				localApp.visibility === IAppVisibility.Offline ||
				(await this.backend.isOffline(appId));
			if (isOffline) {
				return localApp;
			}

			if (!this.backend.queryClient || !this.backend.profile) {
				return localApp;
			}

			const promise = injectDataFunction(
				() => this.fetchRemoteApp(appId),
				this,
				this.backend.queryClient,
				this.getApp,
				[appId],
				[],
				localApp,
			);
			this.backend.backgroundTaskHandler(promise);

			return localApp;
		}

		return this.fetchRemoteApp(appId);
	}
	async updateApp(app: IApp): Promise<void> {
		const isOffline = await this.backend.isOffline(app.id);

		if (isOffline) {
			await invoke("update_app", {
				app: app,
			});
			return;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Profile, auth or query client not set. Cannot update app.",
			);
		}

		await fetcher(
			this.backend.profile,
			`apps/${app.id}`,
			{
				method: "PUT",
				body: JSON.stringify({
					app: app,
				}),
			},
			this.backend.auth,
		);
	}

	async getAppMeta(appId: string, language?: string): Promise<IMetadata> {
		const isOffline = await this.backend.isOffline(appId);
		let meta: IMetadata | undefined = undefined;

		try {
			meta = stabilizeMetadata(
				await invoke<IMetadata>("get_app_meta", {
					appId: appId,
					language,
				}),
			);
			if (isOffline) {
				return meta;
			}
		} catch (e) {
			console.warn("Failed to get app meta from local cache:", e);
		}

		if (!this.backend.profile || !this.backend.queryClient) {
			if (meta) {
				return meta;
			}
			return this.fetchRemoteAppMeta(appId, language);
		}

		if (meta) {
			const promise = injectDataFunction(
				() => this.fetchRemoteAppMeta(appId, language),
				this,
				this.backend.queryClient,
				this.getAppMeta,
				[appId, language],
				[],
				meta,
			);
			this.backend.backgroundTaskHandler(promise);

			return meta;
		}

		try {
			return await this.fetchRemoteAppMeta(appId, language);
		} catch (error) {
			console.error("Failed to fetch app meta from remote:", error);
			if (meta) {
				return meta;
			}
			throw new Error(
				"Failed to fetch app meta: no local cache available and remote fetch failed.",
			);
		}
	}

	async pushAppMeta(
		appId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (isOffline) {
			await invoke("push_app_meta", {
				appId: appId,
				metadata: metadata,
				language,
			});
			return;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Profile, auth or query client not set. Cannot push app meta.",
			);
		}
		await fetcher(
			this.backend.profile,
			`apps/${appId}/meta?language=${language ?? "en"}`,
			{
				method: "PUT",
				body: JSON.stringify(metadata),
			},
			this.backend.auth,
		);
		await invoke("push_app_meta", {
			appId: appId,
			metadata: metadata,
			language,
		});
	}

	async pushAppMedia(
		appId: string,
		item: IMediaItem,
		file: File,
		language?: string,
	): Promise<void> {
		const yieldControl = () => new Promise((resolve) => setTimeout(resolve, 0));

		const isOffline = await this.backend.isOffline(appId);

		if (isOffline) {
			const uploadUrl = await invoke<string>("push_app_media", {
				appId: appId,
				query: {
					language: language ?? "en",
					item: item,
					extension: file.name.split(".").pop(),
				},
			});
			let fileName = uploadUrl.split("/").pop()?.split("?")[0] ?? file.name;

			if (
				uploadUrl.startsWith("asset://") ||
				uploadUrl.startsWith("http://asset.localhost/")
			) {
				const path = decodeURIComponent(
					uploadUrl
						.replace("http://asset.localhost/", "")
						.replaceAll("asset://localhost/", ""),
				);
				fileName = path.split("/").pop() ?? file.name;

				const parentDir = await dirname(path);
				await mkdir(parentDir, { recursive: true });
				const fileHandle = await openFile(await resolve(path), {
					append: false,
					create: true,
					write: true,
					truncate: true,
				});

				if (!fileHandle) {
					throw new Error(`Failed to open file handle for ${path}`);
				}

				const chunkSize = 8 * 1024 * 1024;
				if (file.size < chunkSize) {
					const bytes = new Uint8Array(await file.arrayBuffer());
					await fileHandle.write(bytes);
					await fileHandle.close();
					await invoke("transform_media", {
						appId: appId,
						mediaItem: fileName,
					});
					return;
				}

				const stream = file.stream();
				const reader = stream.getReader();
				let bytesWritten = 0;
				let chunkCount = 0;

				try {
					while (true) {
						const { done, value } = await reader.read();

						if (done) {
							break;
						}

						await fileHandle.write(value);
						bytesWritten += value.length;
						chunkCount++;

						// Update progress and yield control every few chunks
						if (chunkCount % 5 === 0) {
							await yieldControl();
						}
					}
				} finally {
					reader.releaseLock();
					await fileHandle.close();
				}
				await invoke("transform_media", {
					appId: appId,
					mediaItem: fileName,
				});
			} else {
				try {
					await this.backend.uploadSignedUrl(uploadUrl, file, 0, 1, () => {});
				} catch (error) {
					console.error("Failed to upload file");
					throw error;
				}
			}

			return;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Profile, auth or query client not set. Cannot push app meta.",
			);
		}
		const { signed_url }: { signed_url: string } = await fetcher(
			this.backend.profile,
			`apps/${appId}/meta/media?language=${language ?? "en"}&item=${item}&extension=${file.name.split(".").pop()}`,
			{
				method: "PUT",
			},
			this.backend.auth,
		);

		await fetch(signed_url, {
			method: "PUT",
			body: file,
			headers: {
				"Content-Type": file.type,
			},
		});
	}

	async changeAppVisibility(
		appId: string,
		visibility: IAppVisibility,
	): Promise<void> {
		if (this.backend.profile && this.backend.auth && this.backend.queryClient) {
			await fetcher<IApp>(
				this.backend.profile,
				`apps/${appId}/visibility`,
				{
					method: "PATCH",
					body: JSON.stringify({
						visibility: visibility,
					}),
				},
				this.backend.auth,
			);
		}
	}

	async recordLocalAppVisibility(
		appId: string,
		visibility: IAppVisibility,
	): Promise<void> {
		await appsDB.visibility.put({ visibility, appId });
	}

	async changeAppAllowForking(appId: string, allow: boolean): Promise<void> {
		if (await this.backend.isOffline(appId)) {
			throw new Error("Forking settings are only available for online apps.");
		}

		if (this.backend.profile && this.backend.auth && this.backend.queryClient) {
			await fetcher<IApp>(
				this.backend.profile,
				`apps/${appId}/settings/forking`,
				{
					method: "PATCH",
					body: JSON.stringify({
						allow_forking: allow,
					}),
				},
				this.backend.auth,
			);
		}
	}

	async getForkSettings(appId: string): Promise<IForkSettings> {
		if (await this.backend.isOffline(appId)) {
			throw new Error("Forking settings are only available for online apps.");
		}

		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth not set. Cannot read fork settings.");
		}

		return fetcher<IForkSettings>(
			this.backend.profile,
			`apps/${appId}/settings/forking`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async changeAppForkPolicy(appId: string, policy: IForkPolicy): Promise<void> {
		if (await this.backend.isOffline(appId)) {
			throw new Error("Forking settings are only available for online apps.");
		}

		if (this.backend.profile && this.backend.auth && this.backend.queryClient) {
			await fetcher<IApp>(
				this.backend.profile,
				`apps/${appId}/settings/forking`,
				{
					method: "PATCH",
					body: JSON.stringify({
						fork_policy: policy,
					}),
				},
				this.backend.auth,
			);
		}
	}

	async getForkPreview(
		appId: string,
		target: IForkPreviewTarget,
	): Promise<IForkPreviewResponse> {
		if (!this.backend.profile) {
			throw new Error("Profile not set. Cannot preview fork.");
		}
		return fetcher<IForkPreviewResponse>(
			this.backend.profile,
			`apps/${appId}/fork/preview?target=${target}`,
			{ method: "GET" },
			this.getRemoteAuth(),
		);
	}

	async beginOfflineFork(
		appId: string,
		body: IBeginOfflineForkBody,
	): Promise<IBeginOfflineForkResponse> {
		if (!this.backend.profile) {
			throw new Error("Profile not set. Cannot create offline fork.");
		}
		return fetcher<IBeginOfflineForkResponse>(
			this.backend.profile,
			`apps/${appId}/fork/offline/begin`,
			{
				method: "POST",
				body: JSON.stringify(body),
			},
			this.getRemoteAuth(),
		);
	}

	async onlineFork(
		appId: string,
		body: IOnlineForkBody,
	): Promise<IOnlineForkResponse> {
		const profile = this.backend.profile;
		const auth = this.backend.auth;
		if (!profile || !auth) {
			throw new Error("not authenticated");
		}
		const response = await fetcher<IOnlineForkResponse | IForkJobView>(
			profile,
			`apps/${appId}/fork`,
			{
				method: "POST",
				body: JSON.stringify(body),
			},
			auth,
		);
		return resolveOnlineFork(response, (jobId) =>
			fetcher<IForkJobView>(
				profile,
				`apps/fork/jobs/${jobId}`,
				{ method: "GET" },
				auth,
			),
		);
	}

	async requestJoinApp(appId: string, comment?: string): Promise<void> {
		const auth = this.backend.auth;

		if (!auth?.isAuthenticated) {
			await auth?.signinRedirect();
			return;
		}

		if (this.backend.profile && this.backend.queryClient) {
			await fetcher<IApp>(
				this.backend.profile,
				`apps/${appId}/team/queue`,
				{
					method: "PUT",
					body: JSON.stringify({
						comment: comment,
					}),
				},
				auth,
			);
			return;
		}

		throw new Error("Profile or auth context not available");
	}

	async purchaseApp(appId: string): Promise<IPurchaseResponse> {
		if (this.backend.profile && this.backend.auth && this.backend.queryClient) {
			return fetcher<IPurchaseResponse>(
				this.backend.profile,
				`apps/${appId}/team/purchase`,
				{
					method: "POST",
					body: JSON.stringify({}),
				},
				this.backend.auth,
			);
		}

		if (this.backend.auth) {
			await this.backend.auth.signinRedirect();
		}
		throw new Error("You must be logged in to purchase an app.");
	}

	async getAppComments(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<AppCommentsResponse> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile) {
			return { comments: [], total: 0, offset: 0, limit: 20 };
		}

		const params = new URLSearchParams();
		if (offset != null) params.set("offset", String(offset));
		if (limit != null) params.set("limit", String(limit));
		const qs = params.toString();

		const response = await fetcher<{
			comments: Array<{
				id: string;
				text: string;
				rating: number;
				userId?: string;
				user_id?: string;
				userName?: string | null;
				user_name?: string | null;
				userAvatar?: string | null;
				user_avatar?: string | null;
				createdAt?: string;
				created_at?: string;
				updatedAt?: string;
				updated_at?: string;
			}>;
			total: number;
			offset: number;
			limit: number;
		}>(
			this.backend.profile,
			`apps/${appId}/comments${qs ? `?${qs}` : ""}`,
			undefined,
			this.backend.auth,
		);

		return this.normalizeAppCommentsResponse(response);
	}

	async upsertAppComment(
		appId: string,
		body: UpsertAppCommentRequest,
	): Promise<UpsertAppCommentResponse> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) {
			throw new Error("Reviews are only available for online apps.");
		}

		const response = await fetcher<{ commentId?: string; comment_id?: string }>(
			this.backend.profile,
			`apps/${appId}/comments`,
			{
				method: "PUT",
				body: JSON.stringify(body),
			},
			this.backend.auth,
		);

		return {
			commentId: response.commentId ?? response.comment_id ?? "",
		};
	}

	async deleteAppComment(appId: string, commentId: string): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) {
			throw new Error("Reviews are only available for online apps.");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/comments/${commentId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}

	async listPackages(appId: string): Promise<Record<string, string>> {
		return invoke("app_list_packages", { appId });
	}

	async addPackage(
		appId: string,
		packageId: string,
		version: string,
	): Promise<void> {
		return invoke("app_add_package", { appId, packageId, version });
	}

	async removePackage(appId: string, packageId: string): Promise<void> {
		return invoke("app_remove_package", { appId, packageId });
	}
}
