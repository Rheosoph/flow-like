import { createId } from "@paralleldrive/cuid2";
import type {
	AppCommentsResponse,
	IApp,
	IAppCategory,
	IAppState,
	IAppVisibility,
	IBoard,
	IMetadata,
	IPurchaseResponse,
	UpsertAppCommentRequest,
	UpsertAppCommentResponse,
} from "@flow-like/flow-like-ui";
import {
	IExecutionStage,
	ILogLevel,
	isAzureBlobStorageUrl,
} from "@flow-like/flow-like-ui";
import type { IAppSearchSort } from "@flow-like/flow-like-ui/lib/schema/app/app-search-query";
import type {
	IBeginOfflineForkBody,
	IBeginOfflineForkResponse,
	IForkPreviewResponse,
	IForkPreviewTarget,
	IOnlineForkBody,
	IOnlineForkResponse,
} from "@flow-like/flow-like-ui/lib/schema/app/fork";
import type { IMediaItem } from "@flow-like/flow-like-ui/state/backend-state/app-state";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPatch,
	apiPost,
	apiPut,
} from "./api-utils";

export class WebAppState implements IAppState {
	constructor(private readonly backend: WebBackendRef) {}

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
		const app = await apiPut<IApp>(
			"apps/new",
			{ meta: metadata, bits },
			this.backend.auth,
		);

		// Create an initial board for the app (matching desktop behavior)
		const boardId = createId();
		await apiPut(
			`apps/${app.id}/board/${boardId}`,
			{
				name: template?.name ?? "Initial Board",
				description:
					template?.description ?? "A blank canvas ready for your ideas",
				log_level: template?.log_level ?? ILogLevel.Debug,
				stage: IExecutionStage.Dev,
				execution_mode: template?.execution_mode,
				template,
			},
			this.backend.auth,
		);

		return app;
	}

	async deleteApp(appId: string): Promise<void> {
		await apiDelete(`apps/${appId}`, this.backend.auth);
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
		const params = new URLSearchParams();
		if (id) params.set("id", id);
		if (query) params.set("query", query);
		if (language) params.set("language", language);
		if (category) params.set("category", category);
		if (author) params.set("author", author);
		if (sort) params.set("sort", sort);
		if (tag) params.set("tag", tag);
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());

		if (params.toString() === "") {
			return this.getApps();
		}

		try {
			return await apiGet<[IApp, IMetadata | undefined][]>(
				`apps/search?${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async getApps(): Promise<[IApp, IMetadata | undefined][]> {
		try {
			return await apiGet<[IApp, IMetadata | undefined][]>(
				"apps",
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async getApp(appId: string): Promise<IApp> {
		return apiGet<IApp>(`apps/${appId}`, this.backend.auth);
	}

	async updateApp(app: IApp): Promise<void> {
		await apiPut(`apps/${app.id}`, { app }, this.backend.auth);
	}

	async getAppMeta(appId: string, language?: string): Promise<IMetadata> {
		const params = language ? `?language=${language}` : "";
		return apiGet<IMetadata>(`apps/${appId}/meta${params}`, this.backend.auth);
	}

	async pushAppMeta(
		appId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void> {
		const params = language ? `?language=${language}` : "?language=en";
		await apiPut(`apps/${appId}/meta${params}`, metadata, this.backend.auth);
	}

	async pushAppMedia(
		appId: string,
		item: IMediaItem,
		file: File,
		language?: string,
	): Promise<void> {
		const extension = file.name.split(".").pop();
		const params = new URLSearchParams();
		params.set("item", item);
		params.set("extension", extension ?? "");
		if (language) params.set("language", language);

		// Step 1: Get signed URL from backend
		const { signed_url } = await apiPut<{ signed_url: string }>(
			`apps/${appId}/meta/media?${params}`,
			undefined,
			this.backend.auth,
		);

		// Step 2: Upload file directly to signed URL
		const headers: HeadersInit = {
			"Content-Type": file.type,
		};

		// Azure Blob Storage requires x-ms-blob-type header
		if (isAzureBlobStorageUrl(signed_url)) {
			headers["x-ms-blob-type"] = "BlockBlob";
		}

		const response = await fetch(signed_url, {
			method: "PUT",
			body: file,
			headers,
		});

		if (!response.ok) {
			throw new Error(`Failed to upload media: ${response.statusText}`);
		}
	}

	async changeAppVisibility(
		appId: string,
		visibility: IAppVisibility,
	): Promise<void> {
		await apiPatch(
			`apps/${appId}/visibility`,
			{ visibility },
			this.backend.auth,
		);
	}

	async changeAppAllowForking(appId: string, allow: boolean): Promise<void> {
		await apiPatch(
			`apps/${appId}/settings/forking`,
			{ allow_forking: allow },
			this.backend.auth,
		);
	}

	async getForkPreview(
		appId: string,
		target: IForkPreviewTarget,
	): Promise<IForkPreviewResponse> {
		return apiGet<IForkPreviewResponse>(
			`apps/${appId}/fork/preview?target=${target}`,
			this.backend.auth,
		);
	}

	async beginOfflineFork(
		appId: string,
		body: IBeginOfflineForkBody,
	): Promise<IBeginOfflineForkResponse> {
		return apiPost<IBeginOfflineForkResponse>(
			`apps/${appId}/fork/offline/begin`,
			body,
			this.backend.auth,
		);
	}

	async onlineFork(
		appId: string,
		body: IOnlineForkBody,
	): Promise<IOnlineForkResponse> {
		return apiPost<IOnlineForkResponse>(
			`apps/${appId}/fork`,
			body,
			this.backend.auth,
		);
	}

	async requestJoinApp(appId: string, comment?: string): Promise<void> {
		if (!this.backend.auth?.isAuthenticated) {
			await this.backend.auth?.signinRedirect();
			return;
		}
		await apiPut(`apps/${appId}/team/queue`, { comment }, this.backend.auth);
	}

	async purchaseApp(appId: string): Promise<IPurchaseResponse> {
		if (!this.backend.auth?.isAuthenticated) {
			await this.backend.auth?.signinRedirect();
			throw new Error("Sign in required to purchase an app.");
		}
		return apiPost<IPurchaseResponse>(
			`apps/${appId}/team/purchase`,
			{},
			this.backend.auth,
		);
	}

	async getAppComments(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<AppCommentsResponse> {
		const params = new URLSearchParams();
		if (offset != null) params.set("offset", String(offset));
		if (limit != null) params.set("limit", String(limit));
		const qs = params.toString();

		try {
			const response = await apiGet<{
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
			}>(`apps/${appId}/comments${qs ? `?${qs}` : ""}`, this.backend.auth);

			return this.normalizeAppCommentsResponse(response);
		} catch {
			return { comments: [], total: 0, offset: 0, limit: 20 };
		}
	}

	async upsertAppComment(
		appId: string,
		body: UpsertAppCommentRequest,
	): Promise<UpsertAppCommentResponse> {
		const response = await apiPut<{ commentId?: string; comment_id?: string }>(
			`apps/${appId}/comments`,
			body,
			this.backend.auth,
		);

		return {
			commentId: response.commentId ?? response.comment_id ?? "",
		};
	}

	async deleteAppComment(appId: string, commentId: string): Promise<void> {
		await apiDelete(`apps/${appId}/comments/${commentId}`, this.backend.auth);
	}
}
