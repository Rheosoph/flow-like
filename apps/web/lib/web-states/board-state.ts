import {
	type IBoard,
	type IBoardState,
	ICommentType,
	IConnectionMode,
	type IExecutionMode,
	type IExecutionStage,
	type IGenericCommand,
	type IHub,
	type IIntercomEvent,
	type IJwks,
	type ILog,
	type ILogLevel,
	type ILogMetadata,
	type INode,
	type IOAuthProvider,
	type IRealtimeAccess,
	type IRunContext,
	type IRunPayload,
	type IVersionType,
	type ProgressToastData,
	checkOAuthTokens,
	finishAllProgressToasts,
	showProgressToast,
} from "@tm9657/flow-like-ui";
import type { SurfaceComponent } from "@tm9657/flow-like-ui/components/a2ui/types";
import type {
	CopilotScope,
	UIActionContext,
	UnifiedChatMessage,
	UnifiedCopilotResponse,
} from "@tm9657/flow-like-ui/lib/schema/copilot";
import type { IPrerunBoardResponse } from "@tm9657/flow-like-ui/state/backend-state/types";
import { toast } from "sonner";
import { oauthConsentStore, oauthTokenStore } from "../oauth-db";
import { getOAuthApiBaseUrl, getOAuthService } from "../oauth-service";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPatch,
	apiPost,
	apiPut,
	getApiBaseUrl,
} from "./api-utils";

// Hub configuration cache
let hubCache: IHub | undefined;
let hubCachePromise: Promise<IHub | undefined> | undefined;

async function getHubConfig(profile?: { hub?: string }): Promise<
	IHub | undefined
> {
	if (hubCache) return hubCache;
	if (hubCachePromise) return hubCachePromise;

	const hubUrl = profile?.hub;
	if (!hubUrl) return undefined;

	const url =
		hubUrl.startsWith("http://") || hubUrl.startsWith("https://")
			? `${hubUrl}/api/v1`
			: `https://${hubUrl}/api/v1`;

	hubCachePromise = fetch(url)
		.then((res) => res.json() as Promise<IHub>)
		.then((hub) => {
			hubCache = hub;
			return hub;
		})
		.catch((e) => {
			console.warn("[OAuth] Failed to fetch Hub config:", e);
			return undefined;
		});

	return hubCachePromise;
}

interface ToastEventPayload {
	message: string;
	level: "success" | "error" | "info" | "warning";
}

function handleToastEvent(event: IIntercomEvent): void {
	const payload = event.payload as ToastEventPayload;
	if (!payload?.message) return;

	switch (payload.level) {
		case "success":
			toast.success(payload.message);
			break;
		case "error":
			toast.error(payload.message);
			break;
		case "warning":
			toast.warning(payload.message);
			break;
		default:
			toast.info(payload.message);
	}
}

function handleProgressEvent(event: IIntercomEvent): void {
	const payload = event.payload as ProgressToastData;
	if (!payload?.id) return;
	showProgressToast(payload);
}

export class WebBoardState implements IBoardState {
	constructor(private readonly backend: WebBackendRef) {}

	async getBoards(appId: string): Promise<IBoard[]> {
		try {
			return await apiGet<IBoard[]>(`apps/${appId}/board`, this.backend.auth);
		} catch {
			return [];
		}
	}

	async getCatalog(appId: string): Promise<INode[]> {
		try {
			return await apiGet<INode[]>(
				`apps/${appId}/nodes`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async getBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IBoard> {
		const params = version ? `?version=${version.join(".")}` : "";
		const board = await apiGet<IBoard>(
			`apps/${appId}/board/${boardId}${params}`,
			this.backend.auth,
		);

		// Presign media comments (Image/Video)
		await this.presignMediaComments(appId, boardId, board);

		return board;
	}

	private async presignMediaComments(
		appId: string,
		boardId: string,
		board: IBoard,
	): Promise<void> {
		const mediaComments = Object.values(board.comments).filter(
			(comment) =>
				comment.comment_type === ICommentType.Image ||
				comment.comment_type === ICommentType.Video,
		);

		if (mediaComments.length === 0) return;

		// Build full storage paths for media files (apps/{appId}/upload/boards/{boardId}/{filename})
		const buildFullPath = (filename: string) =>
			`apps/${appId}/upload/boards/${boardId}/${filename}`;

		const prefixes = mediaComments.map((comment) =>
			buildFullPath(comment.content),
		);

		try {
			const results = await apiPost<
				{ prefix: string; url?: string; error?: string }[]
			>(`apps/${appId}/data/download`, { prefixes }, this.backend.auth);

			// Map presigned URLs back to comments
			const urlMap = new Map(
				results.filter((r) => r.url).map((r) => [r.prefix, r.url as string]),
			);

			for (const comment of mediaComments) {
				const prefix = buildFullPath(comment.content);
				const url = urlMap.get(prefix);
				if (url) {
					(comment as any).presigned_url = url;
				}
			}

			// Also presign layer comments
			for (const layer of Object.values(board.layers)) {
				const layerMediaComments = Object.values(layer.comments).filter(
					(comment) =>
						comment.comment_type === ICommentType.Image ||
						comment.comment_type === ICommentType.Video,
				);

				if (layerMediaComments.length === 0) continue;

				const layerPrefixes = layerMediaComments.map((comment) =>
					buildFullPath(comment.content),
				);

				const layerResults = await apiPost<
					{ prefix: string; url?: string; error?: string }[]
				>(
					`apps/${appId}/data/download`,
					{ prefixes: layerPrefixes },
					this.backend.auth,
				);

				const layerUrlMap = new Map(
					layerResults
						.filter((r) => r.url)
						.map((r) => [r.prefix, r.url as string]),
				);

				for (const comment of layerMediaComments) {
					const prefix = buildFullPath(comment.content);
					const url = layerUrlMap.get(prefix);
					if (url) {
						(comment as any).presigned_url = url;
					}
				}
			}
		} catch (error) {
			console.warn("Failed to presign media comments:", error);
		}
	}

	async getRealtimeAccess(
		appId: string,
		boardId: string,
	): Promise<IRealtimeAccess> {
		return apiPost<IRealtimeAccess>(
			`apps/${appId}/board/${boardId}/realtime`,
			undefined,
			this.backend.auth,
		);
	}

	async getRealtimeJwks(appId: string, boardId: string): Promise<IJwks> {
		return apiGet<IJwks>(
			`apps/${appId}/board/${boardId}/realtime`,
			this.backend.auth,
		);
	}

	async createBoardVersion(
		appId: string,
		boardId: string,
		versionType: IVersionType,
	): Promise<[number, number, number]> {
		return apiPatch<[number, number, number]>(
			`apps/${appId}/board/${boardId}?version_type=${versionType}`,
			undefined,
			this.backend.auth,
		);
	}

	async getBoardVersions(
		appId: string,
		boardId: string,
	): Promise<[number, number, number][]> {
		try {
			return await apiGet<[number, number, number][]>(
				`apps/${appId}/board/${boardId}/version`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async deleteBoard(appId: string, boardId: string): Promise<void> {
		await apiDelete(`apps/${appId}/board/${boardId}`, this.backend.auth);
	}

	async getOpenBoards(): Promise<[string, string, string][]> {
		// In web mode, we don't track open boards locally
		return [];
	}

	async getBoardSettings(): Promise<IConnectionMode> {
		const profile = this.backend.profile;
		return profile?.settings?.connection_mode ?? IConnectionMode.Default;
	}

	async executeBoard(
		appId: string,
		boardId: string,
		payload: IRunPayload,
		streamState?: boolean,
		eventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
	): Promise<ILogMetadata | undefined> {
		// Check OAuth tokens before execution
		const board = await this.getBoard(appId, boardId);
		const hub = await getHubConfig(this.backend.profile);
		const oauthService = getOAuthService(
			getOAuthApiBaseUrl(this.backend.profile?.hub),
		);
		const oauthResult = await checkOAuthTokens(board, oauthTokenStore, hub, {
			refreshToken: oauthService.refreshToken.bind(oauthService),
		});

		console.log("[OAuth] Board check result:", {
			requiredProviders: oauthResult.requiredProviders.map((p) => p.id),
			missingProviders: oauthResult.missingProviders.map((p) => p.id),
			hasTokens: Object.keys(oauthResult.tokens),
			skipConsentCheck,
		});

		if (!skipConsentCheck) {
			const consentedIds =
				await oauthConsentStore.getConsentedProviderIds(appId);
			const providersNeedingConsent: IOAuthProvider[] = [];

			// Add providers that are missing tokens
			providersNeedingConsent.push(...oauthResult.missingProviders);

			// Also add providers that have tokens but no consent for this specific app
			for (const provider of oauthResult.requiredProviders) {
				const hasToken = oauthResult.tokens[provider.id] !== undefined;
				const hasConsent = consentedIds.has(provider.id);

				if (hasToken && !hasConsent) {
					console.log(
						`[OAuth] Provider ${provider.id} has token but no consent for app ${appId}`,
					);
					providersNeedingConsent.push(provider);
				}
			}

			if (providersNeedingConsent.length > 0) {
				const error = new Error(
					`Missing OAuth authorization for: ${providersNeedingConsent.map((p) => p.name).join(", ")}`,
				);
				(error as any).missingProviders = providersNeedingConsent;
				(error as any).isOAuthError = true;
				throw error;
			}
		} else {
			// Still need to check for missing tokens even if skipping consent
			if (oauthResult.missingProviders.length > 0) {
				const error = new Error(
					`Missing OAuth tokens for: ${oauthResult.missingProviders.map((p) => p.name).join(", ")}`,
				);
				(error as any).missingProviders = oauthResult.missingProviders;
				(error as any).isOAuthError = true;
				throw error;
			}
		}

		// Collect OAuth tokens to pass to execution
		const oauthTokens =
			Object.keys(oauthResult.tokens).length > 0
				? oauthResult.tokens
				: undefined;

		// Web mode always executes remotely
		return this.executeBoardRemote(
			appId,
			boardId,
			payload,
			streamState,
			eventId,
			cb,
			oauthTokens,
		);
	}

	async executeBoardRemote(
		appId: string,
		boardId: string,
		payload: IRunPayload,
		streamState?: boolean,
		eventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		oauthTokens?: Record<
			string,
			{
				access_token: string;
				refresh_token?: string;
				expires_at?: number;
				token_type?: string;
			}
		>,
	): Promise<ILogMetadata | undefined> {
		const baseUrl = getApiBaseUrl();
		const url = `${baseUrl}/api/v1/apps/${appId}/board/${boardId}/invoke`;

		const headers: HeadersInit = {
			"Content-Type": "application/json",
		};
		if (this.backend.auth?.user?.access_token) {
			headers["Authorization"] =
				`Bearer ${this.backend.auth.user.access_token}`;
		}

		console.log("[OAuth] Sending execution with tokens:", {
			hasOAuthTokens: !!oauthTokens,
			tokenProviders: oauthTokens ? Object.keys(oauthTokens) : [],
		});

		let executionFinished = false;
		try {
			const response = await fetch(url, {
				method: "POST",
				headers,
				body: JSON.stringify({
					node_id: payload.id,
					payload: payload.payload,
					stream_state: streamState ?? true,
					token: this.backend.auth?.user?.access_token,
					oauth_tokens: oauthTokens,
					runtime_variables: payload.runtime_variables,
					profile_id: this.backend.profile?.id,
				}),
			});

			if (!response.ok) {
				throw new Error(`Execution failed: ${response.status}`);
			}

			let foundRunId = false;

			if (response.body) {
				const reader = response.body.getReader();
				const decoder = new TextDecoder();
				let buffer = "";

				while (true) {
					const { done, value } = await reader.read();
					if (done) break;

					buffer += decoder.decode(value, { stream: true });

					// Parse SSE events properly - they're separated by double newlines
					const parts = buffer.split("\n\n");
					buffer = parts.pop() ?? "";

					for (const part of parts) {
						if (!part.trim()) continue;

						// Parse SSE format: "event: xxx\ndata: {...}"
						let eventName = "message";
						const dataLines: string[] = [];

						for (const line of part.split("\n")) {
							if (line.startsWith("event:")) {
								eventName = line.slice(6).trim();
							} else if (line.startsWith("data:")) {
								dataLines.push(line.slice(5).trim());
							} else if (line.startsWith(":")) {
								continue;
							}
						}

						const eventData = dataLines.join("\n");

						if (!eventData || eventData === "keep-alive") continue;

						try {
							const event = JSON.parse(eventData) as IIntercomEvent;

							// Handle run_initiated event to get run ID
							if (
								!foundRunId &&
								eventId &&
								event.event_type === "run_initiated"
							) {
								const runId = event.payload?.run_id;
								if (runId) {
									eventId(runId);
									foundRunId = true;
								}
							}

							// Handle toast events globally
							if (event.event_type === "toast") {
								handleToastEvent(event);
							}

							// Handle progress events globally
							if (event.event_type === "progress") {
								handleProgressEvent(event);
							}

							// Forward event to callback as array (consistent with local execution)
							if (cb) cb([event]);

							// Check for terminal events
							if (
								eventName === "done" ||
								eventName === "completed" ||
								event.event_type === "completed"
							) {
								executionFinished = true;
								finishAllProgressToasts(true);
								break;
							}
							if (event.event_type === "error") {
								executionFinished = true;
								finishAllProgressToasts(false);
								break;
							}
						} catch {
							// Ignore parse errors
						}
					}
				}
			}

			// Ensure progress toasts are finished when stream ends
			if (!executionFinished) {
				finishAllProgressToasts(true);
			}

			// Full metadata will be fetched separately by the caller
			return undefined;
		} catch (error) {
			finishAllProgressToasts(false);
			throw error;
		}
	}

	async listRuns(
		appId: string,
		boardId: string,
		nodeId?: string,
		from?: number,
		to?: number,
		status?: ILogLevel,
		lastMeta?: ILogMetadata,
		offset?: number,
		limit?: number,
	): Promise<ILogMetadata[]> {
		const params = new URLSearchParams();
		if (nodeId) params.set("node_id", nodeId);
		if (from !== undefined) params.set("from", from.toString());
		if (to !== undefined) params.set("to", to.toString());
		if (status) params.set("status", status);
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());

		const queryString = params.toString();
		try {
			const runs = await apiGet<ILogMetadata[]>(
				`apps/${appId}/board/${boardId}/runs${queryString ? `?${queryString}` : ""}`,
				this.backend.auth,
			);
			// Mark all runs as remote
			for (const run of runs) {
				run.is_remote = true;
			}
			return runs;
		} catch {
			return [];
		}
	}

	async queryRun(
		logMeta: ILogMetadata,
		query: string,
		offset?: number,
		limit?: number,
	): Promise<ILog[]> {
		const params = new URLSearchParams();
		params.set("run_id", logMeta.run_id);
		if (query) params.set("query", query);
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());

		try {
			return await apiGet<ILog[]>(
				`apps/${logMeta.app_id}/board/${logMeta.board_id}/logs?${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async undoBoard(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<void> {
		await apiPost(
			`apps/${appId}/board/${boardId}/undo`,
			{ commands },
			this.backend.auth,
		);
	}

	async redoBoard(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<void> {
		await apiPost(
			`apps/${appId}/board/${boardId}/redo`,
			{ commands },
			this.backend.auth,
		);
	}

	async upsertBoard(
		appId: string,
		boardId: string,
		name: string,
		description: string,
		logLevel: ILogLevel,
		stage: IExecutionStage,
		executionMode?: IExecutionMode,
		template?: IBoard,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/board/${boardId}`,
			{
				name,
				description,
				log_level: logLevel,
				stage,
				execution_mode: executionMode,
				template,
			},
			this.backend.auth,
		);
	}

	async closeBoard(boardId: string): Promise<void> {
		// No-op in web mode - we don't track open boards
	}

	async executeCommand(
		appId: string,
		boardId: string,
		command: IGenericCommand,
	): Promise<IGenericCommand> {
		const results = await apiPost<IGenericCommand[]>(
			`apps/${appId}/board/${boardId}`,
			{ commands: [command] },
			this.backend.auth,
		);
		return results[0];
	}

	async executeCommands(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	): Promise<IGenericCommand[]> {
		return apiPost<IGenericCommand[]>(
			`apps/${appId}/board/${boardId}`,
			{ commands },
			this.backend.auth,
		);
	}

	async getExecutionElements(
		appId: string,
		boardId: string,
		pageId: string,
		wildcard?: boolean,
	): Promise<Record<string, unknown>> {
		const params = new URLSearchParams();
		params.set("page_id", pageId);
		if (wildcard !== undefined) params.set("wildcard", String(wildcard));

		try {
			return await apiGet<Record<string, unknown>>(
				`apps/${appId}/board/${boardId}/elements?${params}`,
				this.backend.auth,
			);
		} catch {
			return {};
		}
	}

	async copilot_chat(
		scope: CopilotScope,
		board: IBoard | null,
		selectedNodeIds: string[],
		currentSurface: SurfaceComponent[] | null,
		selectedComponentIds: string[],
		userPrompt: string,
		history: UnifiedChatMessage[],
		onToken?: (token: string) => void,
		modelId?: string,
		token?: string,
		runContext?: IRunContext,
		actionContext?: UIActionContext,
	): Promise<UnifiedCopilotResponse> {
		const baseUrl = getApiBaseUrl();
		const url = `${baseUrl}/api/v1/ai/copilot/chat`;

		const headers: HeadersInit = {
			"Content-Type": "application/json",
		};
		if (this.backend.auth?.user?.access_token) {
			headers["Authorization"] =
				`Bearer ${this.backend.auth.user.access_token}`;
		}

		const response = await fetch(url, {
			method: "POST",
			headers,
			body: JSON.stringify({
				scope,
				board,
				selected_node_ids: selectedNodeIds,
				current_surface: currentSurface,
				selected_component_ids: selectedComponentIds,
				user_prompt: userPrompt,
				history,
				model_id: modelId,
				token,
				run_context: runContext,
				action_context: actionContext,
			}),
		});

		if (!response.ok) {
			throw new Error(`Copilot chat failed: ${response.status}`);
		}

		if (onToken && response.body) {
			const reader = response.body.getReader();
			const decoder = new TextDecoder();
			let buffer = "";
			let result: UnifiedCopilotResponse | undefined;

			while (true) {
				const { done, value } = await reader.read();
				if (done) break;

				buffer += decoder.decode(value, { stream: true });
				const lines = buffer.split("\n");
				buffer = lines.pop() ?? "";

				for (const line of lines) {
					if (line.startsWith("data: ")) {
						try {
							const data = JSON.parse(line.slice(6));
							if (data.token) {
								onToken(data.token);
							}
							if (data.result) {
								result = data.result;
							}
						} catch {
							// Ignore parse errors
						}
					}
				}
			}

			return (
				result ?? {
					message: "",
					commands: [],
					components: [],
					suggestions: [],
					active_scope: "Board" as const,
				}
			);
		}

		return response.json();
	}

	async prerunBoard(
		appId: string,
		boardId: string,
		version?: [number, number, number],
	): Promise<IPrerunBoardResponse> {
		const params = version ? `?version=${version.join(".")}` : "";
		return apiGet<IPrerunBoardResponse>(
			`apps/${appId}/board/${boardId}/prerun${params}`,
			this.backend.auth,
		);
	}
}
